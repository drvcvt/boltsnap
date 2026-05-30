use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Args, DynResult};

/// True if `name` is an executable on `PATH`.
pub fn has_cmd(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| dir.join(name).is_file())
}

pub fn print_doctor() {
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
    println!("Boltsnap doctor");
    println!("================");
    println!("Session: {session}");
    println!();
    println!("Optional compositor IPC (only used for active-window on Wayland):");
    for cmd in ["hyprctl"] {
        println!(
            "  {cmd:<10} {}",
            if has_cmd(cmd) { "ok" } else { "missing" }
        );
    }
    println!();
    println!("Capabilities (no external screenshot/clipboard helpers required):");
    println!("  X11 capture:       in-process via x11rb (root pixmap GetImage)");
    println!("  X11 area/window:   in-process selection overlay (eframe)");
    println!("  X11 active win:    in-process via x11rb (_NET_ACTIVE_WINDOW)");
    println!("  Wayland capture:   in-process via libwayshot (wlr-screencopy)");
    println!("  Wayland area/win:  in-process selection overlay (eframe)");
    println!("  Wayland active win: hyprctl on Hyprland");
    println!("  Clipboard:         in-process via arboard (X11) and wl-clipboard-rs (Wayland)");
    println!();
    println!("Screenshot shelf (Wayland / wlroots):");
    let on_wayland = env::var_os("WAYLAND_DISPLAY").is_some();
    println!("  wayland session:   {}", if on_wayland { "yes" } else { "no" });
    println!(
        "  shelf daemon:      {}",
        if crate::ipc::daemon_alive() {
            "running"
        } else {
            "not running (auto-starts on first wayland capture)"
        }
    );
    println!("  shelf socket:      {}", crate::ipc::socket_path().display());
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
        cache_dir().join("last.png")
    }
}

pub fn edit_output_path(args: &Args) -> Option<PathBuf> {
    if let Some(path) = &args.output {
        Some(normalize_path(path))
    } else if args.save {
        Some(default_save_path())
    } else {
        Some(cache_dir().join("last-edited.png"))
    }
}

pub fn cache_dir() -> PathBuf {
    if let Some(cache) = env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(cache).join("boltsnap")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".cache").join("boltsnap")
    } else {
        env::temp_dir().join("boltsnap")
    }
}

pub fn last_pointer_path() -> PathBuf {
    cache_dir().join("last.txt")
}

pub fn remember_last_screenshot(path: &Path) -> DynResult<()> {
    let path = normalize_path(path);
    if !path.is_file() {
        return Ok(());
    }
    fs::create_dir_all(cache_dir())?;
    fs::write(last_pointer_path(), path.to_string_lossy().as_bytes())?;
    Ok(())
}

pub fn last_screenshot_path() -> DynResult<PathBuf> {
    let pointer = last_pointer_path();
    if pointer.is_file() {
        let value = fs::read_to_string(pointer)?;
        let path = PathBuf::from(value.trim());
        if path.is_file() {
            return Ok(path);
        }
    }
    let fallback = cache_dir().join("last.png");
    if fallback.is_file() {
        Ok(fallback)
    } else {
        Err("no last screenshot yet; run `boltsnap` first, then `boltsnap --edit`".into())
    }
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

pub fn ensure_file(path: &Path) -> DynResult<()> {
    if normalize_path(path).is_file() {
        Ok(())
    } else {
        Err(format!("file not found: {}", path.display()).into())
    }
}

pub fn default_save_path() -> PathBuf {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Pictures")
        .join("Screenshots")
        .join(format!("boltsnap-{}.png", timestamp()))
}

pub fn temp_png(prefix: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "boltsnap-{prefix}-{}-{}.png",
        std::process::id(),
        timestamp()
    ))
}

pub fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
