//! The tray icon's state and its menu actions.

use crate::record::session::{CaptureScope, SessionPhase};

use super::events::DaemonEvent;
use super::{Daemon, notify};

impl Daemon {
    fn tray_snapshot(&self) -> crate::tray::TraySnapshot {
        crate::tray::TraySnapshot {
            prefs: self.recording_prefs.clone(),
            monitors: self.cached_monitors(),
            state: self.recording_snapshot().state,
            replay_running: self.replay_running.as_ref().is_some_and(|r| r.get()),
        }
    }

    pub(super) fn start_tray(&mut self) {
        self.tray = Some(crate::tray::TrayPublisher::spawn(
            self.tray_snapshot(),
            self.event_tx.clone(),
        ));
    }

    pub(super) fn publish_tray_snapshot(&self) {
        let snapshot = self.tray_snapshot();
        if let Some(tray) = &self.tray {
            tray.publish(snapshot);
        }
    }

    pub(super) fn apply_show_frame_pref(&mut self, show: bool) {
        if let Some(session) = self.recording.as_mut() {
            session.show_frame = show;
        }
        if !show {
            self.remove_marker();
        } else if let Some(geo) = self.recording.as_ref().and_then(|session| {
            (session.phase == SessionPhase::Recording)
                .then_some(&session.scope)
                .and_then(|scope| match scope {
                    CaptureScope::Area(geo) => Some(*geo),
                    CaptureScope::Outputs(_) => None,
                })
        }) && let Some(qh) = self.qh.clone()
        {
            self.create_marker(&geo, &qh);
        }
    }

    pub(super) fn handle_tray_action(&mut self, action: crate::tray::TrayAction) {
        use crate::tray::TrayAction;
        self.refresh_prefs();

        match action {
            TrayAction::ReplayToggle | TrayAction::ReplaySave => {
                // Starting waits for the capture to become ready, which takes
                // seconds, so the whole toggle runs off the event loop and the
                // tray is republished once the state has settled.
                let running = self.replay_running.as_ref().is_some_and(|r| r.get());
                let command = match action {
                    TrayAction::ReplayToggle if running => "stop",
                    TrayAction::ReplayToggle => "start",
                    _ => "save",
                };
                let event_tx = self.event_tx.clone();
                std::thread::spawn(move || {
                    if let Err(error) = super::super::replay::call(
                        &serde_json::json!({"version":1,"command":command}),
                    ) {
                        notify(&format!("Replay: {error}"));
                    }
                    let _ = event_tx.send(DaemonEvent::ReplayStateChanged);
                });
            }
            TrayAction::Recording(action) => {
                if let Some(qh) = self.qh.clone()
                    && let Err(error) = self.handle_recording_action(action, &qh)
                {
                    notify(&format!("Recording: {error}"));
                }
            }
            TrayAction::StartRegion => {
                if self.recording.is_some() {
                    notify("A recording is already in progress");
                } else {
                    let result = crate::paths::own_exe()
                        .map_err(|error| format!("find boltsnap executable: {error}"))
                        .and_then(|exe| {
                            crate::paths::spawn_reaped(
                                std::process::Command::new(exe)
                                    .arg("record")
                                    .stdin(std::process::Stdio::null())
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::null()),
                            )
                            .map(|_| ())
                            .map_err(|error| format!("start region selector: {error}"))
                        });
                    if let Err(error) = result {
                        notify(&error);
                    }
                }
            }
            TrayAction::StartDefault => {
                if self.recording.is_some() {
                    notify("A recording is already in progress");
                } else {
                    self.pending_tray_default_recording = true;
                    self.refresh_focus();
                }
            }
            TrayAction::SetDefaultTarget(target) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.default_target = target;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetCursor(mode) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.cursor = mode;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetBothMode(mode) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.both_mode = mode;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetAudioSource(source) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.audio_source = source;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetShowFrame(show) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.show_frame = show;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetDiskAddToShelf(add) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.disk_add_to_shelf = add;
                self.persist_tray_prefs(prefs);
            }
        }
    }
}
