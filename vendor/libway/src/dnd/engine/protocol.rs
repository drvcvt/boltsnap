//! Wayland registry, input and data-device event dispatch.
use super::*;

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
                let pressed = state == wl_pointer::ButtonState::Pressed;
                seat.buttons.retain(|b| *b != button);
                if pressed {
                    seat.buttons.push(button);
                    seat.grab_serial = Some(serial);
                }
                if seat.buttons.is_empty() && seat.touches.is_empty() {
                    seat.grab_serial = None;
                }
                let seat = seat.id;
                s.drag_button(seat, button, pressed);
                return;
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
                    action: Action::None,
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
                    action: announced.action,
                    fixed_action: announced.action != Action::None,
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
                if let Some(a) = announced {
                    a.action = action;
                } else if let Some((name, o)) = s.hover_offer(&offer.id()) {
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
