//! Rectangles the selector snaps to while Ctrl is held (snap plan,
//! `docs/plans/selector/2026-10-27-snap-detection.md`): the one the image's
//! own long straight edges enclose around the pointer (an embedded video, a
//! panel), then the windows, then the screen. Platform neutral: the windows
//! and screens come from the platform, in surface pixels.

use image::RgbaImage;

use super::edit::Rect;

/// A brightness step between neighbours that counts as an edge, in 8-bit
/// levels (probe P6: the video against the page's black and the header's
/// `#272727`).
const STEP: i16 = 20;
/// Share of a side's span an edge has to run unbroken: content inside a
/// video (a lamp, a book) has edges, but none as long as its border.
const RUN: f32 = 0.9;
/// Half the box the search starts from: small, so a pointer next to a corner
/// does not start out across the border.
const START: u32 = 6;
const ROUNDS: usize = 8;
/// A box smaller than this is a letter or an icon, nothing to clip.
pub const MIN_SIDE: u32 = 48;
/// At most this many boxes inside one another.
const LEVELS: usize = 6;

/// Left, top, right, bottom in image pixels, right and bottom exclusive.
pub type Box = (u32, u32, u32, u32);

/// Where an image has edges, as prefix sums: `cols` counts, per column
/// boundary `x` (between columns `x - 1` and `x`), the rows down to `y` with
/// an edge there; `rows` the same along each row boundary. A run's share over
/// any span is then two reads.
pub struct Edges {
    w: u32,
    h: u32,
    cols: Vec<u16>,
    rows: Vec<u16>,
}

impl Edges {
    /// `None` for an image larger than the prefix sums count (65 535 px a
    /// side).
    pub fn new(image: &RgbaImage) -> Option<Self> {
        let (w, h) = image.dimensions();
        if w < 2 || h < 2 || w > u16::MAX as u32 || h > u16::MAX as u32 {
            return None;
        }
        let luma: Vec<i16> = image
            .pixels()
            .map(|p| {
                let [r, g, b, _] = p.0;
                ((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) as i16
            })
            .collect();
        let (wu, hu) = (w as usize, h as usize);
        let at = |x: usize, y: usize| luma[y * wu + x];
        // cols[x * (h + 1) + y]: edges at boundary x in rows 0..y.
        let mut cols = vec![0u16; (wu + 1) * (hu + 1)];
        for x in 1..wu {
            let base = x * (hu + 1);
            for y in 0..hu {
                let edge = (at(x, y) - at(x - 1, y)).abs() > STEP;
                cols[base + y + 1] = cols[base + y] + u16::from(edge);
            }
        }
        let mut rows = vec![0u16; (hu + 1) * (wu + 1)];
        for y in 1..hu {
            let base = y * (wu + 1);
            for x in 0..wu {
                let edge = (at(x, y) - at(x, y - 1)).abs() > STEP;
                rows[base + x + 1] = rows[base + x] + u16::from(edge);
            }
        }
        Some(Self { w, h, cols, rows })
    }

    /// An edge runs down boundary `x` over rows `top..bottom`.
    fn column_runs(&self, x: u32, top: u32, bottom: u32) -> bool {
        let base = x as usize * (self.h as usize + 1);
        let n = self.cols[base + bottom as usize] - self.cols[base + top as usize];
        f32::from(n) >= RUN * (bottom - top) as f32
    }

    /// An edge runs along boundary `y` over columns `left..right`.
    fn row_runs(&self, y: u32, left: u32, right: u32) -> bool {
        let base = y as usize * (self.w as usize + 1);
        let n = self.rows[base + right as usize] - self.rows[base + left as usize];
        f32::from(n) >= RUN * (right - left) as f32
    }

    /// The boxes (left, top, right, bottom; right and bottom exclusive) the
    /// long edges enclose around `(x, y)`, innermost first: a panel in a
    /// page, the video around a sharp shape in it. Each is at least
    /// [`MIN_SIDE`] a side and has a real edge on all four sides (the
    /// screen is its own level).
    pub fn around(&self, (x, y): (u32, u32)) -> Vec<Box> {
        let (w, h) = (self.w, self.h);
        let mut boxes = Vec::new();
        if x >= w || y >= h {
            return boxes;
        }
        let Some(mut inner) = self.innermost((x, y)) else {
            return boxes;
        };
        while boxes.len() < LEVELS {
            boxes.push(inner);
            let (l, t, r, b) = inner;
            // The next one out: the search starts beyond this one's edges.
            let next = self.settle(
                (l.saturating_sub(1), t.saturating_sub(1), r + 1, b + 1),
                inner,
            );
            if next == inner || !self.closed(next) {
                break;
            }
            inner = next;
        }
        boxes
    }

    /// From a small box around the point, each side walks out to the first
    /// boundary whose edge runs along the box found so far, again until
    /// nothing moves. A box too small to mean anything (the pointer on a
    /// letter) starts again from itself grown by [`MIN_SIDE`], so the
    /// letter's short edges no longer count.
    fn innermost(&self, (x, y): (u32, u32)) -> Option<Box> {
        let (w, h) = (self.w, self.h);
        let mut start = (
            x.saturating_sub(START),
            y.saturating_sub(START),
            (x + START).min(w),
            (y + START).min(h),
        );
        loop {
            let found = self.settle((x, y, x + 1, y + 1), start);
            let (left, top, right, bottom) = found;
            if right - left >= MIN_SIDE && bottom - top >= MIN_SIDE {
                return self.closed(found).then_some(found);
            }
            let grown = (
                left.saturating_sub(MIN_SIDE),
                top.saturating_sub(MIN_SIDE),
                (right + MIN_SIDE).min(w),
                (bottom + MIN_SIDE).min(h),
            );
            if grown == start {
                return None;
            }
            start = grown;
        }
    }

    /// Every side of `found` is a real edge, none the image's border: a
    /// band between two lines that run across a window (a terminal's
    /// rules) is no rectangle, and through a translucent window's sides it
    /// would reach the screen's edge (snap plan, Nachtrag 28.10.).
    fn closed(&self, (left, top, right, bottom): Box) -> bool {
        left > 0 && top > 0 && right < self.w && bottom < self.h
    }

    /// Walk the left and top sides out from boundaries `from.0`, `from.1`
    /// and the right and bottom from `from.2`, `from.3`, measuring along
    /// `start`, then along what they found, until nothing moves.
    fn settle(&self, from: Box, start: Box) -> Box {
        let (w, h) = (self.w, self.h);
        let mut found = start;
        for _ in 0..ROUNDS {
            let (_, top, _, bottom) = found;
            let left = (1..=from.0)
                .rev()
                .find(|&b| self.column_runs(b, top, bottom))
                .unwrap_or(0);
            let right = (from.2..w)
                .find(|&b| self.column_runs(b, top, bottom))
                .unwrap_or(w);
            let top = (1..=from.1)
                .rev()
                .find(|&b| self.row_runs(b, left, right))
                .unwrap_or(0);
            let bottom = (from.3..h)
                .find(|&b| self.row_runs(b, left, right))
                .unwrap_or(h);
            let next = (left, top, right, bottom);
            if next == found {
                break;
            }
            found = next;
        }
        found
    }
}

/// Two rectangles the same to the eye: every edge within 4 px.
fn same(a: &Rect, b: &Rect) -> bool {
    (a.x - b.x).abs() <= 4.0
        && (a.y - b.y).abs() <= 4.0
        && (a.right() - b.right()).abs() <= 4.0
        && (a.bottom() - b.bottom()).abs() <= 4.0
}

/// A box inside a window has to be this share of the screen's width and of
/// its height to be offered: a video, a mini player, a dialog or a panel
/// are; a button, a text field, a tab or a thin bar are not (Nutzer 28.10.:
/// „nicht so viel bullshit … mit so einem threshhold (kein hoher)“).
const MIN_SHARE: f64 = 0.1;

/// The levels at `point`, innermost first: what the edges enclose (if large
/// enough on this screen), the windows, the screen; only those holding the
/// point, each once, by size.
pub fn levels(inner: &[Rect], windows: &[Rect], screen: Rect, point: (f64, f64)) -> Vec<Rect> {
    // (rect, known exactly): windows and the screen are, the edges' boxes
    // are a reading of the picture.
    let large = |r: &&Rect| r.w >= screen.w * MIN_SHARE && r.h >= screen.h * MIN_SHARE;
    let mut all: Vec<(Rect, bool)> = inner
        .iter()
        .filter(large)
        .map(|&r| (r, false))
        .chain(windows.iter().map(|&r| (r, true)))
        .chain(std::iter::once((screen, true)))
        .filter(|(r, _)| r.w > 0.0 && r.h > 0.0 && r.contains(point))
        .collect();
    all.sort_by(|(a, _), (b, _)| (a.w * a.h).total_cmp(&(b.w * b.h)));
    let mut out: Vec<(Rect, bool)> = Vec::new();
    for (r, exact) in all {
        match out.iter_mut().find(|(kept, _)| same(kept, &r)) {
            // The same to the eye: a window's own rectangle beats the
            // picture's reading of its edges.
            Some(kept) if exact && !kept.1 => *kept = (r, true),
            Some(_) => {}
            None => out.push((r, exact)),
        }
    }
    out.into_iter().map(|(r, _)| r).collect()
}

#[cfg(test)]
#[path = "snap_tests.rs"]
mod tests;
