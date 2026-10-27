//! The end of a recording: finalizing or discarding it, the clips as cards, and
//! the state that watch clients follow.

use std::os::unix::net::UnixStream;

use crate::record::finalize::{
    FinalizeRequest, FinalizedClip, SaveDestination, finalize_recording,
};
use crate::record::session::{CaptureScope, RecorderTools, StoppedSegment};
use crate::shelf::model::{CardKind, FileLifetime};

use super::anim::AnimKind;
use super::events::DaemonEvent;
use super::placement::{card_output, focused_monitor_name};
use super::start::cleanup_audio_async;
use super::{Daemon, cards};

pub(super) const MAX_RECORDING_WATCHERS: usize = 16;

/// False once a watch client hung up. Watch clients send nothing after their
/// request, so pending bytes are left unread and count as connected.
pub(super) fn watcher_connected(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut byte = 0_u8;
    let peeked = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&raw mut byte).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    match peeked {
        0 => false,
        1.. => true,
        _ => std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock,
    }
}

impl Daemon {
    pub(super) fn start_finalize_worker(&mut self, destination: SaveDestination) {
        let Some(session) = self.recording.as_mut() else {
            return;
        };
        let request = FinalizeRequest {
            segments: std::mem::take(&mut session.completed),
            monitors: session.monitors.clone(),
            both_mode: session.both_mode,
            codec: session.codec.clone(),
            fps: session.profile.fps(),
            destination,
            cursor: session.cursor,
            region: match &session.scope {
                CaptureScope::Area(geometry) => Some(*geometry),
                CaptureScope::Outputs(_) => None,
            },
        };
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = finalize_recording(request, &RecorderTools::default());
            let _ = tx.send(DaemonEvent::Finalized(result));
        });
    }

    pub(super) fn finish_discard(&mut self, segments: Vec<StoppedSegment>) {
        if let Some(session) = self.recording.as_mut() {
            session.add_completed(segments);
        }
        let (paths, audio) = match self.recording.take() {
            Some(session) => (
                session
                    .completed
                    .into_values()
                    .flatten()
                    .collect::<Vec<_>>(),
                session.audio,
            ),
            None => (Vec::new(), None),
        };
        cleanup_audio_async(audio);
        self.remove_marker();
        self.remove_popup();
        std::thread::spawn(move || {
            let cache = crate::paths::rec_dir();
            for path in paths {
                if path.starts_with(&cache) {
                    for track in super::super::cursor_track::track_paths(&path) {
                        let _ = std::fs::remove_file(track);
                    }
                    let mut timestamps = path.clone().into_os_string();
                    timestamps.push(".ts");
                    let _ = std::fs::remove_file(timestamps);
                    let _ = std::fs::remove_file(path);
                }
            }
        });
    }

    /// `shelf_card` adds cards for disk saves even when `record_disk_add_to_shelf`
    /// is off: the user saved to the shelf and only the length sent it to disk.
    pub(super) fn add_finalized_cards(&mut self, clips: Vec<FinalizedClip>, shelf_card: bool) {
        let Some(qh) = self.qh.clone() else {
            return;
        };
        let mut added = false;
        let on_output = card_output(&clips, focused_monitor_name);
        for clip in clips {
            if clip.permanent && !self.recording_prefs.disk_add_to_shelf && !shelf_card {
                continue;
            }
            let size = cards::card_size();
            let placeholder =
                image::RgbaImage::from_pixel(size.w, size.h, image::Rgba([32, 32, 40, 255]));
            let source = clip
                .output
                .as_deref()
                .map(|output| format!("record:{output}"))
                .unwrap_or_else(|| "record".into());
            let id = self.model.add_kind_with_lifetime(
                clip.path.clone(),
                placeholder,
                source,
                CardKind::Video,
                if clip.permanent {
                    FileLifetime::Permanent
                } else {
                    FileLifetime::Temporary
                },
            );
            self.start_anim(id, AnimKind::Appear);
            self.thumbnails.submit(id, clip.path);
            added = true;
        }
        if added {
            self.trim_shelf_cache();
            self.relayout();
            self.place_on_output(on_output.as_deref(), &qh);
            self.draw(&qh);
        }
    }

    pub(super) fn recording_snapshot(&self) -> crate::ipc::RecordingSnapshot {
        let Some(session) = self.recording.as_ref() else {
            return crate::ipc::RecordingSnapshot::idle();
        };
        let (scope, outputs) = match &session.scope {
            CaptureScope::Area(_) => (
                "area".to_string(),
                session
                    .monitors
                    .iter()
                    .map(|monitor| monitor.name.clone())
                    .collect(),
            ),
            CaptureScope::Outputs(outputs) => (
                if outputs.len() > 1 { "both" } else { "output" }.to_string(),
                outputs.clone(),
            ),
        };
        let elapsed_ms = session.elapsed_at(std::time::Instant::now()).as_millis() as u64;
        crate::ipc::RecordingSnapshot {
            state: session.public_state(),
            elapsed_ms: elapsed_ms / 1000 * 1000,
            scope,
            outputs,
            actions_enabled: session.actions_enabled(),
            error: session.last_error.clone(),
        }
    }

    pub(super) fn publish_recording_snapshot(&mut self) {
        use std::io::Write;
        let snapshot = self.recording_snapshot();
        if snapshot == self.last_recording_snapshot {
            return;
        }
        let state_changed = snapshot.state != self.last_recording_snapshot.state;
        let line = snapshot.to_json_line();
        self.watchers.retain_mut(
            |watcher| matches!(watcher.write(line.as_bytes()), Ok(n) if n == line.len()),
        );
        self.last_recording_snapshot = snapshot;
        if state_changed {
            self.publish_tray_snapshot();
        }
    }
}
