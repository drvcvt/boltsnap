//! Protocol state machine. Owns registry, seats, data devices, offers and targets.
use super::{
    Action, Actions, DndEvent, DndLimits, DndOptions, DragId, LocalRect, Outcome, Payload, SeatId,
    TargetId, TargetSpec, TransferId,
    drag::Drag,
    interop::{self, PayloadKind},
    transfer::{self, Done, Reactor},
    uri,
};
use crate::{Error, Result, SurfaceHandle, TransferError};
use std::{
    collections::{BTreeMap, VecDeque},
    os::fd::AsFd,
    time::Instant,
};
use wayland_client::{
    Connection as WlConnection, Dispatch, Proxy, QueueHandle, WEnum,
    backend::ObjectId,
    delegate_noop, event_created_child,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager,
        wl_data_offer, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch,
    },
};
use wl_data_device_manager::DndAction;

const MAX_SEATS: usize = 16;
/// Offers announced by `data_offer` but not yet claimed by `enter`/`selection`.
const MAX_ANNOUNCED: usize = 8;

pub(crate) struct Target {
    pub surface: ObjectId,
    pub spec: TargetSpec,
}
/// Topmost enabled target under the point; ties in priority go to the newest registration.
pub(crate) fn hit_in<'a>(
    targets: impl Iterator<Item = (&'a TargetId, &'a Target)>,
    surface: &ObjectId,
    x: f64,
    y: f64,
) -> Option<TargetId> {
    targets
        .filter(|(_, t)| t.spec.enabled && &t.surface == surface && t.spec.rect.contains(x, y))
        .max_by_key(|(id, t)| (t.spec.priority, **id))
        .map(|(id, _)| *id)
}
struct Announced {
    wl: wl_data_offer::WlDataOffer,
    mimes: Vec<String>,
    source_actions: Actions,
}
/// The offer currently hovering over one of our surfaces.
pub(crate) struct Offer {
    pub wl: wl_data_offer::WlDataOffer,
    pub mimes: Vec<String>,
    pub source_actions: Actions,
    pub action: Action,
    pub serial: u32,
    pub surface: ObjectId,
    pub x: f64,
    pub y: f64,
    /// Effective target: the hit target, if it accepts one of the offered types.
    pub target: Option<TargetId>,
    pub chosen: Option<(String, PayloadKind)>,
}
/// A dropped offer: its data is being read, then it waits for [`Engine::complete`].
pub(crate) struct Drop {
    seat: SeatId,
    wl: wl_data_offer::WlDataOffer,
    target: TargetId,
    mime: String,
    kind: PayloadKind,
    action: Action,
    received: bool,
    source_actions: Actions,
    /// Further accepted types of the same offer, tried when the current one fails to decode.
    fallbacks: VecDeque<(String, PayloadKind)>,
    /// When the first read began; fallbacks share its total deadline.
    started: Instant,
}
pub(crate) struct Seat {
    pub id: SeatId,
    pub wl: wl_seat::WlSeat,
    pub device: Option<wl_data_device::WlDataDevice>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub touch: Option<wl_touch::WlTouch>,
    /// Serial of the press that began the current implicit grab; `None` once every button
    /// and touch point is up, so a drag never starts with a stale serial.
    pub grab_serial: Option<u32>,
    buttons: Vec<u32>,
    touches: Vec<i32>,
    pub offer: Option<Offer>,
}
pub(crate) struct Engine {
    pub id: u64,
    pub conn: WlConnection,
    pub limits: DndLimits,
    pub track_input: bool,
    pub qh: QueueHandle<Engine>,
    pub ready: bool,
    /// Requests were sent since the last `wl_display.sync`; `Ready` waits for another round
    /// so the compositor knows our devices and pointers before the consumer acts.
    bound: bool,
    pub manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub shm: Option<wl_shm::WlShm>,
    pub drags: BTreeMap<DragId, Drag>,
    pub seats: BTreeMap<u32, Seat>,
    announced: VecDeque<Announced>,
    pub targets: BTreeMap<TargetId, Target>,
    drops: BTreeMap<TransferId, Drop>,
    pub reactor: Reactor,
    next_id: u64,
    pub events: VecDeque<DndEvent>,
    /// The reader already woke the consumer for the current batch.
    pub wake_armed: bool,
    pub fatal: Option<Error>,
    /// The event queue overflowed; the session stays failed.
    pub overflowed: bool,
    /// `shutdown` ran; late registry events must not bind anything again.
    shut: bool,
}
impl Engine {
    pub fn new(conn: &WlConnection, qh: QueueHandle<Engine>, id: u64, options: DndOptions) -> Self {
        conn.display().get_registry(&qh, ());
        conn.display().sync(&qh, ());
        Self {
            id,
            conn: conn.clone(),
            limits: options.limits,
            track_input: options.track_input,
            qh,
            ready: false,
            bound: false,
            manager: None,
            compositor: None,
            shm: None,
            drags: BTreeMap::new(),
            seats: BTreeMap::new(),
            announced: VecDeque::new(),
            targets: BTreeMap::new(),
            drops: BTreeMap::new(),
            reactor: Reactor::default(),
            next_id: 0,
            events: VecDeque::new(),
            wake_armed: false,
            fatal: None,
            overflowed: false,
            shut: false,
        }
    }
    pub fn next(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    pub fn push(&mut self, event: DndEvent) {
        if let DndEvent::Hover { seat, .. } = &event {
            for queued in self.events.iter_mut().rev() {
                match queued {
                    DndEvent::Hover { seat: s, .. } if s == seat => {
                        *queued = event;
                        return;
                    }
                    e if e.seat() == Some(*seat) => break,
                    _ => {}
                }
            }
        }
        if self.overflowed {
            return;
        }
        if self.events.len() >= self.limits.max_events {
            self.events
                .push_back(DndEvent::Failed("event queue overflow".into()));
            self.overflowed = true;
            return;
        }
        self.events.push_back(event);
    }
    pub fn register_target(&mut self, surface: ObjectId, spec: TargetSpec) -> Result<TargetId> {
        // Compositors raise a protocol error for a preferred action outside the set.
        if spec.actions.to_wire() == 0 {
            return Err(Error::InvalidInput("TargetSpec::actions is empty"));
        }
        if spec.preferred != Action::None && spec.actions.to_wire() & spec.preferred.to_wire() == 0
        {
            return Err(Error::InvalidInput(
                "TargetSpec::preferred is not in actions",
            ));
        }
        if self.targets.len() >= self.limits.max_targets {
            return Err(Error::LimitExceeded);
        }
        let id = TargetId(self.id, self.next());
        self.targets.insert(id, Target { surface, spec });
        self.rehover_all();
        Ok(id)
    }
    pub fn update_target(
        &mut self,
        id: TargetId,
        rect: Option<LocalRect>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let t = self.targets.get_mut(&id).ok_or(Error::UnknownTarget)?;
        if let Some(r) = rect {
            t.spec.rect = r;
        }
        if let Some(e) = enabled {
            t.spec.enabled = e;
        }
        self.rehover_all();
        Ok(())
    }
    pub fn unregister_target(&mut self, id: TargetId) -> Result<()> {
        self.targets.remove(&id).ok_or(Error::UnknownTarget)?;
        let running: Vec<TransferId> = (self.drops.iter())
            .filter(|(_, d)| d.target == id && !d.received)
            .map(|(t, _)| *t)
            .collect();
        for transfer in running {
            self.fail_drop(transfer, TransferError::TargetGone);
        }
        self.rehover_all();
        Ok(())
    }
    fn rehover_all(&mut self) {
        let names: Vec<u32> = self.seats.keys().copied().collect();
        for name in names {
            self.hover(name);
        }
    }
    /// Re-run target selection for the seat's hover offer; negotiate only on change.
    fn hover(&mut self, name: u32) {
        let Some(seat) = self.seats.get_mut(&name) else {
            return;
        };
        let Some(offer) = seat.offer.as_mut() else {
            return;
        };
        let hit = hit_in(self.targets.iter(), &offer.surface, offer.x, offer.y);
        let chosen =
            hit.and_then(|t| interop::choose_mime(&offer.mimes, &self.targets[&t].spec.accepts));
        let target = hit.filter(|_| chosen.is_some());
        if target != offer.target {
            offer.target = target;
            let v3 = offer.wl.version() >= 3;
            match (target, &chosen) {
                (Some(t), Some((mime, _))) => {
                    let spec = &self.targets[&t].spec;
                    offer.wl.accept(offer.serial, Some(mime.clone()));
                    if v3 {
                        offer.wl.set_actions(
                            DndAction::from_bits_truncate(spec.actions.to_wire()),
                            DndAction::from_bits_truncate(spec.preferred.to_wire()),
                        );
                    }
                }
                _ => {
                    offer.wl.accept(offer.serial, None);
                    if v3 {
                        offer.wl.set_actions(DndAction::empty(), DndAction::empty());
                    }
                }
            }
            offer.chosen = chosen;
        }
        let event = DndEvent::Hover {
            seat: seat.id,
            target: offer.target,
            x: offer.x,
            y: offer.y,
            action: offer.action,
        };
        self.push(event);
    }
    /// Destroy the hover offer of a seat and tell the consumer the drag left.
    fn leave(&mut self, name: u32) {
        let Some(seat) = self.seats.get_mut(&name) else {
            return;
        };
        if let Some(offer) = seat.offer.take() {
            offer.wl.destroy();
            let seat = seat.id;
            self.push(DndEvent::Leave { seat });
        }
    }
    /// Move the hover offer into `drops` and start reading the chosen type.
    fn drop_received(&mut self, name: u32) {
        let Some(seat) = self.seats.get_mut(&name) else {
            return;
        };
        let seat_id = seat.id;
        let Some(offer) = seat.offer.take() else {
            return;
        };
        self.push(DndEvent::Leave { seat: seat_id });
        let Some(target) = offer.target.filter(|_| offer.chosen.is_some()) else {
            offer.wl.destroy();
            return;
        };
        let accepts = &self.targets[&target].spec.accepts;
        let mut fallbacks: VecDeque<_> = interop::choices(&offer.mimes, accepts).into();
        let (mime, kind) = fallbacks.pop_front().expect("chosen implies a choice");
        if self.drops.len() >= self.limits.max_transfers {
            offer.wl.destroy();
            let reason = TransferError::Aborted;
            self.push(DndEvent::TransferFailed {
                seat: seat_id,
                target,
                reason,
            });
            return;
        }
        let id = TransferId(self.id, self.next());
        let started = Instant::now();
        if let Err(reason) = self.receive(id, &offer.wl, &mime, started) {
            offer.wl.destroy();
            self.push(DndEvent::TransferFailed {
                seat: seat_id,
                target,
                reason,
            });
            return;
        }
        let drop = Drop {
            seat: seat_id,
            wl: offer.wl,
            target,
            mime,
            kind,
            action: offer.action,
            received: false,
            source_actions: offer.source_actions,
            fallbacks,
            started,
        };
        self.drops.insert(id, drop);
    }
    /// Ask the source for `mime` and read it through the reactor under transfer `id`.
    fn receive(
        &mut self,
        id: TransferId,
        wl: &wl_data_offer::WlDataOffer,
        mime: &str,
        started: Instant,
    ) -> std::result::Result<(), TransferError> {
        let (read, write) = transfer::open_pipe().map_err(|_| TransferError::Io)?;
        if started.elapsed() >= self.limits.total {
            return Err(TransferError::Timeout);
        }
        wl.receive(mime.into(), write.as_fd());
        drop(write);
        self.reactor
            .push_incoming(id, read, started, Instant::now());
        Ok(())
    }
    fn fail_drop(&mut self, id: TransferId, reason: TransferError) {
        let Some(d) = self.drops.remove(&id) else {
            return;
        };
        self.reactor.cancel_incoming(id);
        d.wl.destroy();
        let (seat, target) = (d.seat, d.target);
        self.push(DndEvent::TransferFailed {
            seat,
            target,
            reason,
        });
    }
    /// Service pipes and turn finished reads into typed payloads.
    pub fn service_transfers(&mut self, now: Instant) {
        for done in self.reactor.service(now, &self.limits) {
            let Done::Incoming { id, result } = done else {
                continue;
            };
            let Some(d) = self.drops.get(&id) else {
                continue;
            };
            let payload = result.and_then(|bytes| match d.kind {
                PayloadKind::Files => {
                    uri::parse_uri_list(&bytes, self.limits.max_entries).map(Payload::Files)
                }
                PayloadKind::Utf8Text | PayloadKind::Latin1Text => {
                    uri::decode_text(&bytes, d.kind).map(Payload::Text)
                }
                PayloadKind::Bytes => Ok(Payload::Bytes {
                    mime: d.mime.clone(),
                    data: bytes,
                }),
            });
            let payload = if d.started.elapsed() >= self.limits.total {
                Err(TransferError::Timeout)
            } else {
                payload
            };
            match payload {
                Ok(payload) => {
                    let d = self.drops.get_mut(&id).unwrap();
                    d.received = true;
                    let (seat, target, action) = (d.seat, d.target, d.action);
                    self.push(DndEvent::Dropped {
                        seat,
                        transfer: id,
                        target,
                        action,
                        payload,
                    });
                }
                // A browser's uri-list holds https links: take the offer's text instead.
                Err(TransferError::Malformed) if !self.drops[&id].fallbacks.is_empty() => {
                    if self.drops[&id].started.elapsed() >= self.limits.total {
                        self.fail_drop(id, TransferError::Timeout);
                        continue;
                    }
                    let d = self.drops.get_mut(&id).unwrap();
                    let (mime, kind) = d.fallbacks.pop_front().unwrap();
                    (d.mime, d.kind) = (mime.clone(), kind);
                    let (wl, started) = (d.wl.clone(), d.started);
                    if let Err(reason) = self.receive(id, &wl, &mime, started) {
                        self.fail_drop(id, reason);
                    }
                }
                Err(reason) => self.fail_drop(id, reason),
            }
        }
    }
    /// Answer a delivered drop. `finish` is sent only on v3 with a copy or move final action;
    /// an `Ask` drop takes the consumer's choice, which the source must offer.
    pub fn complete(&mut self, id: TransferId, outcome: Outcome) -> Result<()> {
        let d = self.drops.get(&id).filter(|d| d.received);
        let d = d.ok_or(Error::UnknownTransfer)?;
        let ask = d.action == Action::Ask && d.wl.version() >= 3;
        if let Outcome::Accepted(chosen) = outcome {
            let valid = if d.wl.version() < 3 {
                chosen != Action::Ask
            } else if ask {
                matches!(chosen, Action::Copy | Action::Move)
                    && d.source_actions.to_wire() & chosen.to_wire() != 0
            } else {
                matches!(chosen, Action::Copy | Action::Move) && chosen == d.action
            };
            if !valid {
                return Err(Error::InvalidInput(
                    "completion must match the negotiated action; Ask needs an offered copy or move",
                ));
            }
        }
        let d = self.drops.remove(&id).unwrap();
        if let Outcome::Accepted(chosen) = outcome
            && d.wl.version() >= 3
        {
            let action = if ask {
                let bits = DndAction::from_bits_truncate(chosen.to_wire());
                d.wl.set_actions(bits, bits);
                chosen
            } else {
                d.action
            };
            if matches!(action, Action::Copy | Action::Move) {
                d.wl.finish();
            }
        }
        d.wl.destroy();
        Ok(())
    }
    fn bind_seat_objects(&mut self, name: u32, caps: Option<wl_seat::Capability>) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let (qh, track) = (self.qh.clone(), self.track_input);
        let Some(seat) = self.seats.get_mut(&name) else {
            return;
        };
        if seat.device.is_none() {
            seat.device = Some(manager.get_data_device(&seat.wl, &qh, name));
            self.bound = true;
        }
        let Some(caps) = caps.filter(|_| track) else {
            return;
        };
        let pointer = caps.contains(wl_seat::Capability::Pointer);
        match (&seat.pointer, pointer) {
            (None, true) => {
                seat.pointer = Some(seat.wl.get_pointer(&qh, name));
                self.bound = true;
            }
            (Some(_), false) => {
                release_pointer(seat.pointer.take());
                seat.buttons.clear();
            }
            _ => {}
        }
        let touch = caps.contains(wl_seat::Capability::Touch);
        match (&seat.touch, touch) {
            (None, true) => {
                seat.touch = Some(seat.wl.get_touch(&qh, name));
                self.bound = true;
            }
            (Some(_), false) => {
                release_touch(seat.touch.take());
                seat.touches.clear();
            }
            _ => {}
        }
        if seat.buttons.is_empty() && seat.touches.is_empty() {
            seat.grab_serial = None;
        }
    }
    fn remove_seat(&mut self, name: u32) {
        self.leave(name);
        if let Some(seat) = self.seats.remove(&name) {
            release_pointer(seat.pointer);
            release_touch(seat.touch);
            if let Some(d) = seat.device.filter(|d| d.version() >= 2) {
                d.release();
            }
            if seat.wl.version() >= 5 {
                seat.wl.release();
            }
            self.push(DndEvent::SeatRemoved(seat.id));
        }
    }
    pub fn shutdown(&mut self) {
        self.shut = true;
        self.shutdown_drags();
        let names: Vec<u32> = self.seats.keys().copied().collect();
        for name in names {
            self.remove_seat(name);
        }
        for a in std::mem::take(&mut self.announced) {
            a.wl.destroy();
        }
        for (_, d) in std::mem::take(&mut self.drops) {
            d.wl.destroy();
        }
        self.reactor = Reactor::default();
        self.events.clear();
    }
    fn hover_offer(&mut self, id: &ObjectId) -> Option<(u32, &mut Offer)> {
        self.seats
            .iter_mut()
            .find_map(|(n, s)| Some((*n, s.offer.as_mut().filter(|o| &o.wl.id() == id)?)))
    }
}
impl Seat {
    /// Forget the current implicit grab; a new press starts the next one.
    pub fn end_grab(&mut self) {
        self.buttons.clear();
        self.touches.clear();
        self.grab_serial = None;
    }
}
fn release_pointer(p: Option<wl_pointer::WlPointer>) {
    if let Some(p) = p.filter(|p| p.version() >= 3) {
        p.release();
    }
}
fn release_touch(t: Option<wl_touch::WlTouch>) {
    if let Some(t) = t.filter(|t| t.version() >= 3) {
        t.release();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Engine {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &WlConnection,
        qh: &QueueHandle<Self>,
    ) {
        if s.shut {
            return;
        }
        match e {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_data_device_manager" if s.manager.is_none() => {
                    s.manager = Some(registry.bind(name, version.min(3), qh, ()));
                    let names: Vec<u32> = s.seats.keys().copied().collect();
                    for n in names {
                        s.bind_seat_objects(n, None);
                    }
                }
                "wl_compositor" if s.compositor.is_none() => {
                    s.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "wl_shm" if s.shm.is_none() => s.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" if s.seats.len() < MAX_SEATS => {
                    let wl = registry.bind(name, version.min(7), qh, name);
                    s.bound = true;
                    let id = SeatId(name);
                    s.seats.insert(
                        name,
                        Seat {
                            id,
                            wl,
                            device: None,
                            pointer: None,
                            touch: None,
                            grab_serial: None,
                            buttons: Vec::new(),
                            touches: Vec::new(),
                            offer: None,
                        },
                    );
                    s.bind_seat_objects(name, None);
                    if s.ready {
                        s.push(DndEvent::SeatAdded(id));
                    }
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => s.remove_seat(name),
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for Engine {
    fn event(
        s: &mut Self,
        _: &wl_callback::WlCallback,
        e: wl_callback::Event,
        _: &(),
        conn: &WlConnection,
        qh: &QueueHandle<Self>,
    ) {
        if !matches!(e, wl_callback::Event::Done { .. }) || s.ready {
            return;
        }
        if std::mem::take(&mut s.bound) {
            conn.display().sync(qh, ());
            return;
        }
        s.ready = true;
        let Some(manager) = &s.manager else {
            s.fatal = Some(Error::Unsupported("wl_data_device_manager"));
            s.push(DndEvent::Failed(
                "compositor offers no wl_data_device_manager".into(),
            ));
            return;
        };
        let version = manager.version();
        let seats = s.seats.values().map(|x| x.id).collect();
        s.push(DndEvent::Ready { version, seats });
    }
}
impl Dispatch<wl_seat::WlSeat, u32> for Engine {
    fn event(
        s: &mut Self,
        _: &wl_seat::WlSeat,
        e: wl_seat::Event,
        name: &u32,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = e
        {
            s.bind_seat_objects(*name, Some(caps));
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, u32> for Engine {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        e: wl_pointer::Event,
        name: &u32,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = s.seats.get_mut(name) else {
            return;
        };
        match e {
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(state),
                ..
            } => {
                seat.buttons.retain(|b| *b != button);
                if state == wl_pointer::ButtonState::Pressed {
                    seat.buttons.push(button);
                    seat.grab_serial = Some(serial);
                }
            }
            // The grab ended elsewhere (for instance our own drag started).
            wl_pointer::Event::Leave { .. } => seat.buttons.clear(),
            _ => return,
        }
        if seat.buttons.is_empty() && seat.touches.is_empty() {
            seat.grab_serial = None;
        }
    }
}
impl Dispatch<wl_touch::WlTouch, u32> for Engine {
    fn event(
        s: &mut Self,
        _: &wl_touch::WlTouch,
        e: wl_touch::Event,
        name: &u32,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seat) = s.seats.get_mut(name) else {
            return;
        };
        match e {
            wl_touch::Event::Down { serial, id, .. } => {
                seat.touches.push(id);
                seat.grab_serial = Some(serial);
            }
            wl_touch::Event::Up { id, .. } => seat.touches.retain(|t| *t != id),
            wl_touch::Event::Cancel => seat.touches.clear(),
            _ => return,
        }
        if seat.buttons.is_empty() && seat.touches.is_empty() {
            seat.grab_serial = None;
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, u32> for Engine {
    fn event(
        s: &mut Self,
        _: &wl_data_device::WlDataDevice,
        e: wl_data_device::Event,
        name: &u32,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_data_device::Event::DataOffer { id } => {
                if s.announced.len() >= MAX_ANNOUNCED
                    && let Some(stale) = s.announced.pop_front()
                {
                    stale.wl.destroy();
                }
                s.announced.push_back(Announced {
                    wl: id,
                    mimes: Vec::new(),
                    source_actions: Actions::default(),
                });
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id: Some(id),
            } => {
                let Some(at) = s.announced.iter().position(|a| a.wl == id) else {
                    return;
                };
                let announced = s.announced.remove(at).unwrap();
                s.leave(*name);
                let Some(seat) = s.seats.get_mut(name) else {
                    announced.wl.destroy();
                    return;
                };
                seat.offer = Some(Offer {
                    wl: announced.wl,
                    mimes: announced.mimes.clone(),
                    source_actions: announced.source_actions,
                    action: Action::None,
                    serial,
                    surface: surface.id(),
                    x,
                    y,
                    target: None,
                    chosen: None,
                });
                let seat = seat.id;
                s.push(DndEvent::Enter {
                    seat,
                    surface: SurfaceHandle::from_surface(&surface),
                    offered: announced.mimes,
                    source_actions: announced.source_actions,
                });
                s.hover(*name);
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                let Some(offer) = s.seats.get_mut(name).and_then(|s| s.offer.as_mut()) else {
                    return;
                };
                (offer.x, offer.y) = (x, y);
                s.hover(*name);
            }
            wl_data_device::Event::Leave => s.leave(*name),
            wl_data_device::Event::Drop => s.drop_received(*name),
            wl_data_device::Event::Selection { id: Some(id) } => {
                // Clipboard offers are not ours to keep.
                s.announced.retain(|a| a.wl != id);
                id.destroy();
            }
            _ => {}
        }
    }
    event_created_child!(Engine, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}
impl Dispatch<wl_data_offer::WlDataOffer, ()> for Engine {
    fn event(
        s: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        e: wl_data_offer::Event,
        _: &(),
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let max_mimes = s.limits.max_mime_types;
        let announced = s.announced.iter_mut().find(|a| &a.wl == offer);
        match e {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Some(a) = announced.filter(|a| a.mimes.len() < max_mimes) {
                    a.mimes.push(mime_type);
                }
            }
            wl_data_offer::Event::SourceActions {
                source_actions: WEnum::Value(bits),
            } => {
                let actions = Actions::from_wire(bits.bits());
                if let Some(a) = announced {
                    a.source_actions = actions;
                } else if let Some((_, o)) = s.hover_offer(&offer.id()) {
                    o.source_actions = actions;
                }
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(bits),
            } => {
                let action = Action::from_wire(bits.bits());
                if let Some((name, o)) = s.hover_offer(&offer.id()) {
                    o.action = action;
                    s.hover(name);
                } else if let Some(d) = s.drops.values_mut().find(|d| &d.wl == offer) {
                    d.action = action;
                }
            }
            _ => {}
        }
    }
}
delegate_noop!(Engine: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(Engine: ignore wl_compositor::WlCompositor);
delegate_noop!(Engine: ignore wl_surface::WlSurface);
delegate_noop!(Engine: ignore wl_shm::WlShm);
delegate_noop!(Engine: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Engine: ignore wl_buffer::WlBuffer);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dnd::Accept;
    fn target(x: f64, priority: i32, enabled: bool) -> Target {
        let rect = LocalRect {
            x,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        Target {
            surface: ObjectId::null(),
            spec: TargetSpec {
                rect,
                accepts: vec![Accept::Files],
                actions: Actions::default(),
                preferred: Action::Copy,
                priority,
                enabled,
            },
        }
    }
    #[test]
    fn hit_prefers_priority_then_recency_and_respects_enabled() {
        let mut targets = BTreeMap::new();
        targets.insert(TargetId(1, 1), target(0., 0, true));
        targets.insert(TargetId(1, 2), target(5., 0, true));
        targets.insert(TargetId(1, 3), target(5., -1, true));
        targets.insert(TargetId(1, 4), target(5., 9, false));
        let s = ObjectId::null();
        assert_eq!(hit_in(targets.iter(), &s, 1., 1.), Some(TargetId(1, 1)));
        assert_eq!(hit_in(targets.iter(), &s, 7., 1.), Some(TargetId(1, 2)));
        assert_eq!(hit_in(targets.iter(), &s, 12., 1.), Some(TargetId(1, 2)));
        assert_eq!(hit_in(targets.iter(), &s, 30., 1.), None);
    }
    #[test]
    fn actions_round_trip_through_wire_bits() {
        let a = Actions {
            copy: true,
            move_: false,
            ask: true,
        };
        assert_eq!(Actions::from_wire(a.to_wire()), a);
        assert_eq!(Action::from_wire(Action::Move.to_wire()), Action::Move);
        assert_eq!(Action::from_wire(8), Action::None);
    }
}
