//! Changes single keys in `config.toml` and leaves everything else as it was:
//! comments, order, unknown keys and invalid values nobody touched.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use toml_edit::{Document, Item, TableLike};

use super::load::{Value, check};
use super::schema;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Set(Value),
    /// Back to the default.
    Remove,
}

/// `text` with `changes` applied. Every change is checked against the schema
/// first; one bad change leaves the text unchanged and says why.
pub fn apply(text: &str, changes: &[(&str, Change)]) -> Result<String, String> {
    let mut document = text
        .parse::<Document>()
        .map_err(|error| format!("config.toml is not valid TOML, fix it first: {error}"))?;
    for (path, change) in changes {
        let key = schema::find(path).ok_or_else(|| format!("unknown setting {path}"))?;
        if let Change::Set(value) = change {
            check(key, value)?;
        }
        let (table, name) = schema::split(key.path);
        let container: &mut dyn TableLike = match table {
            None => document.as_table_mut(),
            Some(table) => {
                if !document.contains_key(table) {
                    document[table] = Item::Table(toml_edit::Table::new());
                }
                document[table]
                    .as_table_like_mut()
                    .ok_or_else(|| format!("{table} in config.toml is not a table"))?
            }
        };
        match change {
            Change::Remove => {
                container.remove(name);
            }
            Change::Set(value) => {
                let mut value = match value {
                    Value::Bool(value) => toml_edit::Value::from(*value),
                    Value::Int(value) => toml_edit::Value::from(*value),
                    Value::Text(value) => toml_edit::Value::from(value.as_str()),
                };
                // Assign in place: `insert` would also replace the key and
                // drop the comment above it.
                match container.get_mut(name) {
                    Some(slot) => {
                        if let Some(old) = slot.as_value() {
                            *value.decor_mut() = old.decor().clone();
                        }
                        *slot = Item::Value(value);
                    }
                    None => {
                        container.insert(name, Item::Value(value));
                    }
                }
            }
        }
    }
    Ok(document.to_string())
}

/// Applies `changes` to the file at `path` under a lock, so two writers never
/// lose each other's change. A symlinked file is written through the link.
/// Nothing is written when the text would stay the same.
pub fn write(path: &Path, changes: &[(&str, Change)]) -> io::Result<()> {
    update(path, |_| changes.to_vec())
}

/// Like `write`, with the changes worked out from the file's current text
/// while the lock is held.
pub fn update<'a>(
    path: &Path,
    changes: impl FnOnce(&str) -> Vec<(&'a str, Change)>,
) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // Next to the given path, not the link target, so no lock file appears in
    // a dotfiles checkout.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(sibling(path, ".lock"))?;
    lock.lock()?;
    let target = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path).map_err(|error| {
            io::Error::new(error.kind(), format!("config.toml points nowhere: {error}"))
        })?,
        _ => path.to_path_buf(),
    };
    let (text, permissions) = match fs::read_to_string(&target) {
        Ok(text) => (text, Some(fs::metadata(&target)?.permissions())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), None),
        Err(error) => return Err(error),
    };
    let changes = changes(&text);
    let updated = apply(&text, &changes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if updated == text {
        return Ok(());
    }
    replace(&target, updated.as_bytes(), permissions)
}

/// Writes a new file beside `target` and renames it over, so a reader sees
/// the old or the new text, never half of it.
fn replace(target: &Path, bytes: &[u8], permissions: Option<fs::Permissions>) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = target.with_file_name(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, target)?;
        // The rename itself is durable once the directory is synced. Not every
        // platform can open a directory; the file is in place either way.
        if let Some(directory) = target.parent()
            && let Ok(directory) = File::open(directory)
        {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{name}{suffix}"))
}
