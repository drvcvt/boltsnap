use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, surface};
use crate::ui::icons::Icon;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, CTL_H, GAP_M, GAP_S, ICON, ICON_ALONE, RADIUS, TextStyle};
use crate::ui::{Canvas, Key, Mods, Rect};

/// A square button with one icon. Its value counts the presses, so the page
/// sees each one as a change.
pub struct IconButton {
    icon: Icon,
    presses: i64,
}

impl IconButton {
    pub fn new(icon: Icon) -> Self {
        Self { icon, presses: 0 }
    }

    fn pressed(&mut self) -> Outcome {
        self.presses += 1;
        Outcome::CHANGED
    }
}

impl Widget for IconButton {
    fn width(&self, _: &Fonts) -> f32 {
        CTL_H
    }

    fn draw(&self, canvas: &mut Canvas, _: &Fonts, rect: Rect, look: Look, _: Instant) {
        surface(canvas, rect, &look);
        let x = rect.x + (rect.w - ICON_ALONE) / 2.0;
        self.icon
            .draw(canvas, icon_box(rect, x, ICON_ALONE), look.text(), 1.0);
    }

    fn value(&self) -> Value {
        Value::Int(self.presses)
    }

    fn press(&mut self, _: Rect, _: (f32, f32), _: u8, _: &Fonts, _: Instant) -> Outcome {
        self.pressed()
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, _: Instant) -> Outcome {
        match key {
            Key::Space | Key::Enter => self.pressed(),
            _ => Outcome::IGNORED,
        }
    }
}

/// A notice across the page: icon and one line of text on a surface, padded
/// the same on all sides.
pub fn draw_banner(
    canvas: &mut Canvas,
    fonts: &Fonts,
    rect: Rect,
    icon: Icon,
    text: &str,
    color: theme::Rgb,
) {
    canvas.fill(rect, RADIUS, theme::RAISE, 1.0);
    let line = rect.inset(GAP_M);
    icon.draw(canvas, icon_box(line, line.x, ICON), color, 1.0);
    let x = line.x + ICON + GAP_S;
    fonts.draw_in(
        canvas,
        TextStyle::Label,
        text,
        Rect::new(x, line.y, line.right() - x, line.h),
        theme::TEXT,
        1.0,
    );
}
