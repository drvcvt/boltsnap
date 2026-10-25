//! The daemon's side of `config.toml`: one ordered writer for every change it
//! makes, and taking in the changes others made.

use std::sync::mpsc;

use boltsnap::settings::{self, Change};

use super::{Daemon, DaemonEvent, notify};

/// Writes change lists in the order they were sent, off the event loop: a
/// write syncs the file to disk.
pub(super) struct Writer {
    jobs: mpsc::Sender<Vec<(String, Change)>>,
}

impl Writer {
    pub(super) fn spawn(event_tx: calloop::channel::Sender<DaemonEvent>) -> Self {
        let (jobs, queue) = mpsc::channel::<Vec<(String, Change)>>();
        std::thread::spawn(move || {
            for changes in queue {
                let changes: Vec<_> = changes
                    .iter()
                    .map(|(key, change)| (key.as_str(), change.clone()))
                    .collect();
                let error = settings::write(&crate::config::config_path(), &changes)
                    .err()
                    .map(|error| error.to_string());
                let _ = event_tx.send(DaemonEvent::SettingsWritten { error });
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
            );
        }
        self.publish_tray_snapshot();
    }

    fn write_settings(&mut self, changes: Vec<(String, Change)>) {
        match self.settings_writer.jobs.send(changes) {
            Ok(()) => self.pending_writes += 1,
            Err(_) => notify("Recording settings could not be saved: the writer stopped"),
        }
    }

    pub(super) fn settings_written(&mut self, error: Option<String>) {
        self.pending_writes = self.pending_writes.saturating_sub(1);
        if let Some(error) = error {
            notify(&format!("Recording settings could not be saved: {error}"));
        }
        // Also undoes a failed write, since the file kept the old value.
        self.refresh_recording_prefs();
    }
}
