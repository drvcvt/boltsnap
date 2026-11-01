//! Caller damage normalization and the short frame history that tells a reused
//! SHM slot which regions changed since the frame it still holds.
use super::layout::Layout;
use crate::{Error, Result};
use std::collections::VecDeque;

/// More rectangles than this collapse to their bounding box.
pub(crate) const MAX_DAMAGE_RECTS: usize = 16;
/// Equal to the slot ring size, so a slot holding the oldest buffer is covered.
const HISTORY_FRAMES: usize = 3;

/// A rectangle in buffer pixels. Damage lists are clamped to the buffer; a rect
/// with zero width or height, or fully outside the buffer, is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    /// Left edge in buffer pixels.
    pub x: u32,
    /// Top edge in buffer pixels.
    pub y: u32,
    /// Width in buffer pixels.
    pub width: u32,
    /// Height in buffer pixels.
    pub height: u32,
}

impl PixelRect {
    /// Create a rectangle from its top-left corner and size.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn clamped(self, width: u32, height: u32) -> Option<Self> {
        let right = self.x.saturating_add(self.width).min(width);
        let bottom = self.y.saturating_add(self.height).min(height);
        (self.x < right && self.y < bottom)
            .then(|| Self::new(self.x, self.y, right - self.x, bottom - self.y))
    }

    pub(super) fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }

    pub(super) fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }

    fn union(self, other: Self) -> Self {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(x, y, right - x, bottom - y)
    }
}

/// Clamp, drop empty rects and collapse beyond [`MAX_DAMAGE_RECTS`]. `None`
/// means the whole buffer. Fully empty input is the caller's error.
pub(crate) fn normalize(damage: Option<&[PixelRect]>, layout: Layout) -> Result<Vec<PixelRect>> {
    // Layout dimensions are validated positive i32 values.
    let (width, height) = (layout.width as u32, layout.height as u32);
    let Some(damage) = damage else {
        return Ok(vec![PixelRect::new(0, 0, width, height)]);
    };
    let mut rects = Vec::with_capacity(MAX_DAMAGE_RECTS);
    let (mut count, mut bounds) = (0_usize, None::<PixelRect>);
    for rect in damage.iter().filter_map(|rect| rect.clamped(width, height)) {
        count += 1;
        bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
        if rects.len() < MAX_DAMAGE_RECTS {
            rects.push(rect);
        }
    }
    let bounds = bounds.ok_or(Error::InvalidInput("empty CPU damage"))?;
    if count > MAX_DAMAGE_RECTS {
        rects.clear();
        rects.push(bounds);
    }
    Ok(rects)
}

/// Surface state under which damage rects of consecutive frames are comparable.
pub(crate) type HistoryKey = (u32, u32, u32, u32, u32);

/// Damage of the most recent presented frames, ascending by sequence.
#[derive(Default)]
pub(crate) struct History {
    key: Option<HistoryKey>,
    frames: VecDeque<(u64, Vec<PixelRect>)>,
}

impl History {
    /// Whether the previously committed buffer is the frame before `sequence`
    /// under the same surface state, so compositor damage may stay partial.
    pub(crate) fn follows(&self, key: HistoryKey, sequence: u64) -> bool {
        self.key == Some(key)
            && self
                .frames
                .back()
                .is_some_and(|(last, _)| last.checked_add(1) == Some(sequence))
    }

    /// Rects to rewrite in a slot that still holds frame `valid`, or `None` for
    /// a full copy: a fresh slot, or a missing frame between it and `sequence`.
    pub(crate) fn stale(
        &self,
        key: HistoryKey,
        valid: Option<u64>,
        sequence: u64,
        current: &[PixelRect],
    ) -> Option<Vec<PixelRect>> {
        let valid = valid.filter(|valid| *valid < sequence)?;
        let mut rects = current.to_vec();
        for missed in valid + 1..sequence {
            let (_, damage) = self
                .frames
                .iter()
                .find(|(seen, _)| *seen == missed && self.key == Some(key))?;
            rects.extend_from_slice(damage);
        }
        Some(rects)
    }

    pub(crate) fn record(&mut self, key: HistoryKey, sequence: u64, damage: &[PixelRect]) {
        if self.key != Some(key) {
            self.frames.clear();
            self.key = Some(key);
        }
        if self.frames.len() == HISTORY_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back((sequence, damage.to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new(10, 8).unwrap()
    }

    #[test]
    fn rects_clamp_ignore_empties_and_reject_all_empty_lists() {
        let list = [
            PixelRect::new(8, 6, 100, 100),
            PixelRect::new(2, 2, 0, 5),
            PixelRect::new(10, 0, 3, 3),
            PixelRect::new(u32::MAX, 0, 9, 9),
        ];
        assert_eq!(
            normalize(Some(&list), layout()).unwrap(),
            [PixelRect::new(8, 6, 2, 2)]
        );
        assert!(normalize(Some(&list[1..]), layout()).is_err());
        assert!(normalize(Some(&[]), layout()).is_err());
        assert_eq!(
            normalize(None, layout()).unwrap(),
            [PixelRect::new(0, 0, 10, 8)]
        );
    }

    #[test]
    fn more_than_sixteen_rects_collapse_but_sixteen_stay() {
        let list: Vec<_> = (0..17)
            .map(|i| PixelRect::new(i % 9, i % 7, 1, 1))
            .collect();
        assert_eq!(normalize(Some(&list[..16]), layout()).unwrap().len(), 16);
        assert_eq!(
            normalize(Some(&list), layout()).unwrap(),
            [PixelRect::new(0, 0, 9, 7)]
        );
    }

    #[test]
    fn stale_union_needs_every_intermediate_frame_under_one_key() {
        let key = (1, 1, 1, 1, 120);
        let rect = |x| [PixelRect::new(x, 0, 1, 1)];
        let mut history = History::default();
        for sequence in 1..=3 {
            history.record(key, sequence, &rect(sequence as u32));
        }
        assert!(history.follows(key, 4));
        assert!(!history.follows(key, 5));
        assert!(!history.follows((2, 1, 1, 1, 120), 4));
        let current = rect(9);
        assert_eq!(history.stale(key, None, 4, &current), None);
        assert_eq!(history.stale(key, Some(3), 4, &current).unwrap(), current);
        assert_eq!(history.stale(key, Some(1), 4, &current).unwrap().len(), 3);
        // Frame 1 leaves the ring when frame 4 arrives; a slot holding it needs 2..=4.
        history.record(key, 4, &rect(4));
        assert_eq!(history.stale(key, Some(1), 5, &current).unwrap().len(), 4);
        assert_eq!(history.stale(key, Some(0), 5, &current), None);
        assert_eq!(history.stale((2, 1, 1, 1, 120), Some(3), 5, &current), None);
        history.record((2, 1, 1, 1, 120), 5, &current);
        let after = history.stale((2, 1, 1, 1, 120), Some(4), 6, &current);
        assert_eq!(after.unwrap().len(), 2);
    }
}
