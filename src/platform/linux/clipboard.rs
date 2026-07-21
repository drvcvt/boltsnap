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

/// Build a `text/uri-list` payload (one `file://` URI + CRLF) for `path`.
pub fn uri_list_for(path: &Path) -> String {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt;

    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut uri = String::from("file://");
    for byte in abs.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(*byte));
        } else {
            write!(uri, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    uri.push_str("\r\n");
    uri
}

/// Serve `path` as a `text/uri-list` selection (a file REFERENCE, not bytes) on
/// the Wayland clipboard. Foreground; call from a detached helper process.
pub fn serve_wayland_uri_list(path: &Path) -> DynResult<()> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};
    let data = uri_list_for(path);
    let mut opts = Options::new();
    opts.foreground(true);
    let prepared = opts
        .prepare_copy(
            Source::Bytes(data.into_bytes().into_boxed_slice()),
            MimeType::Specific("text/uri-list".into()),
        )
        .map_err(|e| format!("prepare_copy failed: {e}"))?;
    prepared
        .serve()
        .map_err(|e| format!("clipboard serve failed: {e}"))?;
    Ok(())
}

/// Put a file reference (text/uri-list) on the clipboard by spawning a detached
/// `__serve-clipboard-uri` self that holds the data source open, so the caller
/// returns immediately. Copies only the path â€” instant regardless of file size.
pub fn copy_uri_to_clipboard(path: &Path) -> DynResult<()> {
    let exe = env::current_exe()?;
    crate::paths::spawn_reaped(
        Command::new(exe)
            .arg("__serve-clipboard-uri")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )?;
    Ok(())
}

pub fn copy_to_clipboard(path: &Path, backend: Backend) -> DynResult<Backend> {
    let backend = backend.resolved()?;
    match backend {
        Backend::Wayland => {
            // Spawn a detached self holding the clipboard data source open
            // so the foreground shell returns immediately.
            let exe = env::current_exe()?;
            crate::paths::spawn_reaped(
                Command::new(exe)
                    .arg("__serve-clipboard")
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
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
        Backend::Windows => return Err("Windows clipboard is unavailable on Linux".into()),
        Backend::Auto => unreachable!(),
    }
    Ok(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_has_file_scheme_and_crlf() {
        let s = uri_list_for(Path::new("/nonexistent/x.mp4"));
        assert!(s.starts_with("file://"), "got {s:?}");
        assert!(s.ends_with("\r\n"), "got {s:?}");
        assert!(s.contains("x.mp4"), "got {s:?}");
    }

    #[test]
    fn uri_list_percent_encodes_path_bytes() {
        assert_eq!(
            uri_list_for(Path::new("/nonexistent/a b#%Ã¤.mp4")),
            "file:///nonexistent/a%20b%23%25%C3%A4.mp4\r\n"
        );
    }
}
