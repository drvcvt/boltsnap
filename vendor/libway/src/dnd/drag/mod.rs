//! Outgoing drags: data source, icon surface and the send path.
mod origin;

use super::{
    Action, DndEvent, DragIcon, DragId, DragOutcome, SeatId,
    engine::Engine,
    representations::{Data, Request},
};
use crate::{Error, Result, buffer::ShmBuffer};
use std::time::Instant;
use wayland_client::{
    Connection as WlConnection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_buffer, wl_data_device_manager::DndAction, wl_data_source, wl_shm, wl_surface},
};

pub(crate) struct Icon {
    surface: wl_surface::WlSurface,
    buffer: wl_buffer::WlBuffer,
    _shm: ShmBuffer,
}
impl Icon {
    fn show(&self, (hx, hy): (i32, i32)) {
        let surface = &self.surface;
        if surface.version() >= 5 {
            surface.offset(-hx, -hy);
            surface.attach(Some(&self.buffer), 0, 0);
        } else {
            surface.attach(Some(&self.buffer), -hx, -hy);
        }
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        } else {
            surface.damage(0, 0, i32::MAX, i32::MAX);
        }
        surface.commit();
    }
    fn destroy(self) {
        self.buffer.destroy();
        self.surface.destroy();
    }
}
pub(crate) struct Drag {
    seat: SeatId,
    source: wl_data_source::WlDataSource,
    data: Data,
    icon: Option<Icon>,
    accepted: bool,
    action: Action,
    sent: bool,
    /// Buttons held when the drag started from a tracked press. Their release reaching
    /// us means the compositor ended the press before it saw the drag.
    held: Vec<u32>,
}

impl Engine {
    pub fn start_drag(&mut self, req: Request) -> Result<DragId> {
        if !self.ready {
            return Err(Error::NotReady);
        }
        req.origin.validate(&self.conn)?;
        let manager = self
            .manager
            .clone()
            .ok_or(Error::Unsupported("wl_data_device_manager"))?;
        let origin = origin::resolve(self, &req)?;
        let seat_id = origin.seat;
        let hotspot = req.icon.as_ref().map(|i| i.hotspot);
        let icon = req.icon.map(|i| self.make_icon(i)).transpose()?;
        let old: Vec<DragId> = (self.drags.iter())
            .filter(|(_, d)| d.seat == seat_id)
            .map(|(id, _)| *id)
            .collect();
        for id in old {
            self.end_drag(id, DragOutcome::Cancelled);
        }
        let id = DragId(self.id, self.next());
        let source = manager.create_data_source(&self.qh, id);
        for mime in req.data.mimes() {
            source.offer(mime.clone());
        }
        if source.version() >= 3 {
            source.set_actions(DndAction::from_bits_truncate(req.actions.to_wire()));
        }
        origin.device.start_drag(
            Some(&source),
            &origin.surface,
            icon.as_ref().map(|i| &i.surface),
            origin.serial,
        );
        // The drag owns the grab now. Compositors may never report the release or touch-up
        // that ends it, so the serial must not outlive this drag.
        self.seats.get_mut(&origin.name).unwrap().end_grab();
        if let Some((icon, hotspot)) = icon.as_ref().zip(hotspot) {
            icon.show(hotspot);
        }
        let drag = Drag {
            seat: seat_id,
            source,
            data: req.data,
            icon,
            accepted: false,
            action: Action::None,
            sent: false,
            held: origin.held,
        };
        self.drags.insert(id, drag);
        Ok(id)
    }
    fn make_icon(&self, icon: DragIcon) -> Result<Icon> {
        let compositor = (self.compositor.clone()).ok_or(Error::Unsupported("wl_compositor"))?;
        let shm = self.shm.clone().ok_or(Error::Unsupported("wl_shm"))?;
        let (w, h) = (icon.width, icon.height);
        let bytes = (w * h * 4) as usize;
        let mut storage = ShmBuffer::new(bytes)?;
        // Premultiplied RGBA8 to little-endian ARGB8888 as wl_shm expects it.
        for (dst, src) in storage
            .map
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(icon.rgba_premultiplied.as_chunks::<4>().0)
        {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        let format = wl_shm::Format::Argb8888;
        let buffer = crate::buffer::create_wl_shm(&shm, &self.qh, &storage, w, h, w * 4, format);
        let surface = compositor.create_surface(&self.qh, ());
        Ok(Icon {
            surface,
            buffer,
            _shm: storage,
        })
    }
    pub fn cancel_drag(&mut self, id: DragId) -> Result<()> {
        if !self.drags.contains_key(&id) {
            return Err(Error::UnknownTransfer);
        }
        self.end_drag(id, DragOutcome::Cancelled);
        Ok(())
    }
    /// Destroy source and icon. Sends already in the reactor still complete: they own their
    /// bytes and fds, and the receiver may still be reading.
    fn end_drag(&mut self, id: DragId, outcome: DragOutcome) {
        let Some(drag) = self.drags.remove(&id) else {
            return;
        };
        drag.source.destroy();
        if let Some(icon) = drag.icon {
            icon.destroy();
        }
        self.push(DndEvent::DragEnded { drag: id, outcome });
    }
    /// A button of `seat` changed. A new press ends the hold that started a drag; the release
    /// of the starting press, with no new press in between, can only reach us if the
    /// compositor had already ended that press when `start_drag` arrived. Hyprland then runs a
    /// drag nobody holds and discards every click until the next release, so cancel it.
    pub(crate) fn drag_button(&mut self, seat: SeatId, button: u32, pressed: bool) {
        let mut dead = Vec::new();
        for (id, d) in self.drags.iter_mut().filter(|(_, d)| d.seat == seat) {
            let held = d.held.len();
            d.held.retain(|b| *b != button);
            if !pressed && held > 0 && d.held.is_empty() {
                dead.push(*id);
            }
        }
        for id in dead {
            self.end_drag(id, DragOutcome::Cancelled);
        }
    }
    pub fn shutdown_drags(&mut self) {
        let ids: Vec<DragId> = self.drags.keys().copied().collect();
        for id in ids {
            self.end_drag(id, DragOutcome::Cancelled);
        }
    }
}

pub(super) fn validate_icon(icon: &DragIcon) -> Result<()> {
    let (w, h) = (icon.width, icon.height);
    if w == 0
        || h == 0
        || w > 1024
        || h > 1024
        || icon.rgba_premultiplied.len() != (w * h * 4) as usize
    {
        return Err(Error::InvalidDimensions);
    }
    let (x, y) = icon.hotspot;
    if x < 0 || y < 0 || x as u32 >= w || y as u32 >= h {
        return Err(Error::InvalidInput("icon hotspot must be inside the image"));
    }
    Ok(())
}

impl Dispatch<wl_data_source::WlDataSource, DragId> for Engine {
    fn event(
        s: &mut Self,
        source: &wl_data_source::WlDataSource,
        e: wl_data_source::Event,
        id: &DragId,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let max = s.limits.max_transfers;
        let Some(drag) = s.drags.get_mut(id) else {
            return;
        };
        match e {
            wl_data_source::Event::Target { mime_type } => {
                drag.accepted = mime_type.is_some();
                let (accepted, action) = (drag.accepted, drag.action);
                s.push(DndEvent::DragFeedback {
                    drag: *id,
                    accepted,
                    action,
                });
            }
            wl_data_source::Event::Action {
                dnd_action: WEnum::Value(bits),
            } => {
                drag.action = Action::from_wire(bits.bits());
                let (accepted, action) = (drag.accepted, drag.action);
                s.push(DndEvent::DragFeedback {
                    drag: *id,
                    accepted,
                    action,
                });
            }
            wl_data_source::Event::Send { mime_type, fd } => {
                if !drag.data.bounded() {
                    drag.sent = true;
                }
                // Unknown types and overload close the pipe at once: the reader sees EOF.
                if s.reactor.outgoing() >= max {
                    return;
                }
                if let Some(bytes) = drag.data.send(&mime_type) {
                    drag.sent = true;
                    s.reactor.push_outgoing(fd, bytes, Instant::now());
                }
            }
            wl_data_source::Event::DndFinished => {
                let outcome = if !drag.data.bounded() || drag.sent {
                    DragOutcome::Dropped(drag.action)
                } else {
                    DragOutcome::Cancelled
                };
                s.end_drag(*id, outcome);
            }
            wl_data_source::Event::Cancelled => {
                // v1/v2 have no dnd_finished: a drop that fetched data ends with cancelled.
                let dropped = source.version() < 3 && drag.sent;
                let outcome = if dropped {
                    DragOutcome::Dropped(Action::None)
                } else {
                    DragOutcome::Cancelled
                };
                s.end_drag(*id, outcome);
            }
            _ => {}
        }
    }
}
