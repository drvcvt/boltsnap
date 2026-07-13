use super::Monitor;
use super::session::RecorderTools;
use crate::config::RecordBothMode;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveDestination {
    Shelf,
    Disk(PathBuf),
}

pub struct FinalizeRequest {
    pub segments: BTreeMap<Option<String>, Vec<PathBuf>>,
    pub monitors: Vec<Monitor>,
    pub both_mode: RecordBothMode,
    pub codec: String,
    pub destination: SaveDestination,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizedClip {
    pub output: Option<String>,
    pub path: PathBuf,
    pub permanent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizeFailure {
    pub error: String,
    pub recoverable_segments: BTreeMap<Option<String>, Vec<PathBuf>>,
    pub completed: Vec<FinalizedClip>,
}

static WORK_ID: AtomicU64 = AtomicU64::new(0);
pub const RECORDING_DISK_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn finalize_recording(
    req: FinalizeRequest,
    tools: &RecorderTools,
) -> Result<Vec<FinalizedClip>, FinalizeFailure> {
    if req.segments.is_empty() || req.segments.values().any(Vec::is_empty) {
        return Err(failure("recording has no segments", req.segments));
    }
    if let Err(error) = fs::create_dir_all(&tools.segment_dir) {
        return Err(failure(
            format!("create recording cache: {error}"),
            req.segments,
        ));
    }

    let mut ready = req.segments;
    for (output, segments) in ready.clone() {
        match finalize_group(output.as_deref(), &segments, tools) {
            Ok(path) => {
                ready.insert(output, vec![path]);
            }
            Err(error) => return Err(failure(error, ready)),
        }
    }

    if req.both_mode == RecordBothMode::Combined && ready.len() > 1 {
        match compose_outputs(&ready, &req.monitors, &req.codec, tools) {
            Ok(path) => {
                ready.clear();
                ready.insert(None, vec![path]);
            }
            Err(error) => return Err(failure(error, ready)),
        }
    }

    match req.destination {
        SaveDestination::Shelf => Ok(clips_from(&ready, false)),
        SaveDestination::Disk(dir) => move_to_disk(ready, &dir),
    }
}

fn failure(
    error: impl Into<String>,
    recoverable_segments: BTreeMap<Option<String>, Vec<PathBuf>>,
) -> FinalizeFailure {
    FinalizeFailure {
        error: error.into(),
        recoverable_segments,
        completed: Vec::new(),
    }
}

fn clips_from(
    segments: &BTreeMap<Option<String>, Vec<PathBuf>>,
    permanent: bool,
) -> Vec<FinalizedClip> {
    segments
        .iter()
        .flat_map(|(output, paths)| {
            paths.iter().map(move |path| FinalizedClip {
                output: output.clone(),
                path: path.clone(),
                permanent,
            })
        })
        .collect()
}

fn finalize_group(
    output: Option<&str>,
    segments: &[PathBuf],
    tools: &RecorderTools,
) -> Result<PathBuf, String> {
    for segment in segments {
        if !segment.is_file() {
            return Err(format!(
                "recording segment is missing: {}",
                segment.display()
            ));
        }
    }
    if segments.len() == 1 {
        return Ok(segments[0].clone());
    }

    ensure_free_space(&tools.segment_dir, source_size(segments)?)?;
    let list = work_path(&tools.segment_dir, "concat", output, "txt");
    let final_path = work_path(&tools.segment_dir, "final", output, "mp4");
    if let Err(error) = write_concat_list(&list, segments) {
        remove_work_file(&list);
        return Err(format!("write concat list: {error}"));
    }
    if let Err(error) = run_ffmpeg(&tools.ffmpeg, &build_concat_args(&list, &final_path)) {
        remove_work_file(&final_path);
        remove_work_file(&list);
        return Err(error);
    }
    if let Err(error) = require_nonempty(&final_path) {
        remove_work_file(&final_path);
        remove_work_file(&list);
        return Err(error);
    }

    for segment in segments {
        if let Err(error) = fs::remove_file(segment) {
            eprintln!(
                "boltsnap: finalized {}, but could not remove segment: {error}",
                segment.display()
            );
        }
    }
    remove_work_file(&list);
    Ok(final_path)
}

fn compose_outputs(
    ready: &BTreeMap<Option<String>, Vec<PathBuf>>,
    monitors: &[Monitor],
    codec: &str,
    tools: &RecorderTools,
) -> Result<PathBuf, String> {
    let selected: Vec<&Monitor> = monitors
        .iter()
        .filter(|monitor| ready.contains_key(&Some(monitor.name.clone())))
        .collect();
    if selected.len() != ready.len() {
        return Err("combined recording is missing monitor layout data".into());
    }
    let ordered_paths: Vec<PathBuf> = selected
        .iter()
        .map(|monitor| ready[&Some(monitor.name.clone())][0].clone())
        .collect();
    ensure_free_space(&tools.segment_dir, source_size(&ordered_paths)?)?;
    let output = work_path(&tools.segment_dir, "combined", None, "mp4");
    let filter = build_xstack_filter_for(&selected)?;
    let args = build_combined_args(&ordered_paths, &filter, codec, &output);
    if let Err(error) = run_ffmpeg(&tools.ffmpeg, &args) {
        remove_work_file(&output);
        return Err(error);
    }
    if let Err(error) = require_nonempty(&output) {
        remove_work_file(&output);
        return Err(error);
    }

    for path in ordered_paths {
        if let Err(error) = fs::remove_file(&path) {
            eprintln!(
                "boltsnap: composed {}, but could not remove input: {error}",
                path.display()
            );
        }
    }
    Ok(output)
}

fn move_to_disk(
    ready: BTreeMap<Option<String>, Vec<PathBuf>>,
    dir: &Path,
) -> Result<Vec<FinalizedClip>, FinalizeFailure> {
    move_to_disk_with(ready, dir, move_final_file)
}

pub fn promote_recording(
    source: &Path,
    dir: &Path,
    output: Option<&str>,
) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|error| format!("create recording directory: {error}"))?;
    loop {
        let destination = crate::paths::unique_recording_path(dir, output);
        match move_final_file(source, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && destination.exists() => {}
            Err(error) => {
                return Err(format!(
                    "save recording to {}: {error}",
                    destination.display()
                ));
            }
        }
    }
}

fn move_to_disk_with(
    mut ready: BTreeMap<Option<String>, Vec<PathBuf>>,
    dir: &Path,
    mut move_file: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<Vec<FinalizedClip>, FinalizeFailure> {
    if let Err(error) = fs::create_dir_all(dir) {
        return Err(failure(
            format!("create recording directory: {error}"),
            ready,
        ));
    }
    let mut clips = Vec::with_capacity(ready.len());
    for (output, paths) in ready.clone() {
        let source = paths[0].clone();
        let destination = loop {
            let destination = crate::paths::unique_recording_path(dir, output.as_deref());
            match move_file(&source, &destination) {
                Ok(()) => break destination,
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists && destination.exists() =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(FinalizeFailure {
                        error: format!("save recording to {}: {error}", destination.display()),
                        recoverable_segments: ready,
                        completed: clips,
                    });
                }
            }
        };
        ready.remove(&output);
        clips.push(FinalizedClip {
            output,
            path: destination,
            permanent: true,
        });
    }
    Ok(clips)
}

pub fn build_concat_args(list: &Path, output: &Path) -> Vec<String> {
    ["-y", "-f", "concat", "-safe", "0", "-i"]
        .into_iter()
        .map(str::to_owned)
        .chain([list.to_string_lossy().into_owned()])
        .chain(["-c".into(), "copy".into()])
        .chain([output.to_string_lossy().into_owned()])
        .collect()
}

fn write_concat_list(path: &Path, segments: &[PathBuf]) -> io::Result<()> {
    let mut file = File::create(path)?;
    for segment in segments {
        writeln!(file, "file '{}'", escape_concat_path(segment))?;
    }
    file.sync_all()
}

fn escape_concat_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}

#[cfg(test)]
fn build_xstack_filter(monitors: &[Monitor]) -> Result<String, String> {
    build_xstack_filter_for(&monitors.iter().collect::<Vec<_>>())
}

fn build_xstack_filter_for(monitors: &[&Monitor]) -> Result<String, String> {
    if monitors.len() < 2 {
        return Err("combined recording requires at least two outputs".into());
    }
    if monitors.iter().any(|monitor| monitor.scale <= 0.0) {
        return Err("monitor scale must be positive".into());
    }
    let scale = monitors
        .iter()
        .map(|monitor| monitor.scale)
        .fold(1.0_f64, f64::max);
    let min_x = monitors.iter().map(|monitor| monitor.x).min().unwrap();
    let min_y = monitors.iter().map(|monitor| monitor.y).min().unwrap();
    let mut filters = String::new();
    let mut inputs = String::new();
    let mut layout = Vec::with_capacity(monitors.len());
    for (index, monitor) in monitors.iter().enumerate() {
        let width = ((monitor.width as f64 / monitor.scale) * scale).round() as u32;
        let height = ((monitor.height as f64 / monitor.scale) * scale).round() as u32;
        filters.push_str(&format!("[{index}:v]scale={width}:{height}[v{index}];"));
        inputs.push_str(&format!("[v{index}]"));
        layout.push(format!(
            "{}_{}",
            ((monitor.x - min_x) as f64 * scale).round() as i64,
            ((monitor.y - min_y) as f64 * scale).round() as i64
        ));
    }
    Ok(format!(
        "{filters}{inputs}xstack=inputs={}:layout={}:fill=black[v]",
        monitors.len(),
        layout.join("|")
    ))
}

fn build_combined_args(
    inputs: &[PathBuf],
    filter: &str,
    codec: &str,
    output: &Path,
) -> Vec<String> {
    let mut args = vec!["-y".into()];
    for input in inputs {
        args.push("-i".into());
        args.push(input.to_string_lossy().into_owned());
    }
    args.extend([
        "-filter_complex".into(),
        filter.into(),
        "-map".into(),
        "[v]".into(),
        "-c:v".into(),
        codec.into(),
    ]);
    args.extend(quality_args(codec));
    args.push(output.to_string_lossy().into_owned());
    args
}

pub fn quality_args(codec: &str) -> Vec<String> {
    if codec.ends_with("_nvenc") {
        [
            "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "16", "-b:v", "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    } else if matches!(codec, "libx264" | "libx265") {
        ["-preset", "veryfast", "-crf", "16"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        ["-q:v", "2"].into_iter().map(str::to_owned).collect()
    }
}

fn run_ffmpeg(program: &Path, args: &[String]) -> Result<(), String> {
    let mut busy_retries = 3;
    let output = loop {
        let result = Command::new(program)
            .args(["-hide_banner", "-loglevel", "error"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match result {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && busy_retries > 0 => {
                busy_retries -= 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            result => break result.map_err(|error| format!("start ffmpeg: {error}"))?,
        }
    };
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if detail.is_empty() {
            Err(format!("ffmpeg exited with {}", output.status))
        } else {
            Err(format!("ffmpeg exited with {}: {detail}", output.status))
        }
    }
}

fn require_nonempty(path: &Path) -> Result<(), String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > 0 => Ok(()),
        Ok(_) => Err(format!("ffmpeg produced an empty file: {}", path.display())),
        Err(error) => Err(format!("ffmpeg produced no output: {error}")),
    }
}

fn source_size(paths: &[PathBuf]) -> Result<u64, String> {
    paths.iter().try_fold(0_u64, |total, path| {
        fs::metadata(path)
            .map(|metadata| total.saturating_add(metadata.len()))
            .map_err(|error| format!("read segment size for {}: {error}", path.display()))
    })
}

pub fn available_space(dir: &Path) -> Result<u64, String> {
    let path = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes())
        .map_err(|_| "recording path contains a NUL byte".to_string())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "check free disk space: {}",
            io::Error::last_os_error()
        ));
    }
    let stat = unsafe { stat.assume_init() };
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}

pub fn check_recording_reserve(dir: &Path) -> Result<(), String> {
    require_recording_reserve(available_space(dir)?)
}

fn require_recording_reserve(free: u64) -> Result<(), String> {
    if free > RECORDING_DISK_RESERVE_BYTES {
        Ok(())
    } else {
        Err(format!(
            "recording stopped to preserve 2 GiB of free disk space ({free} bytes available)"
        ))
    }
}

fn ensure_free_space(dir: &Path, source_bytes: u64) -> Result<(), String> {
    let free = available_space(dir)?;
    let margin = (64 * 1024 * 1024_u64).max(source_bytes / 20);
    let required = source_bytes.saturating_add(margin);
    if free > required {
        Ok(())
    } else {
        Err(format!(
            "insufficient disk space: need more than {required} bytes, {free} available"
        ))
    }
}

fn move_final_file(source: &Path, destination: &Path) -> io::Result<()> {
    let source_dev = fs::metadata(source)?.dev();
    let destination_dev = fs::metadata(destination.parent().unwrap_or(Path::new(".")))?.dev();
    if source_dev == destination_dev {
        rename_noreplace(source, destination)
    } else {
        copy_then_remove(source, destination)
    }
}

fn copy_then_remove(source: &Path, destination: &Path) -> io::Result<()> {
    let size = fs::metadata(source)?.len();
    ensure_free_space(destination.parent().unwrap_or(Path::new(".")), size)
        .map_err(io::Error::other)?;
    let part = PathBuf::from(format!(
        "{}.part-{}",
        destination.display(),
        std::process::id()
    ));
    let result = (|| {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        rename_noreplace(&part, destination)?;
        if let Err(error) = fs::remove_file(source) {
            eprintln!(
                "boltsnap: saved {}, but could not remove source {}: {error}",
                destination.display(),
                source.display()
            );
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(part);
    }
    result
}

fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    let source_c = std::ffi::CString::new(source.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination_c = std::ffi::CString::new(destination.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::ENOSYS) {
        return Err(error);
    }

    fs::hard_link(source, destination)?;
    if let Err(error) = fs::remove_file(source) {
        if let Err(cleanup) = fs::remove_file(destination) {
            eprintln!(
                "boltsnap: failed to roll back no-replace link {}: {cleanup}",
                destination.display()
            );
        }
        return Err(error);
    }
    Ok(())
}

fn remove_work_file(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        eprintln!(
            "boltsnap: could not remove work file {}: {error}",
            path.display()
        );
    }
}

fn work_path(dir: &Path, prefix: &str, output: Option<&str>, extension: &str) -> PathBuf {
    let output = output
        .map(crate::paths::sanitize_recording_output)
        .filter(|name| !name.is_empty())
        .map(|name| format!("-{name}"))
        .unwrap_or_default();
    dir.join(format!(
        "boltsnap-{prefix}-{}-{}{}.{extension}",
        std::process::id(),
        WORK_ID.fetch_add(1, Ordering::Relaxed),
        output
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(label: &str) -> PathBuf {
        loop {
            let path = std::env::temp_dir().join(format!(
                "boltsnap-finalize-{label}-{}-{}",
                std::process::id(),
                WORK_ID.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create unique test directory: {error}"),
            }
        }
    }

    fn file(path: &Path, bytes: &[u8]) -> PathBuf {
        fs::write(path, bytes).unwrap();
        path.to_path_buf()
    }

    fn tools(dir: &Path, ffmpeg: &Path) -> RecorderTools {
        RecorderTools {
            wf_recorder: "unused".into(),
            ffmpeg: ffmpeg.into(),
            segment_dir: dir.into(),
        }
    }

    fn fake_ffmpeg(dir: &Path, succeeds: bool) -> PathBuf {
        let body = if succeeds {
            "#!/bin/sh\nfor last do :; done\nprintf output > \"$last\"\n"
        } else {
            "#!/bin/sh\nexit 7\n"
        };
        executable(dir.join("ffmpeg"), body)
    }

    fn fake_partial_ffmpeg(dir: &Path) -> PathBuf {
        executable(
            dir.join("ffmpeg-partial"),
            "#!/bin/sh\nfor last do :; done\nprintf partial > \"$last\"\nexit 7\n",
        )
    }

    fn executable(path: PathBuf, body: &str) -> PathBuf {
        let mut script = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        script.write_all(body.as_bytes()).unwrap();
        script.sync_all().unwrap();
        drop(script);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn request(
        segments: BTreeMap<Option<String>, Vec<PathBuf>>,
        mode: RecordBothMode,
        destination: SaveDestination,
    ) -> FinalizeRequest {
        FinalizeRequest {
            segments,
            monitors: Vec::new(),
            both_mode: mode,
            codec: "h264_nvenc".into(),
            destination,
        }
    }

    fn monitor(name: &str, x: i32) -> Monitor {
        Monitor {
            name: name.into(),
            description: String::new(),
            x,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: x == 0,
        }
    }

    #[test]
    fn one_segment_fast_path_does_not_invoke_ffmpeg() {
        let dir = temp_dir("single");
        let segment = file(&dir.join("one.mp4"), b"video");
        let clips = finalize_recording(
            request(
                BTreeMap::from([(None, vec![segment.clone()])]),
                RecordBothMode::Separate,
                SaveDestination::Shelf,
            ),
            &tools(&dir, Path::new("/definitely/missing/ffmpeg")),
        )
        .unwrap();
        assert_eq!(clips[0].path, segment);
        assert!(!clips[0].permanent);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concat_uses_demuxer_and_stream_copy() {
        let args = build_concat_args(Path::new("/tmp/list.txt"), Path::new("/tmp/out.mp4"));
        assert_eq!(
            args,
            vec![
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "/tmp/list.txt",
                "-c",
                "copy",
                "/tmp/out.mp4"
            ]
        );
    }

    #[test]
    fn concat_list_escapes_single_quotes() {
        assert_eq!(
            escape_concat_path(Path::new("/tmp/it's.mp4")),
            "/tmp/it'\\''s.mp4"
        );
    }

    #[test]
    fn separate_mode_keeps_one_clip_per_output() {
        let dir = temp_dir("separate");
        let a = file(&dir.join("a.mp4"), b"a");
        let b = file(&dir.join("b.mp4"), b"b");
        let clips = finalize_recording(
            request(
                BTreeMap::from([
                    (Some("DP-3".into()), vec![a]),
                    (Some("DP-1".into()), vec![b]),
                ]),
                RecordBothMode::Separate,
                SaveDestination::Shelf,
            ),
            &tools(&dir, Path::new("unused")),
        )
        .unwrap();
        assert_eq!(clips.len(), 2);
        assert_eq!(
            clips
                .iter()
                .filter_map(|clip| clip.output.as_deref())
                .collect::<Vec<_>>(),
            vec!["DP-1", "DP-3"]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn combined_mode_composes_one_clip_and_removes_inputs_after_success() {
        let dir = temp_dir("combined");
        let a = file(&dir.join("a.mp4"), b"a");
        let b = file(&dir.join("b.mp4"), b"b");
        let ffmpeg = fake_ffmpeg(&dir, true);
        let mut req = request(
            BTreeMap::from([
                (Some("DP-3".into()), vec![a.clone()]),
                (Some("DP-1".into()), vec![b.clone()]),
            ]),
            RecordBothMode::Combined,
            SaveDestination::Shelf,
        );
        req.monitors = vec![monitor("DP-3", 0), monitor("DP-1", 1920)];
        let clips = finalize_recording(req, &tools(&dir, &ffmpeg)).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].output, None);
        assert!(clips[0].path.is_file());
        assert!(!a.exists());
        assert!(!b.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_combined_encode_removes_partial_output_and_keeps_inputs() {
        let dir = temp_dir("partial-combined");
        let a = file(&dir.join("a.mp4"), b"a");
        let b = file(&dir.join("b.mp4"), b"b");
        let ffmpeg = fake_partial_ffmpeg(&dir);
        let mut req = request(
            BTreeMap::from([
                (Some("DP-3".into()), vec![a.clone()]),
                (Some("DP-1".into()), vec![b.clone()]),
            ]),
            RecordBothMode::Combined,
            SaveDestination::Shelf,
        );
        req.monitors = vec![monitor("DP-3", 0), monitor("DP-1", 1920)];
        assert!(finalize_recording(req, &tools(&dir, &ffmpeg)).is_err());
        assert!(a.is_file());
        assert!(b.is_file());
        assert!(
            !fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains("-combined-"))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn combined_layout_uses_hyprland_positions() {
        let filter = build_xstack_filter(&[
            Monitor {
                name: "DP-3".into(),
                description: "BenQ".into(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.0,
                focused: true,
            },
            Monitor {
                name: "DP-1".into(),
                description: "AOC".into(),
                x: 2560,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                focused: false,
            },
        ])
        .unwrap();
        assert!(filter.contains("xstack=inputs=2:layout=0_0|2560_0:fill=black"));
    }

    #[test]
    fn combined_layout_uses_largest_scale_without_downscaling() {
        let filter = build_xstack_filter(&[
            Monitor {
                name: "A".into(),
                description: String::new(),
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
                scale: 2.0,
                focused: true,
            },
            Monitor {
                name: "B".into(),
                description: String::new(),
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                focused: false,
            },
        ])
        .unwrap();
        assert!(filter.contains("[0:v]scale=3840:2160"));
        assert!(filter.contains("[1:v]scale=3840:2160"));
        assert!(filter.contains("layout=0_0|3840_0"));
    }

    #[test]
    fn nvenc_combined_quality_is_visually_lossless() {
        assert_eq!(
            quality_args("h264_nvenc"),
            vec![
                "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "16", "-b:v", "0"
            ]
        );
        assert_eq!(
            quality_args("libx264"),
            vec!["-preset", "veryfast", "-crf", "16"]
        );
        assert_eq!(quality_args("vp9"), vec!["-q:v", "2"]);
    }

    #[test]
    fn insufficient_space_is_rejected() {
        let dir = temp_dir("space");
        let error = ensure_free_space(&dir, u64::MAX / 2).unwrap_err();
        assert!(error.contains("insufficient disk space"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn available_space_reports_real_filesystem_capacity() {
        let dir = temp_dir("available-space");
        assert!(available_space(&dir).unwrap() > 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recording_reserve_requires_more_than_two_gibibytes() {
        assert!(require_recording_reserve(RECORDING_DISK_RESERVE_BYTES).is_err());
        assert!(require_recording_reserve(RECORDING_DISK_RESERVE_BYTES + 1).is_ok());
    }

    #[test]
    fn failed_ffmpeg_preserves_all_segments() {
        let dir = temp_dir("failure");
        let first = file(&dir.join("one.mp4"), b"one");
        let second = file(&dir.join("two.mp4"), b"two");
        let ffmpeg = fake_ffmpeg(&dir, false);
        let result = finalize_recording(
            request(
                BTreeMap::from([(None, vec![first.clone(), second.clone()])]),
                RecordBothMode::Separate,
                SaveDestination::Shelf,
            ),
            &tools(&dir, &ffmpeg),
        );
        let failure = result.unwrap_err();
        assert!(failure.completed.is_empty());
        assert!(first.is_file());
        assert!(second.is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ffmpeg_failure_includes_stderr() {
        let dir = temp_dir("ffmpeg-stderr");
        let ffmpeg = executable(
            dir.join("ffmpeg-error"),
            "#!/bin/sh\nprintf 'nvenc exploded' >&2\nexit 7\n",
        );
        let error = run_ffmpeg(&ffmpeg, &[]).unwrap_err();
        assert!(error.contains("nvenc exploded"), "{error}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_concat_removes_partial_output_and_list_but_keeps_inputs() {
        let dir = temp_dir("partial-concat");
        let first = file(&dir.join("one.mp4"), b"one");
        let second = file(&dir.join("two.mp4"), b"two");
        let ffmpeg = fake_partial_ffmpeg(&dir);
        let result = finalize_recording(
            request(
                BTreeMap::from([(None, vec![first.clone(), second.clone()])]),
                RecordBothMode::Separate,
                SaveDestination::Shelf,
            ),
            &tools(&dir, &ffmpeg),
        );
        assert!(result.is_err());
        assert!(first.is_file());
        assert!(second.is_file());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.contains("-concat-") || name.contains("-final-")
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover work files: {leftovers:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_concat_removes_segments_after_output_exists() {
        let dir = temp_dir("concat-success");
        let first = file(&dir.join("one.mp4"), b"one");
        let second = file(&dir.join("two.mp4"), b"two");
        let ffmpeg = fake_ffmpeg(&dir, true);
        let clips = finalize_recording(
            request(
                BTreeMap::from([(None, vec![first.clone(), second.clone()])]),
                RecordBothMode::Separate,
                SaveDestination::Shelf,
            ),
            &tools(&dir, &ffmpeg),
        )
        .unwrap();
        assert!(clips[0].path.is_file());
        assert!(!first.exists());
        assert!(!second.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn same_filesystem_save_renames_source() {
        let dir = temp_dir("rename");
        let source = file(&dir.join("source.mp4"), b"video");
        let destination = dir.join("saved.mp4");
        move_final_file(&source, &destination).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"video");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn same_filesystem_move_never_overwrites_an_existing_destination() {
        let dir = temp_dir("rename-no-replace");
        let source = file(&dir.join("source.mp4"), b"source");
        let destination = file(&dir.join("saved.mp4"), b"existing");
        let error = move_final_file(&source, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&destination).unwrap(), b"existing");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_disk_save_reports_completed_and_only_unmoved_recoverable_segments() {
        let dir = temp_dir("partial-disk");
        let cache = dir.join("cache");
        let saved = dir.join("saved");
        fs::create_dir_all(&cache).unwrap();
        let first = file(&cache.join("first.mp4"), b"first");
        let second = file(&cache.join("second.mp4"), b"second");
        let ready = BTreeMap::from([
            (Some("DP-1".into()), vec![first.clone()]),
            (Some("DP-3".into()), vec![second.clone()]),
        ]);
        let mut calls = 0;
        let failure = move_to_disk_with(ready, &saved, |source, destination| {
            calls += 1;
            if calls == 2 {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected"))
            } else {
                move_final_file(source, destination)
            }
        })
        .unwrap_err();
        assert_eq!(failure.completed.len(), 1);
        assert!(failure.completed[0].permanent);
        assert_eq!(failure.recoverable_segments.len(), 1);
        assert!(
            failure
                .recoverable_segments
                .contains_key(&Some("DP-3".into()))
        );
        assert!(!first.exists());
        assert!(second.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn disk_save_retries_a_raced_name_without_overwriting_it() {
        let dir = temp_dir("disk-race");
        let cache = dir.join("cache");
        let saved = dir.join("saved");
        fs::create_dir_all(&cache).unwrap();
        let source = file(&cache.join("source.mp4"), b"source");
        let ready = BTreeMap::from([(None, vec![source])]);
        let mut raced = None;
        let mut calls = 0;
        let clips = move_to_disk_with(ready, &saved, |source, destination| {
            calls += 1;
            if calls == 1 {
                fs::write(destination, b"raced")?;
                raced = Some(destination.to_path_buf());
                Err(io::Error::from(io::ErrorKind::AlreadyExists))
            } else {
                move_final_file(source, destination)
            }
        })
        .unwrap();
        assert_eq!(fs::read(raced.unwrap()).unwrap(), b"raced");
        assert_eq!(fs::read(&clips[0].path).unwrap(), b"source");
        assert_eq!(calls, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shelf_promotion_never_overwrites_and_removes_temporary_sources() {
        let dir = temp_dir("promote");
        let destination = dir.join("saved");
        let first = file(&dir.join("first.mp4"), b"first");
        let second = file(&dir.join("second.mp4"), b"second");

        let first_saved = promote_recording(&first, &destination, None).unwrap();
        let second_saved = promote_recording(&second, &destination, None).unwrap();

        assert_ne!(first_saved, second_saved);
        assert_eq!(fs::read(first_saved).unwrap(), b"first");
        assert_eq!(fs::read(second_saved).unwrap(), b"second");
        assert!(!first.exists());
        assert!(!second.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn disk_save_moves_one_segment_to_a_permanent_unique_name() {
        let dir = temp_dir("disk-save");
        let cache = dir.join("cache");
        let saved = dir.join("saved");
        fs::create_dir_all(&cache).unwrap();
        let source = file(&cache.join("source.mp4"), b"video");
        let clips = finalize_recording(
            request(
                BTreeMap::from([(Some("DP 1".into()), vec![source.clone()])]),
                RecordBothMode::Separate,
                SaveDestination::Disk(saved),
            ),
            &tools(&cache, Path::new("unused")),
        )
        .unwrap();
        assert!(clips[0].permanent);
        assert!(clips[0].path.is_file());
        assert!(!source.exists());
        assert!(
            clips[0]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("-DP-1.mp4")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn completed_copy_never_overwrites_target_and_preserves_source_on_commit_failure() {
        let dir = temp_dir("copy-no-replace");
        let source = file(&dir.join("source.mp4"), b"video");
        let destination = file(&dir.join("saved.mp4"), b"existing");
        let error = copy_then_remove(&source, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(source).unwrap(), b"video");
        assert_eq!(fs::read(destination).unwrap(), b"existing");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn committed_cross_filesystem_copy_survives_source_cleanup_failure() {
        let dir = temp_dir("copy-cleanup");
        let source_dir = dir.join("source");
        let destination_dir = dir.join("destination");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        let source = file(&source_dir.join("source.mp4"), b"video");
        let destination = destination_dir.join("saved.mp4");
        fs::set_permissions(&source_dir, fs::Permissions::from_mode(0o555)).unwrap();
        let result = copy_then_remove(&source, &destination);
        fs::set_permissions(&source_dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_ok());
        assert_eq!(fs::read(&destination).unwrap(), b"video");
        assert!(source.is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn real_cross_filesystem_move_dispatches_when_available() {
        let source_root = temp_dir("real-cross-source");
        let Some(other_root) = [Path::new("/dev/shm"), Path::new("/run/user")]
            .into_iter()
            .find(|path| {
                path.is_dir()
                    && fs::metadata(path).map(|m| m.dev()).ok()
                        != fs::metadata(&source_root).map(|m| m.dev()).ok()
            })
        else {
            let _ = fs::remove_dir_all(source_root);
            return;
        };
        let destination_root = other_root.join(format!(
            "boltsnap-cross-test-{}-{}",
            std::process::id(),
            WORK_ID.fetch_add(1, Ordering::Relaxed)
        ));
        if fs::create_dir(&destination_root).is_err() {
            let _ = fs::remove_dir_all(source_root);
            return;
        }
        let source = file(&source_root.join("source.mp4"), b"video");
        let destination = destination_root.join("saved.mp4");
        move_final_file(&source, &destination).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"video");
        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(destination_root);
    }
}
