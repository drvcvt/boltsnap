//! Pointer and keyboard events, turned into the app's [`Input`].

use boltsnap::ui::{Input, Key, Mods, PointerShape};
use smithay_client_toolkit::seat::{
    Capability, SeatHandler, SeatState,
    keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
    pointer::{BTN_LEFT, CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec},
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_keyboard::WlKeyboard, wl_pointer::WlPointer, wl_seat::WlSeat, wl_surface},
};

use super::window::Win;

/// The key a keysym stands for, or `None` for text and unmapped keys.
fn named_key(keysym: Keysym) -> Option<Key> {
    Some(match keysym {
        Keysym::Tab | Keysym::ISO_Left_Tab => Key::Tab,
        Keysym::Up => Key::Up,
        Keysym::Down => Key::Down,
        Keysym::Left => Key::Left,
        Keysym::Right => Key::Right,
        Keysym::Home => Key::Home,
        Keysym::End => Key::End,
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::space => Key::Space,
        Keysym::Escape => Key::Escape,
        Keysym::BackSpace => Key::Backspace,
        Keysym::Delete | Keysym::KP_Delete => Key::Delete,
        _ => return None,
    })
}

/// A pressed key as the app sees it.
pub(super) fn key_input(event: &KeyEvent, mods: Mods) -> Option<Input> {
    if let Some(key) = named_key(event.keysym) {
        return Some(Input::Key { key, mods });
    }
    if mods.ctrl {
        let c = event.keysym.key_char()?.to_ascii_lowercase();
        return c.is_ascii_alphanumeric().then_some(Input::Key {
            key: Key::Char(c),
            mods,
        });
    }
    let text: String = event
        .utf8
        .as_deref()?
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    (!text.is_empty()).then_some(Input::Text(text))
}

impl Win {
    fn key(&mut self, event: KeyEvent) {
        if let Some(input) = key_input(&event, self.mods) {
            self.input(input);
        }
    }
}

impl SeatHandler for Win {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            let cursor = self.compositor.create_surface(qh);
            self.pointer = self
                .seats
                .get_pointer_with_theme(qh, &seat, self.shm.wl_shm(), cursor, ThemeSpec::default())
                .ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self
                .seats
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.handle.clone(),
                    Box::new(|win: &mut Win, _, event| win.key(event)),
                )
                .ok();
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => self.pointer = None,
            Capability::Keyboard => self.keyboard = None,
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for Win {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        let mut at = None;
        for event in events {
            if &event.surface != self.surface() {
                continue;
            }
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            at = Some((x, y));
            let input = match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.shape = None;
                    Input::Move { x, y }
                }
                PointerEventKind::Leave { .. } => Input::Leave,
                PointerEventKind::Motion { .. } => Input::Move { x, y },
                PointerEventKind::Press {
                    button: BTN_LEFT,
                    time,
                    ..
                } => {
                    let clicks = self.clicks.press(time, x, y);
                    Input::Press { x, y, clicks }
                }
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } => Input::Release,
                PointerEventKind::Axis { vertical, .. } => {
                    if vertical.discrete == 0 && vertical.absolute == 0.0 {
                        continue;
                    }
                    Input::Wheel {
                        notches: vertical.discrete,
                        pixels: vertical.absolute as f32,
                    }
                }
                _ => continue,
            };
            self.input(input);
        }
        // The look for where the pointer is now, set only when it changes.
        if let Some(at) = at {
            let shape = self.app.pointer_shape(at, std::time::Instant::now());
            if self.shape != Some(shape)
                && let Some(pointer) = self.pointer.as_ref()
            {
                let icon = match shape {
                    PointerShape::Default => CursorIcon::Default,
                    PointerShape::ResizeX => CursorIcon::EwResize,
                };
                if pointer.set_cursor(conn, icon).is_ok() {
                    self.shape = Some(shape);
                }
            }
        }
    }
}

impl KeyboardHandler for Win {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.mods = Mods::default();
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key(event);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: u32,
    ) {
        self.mods = Mods {
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(keysym: Keysym, utf8: Option<&str>) -> KeyEvent {
        KeyEvent {
            time: 0,
            raw_code: 0,
            keysym,
            utf8: utf8.map(str::to_owned),
        }
    }

    #[test]
    fn keys_map_to_named_keys_shortcuts_or_text() {
        let none = Mods::default();
        let ctrl = Mods {
            ctrl: true,
            shift: false,
        };
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        assert_eq!(
            key_input(&event(Keysym::ISO_Left_Tab, None), shift),
            Some(Input::Key {
                key: Key::Tab,
                mods: shift
            })
        );
        assert_eq!(
            key_input(&event(Keysym::space, Some(" ")), none),
            Some(Input::Key {
                key: Key::Space,
                mods: none
            })
        );
        // With Ctrl, xkb hands over a control character; the keysym names the key.
        assert_eq!(
            key_input(&event(Keysym::c, Some("\u{3}")), ctrl),
            Some(Input::Key {
                key: Key::Char('c'),
                mods: ctrl
            })
        );
        assert_eq!(
            key_input(&event(Keysym::adiaeresis, Some("ä")), none),
            Some(Input::Text("ä".into()))
        );
        assert_eq!(key_input(&event(Keysym::Shift_L, None), shift), None);
    }
}
