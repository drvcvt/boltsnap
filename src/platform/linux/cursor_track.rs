//! Records the pointer track beside a cursor-free recording segment.

use crate::record::Geometry;
use crate::record::cursor::TRACK_HEADER;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub enum Source {
    Output(String),
    /// The output containing the region's centre.
    Region(Geometry),
}

/// Stops and joins the track thread on drop, so the file is complete before
/// the segment is finalized.
pub struct Tracker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Tracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tracker")
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Track file for a segment.
pub fn track_path(segment: &Path) -> PathBuf {
    let mut name = segment.as_os_str().to_owned();
    name.push(".cursor");
    PathBuf::from(name)
}

/// Open the cursor session and start writing `path`. Returns once the session
/// exists, so an unsupported compositor fails the recording start instead of
/// producing a video without a cursor.
pub fn start(source: Source, path: PathBuf) -> Result<Tracker, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready) = mpsc::sync_channel(1);
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("cursor-track".into())
        .spawn(move || {
            if let Err(error) = run(&source, &path, &flag, &ready_tx) {
                let _ = ready_tx.try_send(Err(error.clone()));
                eprintln!("boltsnap daemon: cursor track stopped: {error}");
            }
        })
        .map_err(|error| format!("start cursor track: {error}"))?;
    let tracker = Tracker {
        stop,
        thread: Some(thread),
    };
    match ready.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(tracker),
        Ok(Err(error)) => Err(format!("smooth cursor unavailable: {error}")),
        Err(_) => Err("smooth cursor unavailable: cursor session did not start".into()),
    }
}

fn run(
    source: &Source,
    path: &Path,
    stop: &AtomicBool,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let clock = Clock::now()?;
    let options = libway::CaptureOptions {
        timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let mut connection = libway::Connection::connect(&options).map_err(|e| e.to_string())?;
    let outputs = connection.outputs(&options).map_err(|e| e.to_string())?;
    let output = match source {
        Source::Output(name) => outputs.iter().find(|o| &o.name == name),
        Source::Region(geo) => {
            let (cx, cy) = (
                i64::from(geo.x) + i64::from(geo.w) / 2,
                i64::from(geo.y) + i64::from(geo.h) / 2,
            );
            outputs.iter().find(|o| {
                let r = o.logical;
                (i64::from(r.x)..i64::from(r.x) + i64::from(r.width)).contains(&cx)
                    && (i64::from(r.y)..i64::from(r.y) + i64::from(r.height)).contains(&cy)
            })
        }
    }
    .ok_or("recorded output not found")?
    .clone();
    let mut stream = connection
        .cursor_positions(output.id, options)
        .map_err(|e| e.to_string())?;
    let mut file = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
    writeln!(
        file,
        "{TRACK_HEADER}\noutput {} {}",
        output.logical.x, output.logical.y
    )
    .map_err(|e| e.to_string())?;
    let _ = ready.try_send(Ok(()));
    let mut flushed = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let line = match stream
            .poll_event(Duration::from_millis(50))
            .map_err(|e| e.to_string())?
        {
            Some(libway::CursorEvent::Enter { received_at }) => {
                format!("{} e", clock.us(received_at))
            }
            Some(libway::CursorEvent::Leave { received_at }) => {
                format!("{} l", clock.us(received_at))
            }
            Some(libway::CursorEvent::Position { x, y, received_at }) => {
                format!("{} p {x} {y}", clock.us(received_at))
            }
            Some(libway::CursorEvent::Image { .. }) | None => String::new(),
        };
        if !line.is_empty() {
            writeln!(file, "{line}").map_err(|e| e.to_string())?;
        }
        if flushed.elapsed() > Duration::from_millis(250) {
            file.flush().map_err(|e| e.to_string())?;
            flushed = Instant::now();
        }
    }
    file.flush().map_err(|e| e.to_string())
}

/// Converts `Instant`s to CLOCK_MONOTONIC microseconds, the clock of gsr's
/// first-frame timestamp.
struct Clock {
    instant: Instant,
    monotonic_us: i128,
}

impl Clock {
    fn now() -> Result<Self, String> {
        let mut spec = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let instant = Instant::now();
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut spec) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self {
            instant,
            monotonic_us: i128::from(spec.tv_sec) * 1_000_000 + i128::from(spec.tv_nsec) / 1000,
        })
    }

    fn us(&self, at: Instant) -> u64 {
        let delta = match at.checked_duration_since(self.instant) {
            Some(after) => after.as_micros() as i128,
            None => -(self.instant.duration_since(at).as_micros() as i128),
        };
        (self.monotonic_us + delta).max(0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_path_appends_to_the_segment_name() {
        assert_eq!(
            track_path(Path::new("/tmp/seg.mkv")),
            PathBuf::from("/tmp/seg.mkv.cursor")
        );
    }

    #[test]
    fn clock_maps_instants_around_its_origin() {
        let clock = Clock::now().unwrap();
        let later = clock.instant + Duration::from_millis(5);
        assert_eq!(clock.us(later) as i128 - clock.monotonic_us, 5000);
        let earlier = clock.instant - Duration::from_millis(2);
        assert_eq!(clock.us(earlier) as i128 - clock.monotonic_us, -2000);
    }
}
