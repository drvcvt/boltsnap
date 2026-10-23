//! What `try_wait` cannot see in a running recording: a recorder that hangs
//! without exiting, and free space running out before the finished recording
//! can still be saved.

use crate::record::finalize::{RECORDING_DISK_RESERVE_BYTES, remux_space};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// gsr writes constant-rate Matroska and flushes a cluster every 100 ms, so a
/// segment that stops growing for this long has a recorder that stopped encoding.
pub(super) const STALL: Duration = Duration::from_secs(6);

/// Space a check may lose before the next one, at well over any bitrate gsr uses.
const SLACK_BYTES: u64 = 256 * 1024 * 1024;
/// Warn once this much before the recording would pause.
const WARN_AHEAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Watch {
    sizes: HashMap<PathBuf, (u64, Instant)>,
    warned: bool,
    /// `record_dir` lies on another filesystem than the cache.
    separate_disk: Option<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Space {
    Fine,
    /// Say so once: the recording pauses when free space falls to `stop_at`.
    Low {
        free: u64,
        stop_at: u64,
    },
    Stop(String),
}

impl Watch {
    /// A new recording starts with a clean slate.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Note `size` of an active gsr segment; true once it has not grown for `STALL`.
    pub fn stalled(&mut self, path: &Path, size: u64, now: Instant) -> bool {
        match self.sizes.get_mut(path) {
            Some((last, since)) if size > *last => {
                *last = size;
                *since = now;
                false
            }
            Some((_, since)) => now.saturating_duration_since(*since) >= STALL,
            None => {
                self.sizes.insert(path.to_owned(), (size, now));
                false
            }
        }
    }

    /// Forget segments that are no longer recording.
    pub fn keep_only(&mut self, active: &[&Path]) {
        self.sizes
            .retain(|path, _| active.contains(&path.as_path()));
    }

    pub fn separate_disk(&mut self, cache: &Path, record_dir: impl FnOnce() -> PathBuf) -> bool {
        *self.separate_disk.get_or_insert_with(|| {
            use std::os::unix::fs::MetadataExt;
            let device = |path: &Path| std::fs::metadata(path).map(|meta| meta.dev()).ok();
            let dir = record_dir();
            // A directory that does not exist yet is created next to its parent.
            let target = device(&dir).or_else(|| dir.parent().and_then(device));
            target.is_some_and(|target| Some(target) != device(cache))
        })
    }

    /// `recorded` bytes so far; `remux_on_cache`: saving joins them in the cache.
    pub fn space(&mut self, free: u64, recorded: u64, remux_on_cache: bool) -> Space {
        let stop_at = stop_at(recorded, remux_on_cache);
        if free <= stop_at {
            return Space::Stop(format!(
                "recording paused so it can still be saved: {} free, {} needed",
                gigabytes(free),
                gigabytes(stop_at)
            ));
        }
        if !self.warned && free <= stop_at + WARN_AHEAD_BYTES {
            self.warned = true;
            return Space::Low { free, stop_at };
        }
        Space::Fine
    }
}

impl super::Daemon {
    /// Once a second while recording: a hung recorder, the cache quota and
    /// free space. `Some` is the controls' title and why the recording has to pause.
    pub(super) fn check_recording(&mut self) -> Option<(&'static str, String)> {
        let now = Instant::now();
        let session = self.recording.as_ref()?;
        let dir = crate::paths::rec_dir();
        let size = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
        let active: Vec<&Path> = session.active.iter().map(|r| r.path.as_path()).collect();
        self.watch.keep_only(&active);
        for path in &active {
            // wf-recorder only writes when the screen changes.
            let gsr = path.extension().is_some_and(|ext| ext == "mkv");
            if gsr && self.watch.stalled(path, size(path), now) {
                return Some((
                    "Recorder stalled",
                    format!(
                        "the recorder stopped writing video for {} s",
                        STALL.as_secs()
                    ),
                ));
            }
        }
        let long = session.is_long(now);
        // A long recording is headed for disk, not the temporary cache.
        if !long && let Err(error) = crate::record::finalize::check_recording_cache_limit(&dir) {
            return Some(("Cache full", error));
        }
        let recorded: u64 = active.iter().map(|path| size(path)).sum::<u64>()
            + session
                .completed
                .values()
                .flatten()
                .map(|p| size(p))
                .sum::<u64>();
        let outputs: std::collections::BTreeSet<_> = session
            .active
            .iter()
            .map(|r| &r.output)
            .chain(session.completed.keys())
            .collect();
        // Separate outputs combined at save time are joined in the cache first.
        let composed =
            session.both_mode == crate::config::RecordBothMode::Combined && outputs.len() > 1;
        let remux_on_cache = !long
            || composed
            || !self.watch.separate_disk(&dir, || {
                crate::config::resolve_record_dir(&crate::config::Config::load())
            });
        let free = match crate::record::finalize::available_space(&dir) {
            Ok(free) => free,
            Err(error) => return Some(("Disk check failed", error)),
        };
        match self.watch.space(free, recorded, remux_on_cache) {
            Space::Fine => None,
            Space::Low { free, stop_at } => {
                super::notify(&format!(
                    "Disk space is low: {} free. The recording pauses at {} so it can still be saved.",
                    gigabytes(free),
                    gigabytes(stop_at)
                ));
                None
            }
            Space::Stop(error) => Some(("Low disk space", error)),
        }
    }
}

/// Free space at which a recording pauses: the system reserve, or room to
/// remux what was recorded so far when that remux shares the cache's disk.
fn stop_at(recorded: u64, remux_on_cache: bool) -> u64 {
    let remux = if remux_on_cache {
        remux_space(recorded)
    } else {
        0
    };
    RECORDING_DISK_RESERVE_BYTES.max(remux) + SLACK_BYTES
}

pub(super) fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn a_segment_that_stops_growing_stalls_after_the_window() {
        let mut watch = Watch::default();
        let path = Path::new("/cache/boltsnap-segment-1-1-DP_3.mkv");
        let start = Instant::now();
        assert!(!watch.stalled(path, 0, start));
        assert!(!watch.stalled(path, 0, start + STALL - Duration::from_millis(1)));
        assert!(!watch.stalled(path, 10, start + STALL));
        // Growth restarts the window.
        assert!(!watch.stalled(path, 10, start + STALL * 2 - Duration::from_millis(1)));
        assert!(watch.stalled(path, 10, start + STALL * 2));
        watch.keep_only(&[]);
        assert!(!watch.stalled(path, 10, start + STALL * 3));
    }

    #[test]
    fn small_recordings_keep_the_reserve_and_large_ones_room_to_remux() {
        assert_eq!(stop_at(0, true), 2 * GIB + SLACK_BYTES);
        assert_eq!(stop_at(10 * GIB, false), 2 * GIB + SLACK_BYTES);
        assert_eq!(stop_at(10 * GIB, true), remux_space(10 * GIB) + SLACK_BYTES);
        assert!(stop_at(10 * GIB, true) > 10 * GIB + 10 * GIB / 20);
    }

    #[test]
    fn low_space_warns_once_then_stops() {
        let mut watch = Watch::default();
        let stop = stop_at(GIB, true);
        assert_eq!(watch.space(stop + 3 * GIB, GIB, true), Space::Fine);
        assert_eq!(
            watch.space(stop + GIB, GIB, true),
            Space::Low {
                free: stop + GIB,
                stop_at: stop
            }
        );
        assert_eq!(watch.space(stop + GIB / 2, GIB, true), Space::Fine);
        assert!(matches!(watch.space(stop, GIB, true), Space::Stop(_)));
        watch.reset();
        assert!(matches!(
            watch.space(stop + GIB, GIB, true),
            Space::Low { .. }
        ));
    }
}
