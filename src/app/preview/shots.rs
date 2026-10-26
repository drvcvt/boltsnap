//! The screenshots the previews show (settings plan, 5e): which one is a
//! whole screen, how big they are kept, where one goes to cover a place.
//! Pixels only: the platform reads the save folder and hands them over.

use crate::ui::Rect;
use crate::ui::picture::Picture;

/// At least this big, a screenshot is a whole screen rather than an area.
pub const SCREEN_MIN: (u32, u32) = (1280, 720);
/// The screen picture is kept at most this big.
pub const SCREEN_MAX: (u32, u32) = (1600, 900);
/// The cards' pictures, already in the card's shape (190 : 132).
pub const CARD_PX: (u32, u32) = (384, 267);
/// How far a screenshot's shape may be from the monitors' to fill them.
const SHAPE_SLACK: f32 = 0.02;

/// What the save folder holds, newest first.
#[derive(Default)]
pub struct Shots {
    /// The newest screenshot of a whole screen.
    pub screen: Option<Picture>,
    /// The newest others, for the cards.
    pub cards: Vec<Picture>,
}

/// A screenshot `w × h` is of a whole screen.
pub fn whole_screen((w, h): (u32, u32)) -> bool {
    w >= SCREEN_MIN.0 && h >= SCREEN_MIN.1
}

/// `size` shrunk to fit in `most`, its shape kept; never grown.
pub fn fit((w, h): (u32, u32), (most_w, most_h): (u32, u32)) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (1, 1);
    }
    let scale = (most_w as f32 / w as f32)
        .min(most_h as f32 / h as f32)
        .min(1.0);
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}

/// A picture `size` has the shape of `area` (the monitors together).
pub fn same_shape((w, h): (u32, u32), area: Rect) -> bool {
    if w == 0 || h == 0 || area.w <= 0.0 || area.h <= 0.0 {
        return false;
    }
    let ratio = (w as f32 / h as f32) / (area.w / area.h);
    (ratio - 1.0).abs() <= SHAPE_SLACK
}

/// Where a picture `size` goes to cover `rect`: a wider one keeps its left
/// part (the first monitor of a capture over all of them), a taller one its
/// middle.
pub fn cover((w, h): (u32, u32), rect: Rect) -> Rect {
    if w == 0 || h == 0 {
        return rect;
    }
    let scale = (rect.w / w as f32).max(rect.h / h as f32);
    let (pw, ph) = (w as f32 * scale, h as f32 * scale);
    Rect::new(rect.x, rect.y + (rect.h - ph) / 2.0, pw, ph)
}
