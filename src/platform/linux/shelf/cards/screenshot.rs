//! Full-desktop screenshots the daemon takes itself, over a Wayland
//! connection it keeps open: a shot then skips the client's start, its
//! connection with globals and outputs, and handing the PNG over
//! (capture-speed plan, D1).

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::protocol::CAPTURE_UNAVAILABLE;

use super::PreparedAdd;
use super::cards::prepare_pixels;
use super::placement::focused_monitor_name;

/// The open connection: made by the first shot, dropped by a failed one.
static CONNECTION: Mutex<Option<libway::Connection>> = Mutex::new(None);

/// What a `capture` request came to.
pub(super) enum Taken {
    /// The PNG is at the requested path.
    File,
    /// A card for the shelf, with the PNG, `last.png` and any clipboard copy
    /// already written.
    Card(PreparedAdd),
}

/// A full-desktop screenshot into `output`, or onto the shelf. An error
/// before anything was captured starts with [`CAPTURE_UNAVAILABLE`].
pub(super) fn take(output: Option<PathBuf>, copy: bool) -> Result<Taken, String> {
    let capture = super::super::timing::Span::new("daemon_capture");
    let image = desktop().map_err(|error| format!("{CAPTURE_UNAVAILABLE}: {error}"))?;
    drop(capture);
    let _timing = super::super::timing::Span::new("daemon_capture_output");
    let Some(path) = output else {
        let metadata = super::super::image_transfer::Metadata::new(
            image.width(),
            image.height(),
            "full".into(),
            focused_monitor_name(),
            copy,
        )
        .map_err(|error| error.to_string())?;
        return prepare_pixels(metadata, image.as_raw()).map(Taken::Card);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let png = crate::capture::encode_png(&image)?;
    std::fs::write(&path, png).map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(Taken::File)
}

/// The desktop over the open connection; a connection gone stale since the
/// last shot is replaced once.
fn desktop() -> Result<image::RgbImage, String> {
    let mut connection = CONNECTION.lock().unwrap_or_else(PoisonError::into_inner);
    let start = std::time::Instant::now();
    let mut retried = false;
    loop {
        let options = crate::capture::remaining(start);
        if connection.is_none() {
            *connection = Some(libway::Connection::connect(&options).map_err(|e| e.to_string())?);
        }
        let open = connection.as_mut().expect("connected above");
        match open.capture_desktop(&options) {
            Ok(desktop) => return Ok(crate::capture::rgba_to_rgb(desktop.image)),
            Err(error) => {
                *connection = None;
                if retried {
                    return Err(error.to_string());
                }
                retried = true;
            }
        }
    }
}
