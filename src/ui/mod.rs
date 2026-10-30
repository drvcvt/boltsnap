//! Pixels and input for the Boltsnap window, free of any windowing system:
//! the platform turns its events into [`Input`] and shows the [`Canvas`].

pub mod icons;
pub mod motion;
pub mod text;
pub mod theme;
pub mod widgets;

#[cfg(test)]
mod tests;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

pub use theme::Rgb;

/// Modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Enter,
    Space,
    Escape,
    Backspace,
    Delete,
    /// A letter or digit pressed with Ctrl, lower case.
    Char(char),
}

/// The pointer's look the UI asks the window for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerShape {
    #[default]
    Default,
    /// Over something that drags sideways, such as the shelf preview's
    /// measure.
    ResizeX,
}

/// What the window hands the UI, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Move {
        x: f32,
        y: f32,
    },
    /// The pointer left the window, or the window lost it to a grab.
    Leave,
    /// `clicks` is 2 for the second press of a double click.
    Press {
        x: f32,
        y: f32,
        clicks: u8,
    },
    Release,
    /// Whole wheel `notches`, or touchpad `pixels` when `notches` is 0.
    Wheel {
        notches: i32,
        pixels: f32,
    },
    Key {
        key: Key,
        mods: Mods,
    },
    /// Typed text, without control characters.
    Text(String),
    /// The clipboard's text after an [`Effect::Paste`].
    Pasted(String),
}

/// What the UI asks the window to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Copy(String),
    Paste,
    Close,
    /// The value of the row in this block of the current page changed.
    Changed(usize),
    /// The row in this block asks for its key to go back to the default.
    Reset(usize),
    /// A folder field asks for a folder dialog; the app names its row as
    /// [`Effect::PickFolder`].
    Browse,
    /// The folder field in this block of the current page wants a folder.
    PickFolder(usize),
}

/// Counts a press as the second of a double click when it comes soon after
/// the last one, near it. Times are the windowing system's milliseconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clicks {
    last: Option<(u32, f32, f32, u8)>,
}

impl Clicks {
    const WITHIN_MS: u32 = 400;
    const NEAR: f32 = 4.0;

    pub fn press(&mut self, time: u32, x: f32, y: f32) -> u8 {
        let count = match self.last {
            Some((at, lx, ly, count))
                if time.wrapping_sub(at) <= Self::WITHIN_MS
                    && (x - lx).abs() <= Self::NEAR
                    && (y - ly).abs() <= Self::NEAR =>
            {
                count.saturating_add(1)
            }
            _ => 1,
        };
        self.last = Some((time, x, y, count));
        count
    }
}

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn overlaps(self, other: Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    pub fn inset(self, by: f32) -> Rect {
        Rect::new(
            self.x + by,
            self.y + by,
            self.w - 2.0 * by,
            self.h - 2.0 * by,
        )
    }

    /// A `w`×`h` box centred in this one.
    pub fn centered(self, w: f32, h: f32) -> Rect {
        Rect::new(
            self.x + (self.w - w) / 2.0,
            self.y + (self.h - h) / 2.0,
            w,
            h,
        )
    }
}

/// The window's pixels. Callers draw in logical pixels; `scale` maps them to
/// the buffer.
pub struct Canvas {
    pub pixmap: Pixmap,
    pub scale: f32,
    /// Where the pixmap's first pixel is in the window, in device pixels:
    /// (0, 0) but for a [`Canvas::part`].
    pub origin: (i32, i32),
}

impl Canvas {
    /// `None` for an empty size.
    pub fn new(width: u32, height: u32, scale: u32) -> Option<Self> {
        let scale = scale.max(1);
        Some(Self {
            pixmap: Pixmap::new(width * scale, height * scale)?,
            scale: scale as f32,
            origin: (0, 0),
        })
    }

    /// A canvas for `rect` of this one alone, 2 device pixels larger on
    /// every side and on whole device pixels; `None` when that misses this
    /// canvas. Shifted by whole pixels only, what is drawn into it comes out
    /// as the same pixels it would here (settings paint speed plan, stage D);
    /// [`Canvas::paste`] puts it back.
    pub fn part(&self, rect: Rect) -> Option<Canvas> {
        let (w, h) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
        let device = |v: f32| v * self.scale;
        let left = (device(rect.x).floor() as i32 - 2 - self.origin.0).max(0);
        let top = (device(rect.y).floor() as i32 - 2 - self.origin.1).max(0);
        let right = (device(rect.right()).ceil() as i32 + 2 - self.origin.0).min(w);
        let bottom = (device(rect.bottom()).ceil() as i32 + 2 - self.origin.1).min(h);
        if right <= left || bottom <= top {
            return None;
        }
        Some(Canvas {
            pixmap: Pixmap::new((right - left) as u32, (bottom - top) as u32)?,
            scale: self.scale,
            origin: (self.origin.0 + left, self.origin.1 + top),
        })
    }

    /// `part`'s pixels in place of these where it lies.
    pub fn paste(&mut self, part: &Canvas) {
        let dx = (part.origin.0 - self.origin.0) as usize;
        let dy = (part.origin.1 - self.origin.1) as usize;
        let width = self.pixmap.width() as usize;
        let span = part.pixmap.width() as usize * 4;
        let data = self.pixmap.data_mut();
        for (row, line) in part.pixmap.data().chunks_exact(span).enumerate() {
            let at = ((dy + row) * width + dx) * 4;
            data[at..at + span].copy_from_slice(line);
        }
    }

    /// What the pixmap covers, in logical pixels of the window.
    pub fn bounds(&self) -> Rect {
        Rect::new(
            self.origin.0 as f32 / self.scale,
            self.origin.1 as f32 / self.scale,
            self.width(),
            self.height(),
        )
    }

    pub fn width(&self) -> f32 {
        self.pixmap.width() as f32 / self.scale
    }

    pub fn height(&self) -> f32 {
        self.pixmap.height() as f32 / self.scale
    }

    pub fn clear(&mut self, color: Rgb) {
        self.pixmap
            .fill(tiny_skia::Color::from_rgba8(color.0, color.1, color.2, 255));
    }

    /// Take everything in `rect` towards the opaque `color`, keeping `keep`
    /// (0 to 1) of what is there: what a layer of `color` at `1 − keep`
    /// would do, without blending the pixels that are `color` already.
    /// Dithered and in fine steps: a flat area between two levels shows both
    /// in the right share, so a fade glides instead of stepping level by
    /// level, and its end does not wait in the spring's slow tail.
    pub fn fade(&mut self, rect: Rect, color: Rgb, keep: f32) {
        let keep = (keep.clamp(0.0, 1.0) * FADE_ONE as f32).round() as u32;
        if keep >= FADE_ONE {
            return;
        }
        let width = self.pixmap.width() as usize;
        let height = self.pixmap.height() as usize;
        let device = |v: f32, origin: i32, max: usize| {
            (((v * self.scale).round() - origin as f32).max(0.0) as usize).min(max)
        };
        let (ox, oy) = self.origin;
        let (left, right) = (device(rect.x, ox, width), device(rect.right(), ox, width));
        let (top, bottom) = (
            device(rect.y, oy, height),
            device(rect.bottom(), oy, height),
        );
        let target = [color.0, color.1, color.2, 255];
        let data = self.pixmap.data_mut();
        for row in top..bottom {
            let pixels = data[(row * width + left) * 4..(row * width + right) * 4]
                .as_chunks_mut::<4>()
                .0;
            for (column, pixel) in (left..).zip(pixels) {
                if *pixel == target {
                    continue;
                }
                let threshold = dither(column as i32 + ox, row as i32 + oy);
                for (channel, to) in pixel.iter_mut().zip(target) {
                    *channel =
                        ((*channel as u32 * keep + to as u32 * (FADE_ONE - keep) + threshold)
                            / FADE_ONE) as u8;
                }
            }
        }
    }

    pub(crate) fn transform(&self) -> Transform {
        let scale = Transform::from_scale(self.scale, self.scale);
        match self.origin {
            (0, 0) => scale,
            (x, y) => scale.post_translate(-x as f32, -y as f32),
        }
    }

    /// A band `width` wide just inside `rect`'s rounded outline.
    pub fn ring(&mut self, rect: Rect, radius: f32, width: f32, color: Rgb, alpha: f32) {
        if rect.w <= 2.0 * width || rect.h <= 2.0 * width || alpha <= 0.0 {
            return;
        }
        let mut pb = PathBuilder::new();
        push_squircle(&mut pb, rect, radius);
        push_squircle(&mut pb, rect.inset(width), (radius - width).max(0.0));
        let Some(path) = pb.finish() else {
            return;
        };
        let transform = self.transform();
        self.pixmap.fill_path(
            &path,
            &paint(color, alpha),
            FillRule::EvenOdd,
            transform,
            None,
        );
    }

    /// Fill `rect` with corners of `radius`, at `alpha` (0 to 1).
    pub fn fill(&mut self, rect: Rect, radius: f32, color: Rgb, alpha: f32) {
        if rect.w <= 0.0 || rect.h <= 0.0 || alpha <= 0.0 {
            return;
        }
        let Some(path) = rounded_rect(rect, radius) else {
            return;
        };
        let transform = self.transform();
        self.pixmap.fill_path(
            &path,
            &paint(color, alpha),
            FillRule::Winding,
            transform,
            None,
        );
    }
}

/// `Canvas::fade`'s whole: its mix is exact to 1/4096.
const FADE_ONE: u32 = 4096;

/// The rounding threshold (in 1/`FADE_ONE` of a level) of the window pixel
/// at `(x, y)`: a 4 × 4 ordered dither whose thresholds average one half, so
/// a mix keeps its exact value on average.
fn dither(x: i32, y: i32) -> u32 {
    const BAYER: [u32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    (BAYER[((y & 3) * 4 + (x & 3)) as usize] * 2 + 1) * FADE_ONE / 32
}

pub(crate) fn paint(color: Rgb, alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        color.0,
        color.1,
        color.2,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    paint.anti_alias = true;
    paint
}

/// A rectangle with squircle corners of the radius token `radius` (see
/// [`push_squircle`]): every surface the window draws.
pub(crate) fn rounded_rect(rect: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    push_squircle(&mut pb, rect, radius);
    pb.finish()
}

/// R for a radius token r: (1 − 1/√2) / (1 − 2^(−1/4)).
pub const SQUIRCLE_SCALE: f32 = 1.840_896_4;
/// The corner exponent while R is free.
const SQUIRCLE_N: f32 = 4.0;
/// Largest distance of a flattened corner from the true curve, in logical
/// pixels: 0.1 device pixels at scale 4, the largest the window renders.
pub const SQUIRCLE_TOLERANCE: f32 = 0.025;

/// The corner of a radius token `radius` on a `w`×`h` rectangle, as the
/// half axis R and the exponent n of the superellipse |x/R|ⁿ + |y/R|ⁿ = 1
/// whose quadrant forms it.
///
/// With n = 4 the curvature is 0 where the straight edge runs into the
/// corner, as the edge's own, so there is no kink. R puts the corner's
/// 45° point exactly where the circular corner of `radius` would: the circle
/// sits r(1 − 1/√2) in from both edges there, the superellipse R(1 − 2^(−1/n));
/// equal for R = r · (1 − 1/√2) / (1 − 2^(−1/4)) ≈ 1.84 r. Each token keeps
/// its perceived size, the curve is smooth.
///
/// R is at most half the shorter side. Clamped there, n follows so the 45°
/// point still matches: 1 − 2^(−1/n) = r(1 − 1/√2)/R, so
/// n = −ln 2 / ln(1 − r(1 − 1/√2)/R). With r at half the side that is n = 2,
/// a circle: pills and knobs stay round, and between the two the shape moves
/// steadily from n = 4 to n = 2.
pub fn squircle_shape(radius: f32, w: f32, h: f32) -> (f32, f32) {
    let half = (w.min(h) / 2.0).max(0.0);
    let r = radius.min(half).max(0.0);
    if r <= 0.0 {
        return (0.0, SQUIRCLE_N);
    }
    let free = r * SQUIRCLE_SCALE;
    if free <= half {
        return (free, SQUIRCLE_N);
    }
    let inset = r * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
    let n = -std::f32::consts::LN_2 / (1.0 - inset / half).ln();
    (half, n.clamp(2.0, SQUIRCLE_N))
}

/// `rect` with squircle corners (see [`squircle_shape`]), clockwise from the
/// top edge. Each quadrant is flattened into lines, halving until the curve
/// strays less than [`SQUIRCLE_TOLERANCE`] from each one; straight edges
/// stay exactly where `rect` puts them.
pub fn push_squircle(pb: &mut PathBuilder, rect: Rect, radius: f32) {
    let (big, n) = squircle_shape(radius, rect.w, rect.h);
    let (x, y, right, bottom) = (rect.x, rect.y, rect.right(), rect.bottom());
    if big <= 0.0 {
        pb.move_to(x, y);
        pb.line_to(right, y);
        pb.line_to(right, bottom);
        pb.line_to(x, bottom);
        pb.close();
        return;
    }
    let half_pi = std::f32::consts::FRAC_PI_2;
    // Corner centre, the axis the quadrant starts (θ = 0) and ends on
    // (θ = π/2), and which way θ runs.
    let corners = [
        (
            (right - big, y + big),
            (1.0, 0.0),
            (0.0, -1.0),
            (half_pi, 0.0),
        ),
        (
            (right - big, bottom - big),
            (1.0, 0.0),
            (0.0, 1.0),
            (0.0, half_pi),
        ),
        (
            (x + big, bottom - big),
            (-1.0, 0.0),
            (0.0, 1.0),
            (half_pi, 0.0),
        ),
        ((x + big, y + big), (-1.0, 0.0), (0.0, -1.0), (0.0, half_pi)),
    ];
    pb.move_to(x + big, y);
    CORNERS.with_borrow_mut(|corners_of| {
        // Animated sizes make new shapes every frame; they do not pile up.
        if corners_of.len() > 256 {
            corners_of.clear();
        }
        for ((cx, cy), (ax, ay), (bx, by), (from, to)) in corners {
            let points = corners_of
                .entry((big.to_bits(), n.to_bits(), from > to))
                .or_insert_with(|| corner(big, n, from, to));
            for (u, v) in points.iter() {
                pb.line_to(cx + u * ax + v * bx, cy + u * ay + v * by);
            }
        }
    });
    pb.close();
}

/// Flattened corners by R, n and direction (see [`corner`]).
type Corners = std::collections::HashMap<(u32, u32, bool), Vec<(f32, f32)>>;

thread_local! {
    /// Each corner shape once, by R, n and direction (settings paint speed
    /// plan, stage C).
    static CORNERS: std::cell::RefCell<Corners> = Default::default();
}

/// One quadrant of the superellipse with half axis `big` and exponent `n`,
/// from angle `from` to `to`, flattened (see [`push_squircle`]): its points
/// from the corner's centre, along the two axes it runs between.
fn corner(big: f32, n: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    // Polar form: the ray at angle t meets the curve at
    // R / (cosⁿ t + sinⁿ t)^(1/n); smooth in t, unlike cos^(2/n) t.
    let at = |t: f32| {
        let (sin, cos) = t.sin_cos();
        let (cos, sin) = (cos.max(0.0), sin.max(0.0));
        let k = big / (cos.powf(n) + sin.powf(n)).powf(1.0 / n);
        (cos * k, sin * k)
    };
    let start = at(from);
    let mut points = vec![start];
    flatten(&mut points, &at, (from, start), (to, at(to)), 0);
    points
}

/// Points along `at` from `a` to `b` (parameter and point), halving until
/// the curve's middle is within the tolerance of the chord. At least four
/// pieces, so a quadrant cannot pass as straight from its ends and middle
/// alone.
fn flatten(
    points: &mut Vec<(f32, f32)>,
    at: &impl Fn(f32) -> (f32, f32),
    a: (f32, (f32, f32)),
    b: (f32, (f32, f32)),
    depth: u32,
) {
    let t = (a.0 + b.0) / 2.0;
    let mid = at(t);
    let chord = ((a.1.0 + b.1.0) / 2.0, (a.1.1 + b.1.1) / 2.0);
    let off = (mid.0 - chord.0).hypot(mid.1 - chord.1);
    if depth >= 14 || (depth >= 2 && off <= SQUIRCLE_TOLERANCE) {
        points.push(b.1);
        return;
    }
    flatten(points, at, a, (t, mid), depth + 1);
    flatten(points, at, (t, mid), b, depth + 1);
}

/// Circular corners as cubics, for Lucide's `<rect rx>` in `icons.rs`: icons
/// stay exactly as Lucide draws them.
pub(crate) fn push_rounded(pb: &mut PathBuilder, rect: Rect, radius: f32) {
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    // Cubic handle length for a quarter circle.
    let k = r * 0.552_284_8;
    let (x, y, right, bottom) = (rect.x, rect.y, rect.right(), rect.bottom());
    pb.move_to(x + r, y);
    pb.line_to(right - r, y);
    pb.cubic_to(right - r + k, y, right, y + r - k, right, y + r);
    pb.line_to(right, bottom - r);
    pb.cubic_to(
        right,
        bottom - r + k,
        right - r + k,
        bottom,
        right - r,
        bottom,
    );
    pb.line_to(x + r, bottom);
    pb.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
}
