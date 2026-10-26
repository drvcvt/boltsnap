//! Pointer and keyboard: drawing, editing and the toolbar's controls.

use smithay_client_toolkit::{
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{
            BTN_LEFT, CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
        },
    },
    shell::WaylandSurface,
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{
        wl_keyboard::WlKeyboard, wl_pointer::WlPointer, wl_seat::WlSeat, wl_surface::WlSurface,
    },
};

use super::{Mode, Selector};
use crate::selector::{desktop, edit, render};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecordControlHit {
    Clip,
    Audio,
    Frame,
    Record,
}

pub(super) fn contains(rect: (f64, f64, f64, f64), point: (f64, f64)) -> bool {
    point.0 >= rect.0 && point.0 < rect.0 + rect.2 && point.1 >= rect.1 && point.1 < rect.1 + rect.3
}

pub(super) fn record_control_hit(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    chrome: &render::Chrome,
    point: (f64, f64),
) -> Option<RecordControlHit> {
    let area = boltsnap::ui::Rect::new(0.0, 0.0, surf_w as f32, surf_h as f32);
    let toolbar = render::toolbar::layout(sel, area, &chrome.toolbar)?;
    let point = (point.0 as f32, point.1 as f32);
    toolbar.hit(point).map(|i| {
        [
            RecordControlHit::Record,
            RecordControlHit::Clip,
            RecordControlHit::Frame,
            RecordControlHit::Audio,
        ][i]
    })
}

#[derive(Clone, Copy)]
pub(super) enum Interaction {
    /// Resizing via a handle.
    Resize { handle: edit::Handle },
    /// Moving the whole rect; `grab` is cursor-minus-origin at press time.
    Move { grab: (f64, f64) },
    /// Pressed inside; becomes a Move once the cursor leaves a small slop radius,
    /// otherwise a confirm on release. `press` is the press position.
    ClickInside { press: (f64, f64) },
}

/// Handle hit radius (px) and minimum selection size (px).
pub(super) const HANDLE_R: f64 = 9.0;
pub(super) const MIN_SEL: f64 = 4.0;
/// Pixels of motion before a press-inside counts as a drag (move) rather than a
/// click-to-confirm — tolerates pointer jitter on a confirm click.
pub(super) const DRAG_SLOP: f64 = 3.0;

impl SeatHandler for Selector {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            let cursor_surface = self.compositor.create_surface(qh);
            if let Ok(tp) = self.seat_state.get_pointer_with_theme(
                qh,
                &seat,
                self.shm.wl_shm(),
                cursor_surface,
                ThemeSpec::default(),
            ) {
                self.pointer = Some(tp);
            }
        }
        if cap == Capability::Keyboard
            && self.keyboard.is_none()
            && let Ok(k) = self.seat_state.get_keyboard(qh, &seat, None)
        {
            self.keyboard = Some(k);
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard {
            self.keyboard = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for Selector {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        let mut redraw = false;
        for ev in events {
            if matches!(ev.kind, PointerEventKind::Enter { .. })
                && let Some(p) = self.pointer.as_ref()
            {
                let _ = p.set_cursor(conn, CursorIcon::Crosshair);
            }
            let Some(view) = self
                .views
                .iter()
                .find(|v| v.layer.wl_surface() == &ev.surface)
            else {
                continue;
            };
            // During the implicit button grab, positions remain relative to the
            // starting surface even after the pointer crosses onto another output.
            let (x, y) =
                desktop::pointer_position(view.viewport, ev.position, (self.surf_w, self.surf_h));
            let previous_cursor = self.cursor;
            if !self.record_mode && self.views.len() > 1 {
                // Chrome must follow the pointer onto a real monitor, not land
                // in a gap of an offset desktop layout.
                redraw |= self.views.iter().position(|v| v.contains(previous_cursor))
                    != self.views.iter().position(|v| v.contains((x, y)));
            }
            self.cursor = (x, y);
            if self.record_mode
                && let Mode::Editing { rect } = self.mode
            {
                let sel = (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                redraw |= record_control_hit(
                    sel,
                    self.surf_w,
                    self.surf_h,
                    &self.chrome,
                    previous_cursor,
                ) != record_control_hit(
                    sel,
                    self.surf_w,
                    self.surf_h,
                    &self.chrome,
                    self.cursor,
                );
            }
            match ev.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    match self.mode {
                        Mode::Editing { rect } => {
                            // Toolbar clicks do not alter the selection.
                            if self.record_mode {
                                let sel =
                                    (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                                match record_control_hit(
                                    sel,
                                    self.surf_w,
                                    self.surf_h,
                                    &self.chrome,
                                    (x, y),
                                ) {
                                    Some(RecordControlHit::Clip) => {
                                        if self.frozen.is_some() {
                                            self.result_clip = true;
                                            self.confirm_rect(rect);
                                        } else if !self.clip_reported {
                                            // A control that does nothing when
                                            // pressed is worse than one that
                                            // says why. Once per selection, so
                                            // repeated presses do not stack up.
                                            self.clip_reported = true;
                                            crate::platform::replay::notify_error(
                                                if self.replay_output.is_some() {
                                                    "Replay preview is unavailable or still loading. REC remains available."
                                                } else {
                                                    "No replay buffer to clip from. Start it from the tray menu (Replay buffer \u{2192} Start) or with `boltsnap replay start`."
                                                },
                                            );
                                        }
                                        return;
                                    }
                                    Some(RecordControlHit::Audio) => {
                                        self.audio_enabled = !self.audio_enabled;
                                        self.interaction = None;
                                        self.request_redraw();
                                        return;
                                    }
                                    Some(RecordControlHit::Frame) => {
                                        self.show_frame = !self.show_frame;
                                        self.interaction = None;
                                        self.request_redraw();
                                        return;
                                    }
                                    Some(RecordControlHit::Record) => {
                                        self.confirm_rect(rect);
                                        return;
                                    }
                                    None => {}
                                }
                            }
                            match edit::hit_region(rect, (x, y), HANDLE_R) {
                                edit::Region::Handle(h) => {
                                    self.interaction = Some(Interaction::Resize { handle: h });
                                }
                                edit::Region::Inside => {
                                    self.interaction =
                                        Some(Interaction::ClickInside { press: (x, y) });
                                }
                                edit::Region::Outside => {
                                    self.mode = Mode::Drawing {
                                        anchor: (x, y),
                                        now: (x, y),
                                    };
                                    self.interaction = None;
                                }
                            }
                        }
                        _ => {
                            self.mode = Mode::Drawing {
                                anchor: (x, y),
                                now: (x, y),
                            };
                            self.interaction = None;
                        }
                    }
                    redraw = true;
                }
                PointerEventKind::Motion { .. } => match self.mode {
                    Mode::Drawing { anchor, .. } => {
                        self.mode = Mode::Drawing {
                            anchor,
                            now: (x, y),
                        };
                        redraw = true;
                    }
                    Mode::Editing { rect } => {
                        match self.interaction {
                            Some(Interaction::Resize { handle }) => {
                                let nr = edit::resize_rect(
                                    rect,
                                    handle,
                                    (x, y),
                                    MIN_SEL,
                                    self.surf_w as f64,
                                    self.surf_h as f64,
                                );
                                self.mode = Mode::Editing { rect: nr };
                                redraw = true;
                            }
                            Some(Interaction::Move { grab }) => {
                                let target = edit::Rect {
                                    x: x - grab.0,
                                    y: y - grab.1,
                                    w: rect.w,
                                    h: rect.h,
                                };
                                let nr = edit::move_rect(
                                    target,
                                    0.0,
                                    0.0,
                                    self.surf_w as f64,
                                    self.surf_h as f64,
                                );
                                self.mode = Mode::Editing { rect: nr };
                                redraw = true;
                            }
                            // Promote to a move only once the cursor leaves the
                            // slop radius, so a jittery click still confirms.
                            Some(Interaction::ClickInside { press })
                                if (x - press.0).powi(2) + (y - press.1).powi(2)
                                    > DRAG_SLOP * DRAG_SLOP =>
                            {
                                self.interaction = Some(Interaction::Move {
                                    grab: (x - rect.x, y - rect.y),
                                });
                                redraw = true;
                            }
                            Some(Interaction::ClickInside { .. }) | None => {}
                        }
                        if self.alt_held {
                            redraw = true;
                        }
                    }
                    Mode::Idle => {
                        if self.alt_held {
                            redraw = true;
                        }
                    }
                },
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    match self.mode {
                        Mode::Drawing { anchor, now } => {
                            let rect = edit::Rect::from_corners(anchor, now);
                            if rect.w < MIN_SEL || rect.h < MIN_SEL {
                                self.mode = Mode::Idle;
                            } else if self.instant {
                                self.confirm_rect(rect);
                                return;
                            } else {
                                self.mode = Mode::Editing { rect };
                            }
                            self.interaction = None;
                            redraw = true;
                        }
                        Mode::Editing { rect } => {
                            // A press-inside with no drag is a confirm click.
                            if matches!(self.interaction, Some(Interaction::ClickInside { .. })) {
                                self.confirm_rect(rect);
                                return;
                            }
                            self.interaction = None;
                        }
                        Mode::Idle => {}
                    }
                }
                _ => {}
            }
        }
        if redraw {
            self.request_redraw();
        }
    }
}

impl KeyboardHandler for Selector {
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
        match event.keysym {
            Keysym::Escape => {
                self.result = None;
                self.done = true;
            }
            // Space or Enter confirms the editable selection (Space by habit).
            Keysym::space | Keysym::Return | Keysym::KP_Enter => {
                if let Mode::Editing { rect } = self.mode {
                    self.confirm_rect(rect);
                }
            }
            _ => {}
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
        modifiers: Modifiers,
        _: u32,
    ) {
        if self.alt_held != modifiers.alt {
            self.alt_held = modifiers.alt;
            self.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_control_hit_does_not_confirm_and_the_toolbar_has_no_dead_zone() {
        let chrome = render::Chrome::new(boltsnap::ui::text::Fonts::fallback());
        let sel = (80.0, 80.0, 200.0, 120.0);
        let area = boltsnap::ui::Rect::new(0.0, 0.0, 400.0, 300.0);
        let toolbar = render::toolbar::layout(sel, area, &chrome.toolbar).unwrap();
        let hit = |x: f32, y: f32| record_control_hit(sel, 400, 300, &chrome, (x as f64, y as f64));
        let audio = toolbar.controls[3];
        assert_eq!(
            hit(audio.x + audio.w / 2.0, audio.y + audio.h / 2.0),
            Some(RecordControlHit::Audio)
        );
        // The padding above Record and the gap after it still belong to it.
        let (record, bounds) = (toolbar.controls[0], toolbar.bounds);
        assert_eq!(
            hit(record.x + 2.0, bounds.y),
            Some(RecordControlHit::Record)
        );
        assert_eq!(
            hit(bounds.x, record.y + 2.0),
            Some(RecordControlHit::Record)
        );
        assert_eq!(
            hit(record.right() + 1.0, record.y + 2.0),
            Some(RecordControlHit::Record)
        );
        // Every point of the toolbar is some control's, none outside it.
        let mut y = bounds.y;
        while y < bounds.bottom() {
            let mut x = bounds.x;
            while x < bounds.right() {
                assert!(hit(x, y).is_some(), "dead zone at {x},{y}");
                x += 0.5;
            }
            y += 0.5;
        }
        assert_eq!(hit(bounds.x - 1.0, bounds.y), None);
    }
}
