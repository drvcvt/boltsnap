//! Measuring and drawing text in the type scale.

use std::borrow::Cow;

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont, point};

use super::theme::TextStyle;
use super::{Canvas, Rect, Rgb};

/// The DejaVu subset the selector and the recording controls fall back to
/// (printable ASCII and ×).
const FALLBACK: &[u8] = include_bytes!("../../assets/fonts/dejavu-badge.ttf");

/// `c`'s glyph; a no-break space the face lacks is drawn as a space.
fn glyph(face: &FontVec, c: char) -> GlyphId {
    match face.glyph_id(c) {
        GlyphId(0) if c == '\u{a0}' => face.glyph_id(' '),
        id => id,
    }
}

/// Advance of every glyph in `text`, kerned against the one before, with
/// the byte index each starts at.
fn advances<'a>(
    face: &'a FontVec,
    scaled: &'a impl ScaleFont<&'a FontVec>,
    text: &'a str,
) -> impl Iterator<Item = (usize, GlyphId, f32)> + 'a {
    let mut previous: Option<GlyphId> = None;
    text.char_indices().map(move |(index, c)| {
        let id = glyph(face, c);
        let kern = previous.map_or(0.0, |previous| scaled.kern(previous, id));
        previous = Some(id);
        (index, id, kern)
    })
}

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
        advances(face, &scaled, text)
            .map(|(_, id, kern)| kern + scaled.h_advance(id))
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
        let dots = self.dots(style);
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

    /// `text` cut to `max` wide in the middle, for paths: the start and the
    /// end stay readable.
    pub fn ellipsize_middle<'a>(&self, style: TextStyle, text: &'a str, max: f32) -> Cow<'a, str> {
        if self.width(style, text) <= max {
            return Cow::Borrowed(text);
        }
        let dots = self.dots(style);
        let chars: Vec<char> = text.chars().collect();
        let (mut head, mut tail) = (0, 0);
        let fits = |head: usize, tail: usize| {
            let cut: String = chars[..head].iter().collect::<String>()
                + dots
                + &chars[chars.len() - tail..].iter().collect::<String>();
            self.width(style, &cut) <= max
        };
        // Grow both ends in turn, the end first: a path's last part names it.
        loop {
            let grown = if tail <= head && head + tail < chars.len() && fits(head, tail + 1) {
                tail += 1;
                true
            } else if head + tail < chars.len() && fits(head + 1, tail) {
                head += 1;
                true
            } else if head + tail < chars.len() && fits(head, tail + 1) {
                tail += 1;
                true
            } else {
                false
            };
            if !grown {
                break;
            }
        }
        let start: String = chars[..head].iter().collect();
        let end: String = chars[chars.len() - tail..].iter().collect();
        Cow::Owned(start + dots + &end)
    }

    fn dots(&self, style: TextStyle) -> &'static str {
        if self.face(style).glyph_id('…').0 != 0 {
            "…"
        } else {
            "..."
        }
    }

    /// The x of every caret position in `text`, as (byte index, x).
    pub fn carets(&self, style: TextStyle, text: &str) -> Vec<(usize, f32)> {
        let face = self.face(style);
        let scaled = face.as_scaled(PxScale::from(style.px()));
        let mut x = 0.0;
        let mut carets = Vec::with_capacity(text.len() + 1);
        for (index, id, kern) in advances(face, &scaled, text) {
            x += kern;
            carets.push((index, x));
            x += scaled.h_advance(id);
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
        for (_, id, kern) in advances(face, &scaled, text) {
            caret += kern;
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
