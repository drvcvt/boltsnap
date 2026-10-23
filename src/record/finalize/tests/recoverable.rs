//! R1: a save that fails after a partial remux keeps what it made in the cache
//! under the segment pattern, so recovery saves it after a daemon restart.

use super::*;
use crate::record::recover;

/// Fake ffmpeg that writes its output for the first `ok` calls and fails from
/// then on.
fn ffmpeg_failing_after(dir: &Path, ok: u32) -> PathBuf {
    let count = dir.join("ffmpeg-calls");
    executable(
        dir.join("ffmpeg-failing"),
        &format!(
            "#!/bin/sh\nn=$(cat '{0}' 2>/dev/null || echo 0)\nn=$((n + 1))\necho $n > '{0}'\n\
             [ $n -gt {ok} ] && exit 7\nfor last do :; done\nprintf output > \"$last\"\n",
            count.display()
        ),
    )
}

fn segment(dir: &Path, output: &str) -> PathBuf {
    file(
        &crate::record::session::segment_path(dir, Some(output), "mkv"),
        b"mkv",
    )
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn labels(dir: &Path) -> Vec<String> {
    recover::find(dir).into_iter().map(|o| o.label).collect()
}

#[test]
fn only_cache_work_files_are_renamed_into_the_segment_pattern() {
    let dir = temp_dir("keep-recoverable");
    let elsewhere = temp_dir("keep-recoverable-disk");
    let final_file = file(&work_path(&dir, "final", Some("DP-1"), "mp4"), b"final");
    let combined = file(&work_path(&dir, "combined", None, "mp4"), b"combined");
    let original = segment(&dir, "DP-3");
    let on_disk = file(
        &work_path(&elsewhere, "final", Some("DP-2"), "mp4"),
        b"disk",
    );
    let kept = keep_recoverable(
        BTreeMap::from([
            (Some("DP-1".into()), vec![final_file.clone()]),
            (Some("DP-3".into()), vec![original.clone()]),
            (Some("DP-2".into()), vec![on_disk.clone()]),
            (None, vec![combined.clone()]),
        ]),
        &dir,
    );
    let renamed = &kept[&Some("DP-1".into())][0];
    assert!(!final_file.exists());
    assert!(name(renamed).starts_with("boltsnap-segment-"));
    assert!(name(renamed).ends_with("-DP_1.mp4"));
    assert!(!combined.exists());
    assert!(name(&kept[&None][0]).ends_with("-area.mp4"));
    assert_eq!(kept[&Some("DP-3".into())], vec![original]);
    assert_eq!(kept[&Some("DP-2".into())], vec![on_disk.clone()]);
    assert!(on_disk.is_file());
    assert_eq!(labels(&dir), vec!["DP_1", "DP_3", "area"]);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(elsewhere);
}

#[test]
fn a_failed_second_remux_leaves_both_outputs_recoverable() {
    let dir = temp_dir("separate-second-fails");
    let first = segment(&dir, "DP-1");
    let second = segment(&dir, "DP-3");
    let failure = finalize_recording(
        request(
            BTreeMap::from([
                (Some("DP-1".into()), vec![first.clone()]),
                (Some("DP-3".into()), vec![second.clone()]),
            ]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, &ffmpeg_failing_after(&dir, 1)),
    )
    .unwrap_err();
    let remuxed = &failure.recoverable_segments[&Some("DP-1".into())][0];
    assert!(!first.exists(), "the first output's segment was remuxed");
    assert!(remuxed.is_file());
    assert!(name(remuxed).starts_with("boltsnap-segment-"));
    assert_eq!(
        failure.recoverable_segments[&Some("DP-3".into())],
        vec![second.clone()]
    );
    assert!(second.is_file());
    assert_eq!(labels(&dir), vec!["DP_1", "DP_3"]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_failed_compose_of_remuxed_outputs_leaves_them_recoverable() {
    let dir = temp_dir("combined-compose-fails");
    let a = segment(&dir, "DP-3");
    let b = segment(&dir, "DP-1");
    let mut req = request(
        BTreeMap::from([
            (Some("DP-3".into()), vec![a.clone()]),
            (Some("DP-1".into()), vec![b.clone()]),
        ]),
        RecordBothMode::Combined,
        SaveDestination::Shelf,
    );
    req.monitors = vec![monitor("DP-3", 0), monitor("DP-1", 1920)];
    let failure =
        finalize_recording(req, &tools(&dir, &ffmpeg_failing_after(&dir, 2))).unwrap_err();
    assert!(!a.exists() && !b.exists(), "both outputs were remuxed");
    for paths in failure.recoverable_segments.values() {
        assert!(paths[0].is_file());
        assert!(name(&paths[0]).starts_with("boltsnap-segment-"));
    }
    assert_eq!(labels(&dir), vec!["DP_1", "DP_3"]);
    assert!(
        !fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|entry| name(&entry.path()).starts_with("boltsnap-final-"))
    );
    let _ = fs::remove_dir_all(dir);
}
