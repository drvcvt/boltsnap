use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::{DaemonEvent, MAX_CACHED_CARDS};
use crate::shelf::thumbnail::CardSize;

/// What a card's thumbnail is rendered from.
pub(super) enum Source {
    Image(PathBuf),
    Video(PathBuf),
}

enum Job {
    Video(u64, PathBuf),
    /// Cards waiting in the re-render list.
    Rerender,
}

/// Renders card thumbnails off the event loop, one at a time, at the card
/// size current when the job runs.
pub(super) struct Worker {
    queue: SyncSender<Job>,
    /// Cards to render again at a new card size. A list instead of queue
    /// entries, so a full queue never drops the newest size.
    rerender: Arc<Mutex<Vec<(u64, Source)>>>,
}

impl Worker {
    pub(super) fn spawn(
        events: calloop::channel::Sender<DaemonEvent>,
        size: impl Fn() -> CardSize + Send + 'static,
    ) -> Self {
        let (queue, receiver) = sync_channel::<Job>(MAX_CACHED_CARDS);
        let rerender = Arc::new(Mutex::new(Vec::<(u64, Source)>::new()));
        let list = rerender.clone();
        std::thread::spawn(move || {
            let send = |id, image| {
                events
                    .send(DaemonEvent::ThumbnailReady {
                        id,
                        image,
                        _permit: None,
                    })
                    .is_ok()
            };
            while let Ok(job) = receiver.recv() {
                if let Job::Video(id, path) = job {
                    let image = {
                        let _timing = super::super::timing::Span::new("video_thumbnail");
                        extract(&path, size()).ok()
                    };
                    if !send(id, image) {
                        return;
                    }
                }
                // After every job, so a wake-up lost to a full queue is
                // made up by the job that filled it.
                loop {
                    let next = {
                        let mut list = list.lock().unwrap_or_else(PoisonError::into_inner);
                        (!list.is_empty()).then(|| list.remove(0))
                    };
                    let Some((id, source)) = next else {
                        break;
                    };
                    if !send(id, render(&source, size()).ok()) {
                        return;
                    }
                }
            }
        });
        Self { queue, rerender }
    }

    pub(super) fn submit(&self, id: u64, path: PathBuf) {
        // Keep the existing placeholder when the bounded preview queue is full.
        let _ = self.queue.try_send(Job::Video(id, path));
    }

    /// Renders these cards again; a card already waiting is not rendered
    /// twice.
    pub(super) fn rerender(&self, cards: impl IntoIterator<Item = (u64, Source)>) {
        {
            let mut list = self.rerender.lock().unwrap_or_else(PoisonError::into_inner);
            for (id, source) in cards {
                list.retain(|(waiting, _)| *waiting != id);
                list.push((id, source));
            }
        }
        let _ = self.queue.try_send(Job::Rerender);
    }
}

fn render(source: &Source, size: CardSize) -> Result<image::RgbaImage, String> {
    match source {
        Source::Video(path) => extract(path, size),
        Source::Image(path) => {
            let png = std::fs::read(path).map_err(|error| error.to_string())?;
            Ok(crate::shelf::thumbnail::make_image_card_thumbnail(
                &super::cards::decode_shelf_image(&png)?,
                size.w,
                size.h,
            ))
        }
    }
}

fn extract(path: &Path, size: CardSize) -> Result<image::RgbaImage, String> {
    let CardSize { w, h } = size;
    let filter =
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1");
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
        (w * h * 4) as usize,
        Some(super::super::replay::process::BACKGROUND),
    )?;
    if !result.status.success() {
        return Err("video thumbnail decoding failed".into());
    }
    image::RgbaImage::from_raw(w, h, result.stdout).ok_or("invalid thumbnail size".into())
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
        let size = super::CardSize::from_width(300).unwrap();
        let image = super::extract(&path, size).unwrap();
        assert_eq!(image.dimensions(), size.dims());
        assert!(
            image
                .pixels()
                .all(|p| p[0] > 240 && p[1] < 10 && p[2] < 10 && p[3] == 255)
        );
        assert!(super::extract(&path.with_extension("missing"), size).is_err());
    }
}
