use std::time::Instant;

use super::{Look, Outcome, Value, Widget};
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, TextStyle};
use crate::ui::{Canvas, Key, Mods, Rect};

const TRACK_W: f32 = 160.0;
const TRACK_H: f32 = 4.0;
const KNOB: f32 = 14.0;

/// A whole number in `min..=max` by `step`, with its value written to the
/// right. The knob follows the pointer directly.
pub struct Slider {
    value: i64,
    min: i64,
    max: i64,
    step: i64,
    unit: &'static str,
}

impl Slider {
    pub fn new(value: i64, (min, max, step): (i64, i64, i64), unit: &'static str) -> Self {
        let mut slider = Self {
            value: min,
            min,
            max,
            step: step.max(1),
            unit,
        };
        slider.value = slider.snap(value);
        slider
    }

    fn snap(&self, value: i64) -> i64 {
        let steps = ((value - self.min) as f64 / self.step as f64).round() as i64;
        (self.min + steps * self.step).clamp(self.min, self.max)
    }

    fn label(&self, value: i64) -> String {
        format!("{value} {}", self.unit).trim_end().to_owned()
    }

    fn value_width(&self, fonts: &Fonts) -> f32 {
        fonts
            .width(TextStyle::Label, &self.label(self.min))
            .max(fonts.width(TextStyle::Label, &self.label(self.max)))
            .ceil()
    }

    fn track(&self, rect: Rect) -> Rect {
        Rect::new(rect.x + KNOB / 2.0, rect.y, TRACK_W - KNOB, rect.h)
    }

    fn set(&mut self, value: i64) -> Outcome {
        let value = self.snap(value);
        let changed = value != self.value;
        self.value = value;
        Outcome::changed_if(changed)
    }

    fn set_from_x(&mut self, rect: Rect, x: f32) -> Outcome {
        let track = self.track(rect);
        let t = ((x - track.x) / track.w).clamp(0.0, 1.0) as f64;
        self.set(self.min + (t * (self.max - self.min) as f64).round() as i64)
    }
}

impl Widget for Slider {
    fn width(&self, fonts: &Fonts) -> f32 {
        TRACK_W + GAP_M + self.value_width(fonts)
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, _: Instant) {
        let track = self.track(rect);
        let t = (self.value - self.min) as f32 / (self.max - self.min).max(1) as f32;
        let bar = |x: f32, w: f32| Rect::new(x, rect.y + (rect.h - TRACK_H) / 2.0, w, TRACK_H);
        canvas.fill(bar(track.x, track.w), TRACK_H / 2.0, look.surface(), 1.0);
        let fill = if look.enabled {
            theme::ACCENT
        } else {
            theme::DIM
        };
        canvas.fill(bar(track.x, track.w * t), TRACK_H / 2.0, fill, 1.0);
        let knob_x = track.x + track.w * t - KNOB / 2.0;
        let knob = Rect::new(knob_x, rect.y + (rect.h - KNOB) / 2.0, KNOB, KNOB);
        // The focus ring would be a glow; focus lifts the knob's halo instead.
        if look.enabled && (look.focus || look.hover > 0.0) {
            let halo = if look.focus { 1.0 } else { look.hover };
            canvas.fill(knob.inset(-3.0), KNOB / 2.0 + 3.0, theme::HOVER, halo);
        }
        canvas.fill(knob, KNOB / 2.0, fill, 1.0);
        let value_rect = Rect::new(
            rect.x + TRACK_W + GAP_M,
            rect.y,
            self.value_width(fonts),
            rect.h,
        );
        let text = self.label(self.value);
        // Right-aligned so the number's last digit stays put while it changes.
        let x = value_rect.right() - fonts.width(TextStyle::Label, &text);
        let baseline = fonts.baseline(TextStyle::Label, rect.y, rect.h);
        fonts.draw(
            canvas,
            TextStyle::Label,
            &text,
            (x, baseline),
            look.text(),
            1.0,
        );
    }

    fn value(&self) -> Value {
        Value::Int(self.value)
    }

    fn press(&mut self, rect: Rect, (x, _): (f32, f32), _: u8, _: &Fonts, _: Instant) -> Outcome {
        if x > rect.x + TRACK_W {
            return Outcome::HANDLED;
        }
        self.set_from_x(rect, x)
    }

    fn drag(&mut self, rect: Rect, (x, _): (f32, f32), _: &Fonts) -> Outcome {
        self.set_from_x(rect, x)
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, _: Instant) -> Outcome {
        match key {
            Key::Left => self.set(self.value - self.step),
            Key::Right => self.set(self.value + self.step),
            Key::Home => self.set(self.min),
            Key::End => self.set(self.max),
            _ => Outcome::IGNORED,
        }
    }
}
