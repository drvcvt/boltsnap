//! Protocol state machine. Owns registry, seats, data devices, offers and targets.
mod protocol;
mod seat;
#[cfg(test)]
mod tests;
mod transfers;
use super::{
    Action, Actions, DndEvent, DndLimits, DndOptions, DragId, LocalRect, Outcome, SeatId, TargetId,
    TargetSpec, TransferId,
    drag::Drag,
    interop::{self, PayloadKind},
    transfer::Reactor,
};
use crate::{Error, Result, SurfaceHandle, TransferError};
use seat::{release_pointer, release_touch};
use std::{
    collections::{BTreeMap, VecDeque},
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
    /// Hyprland sends the only `action` it ever will before `enter`.
    action: Action,
}
/// The offer currently hovering over one of our surfaces.
pub(crate) struct Offer {
    pub wl: wl_data_offer::WlDataOffer,
    pub mimes: Vec<String>,
    pub source_actions: Actions,
    pub action: Action,
    /// The compositor fixed the action before `enter` (Hyprland).
    pub fixed_action: bool,
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
    /// The `enter` serial when the compositor fixed the action before it (Hyprland). Such a
    /// compositor keeps the drag, and discards every click, until the offer is finished;
    /// finishing after accepting nothing is its abort.
    abort_serial: Option<u32>,
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
    /// Pointer buttons currently held on our surfaces.
    pub buttons: Vec<u32>,
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
            abort_serial: offer.fixed_action.then_some(offer.serial),
            fallbacks,
            started,
        };
        self.drops.insert(id, drop);
    }
    /// Ask the source for `mime` and read it through the reactor under transfer `id`.
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
        if outcome == Outcome::Rejected {
            abort(&d);
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
/// Ends a refused drop's drag on a compositor that needs a finish for it; see `abort_serial`.
fn abort(d: &Drop) {
    if let Some(serial) = d.abort_serial {
        d.wl.accept(serial, None);
        d.wl.finish();
    }
}
