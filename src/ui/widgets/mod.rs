//! The controls a row can hold. Each draws itself into the rect the page
//! gives it and answers input in that rect's coordinates.

mod button;
mod dropdown;
mod segment;
mod slider;
mod stepper;
mod switch;
mod text_field;

use std::time::Instant;

pub use button::{IconButton, banner_height, banner_lines, draw_banner};
pub use dropdown::Dropdown;
pub use segment::{Part, Segment};
pub use slider::Slider;
pub use stepper::Stepper;
pub use switch::Switch;
pub use text_field::TextField;

use super::motion::Spring;
use super::text::Fonts;
use super::theme::{self, Rgb};
use super::{Canvas, Effect, Key, Mods, Rect};

/// How a control should look this frame. The page fades every value with a
/// spring; `focused` is the plain state for logic such as the caret.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    /// 0 to 1 each.
    pub hover: f32,
    pub press: f32,
    pub focus: f32,
    /// 0 to 1: the focus outline, shown only for focus from the keyboard.
    pub ring: f32,
    pub focused: bool,
    pub enabled: bool,
}

impl Look {
    /// The control's surface: one step lighter on hover, one more while
    /// pressed. Focus shows as an outline (`focus_ring`).
    pub fn surface(&self) -> Rgb {
        if !self.enabled {
            return theme::RAISE;
        }
        let lifted = theme::mix(theme::RAISE, theme::HOVER, self.hover);
        theme::mix(lifted, theme::SELECTED, self.press)
    }

    pub fn text(&self) -> Rgb {
        if self.enabled {
            theme::TEXT
        } else {
            theme::DIM
        }
    }
}

/// What a control did with an input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub handled: bool,
    /// Its value changed.
    pub changed: bool,
    pub effect: Option<Effect>,
}

impl Outcome {
    pub const IGNORED: Outcome = Outcome {
        handled: false,
        changed: false,
        effect: None,
    };
    pub const HANDLED: Outcome = Outcome {
        handled: true,
        changed: false,
        effect: None,
    };
    pub const CHANGED: Outcome = Outcome {
        handled: true,
        changed: true,
        effect: None,
    };

    pub fn changed_if(changed: bool) -> Outcome {
        if changed {
            Outcome::CHANGED
        } else {
            Outcome::HANDLED
        }
    }
}

/// A control's value, for the page that maps it to a setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Choice(usize),
    Text(String),
}

pub trait Widget {
    /// Width the control wants when `room` is free beside the row's name;
    /// more than `room` makes the row stack. Its height is always
    /// `theme::CTL_H`.
    fn width(&self, fonts: &Fonts, room: f32) -> f32;

    /// In a stacked row, the control takes the row's whole width.
    fn stretches(&self) -> bool {
        false
    }

    /// A click anywhere on its row is a click on the control, like the
    /// label of a checkbox (settings plan, 5c S4).
    fn row_click(&self) -> bool {
        false
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, now: Instant);

    fn value(&self) -> Value;

    /// Show `value`, which changed elsewhere (the tray, the file), as if it
    /// had been chosen here: what glides when chosen glides, numbers and
    /// text jump (settings plan, 5c S6). A value of the wrong kind is ignored.
    fn adopt(&mut self, _value: &Value, _now: Instant) {}

    /// The words the control shows, for the copy check.
    fn labels(&self) -> Vec<&str> {
        Vec::new()
    }

    /// A press inside `rect`. A control that returns `handled` keeps the
    /// pointer until the release.
    fn press(
        &mut self,
        _rect: Rect,
        _at: (f32, f32),
        _clicks: u8,
        _fonts: &Fonts,
        _now: Instant,
    ) -> Outcome {
        Outcome::IGNORED
    }

    /// The captured pointer moved.
    fn drag(&mut self, _rect: Rect, _at: (f32, f32), _fonts: &Fonts) -> Outcome {
        Outcome::IGNORED
    }

    fn release(&mut self, _now: Instant) {}

    /// The pointer is at `at` over the control, or left it (`None`); for
    /// parts that light up on their own.
    fn hover(&mut self, _rect: Rect, _at: Option<(f32, f32)>, _fonts: &Fonts, _now: Instant) {}

    fn key(&mut self, _key: Key, _mods: Mods, _fonts: &Fonts, _now: Instant) -> Outcome {
        Outcome::IGNORED
    }

    fn text(&mut self, _text: &str, _now: Instant) -> Outcome {
        Outcome::IGNORED
    }

    /// Losing focus; a text field commits here.
    fn blur(&mut self, _now: Instant) -> Outcome {
        Outcome::IGNORED
    }

    fn animating(&self, _now: Instant) -> bool {
        false
    }

    /// An open list drawn above the page, if any; `anchor` is the control,
    /// `bounds` the area the list must stay in.
    fn overlay(&self, _anchor: Rect, _bounds: Rect, _fonts: &Fonts, _now: Instant) -> Option<Rect> {
        None
    }

    /// The overlay is open and takes input.
    fn overlay_open(&self) -> bool {
        false
    }

    fn draw_overlay(&self, _canvas: &mut Canvas, _fonts: &Fonts, _list: Rect, _now: Instant) {}

    /// The pointer moved while the overlay is open.
    fn overlay_hover(&mut self, _list: Rect, _at: (f32, f32), _now: Instant) {}

    /// The wheel turned over the open overlay; `true` if it scrolled it.
    fn overlay_wheel(&mut self, _notches: i32, _pixels: f32, _now: Instant) -> bool {
        false
    }

    /// A press while the overlay is open; `list` is where it is drawn.
    fn overlay_press(&mut self, _list: Rect, _at: (f32, f32), _now: Instant) -> Outcome {
        Outcome::IGNORED
    }

    fn close_overlay(&mut self, _now: Instant) {}
}

/// Fades toward 1 where `on`, 0 elsewhere; one spring per part.
pub(crate) fn light(springs: &mut [Spring], on: impl Fn(usize) -> bool, now: Instant) {
    for (index, spring) in springs.iter_mut().enumerate() {
        spring.retarget(f32::from(u8::from(on(index))), theme::QUICK, now);
    }
}

/// The control's rounded surface.
pub(crate) fn surface(canvas: &mut Canvas, rect: Rect, look: &Look) {
    canvas.fill(rect, theme::RADIUS, look.surface(), 1.0);
    focus_ring(canvas, rect, theme::RADIUS, look.ring);
}

/// The keyboard focus outline: `FOCUS_W` around `rect`, concentric with its
/// corners, in `SUB` on the group or sidebar behind (no glow).
pub(crate) fn focus_ring(canvas: &mut Canvas, rect: Rect, radius: f32, strength: f32) {
    let w = theme::FOCUS_W;
    canvas.ring(rect.inset(-w), radius + w, w, theme::SUB, strength);
}

/// A `size` icon box at `x`, centred on `line`'s middle, where text drawn
/// with [`Fonts::baseline`] has its capitals' middle.
pub(crate) fn icon_box(line: Rect, x: f32, size: f32) -> Rect {
    Rect::new(x, line.y + (line.h - size) / 2.0, size, size)
}
