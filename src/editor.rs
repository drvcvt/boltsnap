use std::path::PathBuf;
use std::process::Command;

use crate::clipboard::copy_to_clipboard;
use crate::config::{Config, resolve_editor};
use crate::paths::temp_png;
use crate::{Backend, DynResult};

/// Open `image_path` in the annotation editor (eddy) and return the path the
/// edited image was written to.
///
/// eddy is a drop-in swappy replacement: it reads `-f` and, on save, writes the
/// `-o` path. The editor command is resolved via `editor_override` →
/// `$BOLTSNAP_EDITOR` → config → `eddy` on PATH → `~/projects/eddy/build/eddy`.
///
/// We always pass `--no-copy` so boltsnap owns the clipboard (its Wayland copy
/// persists via the `__serve-clipboard` helper, which a short-lived editor's own
/// clipboard would not). boltsnap then copies the saved file itself when asked.
pub fn run_editor(
    image_path: PathBuf,
    output_path: Option<PathBuf>,
    copy_after: bool,
    backend: Backend,
    editor_override: Option<String>,
) -> DynResult<PathBuf> {
    let output = output_path.unwrap_or_else(|| temp_png("edited"));

    let editor = resolve_editor(editor_override.as_deref(), &Config::load()).ok_or(
        "no editor found — install eddy, set `editor` in ~/.config/boltsnap/config.toml, \
         or set $BOLTSNAP_EDITOR",
    )?;

    let status = Command::new(&editor)
        .arg("-f")
        .arg(&image_path)
        .arg("-o")
        .arg(&output)
        .arg("--no-copy")
        .status()
        .map_err(|e| format!("failed to launch editor '{editor}': {e}"))?;
    if !status.success() {
        return Err(format!("editor '{editor}' exited with status {status}").into());
    }

    // The editor only writes `-o` when the user saves; copy only if it did.
    if copy_after && output.exists() {
        copy_to_clipboard(&output, backend)?;
    }
    Ok(output)
}
