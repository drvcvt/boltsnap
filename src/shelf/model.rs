use std::path::PathBuf;

use image::RgbaImage;

pub struct Thumb {
    pub id: u64,
    pub png_path: PathBuf,
    pub thumb: RgbaImage,
    /// Capture mode label ("area"/"full"/…). Retained with the thumbnail for
    /// future display/grouping; not surfaced in the UI yet.
    #[allow(dead_code)]
    pub source: String,
}

#[derive(Default)]
pub struct ShelfModel {
    thumbs: Vec<Thumb>, // index 0 = newest
    next_id: u64,
}

impl ShelfModel {
    pub fn new() -> Self {
        Self { thumbs: Vec::new(), next_id: 1 }
    }

    /// Insert a new thumbnail at the top of the shelf; returns its id.
    pub fn add(&mut self, png_path: PathBuf, thumb: RgbaImage, source: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.thumbs.insert(0, Thumb { id, png_path, thumb, source });
        id
    }

    pub fn remove(&mut self, id: u64) -> Option<Thumb> {
        let pos = self.thumbs.iter().position(|t| t.id == id)?;
        Some(self.thumbs.remove(pos))
    }

    pub fn get(&self, id: u64) -> Option<&Thumb> {
        self.thumbs.iter().find(|t| t.id == id)
    }

    pub fn replace_thumb(&mut self, id: u64, thumb: RgbaImage) -> bool {
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == id) {
            t.thumb = thumb;
            true
        } else {
            false
        }
    }

    pub fn is_empty(&self) -> bool {
        self.thumbs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.thumbs.len()
    }

    /// Iterate newest-first (top of the shelf first).
    pub fn newest_first(&self) -> impl Iterator<Item = &Thumb> {
        self.thumbs.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img() -> RgbaImage {
        RgbaImage::new(2, 2)
    }

    #[test]
    fn add_assigns_unique_ids_newest_first() {
        let mut m = ShelfModel::new();
        let a = m.add(PathBuf::from("/tmp/a.png"), img(), "area".into());
        let b = m.add(PathBuf::from("/tmp/b.png"), img(), "full".into());
        assert_ne!(a, b);
        let ids: Vec<u64> = m.newest_first().map(|t| t.id).collect();
        assert_eq!(ids, vec![b, a]); // newest first
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn remove_returns_thumb_and_shrinks() {
        let mut m = ShelfModel::new();
        let a = m.add(PathBuf::from("/tmp/a.png"), img(), "area".into());
        let removed = m.remove(a).unwrap();
        assert_eq!(removed.png_path, PathBuf::from("/tmp/a.png"));
        assert!(m.is_empty());
        assert!(m.remove(a).is_none());
    }

    #[test]
    fn replace_thumb_swaps_image_keeps_id() {
        let mut m = ShelfModel::new();
        let a = m.add(PathBuf::from("/tmp/a.png"), RgbaImage::new(2, 2), "area".into());
        assert!(m.replace_thumb(a, RgbaImage::new(4, 4)));
        assert_eq!(m.get(a).unwrap().thumb.dimensions(), (4, 4));
        assert!(!m.replace_thumb(999, RgbaImage::new(1, 1)));
    }
}
