//! Pixels and input for the Boltsnap window, free of any windowing system:
//! the platform turns its events into [`Input`] and shows the [`Canvas`].

pub mod icons;
pub mod motion;
pub mod text;
pub mod theme;

#[cfg(test)]
mod tests;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

pub use theme::Rgb;

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
}

impl Canvas {
    /// `None` for an empty size.
    pub fn new(width: u32, height: u32, scale: u32) -> Option<Self> {
        let scale = scale.max(1);
        Some(Self {
            pixmap: Pixmap::new(width * scale, height * scale)?,
            scale: scale as f32,
        })
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

    pub(crate) fn transform(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
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

/// A rounded rectangle with circular corners (radius clamped to half the
/// shorter side).
pub(crate) fn rounded_rect(rect: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    push_rounded(&mut pb, rect, radius);
    pb.finish()
}

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
