//! Follows the pointer for smooth-cursor recordings: feeds the gsr plugins
//! live and records the track beside the segment for `X.cursor.json`.

use crate::record::Geometry;
use crate::record::cursor::TRACK_HEADER;
use crate::record::cursor_motion::feed_line;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
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

/// Track file of the `index`th tracker of a segment; a combined stream has one
/// tracker per output.
pub fn track_path_indexed(segment: &Path, index: usize) -> PathBuf {
    let mut name = segment.as_os_str().to_owned();
    name.push(".cursor");
    if index > 0 {
        name.push(index.to_string());
    }
    PathBuf::from(name)
}

/// Existing track files of a segment, in tracker order.
pub fn track_paths(segment: &Path) -> Vec<PathBuf> {
    (0..)
        .map(|index| track_path_indexed(segment, index))
        .take_while(|path| path.is_file())
        .collect()
}

/// File name of the gpu-screen-recorder plugin that draws the smooth cursor.
pub const PLUGIN: &str = "libboltsnap_gsr_cursor.so";

/// The plugin beside the running binary, or in `../lib/boltsnap/` for
/// packaged installs.
pub fn plugin_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let dir = exe.parent().unwrap_or(Path::new("."));
    plugin_in(dir).ok_or_else(|| {
        format!(
            "smooth cursor needs {PLUGIN} beside boltsnap ({})",
            dir.display()
        )
    })
}

fn plugin_in(dir: &Path) -> Option<PathBuf> {
    [dir.join(PLUGIN), dir.join("../lib/boltsnap").join(PLUGIN)]
        .into_iter()
        .find(|path| path.is_file())
}

/// Pipes from a segment's trackers to its gsr plugins: every tracker writes to
/// every plugin, so each plugin follows the pointer across all recorded outputs.
pub struct Feeds {
    /// Read ends, one per plugin, inherited by gsr.
    pub readers: Vec<OwnedFd>,
    writers: Arc<[File]>,
}

impl Feeds {
    pub fn new(count: usize) -> Result<Self, String> {
        let mut readers = Vec::with_capacity(count);
        let mut writers = Vec::with_capacity(count);
        for _ in 0..count {
            let mut fds = [0; 2];
            if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                return Err(format!("cursor feed: {}", std::io::Error::last_os_error()));
            }
            let (reader, writer) =
                unsafe { (OwnedFd::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
            // A stalled plugin must never block the tracker.
            unsafe {
                libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
            }
            readers.push(reader);
            writers.push(writer);
        }
        Ok(Self {
            readers,
            writers: writers.into(),
        })
    }
}

/// Logical rectangle of the tracked output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
}

/// Open the cursor session, feed `feeds` as tracker `index` and record `track`.
/// Returns once the session exists, so an unsupported compositor fails the
/// recording start instead of producing a video without a cursor.
pub fn start(
    source: Source,
    index: usize,
    track: Option<PathBuf>,
    feeds: &Feeds,
) -> Result<(Tracker, Area), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready) = mpsc::sync_channel(1);
    let flag = stop.clone();
    let writers = feeds.writers.clone();
    let thread = std::thread::Builder::new()
        .name("cursor-track".into())
        .spawn(move || {
            let sinks = Sinks {
                index,
                writers,
                track: None,
            };
            if let Err(error) = run(&source, track.as_deref(), sinks, &flag, &ready_tx) {
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
        Ok(Ok(area)) => Ok((tracker, area)),
        Ok(Err(error)) => Err(format!("smooth cursor unavailable: {error}")),
        Err(_) => Err("smooth cursor unavailable: cursor session did not start".into()),
    }
}

/// Where a tracker's events go.
struct Sinks {
    index: usize,
    writers: Arc<[File]>,
    track: Option<BufWriter<File>>,
}

impl Sinks {
    fn event(&mut self, us: u64, line: &str, position: Option<(f64, f64)>) {
        let feed = feed_line(self.index, us, position);
        for mut writer in self.writers.iter() {
            // Lines are shorter than PIPE_BUF, so writes are atomic between
            // trackers. A full or closed pipe drops the line.
            let _ = writer.write(feed.as_bytes());
        }
        self.record(|track| writeln!(track, "{us} {line}"));
    }

    /// Write to the track file. A failing file only costs `X.cursor.json`,
    /// never the live cursor.
    fn record(&mut self, write: impl FnOnce(&mut BufWriter<File>) -> std::io::Result<()>) {
        if let Some(track) = &mut self.track
            && let Err(error) = write(track)
        {
            eprintln!("boltsnap daemon: cursor track file: {error}");
            self.track = None;
        }
    }
}

fn run(
    source: &Source,
    track: Option<&Path>,
    mut sinks: Sinks,
    stop: &AtomicBool,
    ready: &mpsc::SyncSender<Result<Area, String>>,
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
    if let Some(path) = track {
        let mut file = BufWriter::new(File::create(path).map_err(|e| e.to_string())?);
        writeln!(
            file,
            "{TRACK_HEADER}\noutput {} {}",
            output.logical.x, output.logical.y
        )
        .map_err(|e| e.to_string())?;
        sinks.track = Some(file);
    }
    let (ox, oy) = (f64::from(output.logical.x), f64::from(output.logical.y));
    let _ = ready.try_send(Ok(Area {
        x: ox,
        y: oy,
        width: f64::from(output.logical.width),
    }));
    let mut flushed = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match stream
            .poll_event(Duration::from_millis(50))
            .map_err(|e| e.to_string())?
        {
            // Visibility follows positions; enter only goes to the track file.
            Some(libway::CursorEvent::Enter { received_at }) => {
                let us = clock.us(received_at);
                sinks.record(|track| writeln!(track, "{us} e"));
            }
            Some(libway::CursorEvent::Leave { received_at }) => {
                sinks.event(clock.us(received_at), "l", None);
            }
            Some(libway::CursorEvent::Position { x, y, received_at }) => {
                let (x, y) = (f64::from(x), f64::from(y));
                sinks.event(
                    clock.us(received_at),
                    &format!("p {x} {y}"),
                    Some((ox + x, oy + y)),
                );
            }
            Some(libway::CursorEvent::Image { .. }) | None => {}
        }
        if flushed.elapsed() > Duration::from_millis(250) {
            sinks.record(|track| track.flush());
            flushed = Instant::now();
        }
    }
    sinks.record(|track| track.flush());
    Ok(())
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
            track_path_indexed(Path::new("/tmp/seg.mkv"), 0),
            PathBuf::from("/tmp/seg.mkv.cursor")
        );
        assert_eq!(
            track_path_indexed(Path::new("/tmp/seg.mkv"), 1),
            PathBuf::from("/tmp/seg.mkv.cursor1")
        );
    }

    #[test]
    fn every_plugin_feed_gets_every_tracker_event_without_blocking() {
        let feeds = Feeds::new(2).unwrap();
        let mut sinks = Sinks {
            index: 1,
            writers: feeds.writers.clone(),
            track: None,
        };
        sinks.event(7, "p 1 2", Some((1921.0, 2.0)));
        for reader in &feeds.readers {
            let mut buffer = [0u8; 64];
            let n = unsafe { libc::read(reader.as_raw_fd(), buffer.as_mut_ptr().cast(), 64) };
            assert_eq!(&buffer[..n as usize], b"1 7 p 1921 2\n");
        }
        // Nobody reads: a full pipe drops lines instead of stalling the tracker.
        for _ in 0..20_000 {
            sinks.event(8, "l", None);
        }
    }

    #[test]
    fn plugin_is_found_beside_the_binary_or_in_lib() {
        let root = std::env::temp_dir().join(format!("boltsnap-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(root.join("lib/boltsnap")).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        assert_eq!(plugin_in(&bin), None);
        std::fs::write(root.join("lib/boltsnap").join(PLUGIN), b"").unwrap();
        assert_eq!(
            plugin_in(&bin),
            Some(bin.join("../lib/boltsnap").join(PLUGIN))
        );
        std::fs::write(bin.join(PLUGIN), b"").unwrap();
        assert_eq!(plugin_in(&bin), Some(bin.join(PLUGIN)));
        let _ = std::fs::remove_dir_all(root);
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
