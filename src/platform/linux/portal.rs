//! xdg-desktop-portal helpers for compositors without wlr-screencopy (KWin).
//!
//! libwayshot needs `zwlr_screencopy_manager_v1`, which KWin does not expose.
//! The portal Screenshot call is answered by xdg-desktop-portal-kde without a
//! dialog (non-interactive) and hands back a PNG of the whole desktop.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use image::RgbaImage;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const TIMEOUT: Duration = Duration::from_secs(15);

/// Capture the whole desktop through `org.freedesktop.portal.Screenshot`.
/// The file the portal writes is decoded and removed again.
pub fn screenshot() -> Result<RgbaImage, String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(screenshot_blocking());
    });
    rx.recv_timeout(TIMEOUT)
        .unwrap_or_else(|_| Err("portal screenshot timed out".to_string()))
}

fn screenshot_blocking() -> Result<RgbaImage, String> {
    let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;
    let sender = conn
        .unique_name()
        .ok_or("no unique bus name")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!("boltsnap{}", std::process::id());
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");

    // Subscribe before calling so a fast Response cannot be missed.
    let request = Proxy::new(
        &conn,
        PORTAL,
        request_path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(|e| format!("request proxy: {e}"))?;
    let mut responses = request
        .receive_signal("Response")
        .map_err(|e| format!("subscribe Response: {e}"))?;

    let portal = Proxy::new(
        &conn,
        PORTAL,
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(|e| format!("portal proxy: {e}"))?;
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("interactive", Value::from(false));
    let _request_path: OwnedObjectPath = portal
        .call("Screenshot", &("", options))
        .map_err(|e| format!("Screenshot call: {e}"))?;

    let msg = responses
        .next()
        .ok_or("portal closed the request without a response")?;
    let (code, mut results): (u32, HashMap<String, OwnedValue>) = msg
        .body()
        .deserialize()
        .map_err(|e| format!("Response decode: {e}"))?;
    if code != 0 {
        return Err(format!(
            "portal screenshot denied or cancelled (code {code})"
        ));
    }
    let uri = results
        .remove("uri")
        .and_then(|v| String::try_from(v).ok())
        .ok_or("portal response carries no uri")?;
    let path = uri
        .strip_prefix("file://")
        .map(percent_decode)
        .ok_or_else(|| format!("unexpected portal uri {uri}"))?;
    let path = PathBuf::from(path);
    let img = image::open(&path)
        .map_err(|e| format!("decode {}: {e}", path.display()))?
        .to_rgba8();
    let _ = std::fs::remove_file(&path);
    Ok(img)
}

/// Name of the output KWin considers active. Used to pick the capture and
/// overlay output on Plasma, where hyprctl is not available.
pub fn kwin_active_output_name() -> Option<String> {
    let conn = Connection::session().ok()?;
    let kwin = Proxy::new(&conn, "org.kde.KWin", "/KWin", "org.kde.KWin").ok()?;
    kwin.call("activeOutputName", &()).ok()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("/a%20b/c%C3%A4.png"), "/a b/cä.png");
        assert_eq!(percent_decode("/plain.png"), "/plain.png");
        assert_eq!(percent_decode("/bad%zz%4"), "/bad%zz%4");
    }
}
