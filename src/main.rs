use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

mod capture;
mod clipboard;
mod editor;
mod ipc;
mod paths;
mod select;
mod shelf;

use crate::capture::{capture, strip_uniform_border};
use crate::clipboard::{copy_to_clipboard, serve_wayland_clipboard};
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
    save: bool,
    output: Option<PathBuf>,
    backend: Backend,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "area".to_string(),
            command_explicit: false,
            image: None,
            edit: false,
            copy: true,
            save: false,
            output: None,
            backend: Backend::Auto,
        }
    }
}

fn usage() -> &'static str {
    "\
Usage:
  boltsnap [area|full|window|active-window] [-o PATH|-] [--save] [--no-copy] [--backend auto|x11|wayland]
  boltsnap --edit                         open last screenshot in editor
  boltsnap [area|window|full] --edit      capture then edit
  boltsnap edit [IMAGE] [-o PATH] [--no-copy]
  boltsnap doctor

Examples:
  boltsnap                                area, copy PNG, remember as last
  boltsnap --edit                         open last screenshot in editor
  boltsnap window                         pick window, copy PNG
  boltsnap full --no-copy -o /tmp/x.png   write file, no clipboard
  boltsnap area --no-copy -o - | swappy -f -    pipe to external editor
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
            "--copy" => args.copy = true,
            "--no-copy" => args.copy = false,
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
        "__serve-clipboard" => {
            // Detached child kept alive to serve Wayland paste requests.
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard needs a PNG path")?;
            return serve_wayland_clipboard(&path);
        }
        "edit" => {
            let image = args.image.clone().unwrap_or(last_screenshot_path()?);
            ensure_file(&image)?;
            let output = edit_output_path(&args);
            let result = run_editor(normalize_path(&image), output, args.copy, args.backend)?;
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
    )?;
    remember_last_screenshot(&result)?;
    println!("Edited last screenshot: {}", result.display());
    Ok(())
}

fn capture_flow(args: &Args) -> DynResult<()> {
    let mode = CaptureMode::parse(&args.command)?;

    if is_stdout_target(args) {
        return capture_to_stdout(mode, args.backend);
    }

    let raw_is_temp = args.edit;
    let raw_path = if args.edit {
        temp_png("raw")
    } else {
        target_path(args)
    };
    let backend = capture(mode, &raw_path, args.backend)?;
    let _ = strip_uniform_border(&raw_path);
    let mut final_path = raw_path.clone();

    if args.edit {
        let output = edit_output_path(args);
        final_path = run_editor(raw_path.clone(), output, args.copy, args.backend)?;
        remember_last_screenshot(&final_path)?;
        if raw_is_temp && final_path != raw_path {
            let _ = fs::remove_file(&raw_path);
        }
    } else if args.copy {
        copy_to_clipboard(&raw_path, args.backend)?;
        remember_last_screenshot(&raw_path)?;
    } else {
        remember_last_screenshot(&raw_path)?;
    }

    let verb = if args.copy { "copied" } else { "captured" };
    println!(
        "Boltsnap {verb} {} via {}: {}",
        mode.label(),
        backend.as_str(),
        final_path.display()
    );
    Ok(())
}

fn is_stdout_target(args: &Args) -> bool {
    args.output.as_deref().and_then(|p| p.to_str()) == Some("-")
}

fn capture_to_stdout(mode: CaptureMode, backend: Backend) -> DynResult<()> {
    let tmp = temp_png("stdout");
    capture(mode, &tmp, backend)?;
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
    use image::ImageBuffer;

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

}
