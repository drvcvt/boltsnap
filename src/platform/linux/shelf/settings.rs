//! The daemon's side of `config.toml`: one ordered writer for every change it
//! makes, taking in the changes others made, and the `*_settings` requests.

use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use boltsnap::settings::{self, Change};

use super::{Daemon, DaemonEvent, notify};

/// More queued `set_settings` than a settings window could ever send.
const MAX_PENDING_WRITES: usize = 32;
const MAX_SETTINGS_WATCHERS: usize = 16;

struct Job {
    changes: Vec<(String, Change)>,
    /// The client to answer; `None` for the daemon's own writes.
    reply: Option<UnixStream>,
}

/// Writes change lists in the order they were sent, off the event loop: a
/// write syncs the file to disk.
pub(super) struct Writer {
    jobs: mpsc::Sender<Job>,
}

impl Writer {
    pub(super) fn spawn(event_tx: calloop::channel::Sender<DaemonEvent>) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for job in queue {
                let changes: Vec<_> = job
                    .changes
                    .iter()
                    .map(|(key, change)| (key.as_str(), change.clone()))
                    .collect();
                let error = settings::write(&crate::config::config_path(), &changes)
                    .err()
                    .map(|error| error.to_string());
                let _ = event_tx.send(DaemonEvent::SettingsWritten {
                    reply: job.reply,
                    error,
                });
            }
        });
        Self { jobs }
    }
}

impl Daemon {
    /// Adopt the file as it is now: an edit by hand or by the selector, or
    /// the outcome of our own writes. A write still queued is newer than the
    /// file, so this waits until the last one has finished.
    pub(super) fn refresh_recording_prefs(&mut self) {
        if self.pending_writes > 0 {
            return;
        }
        let prefs = crate::config::Config::load().recording_prefs();
        if prefs == self.recording_prefs {
            return;
        }
        let show_frame_changed = self.recording_prefs.show_frame != prefs.show_frame;
        self.recording_prefs = prefs;
        if show_frame_changed {
            self.apply_show_frame_pref(self.recording_prefs.show_frame);
        }
        self.publish_tray_snapshot();
    }

    /// Shows `prefs` at once and writes only the keys that changed, so an
    /// edit made meanwhile to any other key stays.
    pub(super) fn persist_tray_prefs(&mut self, prefs: crate::config::RecordingPrefs) {
        let changes = crate::config::pref_changes(&self.recording_prefs, &prefs);
        if !changes.is_empty() {
            let show_frame_changed = self.recording_prefs.show_frame != prefs.show_frame;
            self.recording_prefs = prefs;
            if show_frame_changed {
                self.apply_show_frame_pref(self.recording_prefs.show_frame);
            }
            self.write_settings(
                changes
                    .into_iter()
                    .map(|(key, change)| (key.to_owned(), change))
                    .collect(),
                None,
            );
        }
        self.publish_tray_snapshot();
    }

    pub(super) fn send_settings(&self, stream: UnixStream) {
        self.write_response(
            stream,
            crate::ipc::Response::ok_settings(crate::config::settings_state()),
        );
    }

    /// Answers once the patch is written, with the state read back from the
    /// file or the reason it was refused.
    pub(super) fn set_settings(&mut self, patch: Vec<(String, Change)>, stream: UnixStream) {
        if self.pending_writes >= MAX_PENDING_WRITES {
            super::reject_busy(stream);
            return;
        }
        self.write_settings(patch, Some(stream));
    }

    pub(super) fn watch_settings(&mut self, mut stream: UnixStream) {
        self.settings_watchers.retain(super::watcher_connected);
        if self.settings_watchers.len() >= MAX_SETTINGS_WATCHERS {
            eprintln!(
                "boltsnap daemon: refused a settings watch; {MAX_SETTINGS_WATCHERS} are open"
            );
            return;
        }
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_nonblocking(true);
        let line = state_line(&crate::config::settings_state());
        if matches!(stream.write(line.as_bytes()), Ok(written) if written == line.len()) {
            self.settings_watchers.push(stream);
        }
    }

    /// Sends the state to every settings watcher if it changed since the last
    /// time; a reload that finds nothing new, such as after our own write,
    /// sends nothing.
    pub(super) fn publish_settings(&mut self) {
        let state = crate::config::settings_state();
        if self.last_settings.as_ref() == Some(&state) {
            return;
        }
        let line = state_line(&state);
        self.settings_watchers.retain_mut(
            |watcher| matches!(watcher.write(line.as_bytes()), Ok(n) if n == line.len()),
        );
        self.last_settings = Some(state);
    }

    fn write_settings(&mut self, changes: Vec<(String, Change)>, reply: Option<UnixStream>) {
        match self.settings_writer.jobs.send(Job { changes, reply }) {
            Ok(()) => self.pending_writes += 1,
            Err(mpsc::SendError(job)) => {
                let error = "the settings writer stopped";
                match job.reply {
                    Some(stream) => self.write_response(stream, crate::ipc::Response::error(error)),
                    None => notify(&format!("Recording settings could not be saved: {error}")),
                }
            }
        }
    }

    pub(super) fn settings_written(&mut self, reply: Option<UnixStream>, error: Option<String>) {
        self.pending_writes = self.pending_writes.saturating_sub(1);
        match (reply, error) {
            (Some(stream), Some(error)) => {
                self.write_response(stream, crate::ipc::Response::error(error))
            }
            (Some(stream), None) => self.send_settings(stream),
            (None, Some(error)) => {
                notify(&format!("Recording settings could not be saved: {error}"))
            }
            (None, None) => {}
        }
        // Also undoes a failed tray write, since the file kept the old value.
        self.refresh_recording_prefs();
        self.publish_settings();
    }
}

fn state_line(state: &settings::State) -> String {
    format!("{}\n", state.to_json())
}

/// How long a burst of file events may last before the file is read: an
/// editor's save is several events.
const SETTLE_MS: i32 = 150;
const WATCH_MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE;

/// Calls `changed` once per burst of writes, replacements or removals of
/// `path`, after the burst settled. Watches the directory, since editors and
/// our writer replace the file; for a symlinked file also the directory of
/// its target. Creates the directory if it is missing.
pub(super) fn watch_config(path: PathBuf, changed: impl Fn() + Send + 'static) -> io::Result<()> {
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut inotify = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut watched = Vec::new();
    add_watches(&inotify, &path, &mut watched)?;
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            let read = match inotify.read(&mut buffer) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    eprintln!("boltsnap daemon: config watcher stopped: {error}");
                    return;
                }
            };
            if !events(&buffer[..read])
                .iter()
                .any(|event| watched.contains(event))
            {
                continue;
            }
            while readable(&inotify, SETTLE_MS) && inotify.read(&mut buffer).is_ok() {}
            // The link may point elsewhere now.
            let _ = add_watches(&inotify, &path, &mut watched);
            changed();
        }
    });
    Ok(())
}

/// Watches the directory of `path` and of its symlink target, noting each
/// watch with the file name that matters in it. Only the first must work.
fn add_watches(inotify: &File, path: &Path, watched: &mut Vec<(i32, OsString)>) -> io::Result<()> {
    let mut files = vec![path.to_path_buf()];
    if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
        && let Ok(target) = std::fs::canonicalize(path)
    {
        files.push(target);
    }
    for (index, file) in files.iter().enumerate() {
        let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
            continue;
        };
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        let dir = CString::new(dir.as_os_str().as_bytes())?;
        let wd = unsafe { libc::inotify_add_watch(inotify.as_raw_fd(), dir.as_ptr(), WATCH_MASK) };
        if wd < 0 {
            if index == 0 {
                return Err(io::Error::last_os_error());
            }
            continue;
        }
        let entry = (wd, name.to_os_string());
        if !watched.contains(&entry) {
            watched.push(entry);
        }
    }
    Ok(())
}

/// The watch and file name of each event in a buffer read from inotify.
fn events(buffer: &[u8]) -> Vec<(i32, OsString)> {
    const HEADER: usize = std::mem::size_of::<libc::inotify_event>();
    let mut events = Vec::new();
    let mut at = 0;
    while let Some(header) = buffer.get(at..at + HEADER) {
        let wd = i32::from_ne_bytes(header[..4].try_into().unwrap());
        let len = u32::from_ne_bytes(header[12..16].try_into().unwrap()) as usize;
        let Some(name) = buffer.get(at + HEADER..at + HEADER + len) else {
            break;
        };
        // The name is padded with NULs to keep the next event aligned.
        let name = name.split(|byte| *byte == 0).next().unwrap_or_default();
        events.push((wd, OsString::from_vec(name.to_vec())));
        at += HEADER + len;
    }
    events
}

fn readable(file: &File, timeout_ms: i32) -> bool {
    let mut poll = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut poll, 1, timeout_ms) > 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn inotify_events_are_split_with_their_names() {
        let mut buffer = Vec::new();
        for (wd, name) in [(1_i32, &b"config.toml"[..]), (2, b"")] {
            let padded = name.len().div_ceil(16) * 16;
            buffer.extend(wd.to_ne_bytes());
            buffer.extend(libc::IN_MOVED_TO.to_ne_bytes());
            buffer.extend(0_u32.to_ne_bytes());
            buffer.extend((padded as u32).to_ne_bytes());
            buffer.extend(name);
            buffer.resize(buffer.len() + padded - name.len(), 0);
        }
        assert_eq!(
            events(&buffer),
            vec![(1, "config.toml".into()), (2, "".into())]
        );
        // A cut-off event is dropped, not misread.
        assert_eq!(events(&buffer[..20]), vec![]);
    }

    #[test]
    fn the_config_watcher_reacts_to_the_file_only() {
        let dir = std::env::temp_dir().join(format!("boltsnap-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let elsewhere = dir.join("dotfiles");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let real = elsewhere.join("boltsnap.toml");
        std::fs::write(&real, "").unwrap();
        let config = dir.join("boltsnap");
        let path = config.join("config.toml");
        let (tx, rx) = mpsc::channel();
        // The watcher creates the missing config directory.
        watch_config(path.clone(), move || {
            let _ = tx.send(());
        })
        .unwrap();
        assert!(config.is_dir());
        let quiet = Duration::from_millis(400);
        let seen = Duration::from_secs(3);

        std::fs::write(config.join("other.toml"), "x").unwrap();
        std::fs::write(config.join("config.toml.lock"), "").unwrap();
        assert!(rx.recv_timeout(quiet).is_err());

        std::fs::write(&path, "record_fps = 60\n").unwrap();
        assert!(rx.recv_timeout(seen).is_ok());
        // One burst, one call.
        assert!(rx.recv_timeout(quiet).is_err());

        settings::write(
            &path,
            &[("record_fps", Change::Set(settings::Value::Int(120)))],
        )
        .unwrap();
        assert!(rx.recv_timeout(seen).is_ok());

        // Re-pointed to a file elsewhere: editing that file counts too.
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        assert!(rx.recv_timeout(seen).is_ok());
        std::fs::write(&real, "record_fps = 30\n").unwrap();
        assert!(rx.recv_timeout(seen).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
