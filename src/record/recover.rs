//! Recordings a dead daemon left behind. gsr writes crash-safe Matroska, so a
//! segment that outlived a crash, a logout or a lost output is a playable
//! recording. The next daemon start saves it next to the user's recordings
//! instead of deleting it with the other cache leftovers.

use super::finalize::{finalize_group, move_final_file};
use super::session::RecorderTools;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One recording's segments of one output, oldest first.
#[derive(Debug, PartialEq, Eq)]
pub struct Orphan {
    pub label: String,
    pub segments: Vec<PathBuf>,
}

/// `boltsnap-segment-<pid>-<id>-<label>.<mkv|mp4>` as (pid, id, label).
fn segment_name(name: &str) -> Option<(u32, u64, &str)> {
    let rest = name.strip_prefix("boltsnap-segment-")?;
    let stem = rest
        .strip_suffix(".mkv")
        .or_else(|| rest.strip_suffix(".mp4"))?;
    let (pid, rest) = stem.split_once('-')?;
    let (id, label) = rest.split_once('-')?;
    Some((pid.parse().ok()?, id.parse().ok()?, label))
}

/// Non-empty segments in `dir`, grouped per daemon and output. Empty ones
/// never held a frame and stay for the regular cleanup.
pub fn find(dir: &Path) -> Vec<Orphan> {
    let mut groups: BTreeMap<(u32, String), Vec<(u64, PathBuf)>> = BTreeMap::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let Some((pid, id, label)) = name.to_str().and_then(segment_name) else {
            continue;
        };
        if entry
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        {
            groups
                .entry((pid, label.to_owned()))
                .or_default()
                .push((id, entry.path()));
        }
    }
    groups
        .into_iter()
        .map(|((_, label), mut segments)| {
            segments.sort();
            Orphan {
                label,
                segments: segments.into_iter().map(|(_, path)| path).collect(),
            }
        })
        .collect()
}

/// Save `orphan` in `dir` as `boltsnap-<time>-<label>-recovered.mp4`. If the
/// segments cannot be joined, each one is moved there unchanged. Segments are
/// only removed once their content is saved.
pub fn save(orphan: &Orphan, dir: &Path, tools: &RecorderTools) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("create recording directory: {e}"))?;
    let name = format!("{}-recovered", orphan.label);
    // The frame rate it was recorded at is gone; keep the stored times.
    match finalize_group(Some(&orphan.label), &orphan.segments, None, dir, tools) {
        Ok(joined) => {
            let saved = publish(&joined, dir, &name, "mp4")?;
            super::cursor_sidecar::remove_segment_files(&orphan.segments);
            Ok(vec![saved])
        }
        Err(join_error) => {
            let mut saved = Vec::new();
            for segment in &orphan.segments {
                let extension = segment
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("mkv");
                let path = publish(segment, dir, &name, extension)
                    .map_err(|e| format!("{e} (joining failed: {join_error})"))?;
                super::cursor_sidecar::remove_segment_files(std::slice::from_ref(segment));
                saved.push(path);
            }
            Ok(saved)
        }
    }
}

/// Save every orphan in `record_dir` off the startup path and say where each
/// went. One that cannot be saved stays in the cache for the next start.
pub fn save_in_background(orphans: Vec<Orphan>) {
    if orphans.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let dir = crate::config::resolve_record_dir(&crate::config::Config::load());
        let tools = RecorderTools::default();
        for orphan in &orphans {
            let message = match save(orphan, &dir, &tools) {
                Ok(paths) => format!(
                    "Recovered an unsaved recording: {}",
                    paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Err(error) => format!("Unsaved recording kept in the cache: {error}"),
            };
            eprintln!("boltsnap daemon: {message}");
            crate::platform::shelf::notify(&message);
        }
    });
}

fn publish(source: &Path, dir: &Path, name: &str, extension: &str) -> Result<PathBuf, String> {
    loop {
        let destination = crate::paths::unique_saved_path(dir, Some(name), extension);
        match move_final_file(source, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "save recovered recording to {}: {error}",
                    destination.display()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boltsnap-recover-{label}-{}-{}",
            std::process::id(),
            crate::paths::timestamp()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn segment_names_parse_and_other_files_do_not() {
        assert_eq!(
            segment_name("boltsnap-segment-41-7-DP_3.mkv"),
            Some((41, 7, "DP_3"))
        );
        assert_eq!(
            segment_name("boltsnap-segment-41-12-area.mp4"),
            Some((41, 12, "area"))
        );
        assert_eq!(segment_name("boltsnap-segment-41-7-DP_3.mkv.ts"), None);
        assert_eq!(segment_name("boltsnap-final-41-7-DP-3.mp4"), None);
        assert_eq!(segment_name("clip-1-2-3.mp4"), None);
    }

    #[test]
    fn orphans_group_per_daemon_and_output_in_segment_order() {
        let dir = dir("find");
        for (name, bytes) in [
            ("boltsnap-segment-9-10-DP_1.mkv", &b"b"[..]),
            ("boltsnap-segment-9-2-DP_1.mkv", b"a"),
            ("boltsnap-segment-9-3-DP_3.mkv", b"c"),
            ("boltsnap-segment-8-1-DP_1.mkv", b"d"),
            ("boltsnap-segment-9-4-DP_3.mkv", b""),
            ("boltsnap-segment-9-2-DP_1.mkv.ts", b"t"),
        ] {
            fs::write(dir.join(name), bytes).unwrap();
        }
        let found = find(&dir);
        let names: Vec<(String, Vec<String>)> = found
            .iter()
            .map(|orphan| {
                (
                    orphan.label.clone(),
                    orphan
                        .segments
                        .iter()
                        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("DP_1".into(), vec!["boltsnap-segment-8-1-DP_1.mkv".into()]),
                (
                    "DP_1".into(),
                    vec![
                        "boltsnap-segment-9-2-DP_1.mkv".into(),
                        "boltsnap-segment-9-10-DP_1.mkv".into()
                    ]
                ),
                ("DP_3".into(), vec!["boltsnap-segment-9-3-DP_3.mkv".into()]),
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unjoinable_segments_are_moved_unchanged_and_never_lost() {
        let cache = dir("cache");
        let target = dir("target");
        let segment = cache.join("boltsnap-segment-5-1-area.mkv");
        fs::write(&segment, b"not matroska").unwrap();
        let tools = RecorderTools {
            gsr: None,
            wf_recorder: "wf-recorder".into(),
            ffmpeg: "false".into(),
            segment_dir: cache.clone(),
        };
        let orphan = find(&cache).pop().unwrap();
        let saved = save(&orphan, &target, &tools).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(!segment.exists());
        assert_eq!(fs::read(&saved[0]).unwrap(), b"not matroska");
        let name = saved[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-area-recovered.mkv"), "{name}");
        let _ = fs::remove_dir_all(cache);
        let _ = fs::remove_dir_all(target);
    }
}
