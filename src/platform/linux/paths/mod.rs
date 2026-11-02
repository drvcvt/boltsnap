use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Args, DynResult};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

/// True if `name` is an executable on `PATH`.
pub fn has_cmd(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| dir.join(name).is_file())
}

/// This binary, to start another copy of. An update that replaced the file
/// (`cargo install`, a package upgrade) leaves a long-lived process such as
/// the daemon running from a deleted file, which Linux names `… (deleted)`;
/// the file now at the original path is the new build, and the one to start.
pub fn own_exe() -> io::Result<PathBuf> {
    let exe = env::current_exe()?;
    Ok(replaced_by(&exe)
        .filter(|path| path.is_file())
        .unwrap_or(exe))
}

fn replaced_by(exe: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let original = exe.as_os_str().as_bytes().strip_suffix(b" (deleted)")?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(original)))
}

/// Spawn a fire-and-forget child without leaving a zombie in long-lived callers.
pub fn spawn_reaped(command: &mut Command) -> io::Result<u32> {
    let mut child = command.spawn()?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

pub fn print_doctor() {
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
    println!("Boltsnap doctor");
    println!("================");
    println!("Session: {session}");
    println!();
    println!("Optional compositor IPC (only used for active-window on Wayland):");
    let cmd = "hyprctl";
    println!(
        "  {cmd:<10} {}",
        if has_cmd(cmd) { "ok" } else { "missing" }
    );
    println!();
    println!("Capabilities (no external screenshot/clipboard helpers required):");
    println!("  X11 capture:       in-process via x11rb (root pixmap GetImage)");
    println!("  X11 area:          unavailable");
    println!("  X11 window:        in-process x11rb picker");
    println!("  X11 active win:    in-process via x11rb (_NET_ACTIVE_WINDOW)");
    println!(
        "  Wayland capture:   in-process via libway (EXT image-copy-capture / WLR screencopy), xdg-desktop-portal fallback (KWin)"
    );
    println!("  Wayland area/win:  in-process selection overlay (tiny-skia)");
    println!("  Wayland active win: hyprctl on Hyprland");
    println!("  Clipboard:         in-process via arboard (X11) and wl-clipboard-rs (Wayland)");
    println!();
    println!("Screenshot shelf (Wayland / wlroots):");
    let on_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    println!(
        "  wayland session:   {}",
        if on_wayland { "yes" } else { "no" }
    );
    println!(
        "  shelf daemon:      {}",
        if crate::ipc::daemon_alive() {
            "running"
        } else {
            "not running (auto-starts on first wayland capture)"
        }
    );
    println!(
        "  shelf socket:      {}",
        crate::ipc::socket_path().display()
    );
}

pub fn self_test() -> DynResult<()> {
    print_doctor();
    println!("\nInternal parser/render tests are covered by `cargo test`.");
    Ok(())
}

pub fn target_path(args: &Args) -> PathBuf {
    if let Some(path) = &args.output {
        normalize_path(path)
    } else if args.save {
        default_save_path()
    } else {
        unique_path_in(&cache_dir(), "capture", "png")
    }
}

pub fn cache_dir() -> PathBuf {
    if let Some(cache) = xdg_base(env::var_os("XDG_CACHE_HOME")) {
        cache.join("boltsnap")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".cache").join("boltsnap")
    } else {
        env::temp_dir().join("boltsnap")
    }
}

pub fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    config_home().join("boltsnap")
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
fn config_home() -> PathBuf {
    xdg_base(env::var_os("XDG_CONFIG_HOME")).unwrap_or_else(|| home_dir().join(".config"))
}

/// An `XDG_*_HOME` value, which the spec says to ignore when it is empty or
/// relative.
fn xdg_base(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

pub fn default_screenshot_dir() -> PathBuf {
    pictures_dir().join("boltsnap")
}

/// The user's pictures folder as xdg-user-dirs names it in
/// `user-dirs.dirs` (`~/Bilder` on a German system), else `~/Pictures`.
pub fn pictures_dir() -> PathBuf {
    let home = home_dir();
    fs::read_to_string(config_home().join("user-dirs.dirs"))
        .ok()
        .and_then(|text| pictures_dir_in(&text, &home))
        .unwrap_or_else(|| home.join("Pictures"))
}

/// `XDG_PICTURES_DIR` from a `user-dirs.dirs` text: `"$HOME/…"` or an
/// absolute path, the only two forms the file allows.
fn pictures_dir_in(user_dirs: &str, home: &Path) -> Option<PathBuf> {
    let value = user_dirs
        .lines()
        .filter_map(|line| line.trim().strip_prefix("XDG_PICTURES_DIR="))
        .last()?;
    let value = value.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        Some(_) => return None,
        None => PathBuf::from(value),
    };
    path.is_absolute().then_some(path)
}

pub fn normalize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub fn default_save_path() -> PathBuf {
    pictures_dir()
        .join("Screenshots")
        .join(format!("boltsnap-{}.png", timestamp()))
}

pub fn temp_png(prefix: &str) -> PathBuf {
    temp_file(prefix, "png")
}

/// A unique temp path `boltsnap-<prefix>-<pid>-<ts>-<seq>.<ext>` in the system temp
/// dir. Generalizes `temp_png` for recordings (`mp4`) and their thumbnails.
pub fn temp_file(prefix: &str, ext: &str) -> PathBuf {
    unique_path_in(&env::temp_dir(), prefix, ext)
}

pub(crate) fn unique_path_in(dir: &Path, prefix: &str, ext: &str) -> PathBuf {
    dir.join(format!(
        "boltsnap-{prefix}-{}-{}-{}.{ext}",
        std::process::id(),
        timestamp(),
        NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

pub fn publish_last_png(png: &[u8]) -> io::Result<PathBuf> {
    publish_last_png_at(&cache_dir(), png)
}

/// `last.png` as one more name of `file`, which holds `png`: nothing is
/// written where both lie on one file system. An editor saving over the file
/// in place changes `last.png` with it.
pub fn publish_last_linked(file: &Path, png: &[u8]) -> io::Result<PathBuf> {
    publish_last_at(&cache_dir(), |pending| link_or_write(file, pending, png))
}

/// `to` as a link to `file`, or `png` written there when no link can be made
/// (another file system, or `file` gone).
fn link_or_write(file: &Path, to: &Path, png: &[u8]) -> io::Result<()> {
    fs::hard_link(file, to).or_else(|_| fs::write(to, png))
}

fn publish_last_png_at(dir: &Path, png: &[u8]) -> io::Result<PathBuf> {
    publish_last_at(dir, |pending| fs::write(pending, png))
}

/// Puts the new `last.png` at a pending name, then renames it over the old.
fn publish_last_at(dir: &Path, put: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let pending = unique_path_in(dir, "last", "png");
    let last = dir.join("last.png");
    put(&pending)?;
    if let Err(error) = fs::rename(&pending, &last) {
        let _ = fs::remove_file(pending);
        return Err(error);
    }
    Ok(last)
}

fn is_orphan_shelf_temp(name: &str) -> bool {
    (name.starts_with("boltsnap-shelf-") && name.ends_with(".png"))
        || name.starts_with("boltsnap-shelf-video-")
}

/// Delete orphaned shelf media in the temp dir. Called at daemon startup: with no
/// daemon running the shelf is empty, so every such file is from a previous run or
/// crash and safe to remove. Returns the number of files removed.
pub fn clean_orphan_shelf_temps() -> usize {
    let dir = env::temp_dir();
    let mut removed = 0;
    let Ok(entries) = fs::read_dir(&dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_orphan_shelf_temp(&name) && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Disk-backed directory for screen recordings — `$XDG_CACHE_HOME/boltsnap/rec`
/// (else `~/.cache/boltsnap/rec`). Recordings are many MB, so they must NOT go in
/// the system temp dir, which on this setup is tmpfs (RAM-backed).
pub fn rec_dir() -> PathBuf {
    cache_dir().join("rec")
}

/// Delete leftover recording files in `rec_dir` except `keep`. Called at
/// daemon startup: the shelf is RAM-only, so any recording file present is an
/// orphan from a previous run/crash. Recordings are large, so cleaning them
/// matters more than shelf temps. `keep` holds unsaved recordings that are
/// recovered instead. Returns the number of files removed.
pub fn clean_orphan_rec_files(keep: &[PathBuf]) -> usize {
    let mut removed = 0;
    let Ok(entries) = fs::read_dir(rec_dir()) else {
        return 0;
    };
    for entry in entries.flatten() {
        if is_orphan_rec_file(&entry.file_name().to_string_lossy())
            && !keep.contains(&entry.path())
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Recordings and shelf copies, plus the replay worker's `clip-*.mp4` exports
/// (`.mkv` from older workers) and the `clip-*.mp4.<pid>.<seq>.partial` files
/// a failed remux leaves.
fn is_orphan_rec_file(name: &str) -> bool {
    let is_clip = |name: &str| {
        name.starts_with("clip-") && (name.ends_with(".mp4") || name.ends_with(".mkv"))
    };
    let number = |field: &str| !field.is_empty() && field.bytes().all(|b| b.is_ascii_digit());
    let remux_partial = name.strip_suffix(".partial").is_some_and(|rest| {
        let mut fields = rest.rsplitn(3, '.');
        matches!(
            (fields.next(), fields.next(), fields.next()),
            (Some(seq), Some(pid), Some(clip)) if number(seq) && number(pid) && is_clip(clip)
        )
    });
    name.starts_with("boltsnap-") || is_clip(name) || remux_partial
}

pub fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn sanitize_recording_output(output: &str) -> String {
    output
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Pick a non-existing permanent recording path without overwriting an older clip.
pub fn unique_recording_path(dir: &Path, output: Option<&str>) -> PathBuf {
    unique_saved_path(dir, output, "mp4")
}

/// Pick a free `boltsnap-<local time>[-<output>][-N].<ext>` in `dir` (no
/// extension when `ext` is empty). The whole stem must be free: `X.mp4`,
/// `X.mkv` and `X.webm` would share the sidecar `X.cursor.json`, so any of
/// them, the sidecar or the name itself existing as anything (a dangling
/// symlink included) takes it. Callers still publish without replacing and
/// pick again when they lose a race.
pub fn unique_saved_path(dir: &Path, output: Option<&str>, ext: &str) -> PathBuf {
    unique_saved_path_at(dir, output, &local_timestamp(), ext)
}

pub(crate) fn unique_saved_path_at(
    dir: &Path,
    output: Option<&str>,
    stamp: &str,
    ext: &str,
) -> PathBuf {
    let output = output
        .map(sanitize_recording_output)
        .filter(|name| !name.is_empty())
        .map(|name| format!("-{name}"))
        .unwrap_or_default();
    for suffix in 0_u32.. {
        let suffix = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let dot = if ext.is_empty() { "" } else { "." };
        let path = dir.join(format!("boltsnap-{stamp}{output}{suffix}{dot}{ext}"));
        let taken = |path: &Path| fs::symlink_metadata(path).is_ok();
        let stem_taken = ["cursor.json", "mp4", "mkv", "webm"]
            .into_iter()
            .any(|sibling| taken(&path.with_extension(sibling)));
        if !taken(&path) && !stem_taken {
            return path;
        }
    }
    unreachable!()
}

/// Local wall-clock stamp `YYYY-MM-DD_HH-MM-SS` via `date` (correct local time,
/// no date-crate dependency — matching how the codebase already shells out to
/// `hyprctl`). Falls back to epoch millis if `date` is unavailable.
/// Local time as `YYYY-MM-DD_HH-MM-SS`, without spawning `date`.
pub fn local_timestamp() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return timestamp().to_string();
    }
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests;
