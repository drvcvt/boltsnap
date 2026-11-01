//! Synthetic compositor DnD protocol requests and destruction ledger.
use super::*;

impl GlobalDispatch<wl_compositor::WlCompositor, DndGlobal> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_compositor::WlCompositor>,
        _: &DndGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(r, ());
    }
}
impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        r: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_compositor::Request::CreateSurface { id } => {
                let surface = init.init(id, ());
                s.metrics.dnd.surfaces.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().surfaces.push(surface);
            }
            wl_compositor::Request::CreateRegion { id } => {
                init.init(id, ());
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_surface::WlSurface,
        _: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, surface: &wl_surface::WlSurface, _: &()) {
        s.metrics.dnd.surfaces.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut() {
            d.surfaces.retain(|x| x != surface);
        }
    }
}
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_region::WlRegion,
        _: wl_region::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<wl_seat::WlSeat, DndGlobal> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_seat::WlSeat>,
        _: &DndGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        let seat = init.init(r, DndGlobal);
        seat.capabilities(wl_seat::Capability::Pointer | wl_seat::Capability::Touch);
        if seat.version() >= 2 {
            seat.name("dnd-seat".into());
        }
    }
}
impl Dispatch<wl_seat::WlSeat, DndGlobal> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        r: wl_seat::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_seat::Request::GetPointer { id } => {
                let p = init.init(id, DndGlobal);
                s.metrics.dnd.pointers.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().pointers.push(p);
            }
            wl_seat::Request::GetTouch { id } => {
                init.init(id, DndGlobal);
            }
            wl_seat::Request::GetKeyboard { id } => {
                init.init(id, DndGlobal);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, DndGlobal> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_pointer::WlPointer,
        _: wl_pointer::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, p: &wl_pointer::WlPointer, _: &DndGlobal) {
        s.metrics.dnd.pointers.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut() {
            d.pointers.retain(|x| x != p);
        }
    }
}
impl Dispatch<wl_touch::WlTouch, DndGlobal> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_touch::WlTouch,
        _: wl_touch::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, DndGlobal> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_keyboard::WlKeyboard,
        _: wl_keyboard::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<wl_data_device_manager::WlDataDeviceManager, DndGlobal> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_data_device_manager::WlDataDeviceManager>,
        _: &DndGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(r, ());
    }
}
impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_device_manager::WlDataDeviceManager,
        r: wl_data_device_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_device_manager::Request::GetDataDevice { id, .. } => {
                let device = init.init(id, ());
                s.metrics.dnd.devices.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().devices.push(device);
            }
            wl_data_device_manager::Request::CreateDataSource { id } => {
                let source = init.init(id, ());
                s.metrics.dnd.sources_alive.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().source = Some(source);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_device::WlDataDevice,
        r: wl_data_device::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_data_device::Request::StartDrag { serial, icon, .. } = r {
            s.metrics
                .dnd
                .start_drags
                .lock()
                .unwrap()
                .push((serial, icon.is_some()));
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, device: &wl_data_device::WlDataDevice, _: &()) {
        s.metrics.dnd.devices.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut() {
            d.devices.retain(|x| x != device);
        }
    }
}
impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_source::WlDataSource,
        r: wl_data_source::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_source::Request::Offer { mime_type } => {
                s.metrics.dnd.source_mimes.lock().unwrap().push(mime_type);
            }
            wl_data_source::Request::SetActions {
                dnd_actions: WEnum::Value(a),
            } => {
                s.metrics.dnd.actions.lock().unwrap().push((a.bits(), 0));
            }
            _ => {}
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, source: &wl_data_source::WlDataSource, _: &()) {
        s.metrics.dnd.sources_alive.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut().filter(|d| d.source.as_ref() == Some(source)) {
            d.source = None;
        }
    }
}
impl Dispatch<wl_data_offer::WlDataOffer, OfferData> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        offer: &wl_data_offer::WlDataOffer,
        r: wl_data_offer::Request,
        data: &OfferData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_offer::Request::Accept { mime_type, .. } => {
                data.accepted.store(mime_type.is_some(), Ordering::SeqCst);
                s.metrics.dnd.accepts.lock().unwrap().push(mime_type);
            }
            wl_data_offer::Request::SetActions {
                dnd_actions: WEnum::Value(a),
                preferred_action: WEnum::Value(p),
            } => {
                s.metrics
                    .dnd
                    .actions
                    .lock()
                    .unwrap()
                    .push((a.bits(), p.bits()));
                // Weston: the preferred action must be a single one out of the set.
                if p.bits().count_ones() > 1 || (p.bits() != 0 && a.bits() & p.bits() == 0) {
                    s.metrics.dnd.protocol_errors.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                if s.dnd.as_ref().unwrap().config.hyprland_actions {
                    return;
                }
                let chosen = DndServer::action_for(a.bits(), p.bits(), data.source_actions);
                data.current.store(chosen, Ordering::SeqCst);
                offer.action(DndAction::from_bits_truncate(chosen));
            }
            wl_data_offer::Request::Receive { mime_type, fd } => {
                receive(s, data, mime_type, fd);
            }
            wl_data_offer::Request::Finish => {
                // Weston: finishing with none or ask is INVALID_OFFER.
                if !matches!(data.current.load(Ordering::SeqCst), 1 | 2) {
                    s.metrics.dnd.protocol_errors.fetch_add(1, Ordering::SeqCst);
                }
                // Hyprland: a finish without an accepted type aborts the drag.
                let hyprland = s.dnd.as_ref().unwrap().config.hyprland_actions;
                if hyprland && !data.accepted.load(Ordering::SeqCst) {
                    s.metrics.dnd.aborts.fetch_add(1, Ordering::SeqCst);
                }
                s.metrics.dnd.finishes.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, offer: &wl_data_offer::WlDataOffer, _: &OfferData) {
        s.metrics.dnd.offers_alive.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut().filter(|d| d.offer.as_ref() == Some(offer)) {
            d.offer = None;
        }
    }
}

fn receive(s: &mut State, data: &OfferData, mime_type: String, fd: OwnedFd) {
    s.metrics
        .dnd
        .receives
        .lock()
        .unwrap()
        .push(mime_type.clone());
    s.metrics.dnd.pipes.lock().unwrap().push(inode(&fd));
    let config = s.dnd.as_ref().unwrap().config.clone();
    if let Some(tx) = &config.controlled_pipes {
        let _ = tx.send(PipePeer::Incoming {
            mime: mime_type,
            write: fd,
        });
        return;
    }
    let bytes = data
        .data
        .iter()
        .find(|(m, _)| *m == mime_type)
        .map(|(_, b)| b.clone());
    thread::spawn(move || {
        let mut file = std::fs::File::from(fd);
        for chunk in bytes.as_deref().unwrap_or_default().chunks(4096) {
            if !config.writer_delay.is_zero() {
                thread::sleep(config.writer_delay);
            }
            if file.write_all(chunk).is_err() {
                return;
            }
        }
        if config.hold_writer_open {
            thread::sleep(Duration::from_secs(30));
        }
    });
}
