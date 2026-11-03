use std::env;
use std::error::Error;
use std::io::Write;
use std::path::PathBuf;

/// `eprintln!` that never panics: a full disk under `daemon.log` must not
/// take down the thread that is saving a recording.
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log_to(&mut std::io::stderr(), format_args!($($arg)*))
    };
}

mod cli;
mod config;
mod platform;
mod record;
mod selector;
mod shelf;

pub(crate) use crate::platform::{capture, clipboard, ipc, paths, tray};
pub(crate) use boltsnap::{image_model, protocol};
pub(crate) use cli::{Args, Backend, CaptureMode};

use crate::cli::{capture_flow, parse_args, record_flow, recording_command, recording_control};
use crate::clipboard::{serve_wayland_clipboard, serve_wayland_uri_list};
use crate::paths::{print_doctor, self_test};

pub type DynResult<T> = Result<T, Box<dyn Error>>;

/// Writes one log line; a failed write is dropped, see `log!`.
fn log_to(out: &mut impl Write, line: std::fmt::Arguments) {
    let _ = writeln!(out, "{line}");
}

fn main() {
    #[cfg(target_os = "linux")]
    crate::platform::timing::mark("process_entry");
    if let Err(err) = run() {
        eprintln!("boltsnap: {err}");
        std::process::exit(2);
    }
}

fn run() -> DynResult<()> {
    let raw: Vec<String> = env::args().collect();
    // Its own flags; the screenshot flags do not apply.
    if raw.get(1).is_some_and(|command| command == "settings") {
        return crate::platform::app::cli(&raw[2..]);
    }
    let args = parse_args(&raw)?;

    match args.command.as_str() {
        "doctor" => {
            print_doctor();
            Ok(())
        }
        "self-test" => self_test(),
        "record" => record_flow(&args),
        "recording" => recording_command(&args),
        #[cfg(target_os = "linux")]
        "replay" => crate::platform::replay::cli(&args.tail),
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
            crate::shelf::debug_render(&out)
        }
        "__serve-clipboard" => {
            // Detached child kept alive to serve Wayland paste requests.
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard needs a PNG path")?;
            serve_wayland_clipboard(&path)
        }
        #[cfg(target_os = "linux")]
        "__serve-clipboard-once" => {
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard-once needs a PNG path")?;
            crate::clipboard::serve_wayland_clipboard_once(&path)
        }
        "__serve-clipboard-uri" => {
            // Detached child kept alive to serve Wayland file-reference paste.
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard-uri needs a path")?;
            serve_wayland_uri_list(&path)
        }
        command => {
            CaptureMode::parse(command)?;
            capture_flow(&args)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_line_to_a_full_disk_is_dropped_not_a_panic() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("no space left on device"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        log_to(&mut Full, format_args!("saved {}", 1));
    }
}
