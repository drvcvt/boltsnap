use std::path::PathBuf;
use std::process::Command;

use crate::clipboard::copy_to_clipboard;
use crate::paths::{has_cmd, temp_png};
use crate::{Backend, DynResult};

/// Open `image_path` in the external annotation editor (swappy) and return the
/// path the edited image was written to.
///
/// The built-in egui editor was retired in favour of swappy so boltsnap can
/// focus on capture + the shelf; a native (skia-based) editor may replace swappy
/// later behind this same entry point. swappy reads `-f` and, on save (Ctrl+S /
/// the save button), writes to the `-o` path. If the user closes without saving,
/// the output file is left untouched — we only copy to the clipboard when an
/// edited file actually exists.
pub fn run_editor(
    image_path: PathBuf,
    output_path: Option<PathBuf>,
    copy_after: bool,
    backend: Backend,
) -> DynResult<PathBuf> {
    let output = output_path.unwrap_or_else(|| temp_png("edited"));

    if !has_cmd("swappy") {
        return Err(
            "swappy not found — install it (e.g. `pacman -S swappy`) to edit screenshots".into(),
        );
    }

    let status = Command::new("swappy")
        .arg("-f")
        .arg(&image_path)
        .arg("-o")
        .arg(&output)
        .status()
        .map_err(|e| format!("failed to launch swappy: {e}"))?;
    if !status.success() {
        return Err(format!("swappy exited with status {status}").into());
    }

    // swappy only writes `-o` when the user saves; copy only if it did.
    if copy_after && output.exists() {
        copy_to_clipboard(&output, backend)?;
    }
    Ok(output)
}
