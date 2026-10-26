use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, light, surface};
use crate::ui::icons::Icon;
use crate::ui::motion::Spring;
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
    /// `−` and `+`: under the pointer, and held down.
    lit: [Spring; 2],
    down: [Spring; 2],
}

impl Stepper {
    pub fn new(value: i64, min: i64, max: i64, now: Instant) -> Self {
        let off = Spring::settled(0.0, now);
        Self {
            value: value.clamp(min, max),
            min,
            max,
            lit: [off; 2],
            down: [off; 2],
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
    fn width(&self, fonts: &Fonts, _: f32) -> f32 {
        2.0 * CTL_H + self.value_width(fonts)
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, now: Instant) {
        // The buttons light on their own; the whole surface only for focus.
        surface(
            canvas,
            rect,
            &Look {
                hover: 0.0,
                press: 0.0,
                ..look
            },
        );
        let (minus, plus) = Self::buttons(rect);
        for (index, button, icon, live) in [
            (0, minus, Icon::Minus, self.value > self.min),
            (1, plus, Icon::Plus, self.value < self.max),
        ] {
            let live = live && look.enabled;
            let lit = self.lit[index].value(now);
            if live && lit > 0.0 {
                let fill = theme::mix(theme::HOVER, theme::SELECTED, self.down[index].value(now));
                canvas.fill(button.inset(INSET), RADIUS - INSET, fill, lit);
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

    fn press(&mut self, rect: Rect, (x, y): (f32, f32), _: u8, _: &Fonts, now: Instant) -> Outcome {
        let (minus, plus) = Self::buttons(rect);
        let (index, by) = if minus.contains(x, y) {
            (0, -1)
        } else if plus.contains(x, y) {
            (1, 1)
        } else {
            return Outcome::HANDLED;
        };
        light(&mut self.down, |i| i == index, now);
        self.set(self.value + by)
    }

    fn release(&mut self, now: Instant) {
        light(&mut self.down, |_| false, now);
    }

    fn hover(&mut self, rect: Rect, at: Option<(f32, f32)>, _: &Fonts, now: Instant) {
        let (minus, plus) = Self::buttons(rect);
        let over = [minus, plus].map(|button| at.is_some_and(|(x, y)| button.contains(x, y)));
        light(&mut self.lit, |index| over[index], now);
    }

    fn animating(&self, now: Instant) -> bool {
        self.lit
            .iter()
            .chain(&self.down)
            .any(|spring| !spring.done(now))
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, _: Instant) -> Outcome {
        match key {
            Key::Left => self.set(self.value - 1),
            Key::Right => self.set(self.value + 1),
            _ => Outcome::IGNORED,
        }
    }
}
