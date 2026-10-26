//! Measuring and drawing text in the type scale.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont, point};

use super::theme::TextStyle;
use super::{Canvas, Rect, Rgb};

/// The DejaVu subset the selector and the recording controls fall back to
/// (printable ASCII and ×).
const FALLBACK: &[u8] = include_bytes!("../../assets/fonts/dejavu-badge.ttf");

/// `c`'s glyph; a no-break space or minus sign the face lacks is drawn as a
/// space or a hyphen.
fn glyph(face: &FontVec, c: char) -> GlyphId {
    match face.glyph_id(c) {
        GlyphId(0) if c == '\u{a0}' => face.glyph_id(' '),
        GlyphId(0) if c == '\u{2212}' => face.glyph_id('-'),
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

/// Glyph origins are placed to a quarter of a device pixel horizontally;
/// baselines sit on whole pixels.
const SUBPIXEL: f32 = 4.0;

/// (face, glyph, size bits, quarter-pixel offset).
type GlyphKey = (bool, u16, u32, u8);

/// A rasterized glyph: its coverage, `w` wide, placed at (`x`, `y`) from
/// its origin's whole pixel.
struct Coverage {
    x: i32,
    y: i32,
    w: usize,
    alpha: Vec<f32>,
}

/// The UI face at the two weights the scale uses.
pub struct Fonts {
    medium: FontVec,
    demibold: FontVec,
    /// Capital height per unit of `PxScale`, for each face.
    caps: [f32; 2],
    /// Every glyph is rasterized once per size and offset, not every frame.
    glyphs: RefCell<HashMap<GlyphKey, Option<Coverage>>>,
    /// A number of its own: what was measured with one `Fonts` can be kept
    /// until the window loads another.
    id: u64,
}

/// How tall `H` is against the scale ab_glyph sizes by (ascent − descent).
fn cap_ratio(face: &FontVec) -> f32 {
    face.outline(face.glyph_id('H'))
        .map(|outline| outline.bounds.height().abs() / face.height_unscaled())
        .filter(|ratio| *ratio > 0.0)
        .unwrap_or(0.7)
}

impl Fonts {
    pub fn new(medium: FontVec, demibold: FontVec) -> Self {
        let caps = [cap_ratio(&medium), cap_ratio(&demibold)];
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            medium,
            demibold,
            caps,
            glyphs: RefCell::new(HashMap::new()),
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Which loaded font this is (see `id`).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The size `style` draws at: the one that makes its capitals exactly
    /// `style.cap()` pixels tall, so with a whole-pixel baseline their top
    /// and bottom edges fall on pixel rows (a size hint for unhinted faces).
    pub fn size(&self, style: TextStyle) -> f32 {
        style.cap() / self.caps[usize::from(style.strong())]
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
        let scaled = face.as_scaled(PxScale::from(self.size(style)));
        advances(face, &scaled, text)
            .map(|(_, id, kern)| kern + scaled.h_advance(id))
            .sum()
    }

    /// Height of a capital (an `H` sits on the baseline).
    pub fn cap_height(&self, style: TextStyle) -> f32 {
        let face = self.face(style);
        let scaled = face.as_scaled(PxScale::from(self.size(style)));
        face.outline(face.glyph_id('H'))
            .map(|outline| outline.bounds.height().abs() * scaled.v_scale_factor())
            .unwrap_or(style.cap())
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

    /// `text` in lines at most `width` wide, at most `max_lines` of them.
    /// Lines break between words; a word that does not fit on a line of its
    /// own breaks after its last `/`, `-` or `_` that fits, or between
    /// characters. Text left over after `max_lines` ends the last line
    /// with an ellipsis.
    pub fn wrap(&self, style: TextStyle, text: &str, width: f32, max_lines: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in text.split(' ') {
            let joined = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if self.width(style, &joined) <= width {
                line = joined;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            while !rest.is_empty() && self.width(style, rest) > width {
                let cut = self.cut(style, rest, width);
                lines.push(rest[..cut].to_owned());
                rest = &rest[cut..];
            }
            line = rest.to_owned();
        }
        if !line.is_empty() || lines.is_empty() {
            lines.push(line);
        }
        let max_lines = max_lines.max(1);
        if lines.len() > max_lines {
            let rest = lines.split_off(max_lines - 1).join(" ");
            lines.push(self.ellipsize(style, &rest, width).into_owned());
        }
        lines
    }

    /// Where to cut `word` so its start fits `width`: after the last `/`,
    /// `-` or `_` that fits, else after the last character that fits, and
    /// after one character at least.
    fn cut(&self, style: TextStyle, word: &str, width: f32) -> usize {
        let mut fit = 0;
        let mut soft = None;
        // Carets are in order: (byte index, x) before each character, then
        // the end; one pass finds the last that fits.
        for (index, x) in self.carets(style, word).into_iter().skip(1) {
            if x > width {
                break;
            }
            fit = index;
            let before = word[..index].chars().next_back();
            if matches!(before, Some('/' | '-' | '_')) && index < word.len() {
                soft = Some(index);
            }
        }
        let first = word.chars().next().map_or(word.len(), char::len_utf8);
        soft.unwrap_or(fit).max(first)
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
        let scaled = face.as_scaled(PxScale::from(self.size(style)));
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
        let scale = PxScale::from(self.size(style) * canvas.scale);
        let scaled = face.as_scaled(scale);
        let width = canvas.pixmap.width() as i32;
        let device = |v: f32| (v * canvas.scale).round() as i32;
        let (left, top) = (device(clip.x).max(0), device(clip.y).max(0));
        let right = device(clip.right()).min(width);
        let bottom = device(clip.bottom()).min(canvas.pixmap.height() as i32);
        let data = canvas.pixmap.data_mut();
        // Start and baseline on whole device pixels: a fractional baseline
        // smears every horizontal edge over two rows.
        let mut caret = (x * canvas.scale).round();
        let baseline = (baseline * canvas.scale).round() as i32;
        let mut glyphs = self.glyphs.borrow_mut();
        for (_, id, kern) in advances(face, &scaled, text) {
            caret += kern;
            let whole = caret.floor();
            let quarter = ((caret - whole) * SUBPIXEL).round();
            let (whole, quarter) = if quarter >= SUBPIXEL {
                (whole + 1.0, 0.0)
            } else {
                (whole, quarter)
            };
            caret += scaled.h_advance(id);
            let key = (style.strong(), id.0, scale.y.to_bits(), quarter as u8);
            let coverage = glyphs.entry(key).or_insert_with(|| {
                let at = point(quarter / SUBPIXEL, 0.0);
                let outlined = face.outline_glyph(id.with_scale_and_position(scale, at))?;
                let bounds = outlined.px_bounds();
                let w = bounds.width() as usize;
                let mut alpha = vec![0.0; w * bounds.height() as usize];
                outlined.draw(|gx, gy, c| alpha[gy as usize * w + gx as usize] = c);
                Some(Coverage {
                    x: bounds.min.x as i32,
                    y: bounds.min.y as i32,
                    w,
                    alpha,
                })
            });
            let Some(coverage) = coverage else { continue };
            let (ox, oy) = (whole as i32 + coverage.x, baseline + coverage.y);
            for (row, line) in coverage.alpha.chunks_exact(coverage.w.max(1)).enumerate() {
                let py = oy + row as i32;
                if py < top || py >= bottom {
                    continue;
                }
                for (col, c) in line.iter().enumerate() {
                    let px = ox + col as i32;
                    if *c <= 0.0 || px < left || px >= right {
                        continue;
                    }
                    let a = (c * alpha).clamp(0.0, 1.0);
                    let i = ((py * width + px) * 4) as usize;
                    // Premultiplied source-over with an opaque colour.
                    let over = |s: u8, d: u8| (s as f32 * a + d as f32 * (1.0 - a)).round() as u8;
                    data[i] = over(color.0, data[i]);
                    data[i + 1] = over(color.1, data[i + 1]);
                    data[i + 2] = over(color.2, data[i + 2]);
                    data[i + 3] = over(255, data[i + 3]);
                }
            }
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
