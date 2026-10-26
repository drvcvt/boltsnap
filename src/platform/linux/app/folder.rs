//! The folder dialog: `org.freedesktop.portal.FileChooser.OpenFile` with
//! `directory`, drawn by the desktop's portal (settings plan, 5c S11). The
//! request and its answer go as in `capture/portal.rs`.

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.portal.FileChooser";

static AVAILABLE: OnceLock<bool> = OnceLock::new();

/// A folder dialog's answer for the row (page, block) that asked.
pub(super) type Picked = (super::save::Tag, Result<Option<PathBuf>, String>);

/// The desktop answers folder dialogs (`directory` needs version 3).
pub(super) fn available() -> bool {
    AVAILABLE.get().copied().unwrap_or(false)
}

/// Ask the bus once, off the window's loop; `changed` gets a signal when
/// the dialog turns out to be there.
pub(super) fn probe(changed: calloop::channel::Sender<()>) {
    std::thread::spawn(move || {
        let version = Connection::session().ok().and_then(|conn| {
            Proxy::new(&conn, PORTAL, DESKTOP, CHOOSER)
                .ok()?
                .get_property::<u32>("version")
                .ok()
        });
        let there = version.is_some_and(|version| version >= 3);
        let _ = AVAILABLE.set(there);
        if there {
            let _ = changed.send(());
        }
    });
}

/// A folder chosen in the dialog, which opens at `start`; `None` when the
/// dialog was closed without one. Blocks until the person decides.
pub(super) fn pick(start: &Path) -> Result<Option<PathBuf>, String> {
    let conn = Connection::session().map_err(|e| format!("session bus: {e}"))?;
    let sender = conn
        .unique_name()
        .ok_or("no unique bus name")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = format!("boltsnapfolder{}", std::process::id());
    let request_path = format!("{DESKTOP}/request/{sender}/{token}");
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
    let chooser =
        Proxy::new(&conn, PORTAL, DESKTOP, CHOOSER).map_err(|e| format!("portal proxy: {e}"))?;
    let mut start_bytes = start.as_os_str().as_bytes().to_vec();
    start_bytes.push(0);
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("directory", Value::from(true));
    options.insert("modal", Value::from(true));
    options.insert("current_folder", Value::from(start_bytes));
    let _request: OwnedObjectPath = chooser
        .call("OpenFile", &("", "Choose a folder", options))
        .map_err(|e| format!("folder dialog: {e}"))?;
    let message = responses
        .next()
        .ok_or("the folder dialog closed without an answer")?;
    let (code, mut results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|e| format!("folder dialog answer: {e}"))?;
    match code {
        0 => {}
        // Cancelled by the person.
        1 => return Ok(None),
        code => return Err(format!("the folder dialog failed (code {code})")),
    }
    let uris = results
        .remove("uris")
        .and_then(|value| Vec::<String>::try_from(value).ok())
        .unwrap_or_default();
    let Some(uri) = uris.first() else {
        return Ok(None);
    };
    let path = uri
        .strip_prefix("file://")
        .map(crate::platform::portal::percent_decode)
        .ok_or_else(|| format!("the folder dialog gave {uri}, not a folder"))?;
    Ok(Some(PathBuf::from(path)))
}
