use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Backend, DynResult};

pub fn serve_wayland_clipboard(path: &Path) -> DynResult<()> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};
    let data = fs::read(path)?;
    let mut opts = Options::new();
    opts.foreground(true);
    let prepared = opts
        .prepare_copy(
            Source::Bytes(data.into_boxed_slice()),
            MimeType::Specific("image/png".into()),
        )
        .map_err(|e| format!("prepare_copy failed: {e}"))?;
    prepared
        .serve()
        .map_err(|e| format!("clipboard serve failed: {e}"))?;
    Ok(())
}

pub fn copy_to_clipboard(path: &Path, backend: Backend) -> DynResult<Backend> {
    let backend = backend.resolved()?;
    match backend {
        Backend::Wayland => {
            // Spawn a detached self holding the clipboard data source open
            // so the foreground shell returns immediately.
            let exe = env::current_exe()?;
            Command::new(exe)
                .arg("__serve-clipboard")
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        Backend::X11 => {
            // arboard's set_image owns the X11 selection; on X11 it forks a
            // helper that lives until another client takes the selection,
            // so we don't need an external xclip helper.
            let img = image::open(path)?.to_rgba8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let bytes = std::borrow::Cow::Owned(img.into_raw());
            let mut clipboard =
                arboard::Clipboard::new().map_err(|e| format!("X11 clipboard open failed: {e}"))?;
            clipboard
                .set_image(arboard::ImageData {
                    width: w,
                    height: h,
                    bytes,
                })
                .map_err(|e| format!("X11 clipboard set_image failed: {e}"))?;
        }
        Backend::Auto => unreachable!(),
    }
    Ok(backend)
}
