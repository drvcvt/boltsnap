use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

mod capture;
mod clipboard;
mod config;
mod editor;
mod ipc;
mod paths;
mod record;
mod select_skia;
mod shelf;

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
}

impl Backend {
    fn parse(value: &str) -> DynResult<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "x11" => Ok(Self::X11),
            "wayland" => Ok(Self::Wayland),
            other => Err(format!("unknown backend '{other}', use auto/x11/wayland").into()),
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
    /// Wayland-only (the X11 path has no interactive region selector).
    instant: bool,
    /// Override the shelf save directory (daemon).
    save_dir: Option<PathBuf>,
    /// Override the annotation editor command (edit).
    editor_cmd: Option<String>,
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
        Backend::Wayland => PostCapture::Shelf {
            copy: args.copy_explicit && args.copy,
        },
        _ => PostCapture::CopyOnly,
    }
}

fn usage() -> &'static str {
    "\
Usage:
  boltsnap [area|full|window|active-window] [-o PATH|-] [--save] [--no-copy] [--instant] [--backend auto|x11|wayland]
  boltsnap area --instant                 select a region, capture on release (no edit handles)
  boltsnap --edit                         open last screenshot in editor
  boltsnap [area|window|full] --edit      capture then edit
  boltsnap edit [IMAGE] [-o PATH] [--no-copy]
  boltsnap daemon [--save-dir DIR]        run the screenshot shelf
  boltsnap record [--editor CMD]          select an area and screen-record it (Wayland)
  boltsnap record full                    record the whole focused monitor (no selector)
  boltsnap [COMMAND] [--editor CMD]       annotate with a specific editor
  Config: ~/.config/boltsnap/config.toml  (save_dir, editor, record_codec, record_dir)
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
        "stop" => {
            // Stop an in-progress recording (no-op if nothing is recording). Bind
            // to a key for a keyboard stop alongside the indicator's Stop button.
            if crate::ipc::daemon_alive() {
                crate::ipc::send_to_shelf(crate::ipc::Request::StopRecording)?;
            }
            Ok(())
        }
        "daemon" => crate::shelf::run_daemon(args.save_dir.clone()),
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
    )?;
    remember_last_screenshot(&result)?;
    println!("Edited last screenshot: {}", result.display());
    Ok(())
}

fn record_flow(args: &Args) -> DynResult<()> {
    if !crate::paths::has_cmd("wf-recorder") {
        return Err(
            "wf-recorder not found — install it to record (e.g. pacman -S wf-recorder)".into(),
        );
    }
    // Optional target: `boltsnap record full` records the whole focused monitor
    // (instant, no selector); absent or `area` opens the region selector.
    let target = args
        .image
        .as_deref()
        .and_then(|p| p.to_str())
        .unwrap_or("area");
    if matches!(target, "full" | "screen" | "fullscreen") {
        let Some(name) = crate::shelf::focused_monitor_name() else {
            return Err(
                "could not determine the focused monitor (needs Hyprland) for fullscreen record"
                    .into(),
            );
        };
        crate::ipc::send_to_shelf(crate::ipc::Request::StartRecordingOutput { name })?;
        return Ok(());
    }
    let mut prefs = crate::config::Config::load().recording_prefs();
    let selection = crate::select_skia::run_select_record(prefs.show_frame)?;
    if selection.show_frame != prefs.show_frame {
        prefs.show_frame = selection.show_frame;
        crate::config::save_recording_prefs(&prefs)?;
    }
    let Some(rect) = selection.rect else {
        return Ok(()); // cancelled
    };
    let (ox, oy) = crate::shelf::focused_monitor_origin().unwrap_or((0, 0));
    let geo = crate::record::to_global_geometry(rect.x, rect.y, rect.w, rect.h, ox, oy);
    crate::ipc::send_to_shelf(crate::ipc::Request::StartRecording {
        x: geo.x,
        y: geo.y,
        w: geo.w,
        h: geo.h,
        show_frame: prefs.show_frame,
    })?;
    Ok(())
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
    let resolved = capture(mode, &output, args.backend, args.instant)?;
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
    let session = env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_lowercase();
    if session == "wayland" || (session.is_empty() && env::var_os("WAYLAND_DISPLAY").is_some()) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
