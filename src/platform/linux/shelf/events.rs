//! What the daemon's threads hand its loop, and how the loop answers each.

use std::os::unix::net::UnixStream;

use crate::record::finalize::{FinalizeFailure, FinalizedClip};
use crate::record::session::StopChildrenResult;

use super::actions::PreparedAdd;
use super::anim::AnimKind;
use super::limits::Permit;
use super::start::cleanup_audio_async;
use super::{AfterStop, Daemon, cards, notify};

pub(crate) enum DaemonEvent {
    ThumbnailReady {
        id: u64,
        image: Option<image::RgbaImage>,
        _permit: Option<Permit>,
    },
    ReplayClipReady {
        path: std::path::PathBuf,
        output: String,
        reply: std::sync::mpsc::SyncSender<()>,
    },
    /// The replay buffer came up or went away; the tray entry must follow.
    ReplayStateChanged,
    ClientRequest {
        request: crate::ipc::Request,
        stream: UnixStream,
        _permit: Permit,
    },
    AddPrepared {
        result: Result<PreparedAdd, String>,
        stream: UnixStream,
        _client: Permit,
        _image: Option<Permit>,
    },
    VideoPrepared {
        result: Result<FinalizedClip, String>,
        stream: UnixStream,
        _client: Permit,
    },
    ChildrenStopped {
        after: AfterStop,
        result: StopChildrenResult,
    },
    Finalized(Result<Vec<FinalizedClip>, FinalizeFailure>),
    /// A card save ended. A video card moves to the saved file (`promote`); an
    /// image card is copied and keeps its temporary file.
    CardSaved {
        id: u64,
        promote: bool,
        result: Result<std::path::PathBuf, String>,
    },
    FocusResolved(Result<Vec<crate::record::Monitor>, String>),
    Tray(crate::tray::TrayAction),
    SettingsWritten {
        reply: Option<UnixStream>,
        error: Option<String>,
    },
    /// `config.toml` was written by someone, maybe us.
    ConfigChanged,
}

impl Daemon {
    pub(super) fn handle_daemon_event(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::ThumbnailReady { id, image, _permit } => {
                // One made for an older card size is redone for the new one.
                if let Some(image) = image
                    && image.dimensions() == cards::card_size().dims()
                    && self.model.replace_thumb(id, image)
                    && let Some(qh) = self.qh.clone()
                {
                    self.draw(&qh);
                }
            }
            DaemonEvent::ClientRequest {
                request,
                stream,
                _permit,
            } => {
                self.handle_client_request(request, stream);
            }
            DaemonEvent::AddPrepared {
                result,
                stream,
                _client,
                _image,
            } => {
                let response = match (result, self.qh.clone()) {
                    (Ok(add), Some(qh)) => {
                        let trace = add.trace;
                        self.add_prepared(add, &qh);
                        super::super::timing::mark_request("shelf_model_ack", trace);
                        crate::ipc::Response::ok(None)
                    }
                    (Ok(add), None) => {
                        let _ = std::fs::remove_file(&add.path);
                        crate::ipc::Response::error("shelf is not ready")
                    }
                    (Err(error), _) => {
                        eprintln!("boltsnap daemon: {error}");
                        crate::ipc::Response::error(error)
                    }
                };
                self.write_response(stream, response);
            }
            DaemonEvent::VideoPrepared {
                result,
                stream,
                _client,
            } => {
                let response = match (result, self.qh.is_some()) {
                    (Ok(clip), true) => {
                        let path = clip.path.clone();
                        self.add_finalized_cards(vec![clip], true);
                        crate::ipc::Response::ok_path(path)
                    }
                    (Ok(clip), false) => {
                        let _ = std::fs::remove_file(&clip.path);
                        crate::ipc::Response::error("shelf is not ready")
                    }
                    (Err(error), _) => {
                        eprintln!("boltsnap daemon: {error}");
                        crate::ipc::Response::error(error)
                    }
                };
                self.write_response(stream, response);
            }
            DaemonEvent::ChildrenStopped { after, result } => {
                let (segments, error) = match result {
                    StopChildrenResult::Ready(segments) => (segments, None),
                    StopChildrenResult::Failed { kept, error } => (kept, Some(error)),
                };
                match after {
                    AfterStop::Pause | AfterStop::Recover(_) => {
                        let recovery = match after {
                            AfterStop::Recover(error) => Some(error),
                            _ => error,
                        };
                        if let Some(session) = self.recording.as_mut() {
                            let _ = session.finish_pause(segments);
                            if let Some(error) = recovery {
                                session.last_error = Some(error.clone());
                                notify(&format!("Recording paused: {error}"));
                            }
                        }
                        self.draw_popup();
                        if std::mem::take(&mut self.controls_pending)
                            && let Some(qh) = self.qh.clone()
                        {
                            let _ = self.show_recording_controls(&qh);
                        }
                    }
                    AfterStop::Save(destination) => {
                        if let Some(error) = error {
                            if let Some(session) = self.recording.as_mut() {
                                session.add_completed(segments);
                                let _ = session.finalize_failed(error.clone());
                            }
                            notify(&format!("Recording could not be finalized: {error}"));
                            self.draw_popup();
                        } else {
                            if let Some(session) = self.recording.as_mut() {
                                session.add_completed(segments);
                            }
                            self.start_finalize_worker(destination);
                        }
                    }
                    AfterStop::Discard => self.finish_discard(segments),
                }
            }
            DaemonEvent::ReplayClipReady {
                path,
                output,
                reply,
            } => {
                self.add_finalized_cards(
                    vec![FinalizedClip {
                        path,
                        output: Some(output),
                        permanent: false,
                    }],
                    false,
                );
                let _ = reply.send(());
            }
            DaemonEvent::Finalized(Ok(clips)) => {
                let shelf_card = self
                    .recording
                    .as_ref()
                    .is_some_and(|session| session.shelf_card_for_disk);
                let audio = self
                    .recording
                    .as_mut()
                    .and_then(|session| session.audio.take());
                self.recording = None;
                cleanup_audio_async(audio);
                self.remove_marker();
                self.remove_popup();
                self.add_finalized_cards(clips, shelf_card);
            }
            DaemonEvent::Finalized(Err(failure)) => {
                let shelf_card = self
                    .recording
                    .as_ref()
                    .is_some_and(|session| session.shelf_card_for_disk);
                self.add_finalized_cards(failure.completed, shelf_card);
                if let Some(session) = self.recording.as_mut() {
                    session.completed = failure.recoverable_segments;
                    let _ = session.save_failed(failure.error.clone());
                }
                notify(&format!(
                    "Recording could not be finalized: {}",
                    failure.error
                ));
                self.draw_popup();
            }
            DaemonEvent::CardSaved {
                id,
                promote,
                result,
            } => {
                let close = self.saves.finish(id, result.is_ok());
                match result {
                    Ok(path) => {
                        eprintln!("boltsnap daemon: saved {}", path.display());
                        if !promote || self.model.promote(id, path) {
                            self.flash_saved(id);
                        }
                        if close {
                            self.start_anim(id, AnimKind::Disappear);
                        }
                        // Evictions skipped while the card was saving.
                        self.trim_shelf_cache();
                        self.relayout();
                        if let Some(qh) = self.qh.clone() {
                            self.draw(&qh);
                        }
                    }
                    Err(error) => {
                        eprintln!("boltsnap daemon: save failed: {error}");
                        let what = if promote { "Video" } else { "Screenshot" };
                        notify(&format!("{what} could not be saved: {error}"));
                    }
                }
            }
            DaemonEvent::FocusResolved(snapshot) => {
                let connected = self.cached_monitors();
                if let Some(output) = snapshot.as_ref().ok().and_then(|monitors| {
                    monitors
                        .iter()
                        .find(|monitor| monitor.focused)
                        .filter(|monitor| {
                            connected.iter().any(|output| output.name == monitor.name)
                        })
                        .map(|monitor| monitor.name.clone())
                }) {
                    self.focused_output = Some(output);
                }
                self.focus_query_pending = false;
                self.finish_pending_default_recordings(&snapshot);
                if std::mem::take(&mut self.pending_tray_default_recording)
                    && let Err(error) = self.start_default_recording(&snapshot)
                {
                    notify(&error);
                }
                self.finish_pending_controls();
                if self.tray.is_none() {
                    self.start_tray();
                } else {
                    self.publish_tray_snapshot();
                }
            }
            DaemonEvent::ReplayStateChanged => self.publish_tray_snapshot(),
            DaemonEvent::Tray(action) => self.handle_tray_action(action),
            DaemonEvent::SettingsWritten { reply, error } => self.settings_written(reply, error),
            DaemonEvent::ConfigChanged => {
                self.refresh_prefs();
                self.publish_settings();
            }
        }
        self.publish_recording_snapshot();
    }
}
