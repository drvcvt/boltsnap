//! The daemon's side of `config.toml`: one ordered writer for every change it
//! makes, taking in the changes others made, and the `*_settings` requests.

use std::io::Write;
use std::os::unix::net::UnixStream;
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
