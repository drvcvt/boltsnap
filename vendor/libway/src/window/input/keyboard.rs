//! Native keyboard facts. Layout decoding and repeat deadlines belong to the consumer.
use super::*;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, protocol::wl_keyboard};

impl Dispatch<wl_keyboard::WlKeyboard, SeatToken> for WindowState {
    fn event(
        state: &mut Self,
        proxy: &WlKeyboard,
        event: wl_keyboard::Event,
        token: &SeatToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Dropping a rejected/late event also closes an attached keymap fd exactly once.
        if !state.input_active() || !state.input.keyboard_live(*token, proxy) {
            return;
        }
        if let Err(error) = keyboard_event(state, *token, event) {
            state.remember(&error);
        }
    }
}
fn keyboard_event(
    state: &mut WindowState,
    token: SeatToken,
    event: wl_keyboard::Event,
) -> Result<()> {
    match event {
        wl_keyboard::Event::Keymap { format, fd, size } => {
            keyboard_keymap(state, token, format, fd, size)
        }
        wl_keyboard::Event::Enter {
            serial,
            surface,
            keys,
        } => keyboard_enter(state, token, serial, &surface, &keys),
        wl_keyboard::Event::Leave { serial, surface } => {
            keyboard_leave(state, token, serial, &surface)
        }
        wl_keyboard::Event::Key {
            serial,
            time,
            key,
            state: value,
        } => keyboard_key(state, token, serial, time, key, value),
        wl_keyboard::Event::Modifiers {
            serial,
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
        } => {
            let window = state.input.seats[&token.name].keyboard_focus;
            emit(
                state,
                token,
                KeyboardObservation::Modifiers {
                    window,
                    serial,
                    depressed: mods_depressed,
                    latched: mods_latched,
                    locked: mods_locked,
                    group,
                },
            )
        }
        wl_keyboard::Event::RepeatInfo { rate, delay } => {
            if rate < 0 || delay < 0 {
                return Err(Error::InvalidInput("negative repeat parameters"));
            }
            emit(
                state,
                token,
                KeyboardObservation::RepeatInfo {
                    rate: rate as u32,
                    delay_ms: delay as u32,
                },
            )
        }
        _ => Ok(()),
    }
}
fn emit(state: &mut WindowState, token: SeatToken, observation: KeyboardObservation) -> Result<()> {
    state.input_room(1, 0)?;
    state.input_event(token.id, InputObservation::Keyboard(observation));
    Ok(())
}
fn keyboard_enter(
    state: &mut WindowState,
    token: SeatToken,
    serial: u32,
    surface: &WlSurface,
    keys: &[u8],
) -> Result<()> {
    let Some(window) = state.input_window(surface) else {
        return Ok(());
    };
    if state.input.seats[&token.name].keyboard_focus.is_some() {
        return Err(Error::InvalidInput("keyboard Enter before Leave"));
    }
    let held = payload::held_keys(keys, &state.input.held)?;
    state.input_room(1, 1)?; // Protect its eventual cancellation before establishing focus.
    state
        .input
        .seats
        .get_mut(&token.name)
        .expect("live seat")
        .keyboard_focus = Some(window);
    state.input_event(
        token.id,
        InputObservation::Keyboard(KeyboardObservation::Enter {
            window,
            serial,
            held,
        }),
    );
    Ok(())
}
fn keyboard_leave(
    state: &mut WindowState,
    token: SeatToken,
    serial: u32,
    surface: &WlSurface,
) -> Result<()> {
    let Some(window) = state.input_window(surface) else {
        return Ok(());
    };
    if state.input.seats[&token.name].keyboard_focus != Some(window) {
        return Ok(());
    }
    state.input_room(0, 0)?; // Consume the focus's protected cancellation slot.
    state.text_input.blur(token.id, &mut state.events)?;
    state
        .input
        .seats
        .get_mut(&token.name)
        .expect("live seat")
        .keyboard_focus = None;
    state.clipboard.blur(token.id, &mut state.events);
    state
        .events
        .push_back(crate::window::WindowEvent::Input(InputEvent {
            seat: token.id,
            observation: InputObservation::Keyboard(KeyboardObservation::Leave { window, serial }),
        }));
    Ok(())
}

fn keyboard_keymap(
    state: &mut WindowState,
    token: SeatToken,
    format: WEnum<wl_keyboard::KeymapFormat>,
    fd: std::os::fd::OwnedFd,
    size: u32,
) -> Result<()> {
    let map = match format {
        WEnum::Value(wl_keyboard::KeymapFormat::NoKeymap) => {
            drop(fd);
            None
        }
        WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) => {
            Some(payload::keymap(fd, size, &state.input.keymaps)?)
        }
        _ => return Err(Error::Unsupported("keyboard keymap format")),
    };
    emit(state, token, KeyboardObservation::Keymap(map))
}
fn keyboard_key(
    state: &mut WindowState,
    token: SeatToken,
    serial: u32,
    time: u32,
    key: u32,
    value: WEnum<wl_keyboard::KeyState>,
) -> Result<()> {
    let Some(window) = state.input.seats[&token.name].keyboard_focus else {
        return Ok(());
    };
    let down = match value {
        WEnum::Value(wl_keyboard::KeyState::Pressed) => true,
        WEnum::Value(wl_keyboard::KeyState::Released) => false,
        _ => return Err(Error::InvalidInput("invalid native key state")),
    };
    emit(
        state,
        token,
        KeyboardObservation::Key {
            window,
            serial,
            time_ms: time,
            physical: key,
            down,
        },
    )?;
    if down {
        let root = state.tree_root(window);
        state
            .input
            .seats
            .get_mut(&token.name)
            .expect("live seat")
            .latest_input = Some(LatestInput {
            root,
            serial,
            device: InputDevice::Keyboard,
        });
    }
    Ok(())
}
