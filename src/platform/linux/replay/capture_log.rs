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
/// The first minute of reports reaches the log in full; after that a report
/// at the target rate only once an hour, so days of buffer keep their
/// warnings and the end within `MOST_BYTES`.
const FIRST_REPORTS: u32 = 60;
const HOURLY: u32 = 3600;
/// A session's log stops growing here, with a last line saying so;
/// slowdowns still reach the daemon's log.
const MOST_BYTES: usize = 4 * 1024 * 1024;

const LOG: &str = "replay-capture.log";
const STARTING: &str = "replay-capture.log.starting";

/// The capture log for a starting session. It replaces the last session's
/// only in `promote`, so failed restarts never push out the log of the
/// session that died.
pub(super) fn open(dir: &Path) -> Option<File> {
    File::create(dir.join(STARTING)).ok()
}

/// The started session's log becomes `replay-capture.log`, the last one `.old`.
pub(super) fn promote(dir: &Path) {
    let log = dir.join(LOG);
    let _ = std::fs::rename(&log, dir.join("replay-capture.log.old"));
    let _ = std::fs::rename(dir.join(STARTING), log);
}

/// Empty `from` until it closes, each line with the local time into `to`.
/// `fps` set (target and what records): gsr's "update fps" reports are
/// watched, and a slowdown and the recovery go to the daemon's log. Reading
/// never stops early, so the writer never blocks on a full pipe or dies of a
/// closed one. Counts the reports read so far, also those left out.
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
            let got = reported_fps(text);
            let count = match got {
                Some(_) => counted.fetch_add(1, Ordering::Relaxed) + 1,
                None => counted.load(Ordering::Relaxed),
            };
            let routine = pace
                .as_ref()
                .zip(got)
                .is_some_and(|(pace, got)| pace.routine(got, count));
            if !routine && written < MOST_BYTES {
                written += now.len() + text.len() + 2;
                let _ = writeln!(to, "{now} {text}");
                if written >= MOST_BYTES {
                    let _ = writeln!(
                        to,
                        "{now} boltsnap: log full at 4 MiB, later lines left out"
                    );
                }
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

    fn slow(&self, got: u32) -> bool {
        (got as f32) < self.fps as f32 * SLOW
    }

    /// Report number `count`, at `got` fps, only repeats what the log
    /// already says.
    fn routine(&self, got: u32, count: u32) -> bool {
        count > FIRST_REPORTS && count % HOURLY != 0 && !self.slow(got)
    }

    /// A report of `got` fps: a message when the capture just slowed down or
    /// recovered.
    fn note(&mut self, got: u32) -> Option<String> {
        let slow = self.slow(got);
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
    fn after_the_first_minute_only_slow_reports_and_other_lines_are_kept() {
        let (from, mut gsr) = std::io::pipe().unwrap();
        let (mut log, to) = std::io::pipe().unwrap();
        let reports = follow(from, to, Some((60, "replay capture")));
        for n in std::iter::repeat_n(60, 70).chain([30]) {
            writeln!(gsr, "gsr info: update fps: {n}, damage fps: {n}").unwrap();
        }
        writeln!(gsr, "gsr warning: something else").unwrap();
        drop(gsr);
        let mut text = String::new();
        log.read_to_string(&mut text).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 62, "{text}");
        assert!(lines[60].ends_with("update fps: 30, damage fps: 30"));
        assert!(lines[61].ends_with("gsr warning: something else"));
        assert_eq!(reports.load(Ordering::Relaxed), 71);
    }

    #[test]
    fn a_start_that_never_gets_ready_keeps_the_last_log() {
        let dir = crate::paths::temp_file("capture-log-failed", "d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOG), "died\n").unwrap();
        drop(open(&dir).unwrap());
        drop(open(&dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join(LOG)).unwrap(), "died\n");
        for file in [LOG, STARTING] {
            std::fs::remove_file(dir.join(file)).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn every_line_gets_the_time_and_the_old_log_stays() {
        let dir = crate::paths::temp_file("capture-log", "d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("replay-capture.log"), "last session\n").unwrap();
        let log = open(&dir).unwrap();
        promote(&dir);
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
