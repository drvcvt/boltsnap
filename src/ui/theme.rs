//! The window's look, in one place: the recording controls' family (dark,
//! opaque, `shelf/paint/controls.rs`), with groups on their own surface.
//! Nothing outside this file names a colour, a text size, a spacing value or
//! a spring. Values: settings plan, Phase 4b.

use super::motion::Token;

pub type Rgb = (u8, u8, u8);

/// The page behind the groups.
pub const WINDOW: Rgb = (0x12, 0x12, 0x12);
/// The navigation column, full height.
pub const SIDEBAR: Rgb = (0x17, 0x17, 0x17);
/// A group of rows.
pub const PANEL: Rgb = (0x1C, 0x1C, 0x1C);
/// Controls, and the navigation's selection.
pub const RAISE: Rgb = (0x26, 0x26, 0x26);
/// One step above `RAISE`: hover and keyboard focus.
pub const HOVER: Rgb = (0x2E, 0x2E, 0x2E);
/// The chosen part of a segment, a pressed control and an open list's
/// marked entry: a step above `HOVER`, so the choice still reads under the
/// pointer.
pub const SELECTED: Rgb = (0x3A, 0x3A, 0x3A);
pub const TEXT: Rgb = (0xEC, 0xEC, 0xEC);
pub const SUB: Rgb = (0x9A, 0x9A, 0x9A);
/// Disabled text only.
pub const DIM: Rgb = (0x6A, 0x6A, 0x6A);
pub const ERROR: Rgb = (0xE5, 0x48, 0x4D);
/// The one emphasis: a switch that is on and a slider's filled part,
/// inverted like the controls' primary button.
pub const ACCENT: Rgb = TEXT;
pub const ON_ACCENT: Rgb = WINDOW;

/// The type scale, as capital heights in whole pixels: each style draws at
/// the size that makes its capitals exactly that tall in the face in use
/// (`Fonts::size`), so they sit on the pixel grid. With Outfit that is about
/// 20, 14.3, 12.9 and 11.4 px. Hierarchy within a step comes from weight and
/// colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// Page title, 600.
    Title,
    /// Row names and control text, 500.
    Label,
    /// Group headings, 600.
    Heading,
    /// A row's hint line, 500.
    Hint,
    /// Small values and key hints, 500.
    Key,
}

impl TextStyle {
    /// Capital height in px. Even for the styles centred in `CTL_H` (30), so
    /// `(30 − cap) / 2` is a whole pixel and centring stays exact.
    pub const fn cap(self) -> f32 {
        match self {
            TextStyle::Title => 14.0,
            TextStyle::Label | TextStyle::Heading => 10.0,
            TextStyle::Hint => 9.0,
            TextStyle::Key => 8.0,
        }
    }

    /// The nominal size, for rules stated in font sizes (an icon beside text
    /// is never taller than this).
    pub const fn px(self) -> f32 {
        self.cap() / 0.7
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
pub const GAP_L: f32 = 16.0;
/// The page's edge and the space between groups.
pub const GAP_XL: f32 = 24.0;

/// Every control is this tall (≈ 2× the label size).
pub const CTL_H: f32 = 30.0;
/// Controls and navigation entries.
pub const RADIUS: f32 = 9.0;
/// A group's surface. Its content sits `GAP_M` in, more than `RADIUS`, so
/// the controls inside take the smaller step instead of a concentric one.
pub const PANEL_RADIUS: f32 = 12.0;
/// A row: one control plus `GAP_S` above and below. A group adds `GAP_XS`
/// above its first and below its last row, so its content sits `GAP_M`
/// from every edge.
pub const ROW_H: f32 = CTL_H + 2.0 * GAP_S;
/// Icon box next to `Label` text: never larger than the text, so the
/// navigation uses it too.
pub const ICON: f32 = 14.0;
/// A standalone icon: on an icon-only button and in the narrow navigation.
pub const ICON_ALONE: f32 = 16.0;
pub const NAV_W: f32 = 200.0;
/// The navigation with icons only: one square entry plus its edge.
pub const NAV_NARROW_W: f32 = CTL_H + 2.0 * GAP_S;
/// Below this window width the navigation shows icons only.
pub const NAV_NARROW_BELOW: f32 = 600.0;
/// The widest the column of groups gets.
pub const CONTENT_MAX: f32 = 680.0;
/// Room a row's name keeps before the row stacks its control below.
pub const LABEL_MIN: f32 = 120.0;
/// The narrowest a dropdown is, so the short ones of a page line up.
pub const MENU_MIN: f32 = 120.0;
/// The scroll thumb.
pub const THUMB_W: f32 = 4.0;
/// Icon stroke in logical pixels at any icon size.
pub const STROKE: f32 = 1.5;
/// The keyboard focus outline, around a control (settings plan, 5c S2).
pub const FOCUS_W: f32 = 2.0;

/// Hover, press, focus, a list opening, text getting brighter.
pub const QUICK: Token = Token { duration: 0.15 };
/// A switch's knob, a segment's choice, the navigation's selection, the
/// marked entry of an open list.
pub const SNAP: Token = Token { duration: 0.20 };
/// Wheel scroll, scrolling to the focused control, a page coming in.
pub const GLIDE: Token = Token { duration: 0.30 };
/// One mouse-wheel notch.
pub const WHEEL_STEP: f32 = 48.0;
