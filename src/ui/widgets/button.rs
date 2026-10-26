use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, surface};
use crate::ui::icons::Icon;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, CTL_H, GAP_M, GAP_S, ICON, ICON_ALONE, PANEL_RADIUS, TextStyle};
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
    fn width(&self, _: &Fonts, _: f32) -> f32 {
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

/// A notice across the page: icon and one line of text on a group's
/// surface, padded the same on all sides.
/// A banner keeps to this many lines; more ends with an ellipsis.
const BANNER_LINES: usize = 4;
/// Line pitch of a banner that takes more than one line: capitals 10 px,
/// `(20 − 10) / 2` a whole pixel.
const BANNER_PITCH: f32 = 20.0;
/// The one line of a single-line banner, as tall as ever.
const BANNER_LINE: f32 = 22.0;

/// A banner's text broken into the lines it is drawn in, `width` being the
/// banner's.
pub fn banner_lines(fonts: &Fonts, width: f32, text: &str) -> Vec<String> {
    let text_w = width - 2.0 * GAP_M - ICON - GAP_S;
    fonts.wrap(TextStyle::Label, text, text_w, BANNER_LINES)
}

/// How tall a banner `width` wide is with `text` broken into its lines.
pub fn banner_height(fonts: &Fonts, width: f32, text: &str) -> f32 {
    let lines = banner_lines(fonts, width, text).len() as f32;
    2.0 * GAP_M + BANNER_LINE.max(lines * BANNER_PITCH)
}

pub fn draw_banner(
    canvas: &mut Canvas,
    fonts: &Fonts,
    rect: Rect,
    icon: Icon,
    text: &str,
    color: theme::Rgb,
) {
    canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
    let lines = banner_lines(fonts, rect.w, text);
    let pitch = if lines.len() > 1 {
        BANNER_PITCH
    } else {
        BANNER_LINE
    };
    let x = rect.x + GAP_M + ICON + GAP_S;
    let line = |index: usize| {
        Rect::new(
            x,
            rect.y + GAP_M + index as f32 * pitch,
            rect.right() - GAP_M - x,
            pitch,
        )
    };
    // The icon sits on the first line's capitals.
    icon.draw(canvas, icon_box(line(0), rect.x + GAP_M, ICON), color, 1.0);
    for (index, text) in lines.iter().enumerate() {
        fonts.draw_in(
            canvas,
            TextStyle::Label,
            text,
            line(index),
            theme::TEXT,
            1.0,
        );
    }
}
