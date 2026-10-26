use std::time::Instant;

use super::{Look, Outcome, Value, Widget};
use crate::ui::motion::Spring;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, SNAP};
use crate::ui::{Canvas, Key, Mods, Rect};

const TRACK_W: f32 = 36.0;
const TRACK_H: f32 = 20.0;
/// Knob inset in the track; its radius is the track's minus this
/// (concentric).
const KNOB_INSET: f32 = 3.0;

/// On or off. The knob slides; the track takes the accent as it arrives.
pub struct Switch {
    on: bool,
    /// 0 is off, 1 is on.
    knob: Spring,
}

impl Switch {
    pub fn new(on: bool, now: Instant) -> Self {
        Self {
            on,
            knob: Spring::settled(f32::from(u8::from(on)), now),
        }
    }

    pub fn set(&mut self, on: bool, now: Instant) {
        self.on = on;
        self.knob.set(f32::from(u8::from(on)), now);
    }

    fn toggle(&mut self, now: Instant) -> Outcome {
        self.on = !self.on;
        self.knob.retarget(f32::from(u8::from(self.on)), SNAP, now);
        Outcome::CHANGED
    }
}

impl Widget for Switch {
    fn width(&self, _: &Fonts, _: f32) -> f32 {
        TRACK_W
    }

    fn draw(&self, canvas: &mut Canvas, _: &Fonts, rect: Rect, look: Look, now: Instant) {
        let track = rect.centered(TRACK_W, TRACK_H);
        let t = self.knob.value(now).clamp(0.0, 1.0);
        let (off, on) = if look.enabled {
            (look.surface(), theme::ACCENT)
        } else {
            (theme::RAISE, theme::DIM)
        };
        canvas.fill(track, TRACK_H / 2.0, theme::mix(off, on, t), 1.0);
        let knob = TRACK_H - 2.0 * KNOB_INSET;
        let x = track.x + KNOB_INSET + t * (TRACK_W - TRACK_H);
        let knob_color = if look.enabled {
            theme::mix(theme::SUB, theme::ON_ACCENT, t)
        } else {
            theme::mix(theme::DIM, theme::RAISE, t)
        };
        canvas.fill(
            Rect::new(x, track.y + KNOB_INSET, knob, knob),
            knob / 2.0,
            knob_color,
            1.0,
        );
    }

    fn value(&self) -> Value {
        Value::Bool(self.on)
    }

    fn press(&mut self, _: Rect, _: (f32, f32), _: u8, _: &Fonts, now: Instant) -> Outcome {
        self.toggle(now)
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, now: Instant) -> Outcome {
        match key {
            Key::Space | Key::Enter => self.toggle(now),
            _ => Outcome::IGNORED,
        }
    }

    fn animating(&self, now: Instant) -> bool {
        !self.knob.done(now)
    }
}
