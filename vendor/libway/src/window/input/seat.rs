//! Registry and capability ownership, with real-release and legacy-credit accounting.
use super::*;
use crate::window::WindowEvent;
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_registry::WlRegistry, wl_seat},
};

pub(in crate::window) fn bind_seat(
    state: &mut WindowState,
    registry: &WlRegistry,
    global: (u32, &str, u32),
    qh: &QueueHandle<WindowState>,
) {
    let (name, interface, version) = global;
    if interface != "wl_seat" || !state.input_active() {
        return;
    }
    let result = admit_seat(state, registry, name, version, qh);
    if let Err(error) = result {
        state.remember(&error);
    }
}
fn admit_seat(
    state: &mut WindowState,
    registry: &WlRegistry,
    name: u32,
    version: u32,
    qh: &QueueHandle<WindowState>,
) -> Result<()> {
    if version == 0 {
        return Err(Error::InvalidInput("invalid seat version"));
    }
    if state.input.seats.contains_key(&name) {
        return Ok(());
    }
    if state.input.seats.len() + state.input.retired_seats.len() >= MAX_SEATS {
        return Err(Error::LimitExceeded);
    }
    state.input_room(1, 0)?;
    let next = state
        .input
        .next_seat
        .checked_add(1)
        .ok_or(Error::LimitExceeded)?;
    let id = InputSeatId {
        connection: state.connection_id,
        sequence: state.input.next_seat,
    };
    let token = SeatToken { name, id };
    let proxy: WlSeat = registry.bind(name, version.min(9), qh, token);
    state.clipboard.add_seat(id, proxy.clone(), qh)?;
    state.text_input.add_seat(id, proxy.clone(), qh)?;
    let staged = StagedPointer::new(&state.input.pointer_records);
    state.input.seats.insert(
        name,
        Seat {
            token,
            proxy,
            keyboard: None,
            pointer: None,
            device: None,
            keyboard_focus: None,
            pointer_focus: None,
            enter: None,
            press: None,
            staged,
        },
    );
    state.input.next_seat = next;
    state.input_event(
        id,
        InputObservation::SeatAdded {
            version: version.min(9),
        },
    );
    Ok(())
}

pub(in crate::window) fn remove_seat(state: &mut WindowState, name: u32) {
    if !state.input.seats.contains_key(&name) {
        return;
    }
    // Existing protected cancellation slots cover all focus/staged records;
    // reserve the additional SeatRemoved before taking any protocol ownership.
    if state.input_room(1, 0).is_err() {
        return;
    }
    let id = state.input.seats[&name].token.id;
    if let Err(error) = state.text_input.remove_seat(id, &mut state.events) {
        state.remember(&error);
        return;
    }
    let mut seat = state.input.seats.remove(&name).expect("checked seat");
    state
        .clipboard
        .remove_seat(seat.token.id, &mut state.events);
    flush_retiring(&mut seat, state);
    cancel::cancel_focus(
        &mut seat,
        &mut state.events,
        None,
        None,
        InputCancelReason::SeatRemoved,
    );
    if let Some(proxy) = seat.keyboard {
        state.input.release_keyboard(proxy);
    }
    if let Some(proxy) = seat.pointer {
        state.input.release_pointer(proxy, seat.device);
    }
    if seat.proxy.version() >= 5 {
        seat.proxy.release();
    } else {
        state.input.retired_seats.push(seat.proxy);
    }
    state.events.push_back(WindowEvent::Input(InputEvent {
        seat: seat.token.id,
        observation: InputObservation::SeatRemoved,
    }));
}
fn flush_retiring(seat: &mut Seat, state: &mut WindowState) {
    if let Some(event) = cancel::take_frame(seat, &state.input.pointer_records, false, true) {
        state.events.push_back(WindowEvent::Input(event));
    }
}

impl Dispatch<wl_seat::WlSeat, SeatToken> for WindowState {
    fn event(
        state: &mut Self,
        proxy: &WlSeat,
        event: wl_seat::Event,
        token: &SeatToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.input_active()
            || !state
                .input
                .seats
                .get(&token.name)
                .is_some_and(|seat| seat.token == *token && seat.proxy == *proxy)
        {
            return;
        }
        let result = match event {
            wl_seat::Event::Capabilities { capabilities } => {
                let bits = match capabilities {
                    WEnum::Value(value) => value.bits(),
                    WEnum::Unknown(value) => value,
                };
                capabilities_changed(state, *token, bits)
            }
            wl_seat::Event::Name { name } => {
                payload::seat_name(name, &state.input.names).and_then(|name| {
                    state.input_room(1, 0)?;
                    state.input_event(token.id, InputObservation::SeatName(name));
                    Ok(())
                })
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            state.remember(&error);
        }
    }
}
fn capabilities_changed(state: &mut WindowState, token: SeatToken, bits: u32) -> Result<()> {
    let seat = &state.input.seats[&token.name];
    let keyboard = bits & wl_seat::Capability::Keyboard.bits() != 0;
    let pointer = bits & wl_seat::Capability::Pointer.bits() != 0;
    let added = usize::from(keyboard && seat.keyboard.is_none())
        + usize::from(pointer && seat.pointer.is_none());
    if state.input.children + added > MAX_CHILDREN {
        return Err(Error::LimitExceeded);
    }
    state.input_room(1, 0)?;
    if !keyboard {
        state.text_input.blur(token.id, &mut state.events)?;
    }
    let mut seat = state
        .input
        .seats
        .remove(&token.name)
        .expect("validated seat");
    if !pointer && seat.pointer.is_some() {
        flush_retiring(&mut seat, state);
    }
    change_keyboard(state, &mut seat, keyboard);
    change_pointer(state, &mut seat, pointer);
    state.input.seats.insert(token.name, seat);
    state.input_event(token.id, InputObservation::Capabilities { bits });
    Ok(())
}
fn change_keyboard(state: &mut WindowState, seat: &mut Seat, enabled: bool) {
    if enabled && seat.keyboard.is_none() {
        seat.keyboard = Some(seat.proxy.get_keyboard(&state.qh, seat.token));
        state.input.children += 1;
    } else if !enabled {
        state.clipboard.blur(seat.token.id, &mut state.events);
        cancel::cancel_focus(
            seat,
            &mut state.events,
            None,
            Some(InputDevice::Keyboard),
            InputCancelReason::CapabilityLost,
        );
        if let Some(proxy) = seat.keyboard.take() {
            state.input.release_keyboard(proxy);
        }
    }
}
fn change_pointer(state: &mut WindowState, seat: &mut Seat, enabled: bool) {
    if enabled && seat.pointer.is_none() {
        let pointer = seat.proxy.get_pointer(&state.qh, seat.token);
        seat.device = state.input.cursor.device(&pointer, &state.qh);
        seat.pointer = Some(pointer);
        state.input.children += 1;
    } else if !enabled && seat.pointer.is_some() {
        state.clipboard.retire_press(seat.token.id);
        seat.press = None;
        cancel::cancel_focus(
            seat,
            &mut state.events,
            None,
            Some(InputDevice::Pointer),
            InputCancelReason::CapabilityLost,
        );
        if let Some(proxy) = seat.pointer.take() {
            state.input.release_pointer(proxy, seat.device.take());
        }
    }
}
