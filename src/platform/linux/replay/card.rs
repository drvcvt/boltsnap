//! The card a save shows before its clip is written (replay plan §13 C1):
//! the frozen frame the selector picked from, cut like the export.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

/// The last frozen frame as it went to the selector, (snapshot, PNG): one,
/// replaced by the next freeze, taken by its save, dropped by its cancel.
static FROZEN: Mutex<Option<(u64, Vec<u8>)>> = Mutex::new(None);
static TOKENS: AtomicU64 = AtomicU64::new(1);

pub(super) fn remember(snapshot: u64, png: &[u8]) {
    *FROZEN.lock().unwrap() = Some((snapshot, png.to_vec()));
}

pub(super) fn forget(snapshot: u64) {
    let mut frozen = FROZEN.lock().unwrap();
    if frozen.as_ref().is_some_and(|(id, _)| *id == snapshot) {
        *frozen = None;
    }
}

/// Ties a save's pending card to its clip.
pub(super) fn token() -> u64 {
    TOKENS.fetch_add(1, Ordering::Relaxed)
}

/// The card picture for save `request` in `size`: its snapshot's frozen
/// frame cut to its crop; `None` for a full-screen save or without the frame.
pub(super) fn picture(request: &Value, size: (u32, u32)) -> Option<image::RgbaImage> {
    let snapshot = request["snapshot"].as_u64()?;
    let png = {
        let mut frozen = FROZEN.lock().unwrap();
        match frozen.take() {
            Some((id, png)) if id == snapshot => png,
            other => {
                *frozen = other;
                return None;
            }
        }
    };
    if request["crop"].is_null() {
        return None;
    }
    region_card(&png, &request["crop"], size)
}

/// `png` cut to `crop` the way the worker cuts the clip (`Crop::from_value`,
/// 4:2:0 edges), then made a card like every other.
fn region_card(png: &[u8], crop: &Value, (w, h): (u32, u32)) -> Option<image::RgbaImage> {
    let frame = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let crop =
        boltsnap::replay::crop::Crop::from_value(crop, frame.width(), frame.height()).ok()?;
    let cut = image::imageops::crop_imm(&frame, crop.x, crop.y, crop.width, crop.height).to_image();
    Some(crate::shelf::thumbnail::make_card_thumbnail(&cut, w, h))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{forget, picture, region_card, remember};

    fn halves() -> Vec<u8> {
        // Left half red, right half blue.
        let frame = image::RgbaImage::from_fn(64, 32, |x, _| {
            image::Rgba(if x < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        });
        let mut png = Vec::new();
        frame
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    #[test]
    fn the_card_shows_the_part_of_the_frame_the_clip_keeps() {
        let crop = json!({"x": 32, "y": 0, "width": 32, "height": 32});
        let card = region_card(&halves(), &crop, (20, 10)).unwrap();
        assert_eq!(card.dimensions(), (20, 10));
        assert!(card.pixels().all(|px| px.0 == [0, 0, 255, 255]));
        // A crop the clip would refuse makes no card either.
        let outside = json!({"x": 60, "y": 0, "width": 32, "height": 32});
        assert!(region_card(&halves(), &outside, (20, 10)).is_none());
    }

    #[test]
    fn only_the_save_of_the_frozen_snapshot_takes_its_frame() {
        let crop = json!({"x": 0, "y": 0, "width": 32, "height": 32});
        remember(5, &halves());
        assert!(picture(&json!({"snapshot": 4, "crop": crop}), (20, 10)).is_none());
        // Still there for the right save, then gone.
        assert!(picture(&json!({"snapshot": 5, "crop": crop}), (20, 10)).is_some());
        assert!(picture(&json!({"snapshot": 5, "crop": crop}), (20, 10)).is_none());
        remember(6, &halves());
        forget(6);
        assert!(picture(&json!({"snapshot": 6, "crop": crop}), (20, 10)).is_none());
    }
}
