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

pub use button::{IconButton, draw_banner};
pub use dropdown::Dropdown;
pub use segment::{Part, Segment};
pub use slider::Slider;
pub use stepper::Stepper;
pub use switch::Switch;
pub use text_field::TextField;

use super::text::Fonts;
use super::theme::{self, Rgb};
use super::{Canvas, Effect, Key, Mods, Rect};

/// How a control should look this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    /// 0 to 1, faded in and out by the page.
    pub hover: f32,
    pub focus: bool,
    pub enabled: bool,
    /// The pointer, when it is over the control, for parts that light up on
    /// their own.
    pub pointer: Option<(f32, f32)>,
}

impl Look {
    /// The control's surface: one step lighter on hover and focus.
    pub fn surface(&self) -> Rgb {
        if !self.enabled {
            return theme::RAISE;
        }
        let lift = if self.focus { 1.0 } else { self.hover };
        theme::mix(theme::RAISE, theme::HOVER, lift)
    }

    pub fn text(&self) -> Rgb {
        if self.enabled {
            theme::TEXT
        } else {
            theme::DIM
        }
    }

    pub fn over(&self, rect: Rect) -> bool {
        self.enabled && self.pointer.is_some_and(|(x, y)| rect.contains(x, y))
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
    /// Width the control wants; its height is always `theme::CTL_H`.
    fn width(&self, fonts: &Fonts) -> f32;

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, now: Instant);

    fn value(&self) -> Value;

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

    fn release(&mut self) {}

    fn key(&mut self, _key: Key, _mods: Mods, _fonts: &Fonts, _now: Instant) -> Outcome {
        Outcome::IGNORED
    }

    fn text(&mut self, _text: &str) -> Outcome {
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

    fn draw_overlay(
        &self,
        _canvas: &mut Canvas,
        _fonts: &Fonts,
        _list: Rect,
        _pointer: Option<(f32, f32)>,
        _now: Instant,
    ) {
    }

    /// A press while the overlay is open; `list` is where it is drawn.
    fn overlay_press(&mut self, _list: Rect, _at: (f32, f32), _now: Instant) -> Outcome {
        Outcome::IGNORED
    }

    fn close_overlay(&mut self, _now: Instant) {}
}

/// The control's rounded surface.
pub(crate) fn surface(canvas: &mut Canvas, rect: Rect, look: &Look) {
    canvas.fill(rect, theme::RADIUS, look.surface(), 1.0);
}

/// A `size` icon box at `x`, centred on `line`'s middle, where text drawn
/// with [`Fonts::baseline`] has its capitals' middle.
pub(crate) fn icon_box(line: Rect, x: f32, size: f32) -> Rect {
    Rect::new(x, line.y + (line.h - size) / 2.0, size, size)
}
