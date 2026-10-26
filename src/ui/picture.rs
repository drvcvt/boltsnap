//! A picture the window shows, such as a screenshot in a preview: its
//! premultiplied pixels at a few sizes, each half the one before, so drawing
//! never shrinks one by more than half and fine detail does not shimmer.

use tiny_skia::{FillRule, FilterQuality, IntSize, Paint, Pattern, Pixmap, SpreadMode, Transform};

use super::{Canvas, Rect, rounded_rect};

/// No step narrower than this is kept.
const SMALLEST: u32 = 64;

pub struct Picture {
    /// Largest first.
    levels: Vec<Pixmap>,
}

impl Picture {
    /// From straight (not premultiplied) RGBA, `width × height`; `None` when
    /// the size is empty or does not match the pixels.
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Option<Picture> {
        let size = IntSize::from_wh(width, height)?;
        for pixel in rgba.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            if alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
        }
        let mut levels = vec![Pixmap::from_vec(rgba, size)?];
        while let Some(next) = levels.last().and_then(half) {
            levels.push(next);
        }
        Some(Picture { levels })
    }

    /// The full size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.levels[0].width(), self.levels[0].height())
    }

    /// How many sizes it keeps.
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// The step to draw `width` device pixels wide from: the smallest that
    /// is not narrower.
    fn level(&self, width: f32) -> &Pixmap {
        self.levels
            .iter()
            .rev()
            .find(|level| level.width() as f32 >= width)
            .unwrap_or(&self.levels[0])
    }
}

/// `pixmap` at half its size, each pixel the mean of four; `None` below
/// `SMALLEST`.
fn half(pixmap: &Pixmap) -> Option<Pixmap> {
    let (w, h) = (pixmap.width() / 2, pixmap.height() / 2);
    if w < SMALLEST || h == 0 {
        return None;
    }
    let from = pixmap.data();
    let stride = pixmap.width() as usize * 4;
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = 2 * y * stride + 2 * x * 4;
            for channel in 0..4 {
                let sum = u32::from(from[at + channel])
                    + u32::from(from[at + 4 + channel])
                    + u32::from(from[at + stride + channel])
                    + u32::from(from[at + stride + 4 + channel]);
                data.push(((sum + 2) / 4) as u8);
            }
        }
    }
    Pixmap::from_vec(data, IntSize::from_wh(w, h)?)
}

impl Canvas {
    /// `picture` stretched over `place`, shown only inside `clip` with corners
    /// of `radius`, at `alpha` (0 to 1). A `place` larger than `clip` crops.
    pub fn picture(&mut self, picture: &Picture, place: Rect, clip: Rect, radius: f32, alpha: f32) {
        if alpha <= 0.0 || place.w <= 0.0 || place.h <= 0.0 || clip.w <= 0.0 || clip.h <= 0.0 {
            return;
        }
        let level = picture.level(place.w * self.scale);
        let Some(path) = rounded_rect(clip, radius) else {
            return;
        };
        // Pattern pixels to logical pixels; the canvas scale follows.
        let pattern = Transform::from_row(
            place.w / level.width() as f32,
            0.0,
            0.0,
            place.h / level.height() as f32,
            place.x,
            place.y,
        );
        let paint = Paint {
            shader: Pattern::new(
                level.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Bilinear,
                alpha,
                pattern,
            ),
            anti_alias: true,
            ..Paint::default()
        };
        let transform = self.transform();
        self.pixmap
            .fill_path(&path, &paint, FillRule::Winding, transform, None);
    }
}
