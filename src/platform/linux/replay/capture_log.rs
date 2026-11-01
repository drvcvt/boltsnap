//! What the recorder and the worker say, kept with the time (replay plan,
//! 14): a lag while the buffer runs left nothing to read, as each start
//! truncated GPU Screen Recorder's log and no line carried a time.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Reports below this share of the target frame rate count as slow.
const SLOW: f32 = 0.9;
/// This many reports in a row (gsr reports once a second) make a change.
const RUN: u32 = 3;
/// A session's log stops growing here (about 16 hours of gsr's reports);
/// slowdowns still reach the daemon's log.
const MOST_BYTES: usize = 4 * 1024 * 1024;

/// The capture log for a new session; the last session's becomes `.old`.
pub(super) fn open(dir: &Path) -> Option<File> {
    let path = dir.join("replay-capture.log");
    let _ = std::fs::rename(&path, dir.join("replay-capture.log.old"));
    File::create(path).ok()
}

/// Empty `from` until it closes, each line with the local time into `to`.
/// `fps` set (target and what records): gsr's "update fps" reports are
/// watched, and a slowdown and the recovery go to the daemon's log. Reading
/// never stops early, so the writer never blocks on a full pipe or dies of a
/// closed one. Counts the reports read so far.
pub fn follow(
    from: impl Read + Send + 'static,
    mut to: impl Write + Send + 'static,
    fps: Option<(u32, &'static str)>,
) -> Arc<AtomicU32> {
    let reports = Arc::new(AtomicU32::new(0));
    let counted = reports.clone();
    std::thread::spawn(move || {
        let mut pace = fps.map(|(fps, what)| Pace::new(fps, what));
        let mut lines = BufReader::new(from);
        let mut line = Vec::new();
        let mut written = 0;
        while lines
            .read_until(b'\n', &mut line)
            .is_ok_and(|read| read > 0)
        {
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end();
            let now = crate::paths::local_timestamp();
            if written < MOST_BYTES {
                written += now.len() + text.len() + 2;
                let _ = writeln!(to, "{now} {text}");
            }
            let got = reported_fps(text);
            if got.is_some() {
                counted.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(note) = pace
                .as_mut()
                .zip(got)
                .and_then(|(pace, got)| pace.note(got))
            {
                // Never `eprintln!`: it panics on a full disk, the thread
                // would end, and gsr die of the closed pipe.
                let _ = writeln!(std::io::stderr(), "boltsnap daemon: {now} {note}");
            }
            line.clear();
        }
    });
    reports
}

/// Whether the capture keeps its frame rate.
struct Pace {
    fps: u32,
    what: &'static str,
    slow: bool,
    run: u32,
}

impl Pace {
    fn new(fps: u32, what: &'static str) -> Self {
        Self {
            fps,
            what,
            slow: false,
            run: 0,
        }
    }

    /// A report of `got` fps: a message when the capture just slowed down or
    /// recovered.
    fn note(&mut self, got: u32) -> Option<String> {
        let slow = (got as f32) < self.fps as f32 * SLOW;
        if slow == self.slow {
            self.run = 0;
            return None;
        }
        self.run += 1;
        if self.run < RUN {
            return None;
        }
        self.slow = slow;
        self.run = 0;
        Some(if slow {
            format!("{} slowed to {got} of {} fps", self.what, self.fps)
        } else {
            format!("{} back at {got} fps", self.what)
        })
    }
}

/// N from gsr's "update fps: N, damage fps: M".
fn reported_fps(line: &str) -> Option<u32> {
    let rest = line.split("update fps:").nth(1)?;
    rest.split(',').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slowdown_and_the_recovery_are_told_once_and_a_single_dip_is_not() {
        let mut pace = Pace::new(60, "replay capture");
        let mut notes = Vec::new();
        for n in [61, 60, 40, 61, 60, 30, 31, 29, 28, 30, 59, 60, 61, 60] {
            notes.extend(pace.note(n));
        }
        assert_eq!(
            notes,
            [
                "replay capture slowed to 29 of 60 fps",
                "replay capture back at 61 fps"
            ]
        );
        assert_eq!(
            reported_fps("gsr info: update fps: 59, damage fps: 12"),
            Some(59)
        );
        assert_eq!(reported_fps("gsr info: something else"), None);
    }

    #[test]
    fn every_line_gets_the_time_and_the_old_log_stays() {
        let dir = crate::paths::temp_file("capture-log", "d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("replay-capture.log"), "last session\n").unwrap();
        let log = open(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("replay-capture.log.old")).unwrap(),
            "last session\n"
        );
        let (read, write) = std::io::pipe().unwrap();
        let reports = follow(read, log, Some((60, "replay capture")));
        let mut write = write;
        writeln!(write, "gsr info: update fps: 60, damage fps: 60").unwrap();
        writeln!(write, "gsr warning: something else").unwrap();
        writeln!(write, "gsr info: update fps: 59, damage fps: 59").unwrap();
        drop(write);
        let path = dir.join("replay-capture.log");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let text = loop {
            let text = std::fs::read_to_string(&path).unwrap();
            if text.lines().count() == 3 || std::time::Instant::now() > deadline {
                break text;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        // `YYYY-MM-DD_HH-MM-SS ` before the line.
        assert_eq!(text.find(' '), Some(19), "{text}");
        assert!(text.ends_with("gsr info: update fps: 59, damage fps: 59\n"));
        // Every report counts, whatever else gsr says in between.
        assert_eq!(reports.load(Ordering::Relaxed), 2);
        for file in ["replay-capture.log", "replay-capture.log.old"] {
            std::fs::remove_file(dir.join(file)).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
    }
}
