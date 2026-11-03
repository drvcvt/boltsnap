//! The command line: what was asked for (`Args`, `Backend`, `CaptureMode`),
//! the help text, and the two flows behind it, screenshots and recordings.

use std::env;
use std::path::PathBuf;

use crate::DynResult;

mod capture;
mod recording;
#[cfg(test)]
mod tests;

pub(crate) use capture::capture_flow;
pub(crate) use recording::{record_flow, recording_command, recording_control};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Backend {
    Auto,
    X11,
    Wayland,
    Windows,
}

impl Backend {
    pub(crate) fn parse(value: &str) -> DynResult<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "x11" => Ok(Self::X11),
            "wayland" => Ok(Self::Wayland),
            "windows" => Ok(Self::Windows),
            other => Err(format!("unknown backend '{other}', use auto/x11/wayland/windows").into()),
        }
    }

    pub(crate) fn resolved(self) -> DynResult<Self> {
        match self {
            Self::Auto => detect_backend(),
            other => Ok(other),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Windows => "windows",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureMode {
    Area,
    Full,
    Window,
    ActiveWindow,
}

impl CaptureMode {
    pub(crate) fn parse(command: &str) -> DynResult<Self> {
        match command {
            "area" | "select" | "selection" | "region" => Ok(Self::Area),
            "full" | "screen" | "fullscreen" => Ok(Self::Full),
            "window" | "win" | "select-window" => Ok(Self::Window),
            "active" | "active-window" | "current" | "current-window" => Ok(Self::ActiveWindow),
            other => Err(format!("unknown capture command '{other}'").into()),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Full => "full",
            Self::Window => "window",
            Self::ActiveWindow => "active-window",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Args {
    pub(crate) command: String,
    pub(crate) image: Option<PathBuf>,
    pub(crate) copy: bool,
    /// True if the user passed --copy or --no-copy explicitly. On Wayland the
    /// shelf is the default sink, so we only auto-copy when copy was asked for.
    pub(crate) copy_explicit: bool,
    pub(crate) save: bool,
    pub(crate) output: Option<PathBuf>,
    pub(crate) backend: Backend,
    /// Skip the selector's editable phase: release captures immediately.
    /// Supported by interactive Wayland and Windows region selectors.
    pub(crate) instant: bool,
    /// Override the shelf save directory (daemon).
    pub(crate) save_dir: Option<PathBuf>,
    pub(crate) tail: Vec<String>,
    pub(crate) json: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "area".to_string(),
            image: None,
            copy: true,
            copy_explicit: false,
            save: false,
            output: None,
            backend: Backend::Auto,
            instant: false,
            save_dir: None,
            tail: Vec::new(),
            json: false,
        }
    }
}

pub(crate) fn usage() -> &'static str {
    "\
Usage:
  boltsnap [area|full|window|active-window] [-o PATH|-] [--save] [--copy|--no-copy] [--instant] [--backend auto|x11|wayland|windows]
  boltsnap area --instant                 select a region, capture on release (no edit handles)
  boltsnap daemon [--save-dir DIR]        run the screenshot shelf
  boltsnap record                         select an area and screen-record it
  boltsnap record full                    record the configured fullscreen target (no selector)
  boltsnap replay start|stop|status|save    experimental Linux replay buffer
  boltsnap recording status --json
  boltsnap recording watch --json
  boltsnap recording show-controls
  boltsnap recording pause|resume|save-shelf|save-disk|discard
  boltsnap stop                           compatibility alias for recording save-shelf
  boltsnap settings [PAGE]                open the settings window (Linux)
  boltsnap settings --render PAGE OUT.png draw a page of the Boltsnap window to a PNG
  Config: platform config directory       (save_dir, record_codec, record_dir)
  boltsnap doctor

Examples:
  boltsnap                                area, copy PNG, remember as last
  boltsnap window                         pick window, copy PNG
  boltsnap full --no-copy -o /tmp/x.png   write file, no clipboard
  boltsnap area --no-copy -o - | eddy -f -      pipe to external editor
"
}

pub(crate) fn parse_args(raw: &[String]) -> DynResult<Args> {
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
            value if value.starts_with('-') => {
                return Err(format!("unknown option '{value}'\n{}", usage()).into());
            }
            value => positional.push(value.to_string()),
        }
        i += 1;
    }

    if let Some(command) = positional.first() {
        args.command = command.to_lowercase();
    }
    if positional.len() > 1 {
        args.image = Some(PathBuf::from(&positional[1]));
    }
    args.tail = positional.into_iter().skip(1).collect();
    Ok(args)
}

fn require_daemon_success(
    response: crate::ipc::Response,
    fallback: &str,
) -> Result<crate::ipc::Response, String> {
    if response.ok {
        Ok(response)
    } else {
        Err(response.error.unwrap_or_else(|| fallback.into()))
    }
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
