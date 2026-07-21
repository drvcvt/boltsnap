use std::path::PathBuf;

use image::RgbaImage;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardKind {
    Image,
    Video,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileLifetime {
    Temporary,
    Permanent,
}

pub struct Thumb {
    pub id: u64,
    pub png_path: PathBuf,
    pub thumb: RgbaImage,
    /// Capture mode label ("area"/"full"/…). Retained with the thumbnail for
    /// future display/grouping; not surfaced in the UI yet.
    #[allow(dead_code)]
    pub source: String,
    pub kind: CardKind,
    pub lifetime: FileLifetime,
}

impl Thumb {
    pub fn delete_file_on_dismiss(&self) -> std::io::Result<()> {
        if self.lifetime == FileLifetime::Temporary {
            std::fs::remove_file(&self.png_path)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
pub struct ShelfModel {
    thumbs: Vec<Thumb>, // index 0 = newest
    next_id: u64,
}

impl ShelfModel {
    pub fn new() -> Self {
        Self {
            thumbs: Vec::new(),
            next_id: 1,
        }
    }

    /// Insert a new card of the given kind at the top of the shelf; returns its id.
    pub fn add_kind(
        &mut self,
        png_path: PathBuf,
        thumb: RgbaImage,
        source: String,
        kind: CardKind,
    ) -> u64 {
        self.add_kind_with_lifetime(png_path, thumb, source, kind, FileLifetime::Temporary)
    }

    pub fn add_kind_with_lifetime(
        &mut self,
        png_path: PathBuf,
        thumb: RgbaImage,
        source: String,
        kind: CardKind,
        lifetime: FileLifetime,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.thumbs.insert(
            0,
            Thumb {
                id,
                png_path,
                thumb,
                source,
                kind,
                lifetime,
            },
        );
        id
    }

    /// Insert an image card (screenshot). Convenience over `add_kind`.
    pub fn add(&mut self, png_path: PathBuf, thumb: RgbaImage, source: String) -> u64 {
        self.add_kind(png_path, thumb, source, CardKind::Image)
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

    pub fn replace_path_with_lifetime(
        &mut self,
        id: u64,
        path: PathBuf,
        lifetime: FileLifetime,
    ) -> bool {
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == id) {
            let previous_lifetime = t.lifetime;
            let previous = std::mem::replace(&mut t.png_path, path);
            t.lifetime = lifetime;
            if previous_lifetime == FileLifetime::Temporary && previous != t.png_path {
                let _ = std::fs::remove_file(previous);
            }
            true
        } else {
            false
        }
    }

    pub fn promote(&mut self, id: u64, path: PathBuf) -> bool {
        let Some(card) = self.thumbs.iter_mut().find(|card| card.id == id) else {
            return false;
        };
        if card.lifetime == FileLifetime::Permanent {
            return false;
        }
        card.png_path = path;
        card.lifetime = FileLifetime::Permanent;
        true
    }

    /// Used by tests and reserved for shelf-state queries; not yet read by the binary.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.thumbs.is_empty()
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.thumbs.len()
    }

    /// Iterate newest-first (top of the shelf first).
    pub fn newest_first(&self) -> impl Iterator<Item = &Thumb> {
        self.thumbs.iter()
    }

    pub fn temporary_video_bytes(&self) -> u64 {
        self.thumbs
            .iter()
            .filter(|card| card.kind == CardKind::Video && card.lifetime == FileLifetime::Temporary)
            .filter_map(|card| std::fs::metadata(&card.png_path).ok())
            .map(|metadata| metadata.len())
            .fold(0, u64::saturating_add)
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
    fn add_carries_kind() {
        let mut m = ShelfModel::new();
        let v = m.add_kind(
            PathBuf::from("/tmp/v.mp4"),
            img(),
            "record".into(),
            CardKind::Video,
        );
        assert_eq!(m.get(v).unwrap().kind, CardKind::Video);
        let i = m.add(PathBuf::from("/tmp/i.png"), img(), "area".into());
        assert_eq!(m.get(i).unwrap().kind, CardKind::Image);
        assert_eq!(m.get(i).unwrap().lifetime, FileLifetime::Temporary);
    }

    #[test]
    fn replace_thumb_swaps_image_keeps_id() {
        let mut m = ShelfModel::new();
        let a = m.add(
            PathBuf::from("/tmp/a.png"),
            RgbaImage::new(2, 2),
            "area".into(),
        );
        assert!(m.replace_thumb(a, RgbaImage::new(4, 4)));
        assert_eq!(m.get(a).unwrap().thumb.dimensions(), (4, 4));
        assert!(!m.replace_thumb(999, RgbaImage::new(1, 1)));
    }

    #[test]
    fn replace_path_keeps_card_identity() {
        let mut m = ShelfModel::new();
        let id = m.add_kind(
            PathBuf::from("/tmp/original.mp4"),
            img(),
            "record".into(),
            CardKind::Video,
        );

        assert!(m.replace_path_with_lifetime(
            id,
            PathBuf::from("/tmp/edited.mp4"),
            FileLifetime::Temporary
        ));
        assert_eq!(
            m.get(id).unwrap().png_path,
            PathBuf::from("/tmp/edited.mp4")
        );
        assert_eq!(m.get(id).unwrap().kind, CardKind::Video);
        assert!(!m.replace_path_with_lifetime(
            999,
            PathBuf::from("/tmp/missing.mp4"),
            FileLifetime::Temporary
        ));
    }

    #[test]
    fn replace_path_removes_previous_temporary_video() {
        let dir = std::env::temp_dir().join(format!(
            "boltsnap-replace-test-{}-{}",
            std::process::id(),
            crate::paths::timestamp()
        ));
        std::fs::create_dir(&dir).unwrap();
        let original = dir.join("original.mp4");
        let edited = dir.join("edited.mp4");
        std::fs::write(&original, b"old").unwrap();
        std::fs::write(&edited, b"new").unwrap();
        let mut model = ShelfModel::new();
        let id = model.add_kind(original.clone(), img(), "record".into(), CardKind::Video);

        assert!(model.replace_path_with_lifetime(id, edited.clone(), FileLifetime::Temporary));
        assert!(!original.exists());
        assert!(edited.is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn permanent_file_survives_card_dismissal() {
        let dir = (0_u32..)
            .map(|n| {
                std::env::temp_dir()
                    .join(format!("boltsnap-lifetime-test-{}-{n}", std::process::id()))
            })
            .find_map(|path| match std::fs::create_dir(&path) {
                Ok(()) => Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => panic!("create unique test directory: {error}"),
            })
            .unwrap();
        let permanent = dir.join("permanent.mp4");
        let temporary = dir.join("temporary.mp4");
        std::fs::write(&permanent, b"keep").unwrap();
        std::fs::write(&temporary, b"remove").unwrap();

        let mut model = ShelfModel::new();
        let keep = model.add_kind_with_lifetime(
            permanent.clone(),
            img(),
            "record".into(),
            CardKind::Video,
            FileLifetime::Permanent,
        );
        let remove = model.add_kind(temporary.clone(), img(), "record".into(), CardKind::Video);
        assert_eq!(model.temporary_video_bytes(), 6);
        model
            .remove(keep)
            .unwrap()
            .delete_file_on_dismiss()
            .unwrap();
        model
            .remove(remove)
            .unwrap()
            .delete_file_on_dismiss()
            .unwrap();

        assert!(permanent.is_file());
        assert!(!temporary.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn promotion_updates_path_and_lifetime_only_once() {
        let mut model = ShelfModel::new();
        let id = model.add_kind(
            PathBuf::from("/tmp/temporary.mp4"),
            img(),
            "record".into(),
            CardKind::Video,
        );

        assert!(model.promote(id, PathBuf::from("/videos/saved.mp4")));
        assert_eq!(
            model.get(id).map(|card| (&card.png_path, card.lifetime)),
            Some((&PathBuf::from("/videos/saved.mp4"), FileLifetime::Permanent))
        );
        assert!(!model.promote(id, PathBuf::from("/videos/duplicate.mp4")));
        assert_eq!(
            model.get(id).unwrap().png_path,
            PathBuf::from("/videos/saved.mp4")
        );
    }
}
