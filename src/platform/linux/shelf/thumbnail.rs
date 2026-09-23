use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::Duration;

use super::{DaemonEvent, MAX_CACHED_CARDS};
use crate::shelf::thumbnail::{CARD_H, CARD_W};

pub(super) struct Worker(SyncSender<(u64, PathBuf)>);

impl Worker {
    pub(super) fn spawn(events: calloop::channel::Sender<DaemonEvent>) -> Self {
        let (sender, receiver) = sync_channel::<(u64, PathBuf)>(MAX_CACHED_CARDS);
        std::thread::spawn(move || {
            while let Ok((id, path)) = receiver.recv() {
                let image = extract(&path).ok();
                if events
                    .send(DaemonEvent::ThumbnailReady {
                        id,
                        image,
                        _permit: None,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self(sender)
    }

    pub(super) fn submit(&self, id: u64, path: PathBuf) {
        // Keep the existing placeholder when the bounded preview queue is full.
        let _ = self.0.try_send((id, path));
    }
}

fn extract(path: &Path) -> Result<image::RgbaImage, String> {
    let filter = format!(
        "scale={CARD_W}:{CARD_H}:force_original_aspect_ratio=increase,crop={CARD_W}:{CARD_H},setsar=1"
    );
    let result = super::super::replay::process::output_bounded(
        Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-threads", "2", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-vf",
                &filter,
                "-filter_threads",
                "1",
                "-frames:v",
                "1",
                "-threads:v",
                "1",
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .stdin(Stdio::null()),
        Duration::from_secs(10),
        (CARD_W * CARD_H * 4) as usize,
    )?;
    if !result.status.success() {
        return Err("video thumbnail decoding failed".into());
    }
    image::RgbaImage::from_raw(CARD_W, CARD_H, result.stdout).ok_or("invalid thumbnail size".into())
}

// Compatibility for an already-running older capture client. Normal thumbnails
// use raw RGBA over a pipe and never create an intermediate PNG file.
pub(super) fn load_legacy(path: &Path) -> Result<image::RgbaImage, String> {
    let result = (|| {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(super::MAX_CACHED_IMAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > super::MAX_CACHED_IMAGE_BYTES {
            return Err("thumbnail PNG is too large".into());
        }
        let image = super::decode_shelf_image(&bytes)?;
        Ok(crate::shelf::thumbnail::make_image_card_thumbnail(
            &image, CARD_W, CARD_H,
        ))
    })();
    // Preserve the old temporary-thumbnail lifecycle.
    let _ = std::fs::remove_file(path);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires the FFmpeg CLI; leaves its synthetic fixture in the temp directory"]
    fn synthetic_video_produces_a_small_rgba_thumbnail() {
        // Optional external-tool regression, run explicitly with --ignored.
        let path = crate::paths::temp_file("thumbnail-test", "mkv");
        let output = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=640x360",
                "-frames:v",
                "1",
                "-c:v",
                "ffv1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let image = super::extract(&path).unwrap();
        assert_eq!(image.dimensions(), (super::CARD_W, super::CARD_H));
        assert!(
            image
                .pixels()
                .all(|p| p[0] > 240 && p[1] < 10 && p[2] < 10 && p[3] == 255)
        );
        assert!(super::extract(&path.with_extension("missing")).is_err());
    }
}
