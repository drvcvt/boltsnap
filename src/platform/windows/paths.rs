use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_RoamingAppData,
    FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::GUID;

use crate::{Args, DynResult};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

pub fn spawn_reaped(command: &mut Command) -> io::Result<u32> {
    let mut child = command.spawn()?;
    let process_id = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(process_id)
}

pub fn print_doctor() {
    println!("Boltsnap doctor");
    println!("================");
    println!("Platform: Windows");
    println!("Native IPC: {}", crate::ipc::socket_path().display());
    println!("Clipboard: native image, CF_HDROP and OLE file drag available");
    println!("Capture: DXGI monitor/region, WGC window, GDI compatibility fallback");
    println!("Selector/shelf/tray: native Windows windows with shared design");
    println!("Recording: WGC + Media Foundation H.264/AAC + WASAPI audio");
    println!("Encoder: Media Foundation selects an AMD/NVIDIA hardware transform");
    println!("Recording limits: one monitor at a time; live watch stream pending");
}

pub fn self_test() -> DynResult<()> {
    print_doctor();
    println!("\nShared protocol tests are available through `cargo test --lib`.");
    Ok(())
}

pub(crate) fn target_path(args: &Args) -> PathBuf {
    if let Some(path) = &args.output {
        normalize_path(path)
    } else if args.save {
        default_save_path()
    } else {
        cache_dir().join("last.png")
    }
}

pub fn home_dir() -> PathBuf {
    known_folder(&FOLDERID_Profile)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    known_folder(&FOLDERID_RoamingAppData)
        .unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
        .join("boltsnap")
}

pub fn cache_dir() -> PathBuf {
    known_folder(&FOLDERID_LocalAppData)
        .unwrap_or_else(|| env::temp_dir())
        .join("boltsnap")
        .join("cache")
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
    default_screenshot_dir().join(format!("boltsnap-{}.png", local_timestamp()))
}

pub fn default_screenshot_dir() -> PathBuf {
    known_folder(&FOLDERID_Pictures)
        .unwrap_or_else(|| home_dir().join("Pictures"))
        .join("Boltsnap")
}

pub fn temp_png(prefix: &str) -> PathBuf {
    temp_file(prefix, "png")
}

pub fn temp_file(prefix: &str, extension: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "boltsnap-{prefix}-{}-{}-{}.{}",
        std::process::id(),
        timestamp(),
        NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed),
        extension
    ))
}

pub fn clean_orphan_shelf_temps() -> usize {
    clean_files(env::temp_dir())
}

pub fn rec_dir() -> PathBuf {
    cache_dir().join("recordings")
}

pub fn rec_file(prefix: &str, extension: &str) -> PathBuf {
    let directory = rec_dir();
    let _ = fs::create_dir_all(&directory);
    directory.join(format!(
        "boltsnap-{prefix}-{}-{}-{}.{}",
        std::process::id(),
        timestamp(),
        NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed),
        extension
    ))
}

pub fn clean_orphan_rec_files() -> usize {
    clean_files(rec_dir())
}

pub fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn boltsnap_filename_ext(stamp: &str, extension: &str) -> String {
    format!("boltsnap-{stamp}.{extension}")
}

pub fn sanitize_recording_output(output: &str) -> String {
    output
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub fn unique_recording_path(directory: &Path, output: Option<&str>) -> PathBuf {
    let output = output
        .map(sanitize_recording_output)
        .filter(|name| !name.is_empty())
        .map(|name| format!("-{name}"))
        .unwrap_or_default();
    for suffix in 0_u32.. {
        let suffix = match suffix {
            0 => String::new(),
            value => format!("-{value}"),
        };
        let path = directory.join(format!(
            "boltsnap-{}{output}{suffix}.mp4",
            local_timestamp()
        ));
        if !path.exists() {
            return path;
        }
    }
    unreachable!()
}

pub fn local_timestamp() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

pub fn default_recording_dir() -> PathBuf {
    known_folder(&FOLDERID_Videos)
        .unwrap_or_else(|| home_dir().join("Videos"))
        .join("Boltsnap")
}

fn known_folder(identifier: &GUID) -> Option<PathBuf> {
    let value = unsafe { SHGetKnownFolderPath(identifier, KF_FLAG_DEFAULT, None).ok()? };
    let path = unsafe { value.to_string().ok().map(PathBuf::from) };
    unsafe { CoTaskMemFree(Some(value.as_ptr().cast())) };
    path
}

fn clean_files(directory: PathBuf) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("boltsnap-"))
        .filter(|entry| fs::remove_file(entry.path()).is_ok())
        .count()
}
