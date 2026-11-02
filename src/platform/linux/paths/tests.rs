#[test]
fn a_replaced_binary_is_started_from_its_original_path() {
    use std::path::Path;
    assert_eq!(
        super::replaced_by(Path::new("/home/u/.cargo/bin/boltsnap (deleted)")),
        Some("/home/u/.cargo/bin/boltsnap".into())
    );
    assert_eq!(super::replaced_by(Path::new("/usr/bin/boltsnap")), None);
}

#[test]
fn local_timestamp_matches_date_format() {
    let stamp = super::local_timestamp();
    let expected = std::process::Command::new("date")
        .arg("+%Y-%m-%d_%H-%M-%S")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    assert_eq!(stamp.len(), 19, "{stamp}");
    // Same second, or the next one if the clock ticked in between.
    assert_eq!(stamp[..16], expected[..16]);
}

use super::*;

#[test]
fn default_capture_work_paths_are_unique() {
    let args = Args::default();
    let first = target_path(&args);
    let second = target_path(&args);

    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(cache_dir().as_path()));
    assert!(
        first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("boltsnap-capture-")
    );
}

#[test]
fn last_png_is_a_link_to_the_card_file_or_a_copy_when_it_cannot_be() {
    use std::os::unix::fs::MetadataExt;
    let dir = env::temp_dir().join(format!(
        "boltsnap-last-link-test-{}-{}",
        std::process::id(),
        timestamp()
    ));
    fs::create_dir(&dir).unwrap();
    let card = dir.join("card.png");
    fs::write(&card, b"png").unwrap();
    let last = publish_last_at(&dir, |p| link_or_write(&card, p, b"png")).unwrap();
    let inode = |path: &Path| fs::metadata(path).unwrap().ino();
    assert_eq!(inode(&last), inode(&card));
    let gone = dir.join("gone.png");
    let last = publish_last_at(&dir, |p| link_or_write(&gone, p, b"copy")).unwrap();
    assert_eq!(fs::read(&last).unwrap(), b"copy");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn publishing_last_png_replaces_complete_file() {
    let dir = env::temp_dir().join(format!(
        "boltsnap-last-png-test-{}-{}",
        std::process::id(),
        timestamp()
    ));
    fs::create_dir(&dir).unwrap();

    publish_last_png_at(&dir, b"first").unwrap();
    publish_last_png_at(&dir, b"second").unwrap();

    assert_eq!(fs::read(dir.join("last.png")).unwrap(), b"second");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    let _ = fs::remove_dir_all(dir);
}

#[cfg(target_os = "linux")]
#[test]
fn detached_child_is_reaped() {
    let pid =
        spawn_reaped(std::process::Command::new("sh").args(["-c", "exit 0"])).expect("spawn child");
    let proc = PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..100 {
        if !proc.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("child {pid} was not reaped");
}

#[test]
fn xdg_base_ignores_empty_and_relative_values() {
    assert_eq!(xdg_base(None), None);
    assert_eq!(xdg_base(Some("".into())), None);
    assert_eq!(xdg_base(Some("config".into())), None);
    assert_eq!(
        xdg_base(Some("/home/u/.config".into())),
        Some(PathBuf::from("/home/u/.config"))
    );
}

#[test]
fn replay_clips_are_orphan_recording_files() {
    assert!(is_orphan_rec_file("boltsnap-final-12-3.mp4"));
    assert!(is_orphan_rec_file("clip-3233529-1790267666021572148-1.mkv"));
    assert!(is_orphan_rec_file("clip-3233529-1790267666021572148-1.mp4"));
    assert!(!is_orphan_rec_file("clip-notes.txt"));
    assert!(!is_orphan_rec_file("other.mkv"));
    // Crop partials already end in .mp4 (.mkv before); remux partials end in .partial.
    assert!(is_orphan_rec_file("clip-3-4-1.partial.mkv"));
    assert!(is_orphan_rec_file("clip-3-4-1.partial.mp4"));
    assert!(is_orphan_rec_file("clip-123-456-1.mkv.123.0.partial"));
    assert!(is_orphan_rec_file("clip-123-456-1.mp4.123.0.partial"));
    for kept in [
        "clip-123-456-1.mkv.x.0.partial",
        "clip-123-456-1.mkv.123..partial",
        "clip-123-456-1.mkv.123.partial",
        "notes.mkv.123.0.partial",
        "clip-notes.txt.123.0.partial",
        "clip-1.mkv.123.0.partial.bak",
    ] {
        assert!(!is_orphan_rec_file(kept), "{kept}");
    }
}

#[test]
fn orphan_shelf_video_names_are_cleanup_candidates() {
    assert!(is_orphan_shelf_temp("boltsnap-shelf-video-12-34.mp4"));
    assert!(is_orphan_shelf_temp("boltsnap-shelf-12-34.png"));
    assert!(!is_orphan_shelf_temp("boltsnap-recording-12-34.mp4"));
}

#[test]
fn permanent_recording_names_are_sanitized_and_never_overwrite() {
    let dir = (0_u32..)
        .map(|n| env::temp_dir().join(format!("boltsnap-path-test-{}-{n}", std::process::id())))
        .find_map(|path| match fs::create_dir(&path) {
            Ok(()) => Some(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(error) => panic!("create unique test directory: {error}"),
        })
        .unwrap();
    let first = unique_saved_path_at(&dir, Some("DP 1/left"), "2026-07-12_12-00-00", "mp4");
    assert!(
        first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("-DP-1-left.mp4")
    );
    fs::write(&first, b"existing").unwrap();
    let second = unique_saved_path_at(&dir, Some("DP 1/left"), "2026-07-12_12-00-00", "mp4");
    assert_ne!(first, second);
    assert!(
        second
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-1.mp4")
    );
    // A replay clip may not reuse the sidecar name of that saved MP4.
    fs::write(second.with_extension("cursor.json"), b"{}").unwrap();
    let third = unique_saved_path_at(&dir, Some("DP 1/left"), "2026-07-12_12-00-00", "mkv");
    assert!(third.to_string_lossy().ends_with("-DP-1-left-2.mkv"));
    let bare = unique_saved_path_at(&dir, None, "2026-07-12_12-00-00", "");
    assert_eq!(bare, dir.join("boltsnap-2026-07-12_12-00-00"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_pictures_folder_comes_from_user_dirs() {
    use std::path::{Path, PathBuf};
    let home = Path::new("/home/u");
    let at = |text: &str| super::pictures_dir_in(text, home);
    let file = "# written by xdg-user-dirs-update\n\
                XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n\
                XDG_PICTURES_DIR=\"$HOME/Bilder\"\n";
    assert_eq!(at(file), Some(PathBuf::from("/home/u/Bilder")));
    assert_eq!(
        at("XDG_PICTURES_DIR=\"/data/photos\""),
        Some(PathBuf::from("/data/photos"))
    );
    assert_eq!(
        at("XDG_PICTURES_DIR=$HOME/Pics"),
        Some(PathBuf::from("/home/u/Pics"))
    );
    assert_eq!(
        at("XDG_PICTURES_DIR=\"$HOME\""),
        Some(PathBuf::from("/home/u"))
    );
    // Commented out, missing, relative or another variable: not set.
    assert_eq!(at("# XDG_PICTURES_DIR=\"$HOME/Bilder\""), None);
    assert_eq!(at("XDG_MUSIC_DIR=\"$HOME/Musik\""), None);
    assert_eq!(at("XDG_PICTURES_DIR=\"Bilder\""), None);
    assert_eq!(at("XDG_PICTURES_DIR=\"$HOMEX/Bilder\""), None);
}
