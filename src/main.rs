use std::env;
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

mod config;
mod editor;
mod platform;
mod record;
mod selector;
mod shelf;

pub(crate) use crate::platform::{capture, clipboard, ipc, paths, tray};
pub(crate) use boltsnap::{image_model, protocol};

use crate::capture::{capture, strip_uniform_border};
use crate::clipboard::{copy_to_clipboard, serve_wayland_clipboard, serve_wayland_uri_list};
use crate::editor::run_editor;
use crate::paths::*;

pub type DynResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    Auto,
    X11,
    Wayland,
    Windows,
}

impl Backend {
    fn parse(value: &str) -> DynResult<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "x11" => Ok(Self::X11),
            "wayland" => Ok(Self::Wayland),
            "windows" => Ok(Self::Windows),
            other => Err(format!("unknown backend '{other}', use auto/x11/wayland/windows").into()),
        }
    }

    fn resolved(self) -> DynResult<Self> {
        match self {
            Self::Auto => detect_backend(),
            other => Ok(other),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Windows => "windows",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureMode {
    Area,
    Full,
    Window,
    ActiveWindow,
}

impl CaptureMode {
    fn parse(command: &str) -> DynResult<Self> {
        match command {
            "area" | "select" | "selection" | "region" => Ok(Self::Area),
            "full" | "screen" | "fullscreen" => Ok(Self::Full),
            "window" | "win" | "select-window" => Ok(Self::Window),
            "active" | "active-window" | "current" | "current-window" => Ok(Self::ActiveWindow),
            other => Err(format!("unknown capture command '{other}'").into()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Full => "full",
            Self::Window => "window",
            Self::ActiveWindow => "active-window",
        }
    }
}

#[derive(Clone, Debug)]
struct Args {
    command: String,
    command_explicit: bool,
    image: Option<PathBuf>,
    edit: bool,
    copy: bool,
    /// True if the user passed --copy or --no-copy explicitly. On Wayland the
    /// shelf is the default sink, so we only auto-copy when copy was asked for.
    copy_explicit: bool,
    save: bool,
    output: Option<PathBuf>,
    backend: Backend,
    /// Skip the selector's editable phase: release captures immediately.
    /// Supported by interactive Wayland and Windows region selectors.
    instant: bool,
    /// Override the shelf save directory (daemon).
    save_dir: Option<PathBuf>,
    /// Override the annotation editor command (edit).
    editor_cmd: Option<String>,
    tail: Vec<String>,
    json: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "area".to_string(),
            command_explicit: false,
            image: None,
            edit: false,
            copy: true,
            copy_explicit: false,
            save: false,
            output: None,
            backend: Backend::Auto,
            instant: false,
            save_dir: None,
            editor_cmd: None,
            tail: Vec::new(),
            json: false,
        }
    }
}

/// What to do with a freshly captured screenshot, after backend resolution.
#[derive(Debug, PartialEq, Eq)]
enum PostCapture {
    Stdout,
    Edit,
    File { copy: bool },
    Shelf { copy: bool },
    CopyOnly,
}

/// Decide the post-capture sink. `backend` is already resolved (no Auto).
fn decide_post_capture(args: &Args, backend: Backend) -> PostCapture {
    if is_stdout_target(args) {
        return PostCapture::Stdout;
    }
    if args.edit {
        return PostCapture::Edit;
    }
    if args.output.is_some() || args.save {
        return PostCapture::File { copy: args.copy };
    }
    match backend {
        Backend::Wayland | Backend::Windows => PostCapture::Shelf {
            copy: args.copy_explicit && args.copy,
        },
        _ => PostCapture::CopyOnly,
    }
}

fn usage() -> &'static str {
    "\
Usage:
  boltsnap [area|full|window|active-window] [-o PATH|-] [--save] [--no-copy] [--instant] [--backend auto|x11|wayland|windows]
  boltsnap area --instant                 select a region, capture on release (no edit handles)
  boltsnap --edit                         open last screenshot in editor
  boltsnap [area|window|full] --edit      capture then edit
  boltsnap edit [IMAGE] [-o PATH] [--no-copy]
  boltsnap daemon [--save-dir DIR]        run the screenshot shelf
  boltsnap record [--editor CMD]          select an area and screen-record it
  boltsnap record full                    record the configured fullscreen target (no selector)
  boltsnap recording status --json
  boltsnap recording watch --json
  boltsnap recording show-controls
  boltsnap recording pause|resume|save-shelf|save-disk|discard
  boltsnap stop                           compatibility alias for recording save-shelf
  boltsnap [COMMAND] [--editor CMD]       annotate with a specific editor
  Config: platform config directory       (save_dir, editor, record_codec, record_dir)
  boltsnap doctor

Examples:
  boltsnap                                area, copy PNG, remember as last
  boltsnap --edit                         open last screenshot in editor
  boltsnap window                         pick window, copy PNG
  boltsnap full --no-copy -o /tmp/x.png   write file, no clipboard
  boltsnap area --no-copy -o - | eddy -f -      pipe to external editor
"
}

fn parse_args(raw: &[String]) -> DynResult<Args> {
    let mut args = Args::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            "--version" => {
                println!("boltsnap {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--edit" => args.edit = true,
            "--copy" => {
                args.copy = true;
                args.copy_explicit = true;
            }
            "--no-copy" => {
                args.copy = false;
                args.copy_explicit = true;
            }
            "--instant" => args.instant = true,
            "--save" => args.save = true,
            "--json" => args.json = true,
            "-o" | "--output" => {
                i += 1;
                let Some(path) = raw.get(i) else {
                    return Err("--output needs a path".into());
                };
                args.output = Some(PathBuf::from(path));
            }
            "--backend" => {
                i += 1;
                let Some(value) = raw.get(i) else {
                    return Err("--backend needs auto, x11, or wayland".into());
                };
                args.backend = Backend::parse(value)?;
            }
            "--save-dir" => {
                i += 1;
                let Some(path) = raw.get(i) else {
                    return Err("--save-dir needs a path".into());
                };
                args.save_dir = Some(PathBuf::from(path));
            }
            "--editor" => {
                i += 1;
                let Some(cmd) = raw.get(i) else {
                    return Err("--editor needs a command".into());
                };
                args.editor_cmd = Some(cmd.clone());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option '{value}'\n{}", usage()).into());
            }
            value => positional.push(value.to_string()),
        }
        i += 1;
    }

    if let Some(command) = positional.first() {
        args.command = command.to_lowercase();
        args.command_explicit = true;
    }
    if positional.len() > 1 {
        args.image = Some(PathBuf::from(&positional[1]));
    }
    args.tail = positional.into_iter().skip(1).collect();
    Ok(args)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("boltsnap: {err}");
        std::process::exit(2);
    }
}

fn run() -> DynResult<()> {
    let raw: Vec<String> = env::args().collect();
    let args = parse_args(&raw)?;

    if args.edit && !args.command_explicit {
        return edit_last_screenshot(&args);
    }

    match args.command.as_str() {
        "doctor" => {
            print_doctor();
            Ok(())
        }
        "self-test" => self_test(),
        "record" => record_flow(&args),
        "recording" => recording_command(&args),
        "stop" => recording_control(crate::record::session::RecordingAction::SaveShelf),
        "daemon" => crate::shelf::run_daemon(args.save_dir.clone()),
        #[cfg(target_os = "windows")]
        "__install-autostart" => crate::platform::autostart::install(),
        #[cfg(target_os = "windows")]
        "__remove-autostart" => crate::platform::autostart::remove(),
        "__debug-render" => {
            // Render the shelf (one sample thumbnail, hovered) straight to a PNG
            // via the real draw path, so styling can be inspected without a
            // compositor. Usage: boltsnap __debug-render /tmp/out.png
            let out = args
                .image
                .clone()
                .unwrap_or_else(|| PathBuf::from("/tmp/boltsnap-debug-render.png"));
            return crate::shelf::debug_render(&out);
        }
        "__serve-clipboard" => {
            // Detached child kept alive to serve Wayland paste requests.
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard needs a PNG path")?;
            return serve_wayland_clipboard(&path);
        }
        "__serve-clipboard-uri" => {
            // Detached child kept alive to serve Wayland file-reference paste.
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard-uri needs a path")?;
            return serve_wayland_uri_list(&path);
        }
        "edit" => {
            let image = args.image.clone().unwrap_or(last_screenshot_path()?);
            ensure_file(&image)?;
            let output = edit_output_path(&args);
            let result = run_editor(
                normalize_path(&image),
                output,
                args.copy,
                args.backend,
                args.editor_cmd.clone(),
                None,
            )?;
            remember_last_screenshot(&result)?;
            println!("Edited image ready: {}", result.display());
            Ok(())
        }
        command => {
            CaptureMode::parse(command)?;
            capture_flow(&args)
        }
    }
}

fn edit_last_screenshot(args: &Args) -> DynResult<()> {
    let image = last_screenshot_path()?;
    ensure_file(&image)?;
    let result = run_editor(
        image.clone(),
        edit_output_path(args),
        args.copy,
        args.backend,
        args.editor_cmd.clone(),
        None,
    )?;
    remember_last_screenshot(&result)?;
    println!("Edited last screenshot: {}", result.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn record_flow(args: &Args) -> DynResult<()> {
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

    if !crate::paths::has_cmd("wf-recorder") {
        return Err(
            "wf-recorder not found — install it to record (e.g. pacman -S wf-recorder)".into(),
        );
    }
    // Optional target: `boltsnap record full` uses the daemon's configured
    // fullscreen target; absent or `area` opens the region selector.
    let target = args
        .image
        .as_deref()
        .and_then(|p| p.to_str())
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
        return Ok(()); // cancelled
    };
    let (ox, oy) = crate::shelf::focused_monitor_origin().unwrap_or((0, 0));
    let geo = crate::record::to_global_geometry(rect.x, rect.y, rect.w, rect.h, ox, oy);
    checked_recording_call(crate::ipc::Request::StartRecording {
        x: geo.x,
        y: geo.y,
        w: geo.w,
        h: geo.h,
        show_frame: prefs.show_frame,
        audio_enabled: selection.audio_enabled,
    })?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn record_flow(args: &Args) -> DynResult<()> {
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

fn is_legacy_recording_status_eof(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::UnexpectedEof
}

fn recording_state_from_status(
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

fn recording_command(args: &Args) -> DynResult<()> {
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
            while reader.read_line(&mut line)? != 0 {
                print!("{line}");
                std::io::stdout().flush()?;
                line.clear();
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

fn recording_control(action: crate::record::session::RecordingAction) -> DynResult<()> {
    checked_recording_call(crate::ipc::Request::RecordingControl { action })?;
    Ok(())
}

fn checked_recording_call(request: crate::ipc::Request) -> DynResult<crate::ipc::Response> {
    let response = crate::ipc::call_daemon(request)
        .map_err(|error| format!("recording daemon unavailable: {error}"))?;
    if response.ok {
        Ok(response)
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "recording command failed".into())
            .into())
    }
}

fn capture_flow(args: &Args) -> DynResult<()> {
    let mode = CaptureMode::parse(&args.command)?;

    if is_stdout_target(args) {
        return capture_to_stdout(mode, args.backend, args.instant);
    }

    // Capture to a temp file for --edit, then let the editor write the final
    // output. This avoids overwriting `-o PATH` before the user saves.
    let output = if args.edit {
        temp_png("shot")
    } else {
        target_path(args)
    };
    let (resolved, capture_output) = capture(mode, &output, args.backend, args.instant)?;
    if matches!(mode, CaptureMode::Window | CaptureMode::ActiveWindow) {
        let _ = strip_uniform_border(&output);
    }

    match decide_post_capture(args, resolved) {
        PostCapture::Stdout => unreachable!("handled above"),
        PostCapture::Edit => {
            let final_path = run_editor(
                output.clone(),
                edit_output_path(args),
                args.copy,
                resolved,
                args.editor_cmd.clone(),
                None,
            )?;
            remember_last_screenshot(&final_path)?;
            println!(
                "Boltsnap edited {} via {}: {}",
                mode.label(),
                resolved.as_str(),
                final_path.display()
            );
        }
        PostCapture::File { copy } => {
            if copy {
                copy_to_clipboard(&output, resolved)?;
            }
            remember_last_screenshot(&output)?;
            let verb = if copy { "copied" } else { "captured" };
            println!(
                "Boltsnap {verb} {} via {}: {}",
                mode.label(),
                resolved.as_str(),
                output.display()
            );
        }
        PostCapture::CopyOnly => {
            copy_to_clipboard(&output, resolved)?;
            remember_last_screenshot(&output)?;
            println!(
                "Boltsnap copied {} via {}: {}",
                mode.label(),
                resolved.as_str(),
                output.display()
            );
        }
        PostCapture::Shelf { copy } => {
            remember_last_screenshot(&output)?;
            if copy {
                copy_to_clipboard(&output, resolved)?;
            }
            let png = fs::read(&output)?;
            crate::ipc::send_to_shelf(crate::ipc::Request::Add {
                source: mode.label().to_string(),
                png,
                output: capture_output,
            })?;
            let suffix = if copy { " (copied)" } else { "" };
            println!("Boltsnap sent {} to shelf{}", mode.label(), suffix);
        }
    }
    Ok(())
}

fn is_stdout_target(args: &Args) -> bool {
    args.output.as_deref().and_then(|p| p.to_str()) == Some("-")
}

fn capture_to_stdout(mode: CaptureMode, backend: Backend, instant: bool) -> DynResult<()> {
    let tmp = temp_png("stdout");
    capture(mode, &tmp, backend, instant)?;
    let bytes = fs::read(&tmp)?;
    let _ = fs::remove_file(&tmp);
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}

fn detect_backend() -> DynResult<Backend> {
    #[cfg(target_os = "windows")]
    {
        return Ok(Backend::Windows);
    }

    #[cfg(target_os = "linux")]
    {
        let session = env::var("XDG_SESSION_TYPE")
            .unwrap_or_default()
            .to_lowercase();
        if session == "wayland" || (session.is_empty() && env::var_os("WAYLAND_DISPLAY").is_some())
        {
            Ok(Backend::Wayland)
        } else if session == "x11" || env::var_os("DISPLAY").is_some() {
            Ok(Backend::X11)
        } else {
            Err(
                "could not detect X11 or Wayland session; pass --backend x11 or --backend wayland"
                    .into(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_recording_status_fallback_accepts_only_eof() {
        assert!(is_legacy_recording_status_eof(&std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "old daemon closed the connection",
        )));
        assert!(!is_legacy_recording_status_eof(&std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "daemon stalled",
        )));
        assert!(!is_legacy_recording_status_eof(&std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed response",
        )));
    }

    #[test]
    fn successful_recording_status_requires_snapshot() {
        let error = recording_state_from_status(crate::ipc::Response::ok(None)).unwrap_err();
        assert_eq!(error, "daemon returned no recording status");

        let state = recording_state_from_status(crate::ipc::Response::ok(Some(
            crate::ipc::RecordingSnapshot::idle(),
        )))
        .unwrap();
        assert_eq!(state, crate::record::session::PublicRecordingState::Idle);
    }

    #[test]
    fn parser_recording_cli_forms() {
        for action in [
            "show-controls",
            "pause",
            "resume",
            "save-shelf",
            "save-disk",
            "discard",
        ] {
            let args = parse_args(&["boltsnap".into(), "recording".into(), action.into()]).unwrap();
            assert_eq!(args.command, "recording");
            assert_eq!(args.tail, [action]);
            assert!(!args.json);
        }
    }

    #[test]
    fn parser_recording_status_and_watch_require_json_flag_data() {
        for action in ["status", "watch"] {
            let args = parse_args(&[
                "boltsnap".into(),
                "recording".into(),
                action.into(),
                "--json".into(),
            ])
            .unwrap();
            assert_eq!(args.tail, [action]);
            assert!(args.json);
        }
    }

    #[test]
    fn parser_defaults_to_area_copy() {
        let args = parse_args(&["boltsnap".into()]).unwrap();
        assert_eq!(args.command, "area");
        assert!(!args.command_explicit);
        assert!(args.copy);
    }

    #[test]
    fn parser_edit_without_command_means_edit_last() {
        let args = parse_args(&["boltsnap".into(), "--edit".into()]).unwrap();
        assert!(args.edit);
        assert!(!args.command_explicit);
        assert_eq!(args.command, "area");
    }

    #[test]
    fn parser_handles_window_modes() {
        let args = parse_args(&["boltsnap".into(), "window".into(), "--edit".into()]).unwrap();
        assert_eq!(
            CaptureMode::parse(&args.command).unwrap(),
            CaptureMode::Window
        );
        assert!(args.edit);
        assert_eq!(
            CaptureMode::parse("active-window").unwrap(),
            CaptureMode::ActiveWindow
        );
        assert_eq!(CaptureMode::parse("select").unwrap(), CaptureMode::Area);
    }

    #[test]
    fn parser_handles_edit_output_no_copy() {
        let args = parse_args(&[
            "boltsnap".into(),
            "edit".into(),
            "a.png".into(),
            "--no-copy".into(),
            "-o".into(),
            "b.png".into(),
        ])
        .unwrap();
        assert_eq!(args.command, "edit");
        assert_eq!(args.image.unwrap(), PathBuf::from("a.png"));
        assert!(!args.copy);
        assert_eq!(args.output.unwrap(), PathBuf::from("b.png"));
    }

    #[test]
    fn parser_handles_instant_flag() {
        let a = parse_args(&[
            "boltsnap".to_string(),
            "area".to_string(),
            "--instant".to_string(),
        ])
        .unwrap();
        assert!(a.instant);
        let d = parse_args(&["boltsnap".to_string(), "area".to_string()]).unwrap();
        assert!(!d.instant);
    }
}
