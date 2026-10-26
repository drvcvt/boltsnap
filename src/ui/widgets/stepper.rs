use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, surface};
use crate::ui::icons::Icon;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, CTL_H, GAP_S, ICON, RADIUS, TextStyle};
use crate::ui::{Canvas, Key, Mods, Rect};

/// Gap around a button's hover surface, as in a segment (concentric).
const INSET: f32 = 3.0;

/// `−  value  +` for a whole number in `min..=max`. A bound disables its
/// button.
pub struct Stepper {
    value: i64,
    min: i64,
    max: i64,
}

impl Stepper {
    pub fn new(value: i64, min: i64, max: i64) -> Self {
        Self {
            value: value.clamp(min, max),
            min,
            max,
        }
    }

    fn value_width(&self, fonts: &Fonts) -> f32 {
        fonts
            .width(TextStyle::Label, &self.min.to_string())
            .max(fonts.width(TextStyle::Label, &self.max.to_string()))
            .ceil()
            + 2.0 * GAP_S
    }

    fn set(&mut self, value: i64) -> Outcome {
        let value = value.clamp(self.min, self.max);
        let changed = value != self.value;
        self.value = value;
        Outcome::changed_if(changed)
    }

    /// The `−` and `+` squares at both ends.
    fn buttons(rect: Rect) -> (Rect, Rect) {
        (
            Rect::new(rect.x, rect.y, CTL_H, CTL_H),
            Rect::new(rect.right() - CTL_H, rect.y, CTL_H, CTL_H),
        )
    }
}

impl Widget for Stepper {
    fn width(&self, fonts: &Fonts) -> f32 {
        2.0 * CTL_H + self.value_width(fonts)
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, _: Instant) {
        surface(canvas, rect, &look);
        let (minus, plus) = Self::buttons(rect);
        for (button, icon, live) in [
            (minus, Icon::Minus, self.value > self.min),
            (plus, Icon::Plus, self.value < self.max),
        ] {
            let live = live && look.enabled;
            if live && look.over(button) {
                canvas.fill(button.inset(INSET), RADIUS - INSET, theme::SELECTED, 1.0);
            }
            let color = if live { theme::TEXT } else { theme::DIM };
            let x = button.x + (button.w - ICON) / 2.0;
            icon.draw(canvas, icon_box(button, x, ICON), color, 1.0);
        }
        let text = self.value.to_string();
        let x = rect.x + (rect.w - fonts.width(TextStyle::Label, &text)) / 2.0;
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

    fn press(&mut self, rect: Rect, (x, y): (f32, f32), _: u8, _: &Fonts, _: Instant) -> Outcome {
        let (minus, plus) = Self::buttons(rect);
        if minus.contains(x, y) {
            self.set(self.value - 1)
        } else if plus.contains(x, y) {
            self.set(self.value + 1)
        } else {
            Outcome::HANDLED
        }
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, _: Instant) -> Outcome {
        match key {
            Key::Left => self.set(self.value - 1),
            Key::Right => self.set(self.value + 1),
            _ => Outcome::IGNORED,
        }
    }
}
