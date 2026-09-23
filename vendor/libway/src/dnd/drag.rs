//! Outgoing drags: data source, icon surface and the send path.
use super::{
    Action, DndEvent, DragData, DragIcon, DragId, DragOutcome, DragRequest, SeatId, engine::Engine,
    interop,
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
    fn destroy(self) {
        self.buffer.destroy();
        self.surface.destroy();
    }
}
pub(crate) struct Drag {
    seat: SeatId,
    source: wl_data_source::WlDataSource,
    data: DragData,
    icon: Option<Icon>,
    accepted: bool,
    action: Action,
    sent: bool,
}

impl Engine {
    pub fn start_drag(&mut self, req: DragRequest) -> Result<DragId> {
        if !self.ready {
            return Err(Error::NotReady);
        }
        req.origin.validate(&self.conn)?;
        let mimes = interop::source_mimes(&req.data);
        if mimes.len() > self.limits.max_mime_types {
            return Err(Error::LimitExceeded);
        }
        for mime in &mimes {
            if mime.is_empty() || mime.contains('\0') {
                return Err(Error::InvalidInput(
                    "MIME names must be nonempty and NUL-free",
                ));
            }
            if mime.len() > 1024 {
                return Err(Error::LimitExceeded);
            }
        }
        if let Some(icon) = &req.icon {
            validate_icon(icon)?;
        }
        if let DragData::Files(paths) | DragData::FilesAndText { paths, .. } = &req.data
            && paths.iter().any(|p| !p.is_absolute())
        {
            return Err(Error::InvalidInput("dragged files need absolute paths"));
        }
        let manager = (self.manager.clone()).ok_or(Error::Unsupported("wl_data_device_manager"))?;
        let name = match req.seat {
            Some(SeatId(n)) => n,
            None => *(self.seats.iter())
                .filter(|(_, s)| req.serial.is_some() || s.grab_serial.is_some())
                .map(|(n, _)| n)
                .next_back()
                .ok_or(Error::InvalidSerial)?,
        };
        let seat = self.seats.get(&name).ok_or(Error::InvalidSerial)?;
        let serial = req
            .serial
            .or(seat.grab_serial)
            .ok_or(Error::InvalidSerial)?;
        let device = seat.device.clone().ok_or(Error::NotReady)?;
        let seat_id = seat.id;
        let origin = wl_surface::WlSurface::from_id(&self.conn, req.origin.id().clone())
            .map_err(|_| Error::Wayland("origin surface is gone".into()))?;
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
        for mime in mimes {
            source.offer(mime);
        }
        if source.version() >= 3 {
            source.set_actions(DndAction::from_bits_truncate(req.actions.to_wire()));
        }
        device.start_drag(
            Some(&source),
            &origin,
            icon.as_ref().map(|i| &i.surface),
            serial,
        );
        // The drag owns the grab now. Compositors may never report the release or touch-up
        // that ends it, so the serial must not outlive this drag.
        self.seats.get_mut(&name).unwrap().end_grab();
        if let Some((icon, (hx, hy))) = icon.as_ref().zip(hotspot) {
            let s = &icon.surface;
            if s.version() >= 5 {
                s.offset(-hx, -hy);
                s.attach(Some(&icon.buffer), 0, 0);
            } else {
                s.attach(Some(&icon.buffer), -hx, -hy);
            }
            if s.version() >= 4 {
                s.damage_buffer(0, 0, i32::MAX, i32::MAX);
            } else {
                s.damage(0, 0, i32::MAX, i32::MAX);
            }
            s.commit();
        }
        let drag = Drag {
            seat: seat_id,
            source,
            data: req.data,
            icon,
            accepted: false,
            action: Action::None,
            sent: false,
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
        for (dst, src) in
            (storage.map.chunks_exact_mut(4)).zip(icon.rgba_premultiplied.chunks_exact(4))
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
    pub fn shutdown_drags(&mut self) {
        let ids: Vec<DragId> = self.drags.keys().copied().collect();
        for id in ids {
            self.end_drag(id, DragOutcome::Cancelled);
        }
    }
}

fn validate_icon(icon: &DragIcon) -> Result<()> {
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
                drag.sent = true;
                // Unknown types and overload close the pipe at once: the reader sees EOF.
                if s.reactor.outgoing() >= max {
                    return;
                }
                if let Some(bytes) = interop::materialize(&mut drag.data, &mime_type) {
                    s.reactor.push_outgoing(fd, bytes, Instant::now());
                }
            }
            wl_data_source::Event::DndFinished => {
                let outcome = DragOutcome::Dropped(drag.action);
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
