//! The daemon socket: reading requests off clients, writing their answers, and
//! importing a client's video into the shelf.

use std::os::unix::net::UnixStream;

use crate::record::finalize::{
    FinalizedClip, RECORDING_CACHE_LIMIT_BYTES, check_recording_cache_capacity,
};
use crate::record::session::RecordingAction;

use super::actions::PreparedAdd;
use super::cards::{prepare_add, prepare_pixels};
use super::controls::recording_controls_visible;
use super::events::DaemonEvent;
use super::finish::{MAX_RECORDING_WATCHERS, watcher_connected};
use super::limits::{ClientLimits, Limit};
use super::start::prepare_recording_cache;
use super::{Daemon, cards};

/// Copy a client's video into the recording cache so the card owns its bytes;
/// later changes to the original do not reach the shelf.
fn prepare_shelf_video(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dir = crate::paths::rec_dir();
    import_shelf_video(path, &dir, |bytes| {
        prepare_recording_cache()?;
        check_recording_cache_capacity(&dir, bytes)
    })
}

/// Opens `path` once, following a symlink, checks that very file and copies
/// the size it had into a new file in `dir`. `reserve` vets that size first.
pub(super) fn import_shelf_video(
    path: &std::path::Path,
    dir: &std::path::Path,
    reserve: impl FnOnce(u64) -> Result<(), String>,
) -> Result<std::path::PathBuf, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // Nonblocking, so a FIFO fails the file check instead of hanging the open.
    let input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("open shelf video: {error}"))?;
    let metadata = input
        .metadata()
        .map_err(|error| format!("inspect shelf video: {error}"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("shelf video is not a non-empty file".into());
    }
    if metadata.len() >= RECORDING_CACHE_LIMIT_BYTES {
        return Err("video exceeds the 2 GiB temporary shelf limit".into());
    }
    reserve(metadata.len())?;
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("mp4");
    let target = crate::paths::unique_path_in(dir, "shelf-video", ext);
    crate::platform::files::copy_noreplace(&mut input.take(metadata.len()), &target)
        .map_err(|error| format!("copy shelf video: {error}"))?;
    // A source that shrank while copying would leave a cut clip.
    match std::fs::metadata(&target) {
        Ok(copied) if copied.len() == metadata.len() => Ok(target),
        _ => {
            let _ = std::fs::remove_file(&target);
            Err("shelf video changed while it was copied".into())
        }
    }
}

pub(super) fn validate_recording_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        Err("recording width and height must be greater than zero".into())
    } else {
        Ok(())
    }
}

pub(super) fn reject_busy(mut stream: UnixStream) {
    use std::io::Write;
    // Never block calloop or create another thread just to reject excess work.
    let _ = stream.set_nonblocking(true);
    let _ = stream.write_all(&crate::ipc::Response::error("shelf is busy; retry shortly").encode());
}

pub(super) fn spawn_client_reader(
    mut stream: UnixStream,
    tx: calloop::channel::Sender<DaemonEvent>,
    limits: &ClientLimits,
) {
    let Some(client) = limits.readers.acquire() else {
        reject_busy(stream);
        return;
    };
    let images = limits.images.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        // Reserve before Request::read allocates the declared payload. The
        // protocol decoder still validates both lengths and the full request.
        let mut prefix = [0; 8];
        if stream.read_exact(&mut prefix).is_err() {
            return;
        }
        let header_len = u32::from_be_bytes(prefix[..4].try_into().unwrap()) as usize;
        let payload_len = u32::from_be_bytes(prefix[4..].try_into().unwrap()) as usize;
        if header_len > crate::protocol::MAX_HEADER_BYTES
            || payload_len > crate::protocol::MAX_PAYLOAD_BYTES
        {
            return;
        }
        let image = if payload_len > 0 {
            let Some(permit) = images.acquire() else {
                reject_busy(stream);
                return;
            };
            Some(permit)
        } else {
            None
        };
        let mut header = vec![0; header_len];
        if stream.read_exact(&mut header).is_err() {
            return;
        }
        let value = serde_json::from_slice::<serde_json::Value>(&header).ok();
        if value
            .as_ref()
            .is_some_and(|v| v["cmd"] == super::super::image_transfer::COMMAND)
        {
            let Some(image) = image.or_else(|| images.acquire()) else {
                reject_busy(stream);
                return;
            };
            let result = (|| -> Result<PreparedAdd, String> {
                use std::io::Write;
                if payload_len != 0 {
                    return Err("pixel offer must not contain a payload".into());
                }
                super::super::image_transfer::same_user(&stream).map_err(|e| e.to_string())?;
                let metadata =
                    super::super::image_transfer::Metadata::parse(value.as_ref().unwrap())
                        .map_err(|e| e.to_string())?;
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                    .map_err(|e| e.to_string())?;
                crate::protocol::write_frame(
                    &mut stream,
                    serde_json::json!({"ready":super::super::image_transfer::COMMAND})
                        .to_string()
                        .as_bytes(),
                    &[],
                )
                .map_err(|e| e.to_string())?;
                stream.flush().map_err(|e| e.to_string())?;
                let fd =
                    super::super::image_transfer::receive_fd(&stream).map_err(|e| e.to_string())?;
                let pixels = super::super::image_transfer::Pixels::map(fd, metadata.len)
                    .map_err(|e| e.to_string())?;
                super::super::timing::mark_request("pixels_received", metadata.trace);
                prepare_pixels(metadata, &pixels)
            })();
            let _ = tx.send(DaemonEvent::AddPrepared {
                result,
                stream,
                _client: client,
                _image: Some(image),
            });
            return;
        }
        let request = crate::ipc::Request::read(
            &mut prefix
                .as_slice()
                .chain(header.as_slice())
                .chain(&mut stream),
        );
        match request {
            Ok(crate::ipc::Request::Add {
                source,
                png,
                output,
                thumb,
            }) => {
                let trace = value.as_ref().and_then(|v| v["trace"].as_u64());
                let _timing = super::super::timing::Span::for_request("shelf_png_prepare", trace);
                // Trust a client thumbnail only from our own user.
                let thumb =
                    thumb.filter(|_| super::super::image_transfer::same_user(&stream).is_ok());
                let result =
                    prepare_add(png, source, output, thumb, cards::card_size()).map(|mut add| {
                        add.trace = trace;
                        add
                    });
                let _ = tx.send(DaemonEvent::AddPrepared {
                    result,
                    stream,
                    _client: client,
                    _image: image,
                });
            }
            Ok(crate::ipc::Request::AddVideo { path, output, .. }) => {
                // The daemon reads the named file, so only its own user may name one.
                let result = super::super::image_transfer::same_user(&stream)
                    .map_err(|error| error.to_string())
                    .and_then(|()| prepare_shelf_video(&path))
                    .map(|path| FinalizedClip {
                        output,
                        path,
                        permanent: false,
                    });
                let _ = tx.send(DaemonEvent::VideoPrepared {
                    result,
                    stream,
                    _client: client,
                });
            }
            Ok(request) => {
                let _ = tx.send(DaemonEvent::ClientRequest {
                    request,
                    stream,
                    _permit: client,
                });
            }
            Err(error) => eprintln!("boltsnap daemon: bad request: {error}"),
        }
    });
}

pub(super) fn spawn_client_writer(mut stream: UnixStream, bytes: Vec<u8>) {
    static WRITERS: std::sync::LazyLock<Limit> = std::sync::LazyLock::new(|| Limit::new(16));
    let Some(permit) = WRITERS.acquire() else {
        reject_busy(stream);
        return;
    };
    std::thread::spawn(move || {
        let _permit = permit;
        use std::io::Write;
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
        let _ = stream.write_all(&bytes);
    });
}

impl Daemon {
    /// Handle one request already read by a worker, keeping IPC reads off calloop.
    pub(super) fn handle_client_request(
        &mut self,
        req: crate::ipc::Request,
        mut stream: std::os::unix::net::UnixStream,
    ) {
        use std::io::Write;
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return,
        };
        match req {
            crate::ipc::Request::Ping => {
                spawn_client_writer(stream, b"PONG".to_vec());
            }
            crate::ipc::Request::Add { .. } | crate::ipc::Request::AddVideo { .. } => {
                self.write_response(
                    stream,
                    crate::ipc::Response::error("add request was not prepared"),
                );
            }
            crate::ipc::Request::RecordingStatus => {
                self.write_response(
                    stream,
                    crate::ipc::Response::ok(Some(self.recording_snapshot())),
                );
            }
            crate::ipc::Request::RecordingWatch => {
                // Idle status never changes, so closed watchers are only noticed
                // here; the limit caps the live ones.
                self.watchers.retain(watcher_connected);
                if self.watchers.len() >= MAX_RECORDING_WATCHERS {
                    eprintln!(
                        "boltsnap daemon: refused a recording watch; {MAX_RECORDING_WATCHERS} are open"
                    );
                    return;
                }
                let _ = stream.set_read_timeout(None);
                let _ = stream.set_nonblocking(true);
                let line = self.recording_snapshot().to_json_line();
                if matches!(stream.write(line.as_bytes()), Ok(written) if written == line.len()) {
                    self.watchers.push(stream);
                }
            }
            crate::ipc::Request::GetSettings => self.send_settings(stream),
            crate::ipc::Request::SetSettings { patch } => self.set_settings(patch, stream),
            crate::ipc::Request::SettingsWatch => self.watch_settings(stream),
            crate::ipc::Request::ShowRecordingControls => {
                if !self
                    .recording
                    .as_ref()
                    .is_some_and(|session| recording_controls_visible(session.phase))
                {
                    self.write_response(
                        stream,
                        crate::ipc::Response::error("recording controls are unavailable"),
                    );
                } else {
                    self.pending_controls.push(stream);
                    self.refresh_focus();
                }
            }
            crate::ipc::Request::RecordingControl { action } => {
                let response = match self.handle_recording_action(action, &qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::StartDefaultRecording => {
                if self.recording.is_some() {
                    self.write_response(
                        stream,
                        crate::ipc::Response::error("a recording is already in progress"),
                    );
                } else {
                    self.pending_default_recordings.push(stream);
                    self.refresh_focus();
                }
            }
            crate::ipc::Request::StartRecording {
                x,
                y,
                w,
                h,
                show_frame,
                audio_enabled,
            } => {
                // The selector saved what it changed; the values are this
                // recording's only, so a change made during the selection stays.
                let geo = crate::record::Geometry { x, y, w, h };
                let response = match self.start_recording(geo, show_frame, audio_enabled, &qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::StartRecordingOutput { name } => {
                let response = match self.start_named_recording_outputs(
                    vec![name],
                    crate::config::RecordBothMode::Separate,
                ) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::StartRecordingOutputs { names, combined } => {
                let response = match self.start_named_recording_outputs(
                    names,
                    if combined {
                        crate::config::RecordBothMode::Combined
                    } else {
                        crate::config::RecordBothMode::Separate
                    },
                ) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            // Its old handler read and then deleted any path a client named.
            crate::ipc::Request::RecordingThumb { .. } => self.write_response(
                stream,
                crate::ipc::Response::error("recording_thumb is no longer supported"),
            ),
            crate::ipc::Request::StopRecording => {
                let response = match self.handle_recording_action(RecordingAction::SaveShelf, &qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
        }
    }

    pub(super) fn write_response(&self, stream: UnixStream, response: crate::ipc::Response) {
        spawn_client_writer(stream, response.encode());
    }
}
