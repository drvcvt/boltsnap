//! The window's look, in one place: the recording controls' family (dark,
//! opaque, `shelf/paint/controls.rs`). Nothing outside this file names a
//! colour, a text size or a spacing value.

use std::time::Duration;

pub type Rgb = (u8, u8, u8);

pub const WINDOW: Rgb = (0x12, 0x12, 0x12);
/// Controls and the navigation's selection.
pub const RAISE: Rgb = (0x22, 0x22, 0x22);
/// One step above `RAISE`: hover and keyboard focus.
pub const HOVER: Rgb = (0x2A, 0x2A, 0x2A);
pub const TEXT: Rgb = (0xEC, 0xEC, 0xEC);
pub const SUB: Rgb = (0x96, 0x96, 0x96);
pub const DIM: Rgb = (0x6A, 0x6A, 0x6A);
pub const ERROR: Rgb = (0xE5, 0x48, 0x4D);
/// The chosen part of a segment and an open list's hovered entry: a step
/// above `HOVER`, so the choice still reads while the pointer is on it.
pub const SELECTED: Rgb = (0x38, 0x38, 0x38);
/// The one emphasis: a switch that is on and a slider's value, inverted like
/// the controls' primary button.
pub const ACCENT: Rgb = TEXT;
pub const ON_ACCENT: Rgb = WINDOW;

/// The type scale. Hierarchy within a step comes from weight and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// Page title, 15/600.
    Title,
    /// Row names and control text, 13/500.
    Label,
    /// Group headings, 13/600.
    Heading,
    /// A row's hint line, 12/500.
    Hint,
    /// Small values and key hints, 11/500.
    Key,
}

impl TextStyle {
    pub const fn px(self) -> f32 {
        match self {
            TextStyle::Title => 15.0,
            TextStyle::Label | TextStyle::Heading => 13.0,
            TextStyle::Hint => 12.0,
            TextStyle::Key => 11.0,
        }
    }

    /// Drawn with the 600 face.
    pub const fn strong(self) -> bool {
        matches!(self, TextStyle::Title | TextStyle::Heading)
    }
}

/// `a` to `b` by `t` (0 to 1).
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    (channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

/// Spacing scale: 4, 8, 12, 16, 24.
pub const GAP_XS: f32 = 4.0;
pub const GAP_S: f32 = 8.0;
pub const GAP_M: f32 = 12.0;
/// The window edge and the page's padding.
pub const EDGE: f32 = 16.0;
/// Between groups of rows.
pub const GROUP_GAP: f32 = 24.0;

/// Every control is this tall (≈ 2× the label size).
pub const CTL_H: f32 = 30.0;
pub const RADIUS: f32 = 9.0;
/// A row: one control plus 7 above and below.
pub const ROW_H: f32 = 44.0;
/// Icon box next to `Label` text: never larger than the text, so the
/// navigation uses it too.
pub const ICON: f32 = 13.0;
/// A standalone icon, alone on an icon-only button.
pub const ICON_ALONE: f32 = 16.0;
pub const NAV_W: f32 = 184.0;
/// Icon stroke in logical pixels at any icon size.
pub const STROKE: f32 = 1.5;

pub const HOVER_FADE: Duration = Duration::from_millis(90);
pub const KNOB_SLIDE: Duration = Duration::from_millis(140);
pub const NAV_SLIDE: Duration = Duration::from_millis(180);
pub const LIST_FADE: Duration = Duration::from_millis(90);
pub const SCROLL_GLIDE: Duration = Duration::from_millis(160);
/// One mouse-wheel notch.
pub const WHEEL_STEP: f32 = 48.0;
