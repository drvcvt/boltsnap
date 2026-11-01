//! What `try_wait` cannot see in a running recording: a recorder that hangs
//! without exiting, and free space running out before the finished recording
//! can still be saved.

use super::super::replay::capture_log::follow;
use crate::record::finalize::{RECORDING_DISK_RESERVE_BYTES, RECORDING_ON_DISK, remux_space};
use crate::record::session::ActiveRecorder;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// gsr writes constant-rate Matroska and flushes a cluster every 100 ms, and
/// reports its frame rate once a second right before encoding, so a segment
/// whose file or reports stand still this long has a recorder that stopped.
pub(super) const STALL: Duration = Duration::from_secs(6);

/// Space a check may lose before the next one, at well over any bitrate gsr uses.
const SLACK_BYTES: u64 = 256 * 1024 * 1024;
/// Warn once this much before the recording would pause.
const WARN_AHEAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Watch {
    sizes: HashMap<PathBuf, (u64, Instant)>,
    /// gsr's fps reports read so far, and when that count last moved.
    reports: HashMap<PathBuf, Arc<AtomicU32>>,
    frames: HashMap<PathBuf, (u64, Instant)>,
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
        RECORDING_ON_DISK.store(false, Ordering::Relaxed);
    }

    /// Each recorder's stderr goes to the daemon's log with the time; gsr's
    /// fps reports in it are counted for `stalled`.
    pub fn listen(&mut self, recorders: &mut [ActiveRecorder], fps: u32) {
        for recorder in recorders {
            if let Some(stderr) = recorder.child.stderr.take() {
                let reports = follow(stderr, std::io::stderr(), Some((fps, "recording")));
                self.reports.insert(recorder.path.clone(), reports);
            }
        }
    }

    /// Note `size` of an active gsr segment: what it stopped doing once its
    /// file, or its fps reports after the first, stood still for `STALL`.
    pub fn stalled(&mut self, path: &Path, size: u64, now: Instant) -> Option<&'static str> {
        if still(&mut self.sizes, path, size, now) {
            return Some("writing");
        }
        // Before the first report, or from a gsr that prints none, only growth counts.
        let reports = self
            .reports
            .get(path)
            .map_or(0, |reports| reports.load(Ordering::Relaxed));
        (reports > 0 && still(&mut self.frames, path, reports.into(), now)).then_some("encoding")
    }

    /// Forget segments that are no longer recording.
    pub fn keep_only(&mut self, active: &[&Path]) {
        let recording = |path: &PathBuf| active.contains(&path.as_path());
        self.sizes.retain(|path, _| recording(path));
        self.reports.retain(|path, _| recording(path));
        self.frames.retain(|path, _| recording(path));
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
            if gsr && let Some(what) = self.watch.stalled(path, size(path), now) {
                return Some((
                    "Recorder stalled",
                    format!(
                        "the recorder stopped {what} video for {} s",
                        STALL.as_secs()
                    ),
                ));
            }
        }
        let long = session.is_long(now);
        RECORDING_ON_DISK.store(long, Ordering::Relaxed);
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

/// Note `value` for `path`; true once it has not grown for `STALL`.
fn still(
    seen: &mut HashMap<PathBuf, (u64, Instant)>,
    path: &Path,
    value: u64,
    now: Instant,
) -> bool {
    match seen.get_mut(path) {
        Some((last, since)) if value > *last => {
            *last = value;
            *since = now;
            false
        }
        Some((_, since)) => now.saturating_duration_since(*since) >= STALL,
        None => {
            seen.insert(path.to_owned(), (value, now));
            false
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
    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_segment_that_stops_growing_stalls_after_the_window() {
        let mut watch = Watch::default();
        let path = Path::new("/cache/boltsnap-segment-1-1-DP_3.mkv");
        let start = Instant::now();
        assert_eq!(watch.stalled(path, 0, start), None);
        assert_eq!(watch.stalled(path, 0, start + STALL - MS), None);
        assert_eq!(watch.stalled(path, 10, start + STALL), None);
        // Growth restarts the window.
        assert_eq!(watch.stalled(path, 10, start + STALL * 2 - MS), None);
        assert_eq!(watch.stalled(path, 10, start + STALL * 2), Some("writing"));
        watch.keep_only(&[]);
        assert_eq!(watch.stalled(path, 10, start + STALL * 3), None);
    }

    #[test]
    fn frames_that_stop_while_the_file_grows_stall() {
        let mut watch = Watch::default();
        let path = Path::new("/cache/boltsnap-segment-1-1-DP_3.mkv");
        let reports = Arc::new(AtomicU32::new(0));
        watch.reports.insert(path.to_owned(), reports.clone());
        let start = Instant::now();
        for second in 0..3 {
            reports.store(second, Ordering::Relaxed);
            let now = start + Duration::from_secs(second.into());
            assert_eq!(watch.stalled(path, second.into(), now), None);
        }
        // The encoder hangs; audio still reaches the file.
        let last = start + Duration::from_secs(2);
        assert_eq!(watch.stalled(path, 10, last + STALL - MS), None);
        assert_eq!(watch.stalled(path, 11, last + STALL), Some("encoding"));
    }

    #[test]
    fn no_reports_falls_back_to_growth() {
        let mut watch = Watch::default();
        let path = Path::new("/cache/boltsnap-segment-1-1-DP_3.mkv");
        watch.reports.insert(path.to_owned(), Arc::default());
        let start = Instant::now();
        for size in 0..10 {
            assert_eq!(watch.stalled(path, size, start + STALL * size as u32), None);
        }
        assert_eq!(watch.stalled(path, 9, start + STALL * 10), Some("writing"));
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
