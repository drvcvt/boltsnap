//! A running recording: its tick, the controls' actions, stopping its
//! recorders.

use wayland_client::QueueHandle;

use crate::record::audio::AudioCapture;
use crate::record::finalize::SaveDestination;
use crate::record::session::{
    CaptureScope, RecorderTools, RecordingAction, SessionPhase, StopChildrenJob,
    StopChildrenResult, spawn_segment,
};

use super::clients::spawn_client_writer;
use super::controls::recording_action_label;
use super::events::DaemonEvent;
use super::placement::spawn_focus_query;
use super::{AfterStop, Daemon};

impl Daemon {
    pub(super) fn tick_recording(&mut self, _qh: &QueueHandle<Self>) {
        let mut recovery = self.recording.as_mut().and_then(|session| {
            if session.phase != SessionPhase::Recording {
                return None;
            }
            session
                .active
                .iter_mut()
                .find_map(|recorder| match recorder.child.try_wait() {
                    Ok(Some(status)) => Some(format!("recorder exited unexpectedly: {status}")),
                    Err(error) => Some(format!("check recorder: {error}")),
                    Ok(None) => None,
                })
                .map(|error| ("Recorder stopped", error))
        });
        if self.recording.is_none() {
            self.watch.reset();
        }
        if recovery.is_none()
            && self.recording.as_ref().is_some_and(|session| {
                session.phase == SessionPhase::Recording
                    && self.last_recording_space_check.elapsed()
                        >= std::time::Duration::from_secs(1)
            })
        {
            self.last_recording_space_check = std::time::Instant::now();
            recovery = self.check_recording();
        }
        if let Some((title, error)) = recovery {
            self.pause_title = Some(title);
            if let Some(session) = self.recording.as_mut() {
                let _ = session.begin_pause(std::time::Instant::now());
            }
            self.remove_marker();
            let children = self
                .recording
                .as_mut()
                .map(|session| std::mem::take(&mut session.active))
                .unwrap_or_default();
            self.stop_children(AfterStop::Recover(error), children);
        }
        let snapshot = self.recording_snapshot();
        if snapshot != self.last_recording_snapshot {
            self.draw_popup();
            self.publish_recording_snapshot();
        }
    }

    pub(super) fn refresh_focus(&mut self) {
        if self.focus_query_pending {
            return;
        }
        self.focus_query_pending = true;
        spawn_focus_query(self.event_tx.clone());
    }

    pub(super) fn finish_pending_controls(&mut self) {
        if self.pending_controls.is_empty() {
            return;
        }
        let response = match self.qh.clone() {
            Some(qh) => {
                self.remove_popup();
                match self.show_recording_controls(&qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                }
            }
            None => crate::ipc::Response::error("recording controls are unavailable"),
        };
        let encoded = response.encode();
        for stream in std::mem::take(&mut self.pending_controls) {
            spawn_client_writer(stream, encoded.clone());
        }
    }

    pub(super) fn finish_pending_default_recordings(
        &mut self,
        focus_snapshot: &Result<Vec<crate::record::Monitor>, String>,
    ) {
        for stream in std::mem::take(&mut self.pending_default_recordings) {
            let response = match self.start_default_recording(focus_snapshot) {
                Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                Err(error) => crate::ipc::Response::error(error),
            };
            self.write_response(stream, response);
        }
    }

    fn reject_pending_controls(&mut self, error: &str) {
        let encoded = crate::ipc::Response::error(error).encode();
        for stream in std::mem::take(&mut self.pending_controls) {
            spawn_client_writer(stream, encoded.clone());
        }
    }

    pub(super) fn handle_recording_action(
        &mut self,
        action: RecordingAction,
        qh: &QueueHandle<Self>,
    ) -> Result<(), String> {
        let session = self
            .recording
            .as_ref()
            .ok_or_else(|| "there is no active recording".to_string())?;
        if !session.can_accept(action) {
            if session.resume_blocked && action == RecordingAction::Resume {
                return Err("cannot resume after a failed save; save again or discard".into());
            }
            return Err(format!(
                "cannot {} while {:?}",
                recording_action_label(action),
                session.phase
            ));
        }
        self.auto_paused = false;
        self.pause_title = None;
        if action != RecordingAction::Pause {
            self.controls_pending = false;
        }

        match action {
            RecordingAction::Pause => {
                let session = self.recording.as_mut().unwrap();
                session.begin_pause(std::time::Instant::now())?;
                let children = std::mem::take(&mut session.active);
                self.remove_marker();
                self.stop_children(AfterStop::Pause, children);
            }
            RecordingAction::Resume => {
                // Gone before the next segment's first frame, whoever resumes.
                let had_controls = self.popup.is_some();
                self.remove_popup();
                let session = self.recording.as_ref().unwrap();
                let active = match spawn_segment(
                    &session.scope,
                    &session.codec,
                    session.profile,
                    session.audio.as_ref().map_or(&[][..], AudioCapture::inputs),
                    session.cursor,
                    (session.both_mode == crate::config::RecordBothMode::Combined)
                        .then_some(session.monitors.as_slice()),
                    &RecorderTools::default(),
                ) {
                    Ok(active) => active,
                    Err(error) => {
                        if let Some(session) = self.recording.as_mut() {
                            session.last_error = Some(error.clone());
                        }
                        self.publish_recording_snapshot();
                        if had_controls {
                            let _ = self.show_recording_controls(qh);
                        }
                        return Err(error);
                    }
                };
                self.recording
                    .as_mut()
                    .unwrap()
                    .resume(active, std::time::Instant::now())?;
                let marker = self.recording.as_ref().and_then(|session| {
                    if session.show_frame {
                        match &session.scope {
                            CaptureScope::Area(geo) => Some(*geo),
                            CaptureScope::Outputs(_) => None,
                        }
                    } else {
                        None
                    }
                });
                if let Some(geo) = marker {
                    self.create_marker(&geo, qh);
                }
            }
            RecordingAction::SaveShelf | RecordingAction::SaveDisk => {
                let session = self.recording.as_mut().unwrap();
                // Long recordings are kept on disk; the shelf still gets a card.
                let long = session.is_long(std::time::Instant::now());
                session.shelf_card_for_disk = action == RecordingAction::SaveShelf && long;
                let destination = if action == RecordingAction::SaveShelf && !long {
                    SaveDestination::Shelf
                } else {
                    SaveDestination::Disk(crate::config::resolve_record_dir(
                        &crate::config::Config::load(),
                    ))
                };
                session.begin_finalize(std::time::Instant::now())?;
                let children = std::mem::take(&mut session.active);
                self.remove_marker();
                self.draw_popup();
                if children.is_empty() {
                    self.start_finalize_worker(destination);
                } else {
                    self.stop_children(AfterStop::Save(destination), children);
                }
            }
            RecordingAction::Discard => {
                let session = self.recording.as_mut().unwrap();
                session.begin_discard(std::time::Instant::now())?;
                let children = std::mem::take(&mut session.active);
                self.reject_pending_controls("recording is being discarded");
                self.remove_marker();
                self.remove_popup();
                if children.is_empty() {
                    self.finish_discard(Vec::new());
                } else {
                    self.stop_children(AfterStop::Discard, children);
                }
            }
        }
        self.draw_popup();
        self.publish_recording_snapshot();
        Ok(())
    }

    fn stop_children(
        &mut self,
        after: AfterStop,
        children: Vec<crate::record::session::ActiveRecorder>,
    ) {
        if children.is_empty() {
            self.handle_daemon_event(DaemonEvent::ChildrenStopped {
                after,
                result: StopChildrenResult::Ready(Vec::new()),
            });
            return;
        }
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let job = StopChildrenJob { children };
            let interrupt_error = job.interrupt().err();
            let result = match (interrupt_error, job.wait()) {
                (None, result) => result,
                (Some(interrupt), StopChildrenResult::Ready(kept)) => StopChildrenResult::Failed {
                    kept,
                    error: interrupt,
                },
                (Some(interrupt), StopChildrenResult::Failed { kept, error }) => {
                    StopChildrenResult::Failed {
                        kept,
                        error: format!("{interrupt}; {error}"),
                    }
                }
            };
            let _ = tx.send(DaemonEvent::ChildrenStopped { after, result });
        });
    }
}
