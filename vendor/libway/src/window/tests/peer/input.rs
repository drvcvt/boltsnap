//! Opt-in native input protocol peer. Object indices retain released predecessors.
use super::{Options, Request, State};
use std::os::fd::{AsFd, OwnedFd};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::GlobalId,
    protocol::{wl_keyboard, wl_pointer, wl_seat},
};

#[derive(Debug)]
pub(in crate::window::tests) enum InputWireEvent {
    Capabilities(usize, u32),
    Name(usize, String),
    RemoveSeat(usize),
    /// Advertise one more version-nine seat, indexed after the existing ones.
    AddSeat,
    Keymap(usize, u32, OwnedFd, u32),
    KeyboardEnter(usize, usize, u32, Vec<u8>),
    KeyboardLeave(usize, usize, u32),
    Key(usize, u32, u32, u32, bool),
    Modifiers(usize, u32, u32, u32, u32, u32),
    Repeat(usize, i32, i32),
    PointerEnter(usize, usize, u32, f64, f64),
    PointerLeave(usize, usize, u32),
    Motion(usize, u32, f64, f64),
    Button(usize, u32, u32, u32, bool),
    Axis(usize, u32, u32, f64),
    Source(usize, u32),
    Discrete(usize, u32, i32),
    Value120(usize, u32, i32),
    Stop(usize, u32, u32),
    Direction(usize, u32, u32),
    Frame(usize),
}
#[derive(Default)]
pub(in crate::window::tests) struct InputState {
    globals: Vec<GlobalId>,
    seats: Vec<Option<wl_seat::WlSeat>>,
    keyboards: Vec<wl_keyboard::WlKeyboard>,
    pointers: Vec<wl_pointer::WlPointer>,
}
pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut InputState) {
    for index in 0..options.seat_count {
        state
            .globals
            .push(dh.create_global::<State, wl_seat::WlSeat, _>(options.seat_version, index));
    }
}
impl GlobalDispatch<wl_seat::WlSeat, usize> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_seat::WlSeat>,
        index: &usize,
        init: &mut DataInit<'_, Self>,
    ) {
        let seat = init.init(resource, *index);
        seat.capabilities(wl_seat::Capability::Keyboard | wl_seat::Capability::Pointer);
        if seat.version() >= 2 {
            seat.name(format!("seat{index}"));
        }
        state.ledger.push(Request::BindSeat(*index, seat.version()));
        state
            .input
            .seats
            .resize(state.input.seats.len().max(index + 1), None);
        state.input.seats[*index] = Some(seat);
    }
}
impl Dispatch<wl_seat::WlSeat, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        request: wl_seat::Request,
        seat: &usize,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_seat::Request::GetKeyboard { id } => {
                let index = state.input.keyboards.len();
                state.input.keyboards.push(init.init(id, index));
                state.ledger.push(Request::Keyboard(*seat, index));
            }
            wl_seat::Request::GetPointer { id } => {
                let index = state.input.pointers.len();
                state.input.pointers.push(init.init(id, index));
                state.ledger.push(Request::Pointer(*seat, index));
            }
            wl_seat::Request::Release => state.ledger.push(Request::ReleaseSeat(*seat)),
            other => panic!("unexpected input request: {other:?}"),
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_keyboard::WlKeyboard,
        request: wl_keyboard::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        assert!(matches!(request, wl_keyboard::Request::Release));
        state.ledger.push(Request::ReleaseKeyboard(*index));
    }
}
impl Dispatch<wl_pointer::WlPointer, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_pointer::WlPointer,
        request: wl_pointer::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        assert!(matches!(request, wl_pointer::Request::Release));
        state.ledger.push(Request::ReleasePointer(*index));
    }
}
pub(super) fn emit(event: InputWireEvent, state: &mut State, dh: &DisplayHandle) {
    match event {
        InputWireEvent::Capabilities(index, bits) => state.input.seats[index]
            .as_ref()
            .unwrap()
            .send_event(wl_seat::Event::Capabilities {
                capabilities: WEnum::Unknown(bits),
            })
            .unwrap(),
        InputWireEvent::Name(index, name) => state.input.seats[index].as_ref().unwrap().name(name),
        InputWireEvent::RemoveSeat(index) => {
            dh.remove_global::<State>(state.input.globals[index].clone())
        }
        InputWireEvent::AddSeat => {
            let index = state.input.globals.len();
            let global = dh.create_global::<State, wl_seat::WlSeat, _>(9, index);
            state.input.globals.push(global);
        }
        keyboard @ (InputWireEvent::Keymap(..)
        | InputWireEvent::KeyboardEnter(..)
        | InputWireEvent::KeyboardLeave(..)
        | InputWireEvent::Key(..)
        | InputWireEvent::Modifiers(..)
        | InputWireEvent::Repeat(..)) => emit_keyboard(keyboard, state),
        pointer => emit_pointer(pointer, state),
    }
}
fn emit_keyboard(event: InputWireEvent, state: &State) {
    let keyboards = &state.input.keyboards;
    match event {
        InputWireEvent::Keymap(index, format, fd, size) => keyboards[index]
            .send_event(wl_keyboard::Event::Keymap {
                format: WEnum::Unknown(format),
                fd: fd.as_fd(),
                size,
            })
            .unwrap(),
        InputWireEvent::KeyboardEnter(index, window, serial, keys) => {
            keyboards[index].enter(serial, &state.windows[window].surface, keys)
        }
        InputWireEvent::KeyboardLeave(index, window, serial) => {
            keyboards[index].leave(serial, &state.windows[window].surface)
        }
        InputWireEvent::Key(index, serial, time, key, down) => keyboards[index].key(
            serial,
            time,
            key,
            if down {
                wl_keyboard::KeyState::Pressed
            } else {
                wl_keyboard::KeyState::Released
            },
        ),
        InputWireEvent::Modifiers(index, serial, depressed, latched, locked, group) => {
            keyboards[index].modifiers(serial, depressed, latched, locked, group)
        }
        InputWireEvent::Repeat(index, rate, delay) => keyboards[index].repeat_info(rate, delay),
        _ => unreachable!("keyboard peer selection"),
    }
}
fn emit_pointer(event: InputWireEvent, state: &State) {
    let pointers = &state.input.pointers;
    match event {
        InputWireEvent::PointerEnter(index, window, serial, x, y) => {
            pointers[index].enter(serial, &state.windows[window].surface, x, y)
        }
        InputWireEvent::PointerLeave(index, window, serial) => {
            pointers[index].leave(serial, &state.windows[window].surface)
        }
        InputWireEvent::Motion(index, time, x, y) => pointers[index].motion(time, x, y),
        InputWireEvent::Button(index, serial, time, button, down) => pointers[index].button(
            serial,
            time,
            button,
            if down {
                wl_pointer::ButtonState::Pressed
            } else {
                wl_pointer::ButtonState::Released
            },
        ),
        InputWireEvent::Axis(index, time, axis, value) => pointers[index]
            .send_event(wl_pointer::Event::Axis {
                time,
                axis: WEnum::Unknown(axis),
                value,
            })
            .unwrap(),
        InputWireEvent::Source(index, source) => pointers[index]
            .send_event(wl_pointer::Event::AxisSource {
                axis_source: WEnum::Unknown(source),
            })
            .unwrap(),
        InputWireEvent::Discrete(index, axis, discrete) => pointers[index]
            .send_event(wl_pointer::Event::AxisDiscrete {
                axis: WEnum::Unknown(axis),
                discrete,
            })
            .unwrap(),
        InputWireEvent::Value120(index, axis, value120) => pointers[index]
            .send_event(wl_pointer::Event::AxisValue120 {
                axis: WEnum::Unknown(axis),
                value120,
            })
            .unwrap(),
        InputWireEvent::Stop(index, time, axis) => pointers[index]
            .send_event(wl_pointer::Event::AxisStop {
                time,
                axis: WEnum::Unknown(axis),
            })
            .unwrap(),
        InputWireEvent::Direction(index, axis, direction) => pointers[index]
            .send_event(wl_pointer::Event::AxisRelativeDirection {
                axis: WEnum::Unknown(axis),
                direction: WEnum::Unknown(direction),
            })
            .unwrap(),
        InputWireEvent::Frame(index) => pointers[index].frame(),
        _ => unreachable!("pointer peer selection"),
    }
}
