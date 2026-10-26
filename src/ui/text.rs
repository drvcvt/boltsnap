//! Measuring and drawing text in the type scale.

use std::borrow::Cow;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};

use super::theme::TextStyle;
use super::{Canvas, Rect, Rgb};

/// The DejaVu subset the selector and the recording controls fall back to
/// (printable ASCII and ×).
const FALLBACK: &[u8] = include_bytes!("../../assets/fonts/dejavu-badge.ttf");

/// The UI face at the two weights the scale uses.
pub struct Fonts {
    medium: FontVec,
    demibold: FontVec,
}

impl Fonts {
    pub fn new(medium: FontVec, demibold: FontVec) -> Self {
        Self { medium, demibold }
    }

    /// The embedded subset, for tests and when no font loads.
    pub fn fallback() -> Self {
        let face = || FontVec::try_from_vec(FALLBACK.to_vec()).expect("embedded font parses");
        Self::new(face(), face())
    }

    fn face(&self, style: TextStyle) -> &FontVec {
        if style.strong() {
            &self.demibold
        } else {
            &self.medium
        }
    }

    /// Advance width of `text`.
    pub fn width(&self, style: TextStyle, text: &str) -> f32 {
        let face = self.face(style);
        let scaled = face.as_scaled(PxScale::from(style.px()));
        text.chars()
            .map(|c| scaled.h_advance(face.glyph_id(c)))
            .sum()
    }

    /// Height of a capital (an `H` sits on the baseline).
    pub fn cap_height(&self, style: TextStyle) -> f32 {
        let face = self.face(style);
        let scaled = face.as_scaled(PxScale::from(style.px()));
        face.outline(face.glyph_id('H'))
            .map(|outline| outline.bounds.height().abs() * scaled.v_scale_factor())
            .unwrap_or(style.px() * 0.7)
    }

    /// The baseline that puts the capitals' middle on the middle of
    /// `top..top + height`.
    pub fn baseline(&self, style: TextStyle, top: f32, height: f32) -> f32 {
        top + (height + self.cap_height(style)) / 2.0
    }

    /// `text` cut to `max` wide, ending in an ellipsis when it had to be cut.
    pub fn ellipsize<'a>(&self, style: TextStyle, text: &'a str, max: f32) -> Cow<'a, str> {
        if self.width(style, text) <= max {
            return Cow::Borrowed(text);
        }
        let dots = if self.face(style).glyph_id('…').0 != 0 {
            "…"
        } else {
            "..."
        };
        let room = max - self.width(style, dots);
        let mut cut = String::new();
        for c in text.chars() {
            let mut next = cut.clone();
            next.push(c);
            if self.width(style, &next) > room {
                break;
            }
            cut = next;
        }
        Cow::Owned(cut.trim_end().to_owned() + dots)
    }

    /// The x of every caret position in `text`, as (byte index, x).
    pub fn carets(&self, style: TextStyle, text: &str) -> Vec<(usize, f32)> {
        let face = self.face(style);
        let scaled = face.as_scaled(PxScale::from(style.px()));
        let mut x = 0.0;
        let mut carets = Vec::with_capacity(text.len() + 1);
        for (index, c) in text.char_indices() {
            carets.push((index, x));
            x += scaled.h_advance(face.glyph_id(c));
        }
        carets.push((text.len(), x));
        carets
    }

    /// Draw `text` with its left baseline point at (`x`, `baseline`).
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        style: TextStyle,
        text: &str,
        at: (f32, f32),
        color: Rgb,
        alpha: f32,
    ) {
        let all = Rect::new(0.0, 0.0, canvas.width(), canvas.height());
        self.draw_clipped(canvas, style, text, at, color, alpha, all);
    }

    /// [`Fonts::draw`], leaving everything outside `clip` untouched.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_clipped(
        &self,
        canvas: &mut Canvas,
        style: TextStyle,
        text: &str,
        (x, baseline): (f32, f32),
        color: Rgb,
        alpha: f32,
        clip: Rect,
    ) {
        let face = self.face(style);
        let scale = PxScale::from(style.px() * canvas.scale);
        let scaled = face.as_scaled(scale);
        let width = canvas.pixmap.width() as i32;
        let device = |v: f32| (v * canvas.scale).round() as i32;
        let (left, top) = (device(clip.x).max(0), device(clip.y).max(0));
        let right = device(clip.right()).min(width);
        let bottom = device(clip.bottom()).min(canvas.pixmap.height() as i32);
        let data = canvas.pixmap.data_mut();
        let mut caret = x * canvas.scale;
        let baseline = baseline * canvas.scale;
        for c in text.chars() {
            let id = face.glyph_id(c);
            let glyph = id.with_scale_and_position(scale, point(caret, baseline));
            caret += scaled.h_advance(id);
            let Some(outlined) = face.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px < left || px >= right || py < top || py >= bottom {
                    return;
                }
                let a = (coverage * alpha).clamp(0.0, 1.0);
                let i = ((py * width + px) * 4) as usize;
                // Premultiplied source-over with an opaque colour.
                let over = |s: u8, d: u8| (s as f32 * a + d as f32 * (1.0 - a)).round() as u8;
                data[i] = over(color.0, data[i]);
                data[i + 1] = over(color.1, data[i + 1]);
                data[i + 2] = over(color.2, data[i + 2]);
                data[i + 3] = over(255, data[i + 3]);
            });
        }
    }

    /// Draw `text` left-aligned in `rect`, capitals centred on its middle,
    /// cut with an ellipsis to fit.
    pub fn draw_in(
        &self,
        canvas: &mut Canvas,
        style: TextStyle,
        text: &str,
        rect: Rect,
        color: Rgb,
        alpha: f32,
    ) {
        let text = self.ellipsize(style, text, rect.w);
        let baseline = self.baseline(style, rect.y, rect.h);
        self.draw(canvas, style, &text, (rect.x, baseline), color, alpha);
    }
}
