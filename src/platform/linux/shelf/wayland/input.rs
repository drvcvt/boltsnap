//! Pointer and keyboard on the shelf and the recording popup.

use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{
    BTN_LEFT, BTN_RIGHT, CursorIcon, PointerEvent, PointerEventKind, PointerHandler,
};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use crate::record::session::RecordingAction;
use crate::shelf::layout::Hit;

use super::controls::{esc_resumes, popup_key_button};
use super::{Daemon, notify};

/// In-flight left-button press, used to distinguish a click from a drag.
pub(super) struct PressState {
    pub(super) id: u64,
    pub(super) hit: Hit,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) serial: u32,
    pub(super) dragging: bool,
}

impl PointerHandler for Daemon {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        let shelf_surface = self.layer.as_ref().map(|l| l.wl_surface().clone());
        let mut redraw = false;
        for ev in events {
            if matches!(ev.kind, PointerEventKind::Press { .. }) && self.drag_source.is_some() {
                self.clear_drag();
            }
            if self.is_popup_surface(&ev.surface) {
                if matches!(ev.kind, PointerEventKind::Enter { .. })
                    && let Some(p) = self.pointer.as_ref()
                {
                    let _ = p.set_cursor(conn, CursorIcon::Default);
                }
                if let PointerEventKind::Press { button, .. } = ev.kind
                    && button == BTN_LEFT
                {
                    self.on_popup_click(ev.position);
                }
                continue;
            }
            // Everything else is shelf input.
            if shelf_surface.as_ref() != Some(&ev.surface) {
                continue;
            }
            // Set the normal arrow when the pointer enters the shelf, instead of
            // inheriting the previously-focused window's cursor (e.g. an I-beam).
            if matches!(ev.kind, PointerEventKind::Enter { .. })
                && let Some(p) = self.pointer.as_ref()
            {
                let _ = p.set_cursor(conn, CursorIcon::Default);
            }
            let (x, y) = ev.position;
            match ev.kind {
                PointerEventKind::Leave { .. } if self.hovered.is_some() => {
                    self.set_hovered(None);
                    redraw = true;
                }
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let now = self.hit_test(x, y).map(|h| match h {
                        Hit::Body(id) | Hit::Save(id) | Hit::Close(id) => id,
                    });
                    if now != self.hovered {
                        self.set_hovered(now);
                        redraw = true;
                    }
                    let start_drag = self
                        .press
                        .as_ref()
                        .map(|p| {
                            !p.dragging
                                && matches!(p.hit, Hit::Body(_))
                                && (x - p.x).powi(2) + (y - p.y).powi(2) > 36.0
                        })
                        .unwrap_or(false);
                    if start_drag {
                        let (id, serial) = {
                            let p = self.press.as_ref().unwrap();
                            (p.id, p.serial)
                        };
                        // Only mark the press as dragging if the drag actually
                        // started; otherwise the eventual Release would wrongly be
                        // swallowed instead of treated as a click.
                        if self.begin_drag(id, serial)
                            && let Some(p) = self.press.as_mut()
                        {
                            p.dragging = true;
                        }
                    }
                }
                PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                    if let Some(hit) = self.hit_test(x, y) {
                        let id = match hit {
                            Hit::Body(i) | Hit::Save(i) | Hit::Close(i) => i,
                        };
                        self.press = Some(PressState {
                            id,
                            hit,
                            x,
                            y,
                            serial,
                            dragging: false,
                        });
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_RIGHT => {
                    if let Some(hit) = self.hit_test(x, y) {
                        let id = match hit {
                            Hit::Body(i) | Hit::Save(i) | Hit::Close(i) => i,
                        };
                        self.copy_card(id);
                    }
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if let Some(p) = self.press.take()
                        && !p.dragging
                    {
                        self.on_click(p.hit, &mut redraw);
                    }
                }
                _ => {}
            }
        }
        if redraw && let Some(qh) = self.qh.clone() {
            self.draw(&qh);
        }
    }
}

impl KeyboardHandler for Daemon {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
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
        _: &WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if self.popup.is_none() {
            return;
        }
        if event.keysym == Keysym::Escape {
            self.discard_armed = None;
            let resume = self.recording.as_ref().is_some_and(|session| {
                esc_resumes(self.auto_paused, session.phase, session.resume_blocked)
            });
            self.remove_popup();
            if resume
                && let Some(qh) = self.qh.clone()
                && let Err(error) = self.handle_recording_action(RecordingAction::Resume, &qh)
            {
                notify(&format!("Recording: {error}"));
            }
        } else if let Some(button) = popup_key_button(event.keysym) {
            self.press_popup_button(button);
        }
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
        _: Modifiers,
        _: u32,
    ) {
    }
}
