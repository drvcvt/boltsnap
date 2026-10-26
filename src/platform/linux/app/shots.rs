//! The newest screenshots in the save folder for the previews (settings
//! plan, 5e): PNG only, newest first, read and shrunk once, never a new
//! capture. Nothing leaves the machine.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use boltsnap::app::shots::{self, CARD_PX, SCREEN_MAX, Shots};
use boltsnap::ui::picture::Picture;
use image::ImageDecoder;

/// The most files looked at: the newest ones only.
const MOST_FILES: usize = 16;
/// The most cards the shelf preview shows.
const MOST_CARDS: usize = 8;
/// What one screenshot may take decoded, as the shelf allows
/// (`linux/shelf/cards.rs`), and its pixels.
const MOST_BYTES: u64 = 256 * 1024 * 1024;
const MOST_PIXELS: u64 = MOST_BYTES / 4;

/// Load on a thread and send what came of it.
pub(super) fn spawn(dir: PathBuf, done: calloop::channel::Sender<Shots>) {
    std::thread::spawn(move || {
        let _ = done.send(load(&dir));
    });
}

/// The newest PNG files in `dir`, newest first.
fn newest(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        })
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            (meta.is_file() && meta.len() <= MOST_BYTES).then_some((meta.modified().ok()?, path))
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files
        .into_iter()
        .take(MOST_FILES)
        .map(|(_, path)| path)
        .collect()
}

/// A decoder for `bytes` within the limits, and the image's size.
fn decoder(bytes: &[u8]) -> Option<(impl ImageDecoder + '_, (u32, u32))> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MOST_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let size = decoder.dimensions();
    (u64::from(size.0) * u64::from(size.1) <= MOST_PIXELS).then_some((decoder, size))
}

/// `image` cut to the shape of `size` and shrunk to it, as a picture.
fn picture(image: &image::DynamicImage, (w, h): (u32, u32)) -> Option<Picture> {
    let rgba = crate::shelf::thumbnail::make_image_card_thumbnail(image, w, h);
    let (w, h) = rgba.dimensions();
    Picture::from_rgba(w, h, rgba.into_raw())
}

/// The newest screenshot of a whole screen in `dir` and the newest others
/// for the cards; files that cannot be read are skipped.
pub fn load(dir: &Path) -> Shots {
    let mut found = Shots::default();
    for path in newest(dir) {
        let cards_full = found.cards.len() >= MOST_CARDS;
        if cards_full && found.screen.is_some() {
            break;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some((decoder, size)) = decoder(&bytes) else {
            continue;
        };
        let screen = found.screen.is_none() && shots::whole_screen(size);
        if !screen && cards_full {
            continue;
        }
        let Ok(image) = image::DynamicImage::from_decoder(decoder) else {
            continue;
        };
        if screen {
            found.screen = picture(&image, shots::fit(size, SCREEN_MAX));
        } else if let Some(card) = picture(&image, CARD_PX) {
            found.cards.push(card);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_whole_screen_is_the_screen_and_the_others_are_cards() {
        let dir = std::env::temp_dir().join(format!("boltsnap-shots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Written in any order; the times say which is newest.
        let write = |name: &str, (w, h): (u32, u32), age: u64| {
            let path = dir.join(name);
            image::RgbImage::from_pixel(w, h, image::Rgb([200, 40, 40]))
                .save_with_format(&path, image::ImageFormat::Png)
                .unwrap();
            let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000 - age);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        };
        write("area-new.png", (400, 150), 10);
        write("screen.PNG", (1920, 1080), 20);
        write("area-old.png", (300, 200), 30);
        std::fs::write(dir.join("broken.png"), b"not a png").unwrap();
        std::fs::write(dir.join("clip.mp4"), b"video").unwrap();
        let shots = load(&dir);
        assert_eq!(shots.screen.as_ref().map(Picture::size), Some((1600, 900)));
        let cards: Vec<_> = shots.cards.iter().map(Picture::size).collect();
        assert_eq!(
            cards,
            vec![CARD_PX, CARD_PX],
            "two areas, the broken one skipped"
        );
        // Nothing there, or only videos: nothing to show and no error.
        assert!(load(&dir.join("missing")).cards.is_empty());
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
            {
                std::fs::remove_file(path).unwrap();
            }
        }
        let only_video = load(&dir);
        assert!(only_video.screen.is_none() && only_video.cards.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
