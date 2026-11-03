//! `record`, `recording …` and `stop`: starting a recording from the
//! selector and steering the daemon's recording.

use std::io::{BufRead, BufReader, Write};

use super::{require_daemon_success, usage};
use crate::{Args, DynResult};

/// Hotkeys and the tray run `record` without a terminal, so an error on stderr
/// alone looks like a selection that silently did nothing.
#[cfg(target_os = "linux")]
pub(crate) fn record_flow(args: &Args) -> DynResult<()> {
    start_recording_flow(args).inspect_err(|error| {
        let error = error.to_string();
        // A clip from the selector names itself.
        let message = if error.starts_with("Replay clip:") {
            error
        } else {
            format!("Recording: {error}")
        };
        crate::platform::shelf::notify(&message);
    })
}

#[cfg(target_os = "linux")]
fn start_recording_flow(args: &Args) -> DynResult<()> {
    let state = match crate::ipc::call_daemon(crate::ipc::Request::RecordingStatus) {
        Ok(response) => recording_state_from_status(response)?,
        // Compatibility with the Task-4 daemon, which closes unknown requests.
        Err(error) if is_legacy_recording_status_eof(&error) => {
            crate::record::session::PublicRecordingState::Idle
        }
        Err(error) => return Err(format!("recording daemon unavailable: {error}").into()),
    };
    if state != crate::record::session::PublicRecordingState::Idle {
        checked_recording_call(crate::ipc::Request::ShowRecordingControls)?;
        return Ok(());
    }

    // Optional target: `boltsnap record full` uses the daemon's configured
    // fullscreen target; absent or `area` opens the region selector.
    let target = args
        .image
        .as_deref()
        .and_then(|p| p.to_str())
        .unwrap_or("area");
    if matches!(target, "full" | "screen" | "fullscreen") {
        require_recorder()?;
        checked_recording_call(crate::ipc::Request::StartDefaultRecording)?;
        return Ok(());
    }
    let prefs = crate::config::Config::load().recording_prefs();
    let mut selection = crate::selector::run_select_record(prefs.show_frame, prefs.audio_enabled)?;
    let frozen = selection.frozen.take();
    if selection.clip {
        let frozen = frozen.ok_or("replay buffer is not ready")?;
        let rect = selection.rect.ok_or("clip selection is missing")?;
        // `record_flow` reports the error; the prefix tells it apart.
        frozen
            .save(rect, selection.surface_size)
            .map_err(|error| format!("Replay clip: {error}"))?;
        return Ok(());
    }
    drop(frozen);
    let changed = |old: bool, new: bool| (old != new).then_some(new);
    let saved = crate::config::save_selector_prefs(
        changed(prefs.show_frame, selection.show_frame),
        changed(prefs.audio_enabled, selection.audio_enabled),
    );
    let Some(rect) = selection.rect else {
        // Cancelled: nothing else saves these, so the error is reported here.
        return saved
            .map_err(|error| format!("could not save the selector settings: {error}").into());
    };
    if let Err(error) = saved {
        // The recording still starts, with the values chosen here.
        let message = format!("Recording settings could not be saved: {error}");
        eprintln!("boltsnap: {message}");
        crate::platform::shelf::notify(&message);
    }
    require_recorder()?;
    let (ox, oy) = selection
        .output_origin
        .or_else(crate::shelf::focused_monitor_origin)
        .unwrap_or((0, 0));
    let geo = crate::record::to_global_geometry(rect.x, rect.y, rect.w, rect.h, ox, oy);
    checked_recording_call(crate::ipc::Request::StartRecording {
        x: geo.x,
        y: geo.y,
        w: geo.w,
        h: geo.h,
        show_frame: selection.show_frame,
        audio_enabled: selection.audio_enabled,
    })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn require_recorder() -> DynResult<()> {
    if crate::paths::has_cmd("gpu-screen-recorder") || crate::paths::has_cmd("wf-recorder") {
        Ok(())
    } else {
        Err("no screen recorder found; install gpu-screen-recorder to start a recording".into())
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn record_flow(args: &Args) -> DynResult<()> {
    let state = match crate::ipc::call_daemon(crate::ipc::Request::RecordingStatus) {
        Ok(response) => recording_state_from_status(response)?,
        Err(error) if is_legacy_recording_status_eof(&error) => {
            crate::record::session::PublicRecordingState::Idle
        }
        Err(error) => return Err(format!("recording daemon unavailable: {error}").into()),
    };
    if state != crate::record::session::PublicRecordingState::Idle {
        checked_recording_call(crate::ipc::Request::ShowRecordingControls)?;
        return Ok(());
    }

    let target = args
        .image
        .as_deref()
        .and_then(|path| path.to_str())
        .unwrap_or("area");
    if matches!(target, "full" | "screen" | "fullscreen") {
        checked_recording_call(crate::ipc::Request::StartDefaultRecording)?;
        return Ok(());
    }

    let mut prefs = crate::config::Config::load().recording_prefs();
    let selection = crate::selector::run_select_record(prefs.show_frame, prefs.audio_enabled)?;
    if selection.show_frame != prefs.show_frame || selection.audio_enabled != prefs.audio_enabled {
        prefs.show_frame = selection.show_frame;
        prefs.audio_enabled = selection.audio_enabled;
        crate::config::save_recording_prefs(&prefs)?;
    }
    let Some(rect) = selection.rect else {
        return Ok(());
    };
    let (origin_x, origin_y) = crate::shelf::focused_monitor_origin().unwrap_or((0, 0));
    let geometry =
        crate::record::to_global_geometry(rect.x, rect.y, rect.w, rect.h, origin_x, origin_y);
    checked_recording_call(crate::ipc::Request::StartRecording {
        x: geometry.x,
        y: geometry.y,
        w: geometry.w,
        h: geometry.h,
        show_frame: prefs.show_frame,
        audio_enabled: selection.audio_enabled,
    })?;
    Ok(())
}

pub(super) fn is_legacy_recording_status_eof(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::UnexpectedEof
}

pub(super) fn recording_state_from_status(
    response: crate::ipc::Response,
) -> Result<crate::record::session::PublicRecordingState, String> {
    if !response.ok {
        return Err(response
            .error
            .unwrap_or_else(|| "recording status failed".into()));
    }
    response
        .snapshot
        .map(|snapshot| snapshot.state)
        .ok_or_else(|| "daemon returned no recording status".into())
}

pub(crate) fn recording_command(args: &Args) -> DynResult<()> {
    use crate::record::session::RecordingAction;

    match (args.tail.as_slice(), args.json) {
        ([command], true) if command == "status" => {
            let response = checked_recording_call(crate::ipc::Request::RecordingStatus)?;
            let snapshot = response
                .snapshot
                .ok_or("daemon returned no recording status")?;
            print!("{}", snapshot.to_json_line());
            std::io::stdout().flush()?;
            Ok(())
        }
        ([command], true) if command == "watch" => {
            let stream = crate::ipc::watch_recording()
                .map_err(|error| format!("recording daemon unavailable: {error}"))?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let mut lines = 0;
            while reader.read_line(&mut line)? != 0 {
                print!("{line}");
                std::io::stdout().flush()?;
                line.clear();
                lines += 1;
            }
            if lines == 0 {
                // The daemon refuses watches beyond its limit by closing.
                return Err("the daemon closed the watch without a status".into());
            }
            Ok(())
        }
        ([command], false) if command == "show-controls" => {
            checked_recording_call(crate::ipc::Request::ShowRecordingControls)?;
            Ok(())
        }
        ([command], false) if command == "pause" => recording_control(RecordingAction::Pause),
        ([command], false) if command == "resume" => recording_control(RecordingAction::Resume),
        ([command], false) if command == "save-shelf" => {
            recording_control(RecordingAction::SaveShelf)
        }
        ([command], false) if command == "save-disk" => {
            recording_control(RecordingAction::SaveDisk)
        }
        ([command], false) if command == "discard" => recording_control(RecordingAction::Discard),
        _ => Err(format!("invalid recording command\n{}", usage()).into()),
    }
}

pub(crate) fn recording_control(action: crate::record::session::RecordingAction) -> DynResult<()> {
    checked_recording_call(crate::ipc::Request::RecordingControl { action })?;
    Ok(())
}

fn checked_recording_call(request: crate::ipc::Request) -> DynResult<crate::ipc::Response> {
    let response = crate::ipc::call_daemon(request)
        .map_err(|error| format!("recording daemon unavailable: {error}"))?;
    require_daemon_success(response, "recording command failed").map_err(Into::into)
}
