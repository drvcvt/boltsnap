use super::Monitor;
use super::session::RecorderTools;
use crate::config::{RecordBothMode, RecordCursor};
use crate::platform::files::{copy_noreplace, rename_noreplace};
use std::collections::BTreeMap;
use std::fs::{self, File};
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
    /// Recording frame rate; the remux snaps frame times back onto its grid.
    pub fps: u32,
    pub destination: SaveDestination,
    /// Smooth modes keep the recorded pointer track as `X.cursor.json`.
    pub cursor: RecordCursor,
    /// Recorded region for area recordings, in logical coordinates.
    pub region: Option<super::Geometry>,
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
pub const RECORDING_CACHE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
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

    let combined = req.both_mode == RecordBothMode::Combined && req.segments.len() > 1;
    // Tracks must be read before concatenation removes their segments. They
    // only feed `X.cursor.json`; the cursor itself is already in the video.
    let mut tracks = BTreeMap::new();
    if req.cursor != RecordCursor::System {
        for (output, segments) in &req.segments {
            let Some((origin, _)) = cursor_frame(&req, output.as_deref(), combined) else {
                continue;
            };
            match super::cursor_sidecar::load(segments, origin, &tools.ffmpeg) {
                Ok(Some(logical)) => {
                    tracks.insert(output.clone(), (logical.samples, logical.size));
                }
                Ok(None) => {}
                Err(error) => eprintln!("boltsnap: read cursor track: {error}"),
            }
        }
    }
    let originals: Vec<PathBuf> = req.segments.values().flatten().cloned().collect();

    // A disk save remuxes straight into its directory, so a large recording
    // needs room there once instead of a second copy in the cache.
    let disk = match &req.destination {
        SaveDestination::Disk(dir) => match fs::create_dir_all(dir) {
            Ok(()) => Some(dir.clone()),
            Err(error) => {
                return Err(failure(
                    format!("create recording directory: {error}"),
                    req.segments,
                ));
            }
        },
        SaveDestination::Shelf => None,
    };
    // Combined outputs are composed again afterwards; only that result is final.
    let remux_dir = match &disk {
        Some(dir) if !combined => dir.clone(),
        _ => tools.segment_dir.clone(),
    };
    let mut ready = req.segments.clone();
    for (output, segments) in ready.clone() {
        match finalize_group(
            output.as_deref(),
            &segments,
            Some(req.fps),
            &remux_dir,
            tools,
        ) {
            Ok(path) => {
                ready.insert(output, vec![path]);
            }
            Err(error) => {
                return Err(failure(error, keep_recoverable(ready, &tools.segment_dir)));
            }
        }
    }

    if req.both_mode == RecordBothMode::Combined && ready.len() > 1 {
        let compose_dir = disk.as_deref().unwrap_or(&tools.segment_dir);
        match compose_outputs(&ready, &req.monitors, &req.codec, compose_dir, tools) {
            Ok(path) => {
                ready.clear();
                ready.insert(None, vec![path]);
            }
            Err(error) => {
                return Err(failure(error, keep_recoverable(ready, &tools.segment_dir)));
            }
        }
        let merged = super::cursor::merge(
            &tracks
                .values()
                .map(|(samples, _)| samples.clone())
                .collect::<Vec<_>>(),
        );
        tracks.clear();
        if !merged.is_empty() {
            // The composed canvas has its own size.
            tracks.insert(None, (merged, None));
        }
    }

    for (output, (samples, size)) in tracks {
        let Some(clip) = ready.get(&output).and_then(|paths| paths.first()) else {
            continue;
        };
        if let Err(error) = write_cursor_track(
            &req,
            output.as_deref(),
            combined,
            clip,
            &samples,
            size,
            tools,
        ) {
            eprintln!("boltsnap: {} has no cursor track: {error}", clip.display());
        }
    }
    super::cursor_sidecar::remove_segment_files(&originals);

    match req.destination {
        SaveDestination::Shelf => Ok(clips_from(&ready, false)),
        SaveDestination::Disk(dir) => move_to_disk(ready, &dir),
    }
}

/// Logical origin and width of a clip for cursor mapping.
fn cursor_frame(
    req: &FinalizeRequest,
    output: Option<&str>,
    combined: bool,
) -> Option<((f64, f64), f64)> {
    let logical = |m: &Monitor| (f64::from(m.x), f64::from(m.y), f64::from(m.width) / m.scale);
    // A combined layout recorded as one stream has no output and no region.
    if combined || (output.is_none() && req.region.is_none()) {
        let mut monitors = req.monitors.iter().filter(|m| m.scale > 0.0).map(logical);
        let first = monitors.next()?;
        let (min_x, min_y, max_x) = monitors.fold(
            (first.0, first.1, first.0 + first.2),
            |(x0, y0, x1), (x, y, w)| (x0.min(x), y0.min(y), x1.max(x + w)),
        );
        return Some(((min_x, min_y), max_x - min_x));
    }
    match output {
        Some(name) => req
            .monitors
            .iter()
            .find(|m| m.name == name && m.scale > 0.0)
            .map(|m| {
                let (x, y, w) = logical(m);
                ((x, y), w)
            }),
        None => req
            .region
            .map(|g| ((f64::from(g.x), f64::from(g.y)), f64::from(g.w))),
    }
}

/// Write `X.cursor.json` for a finished clip; `samples` are logical units
/// relative to the clip origin. `size` is the video size when the segments'
/// probe already knows it; otherwise the clip is probed.
fn write_cursor_track(
    req: &FinalizeRequest,
    output: Option<&str>,
    combined: bool,
    clip: &Path,
    samples: &[super::cursor::Sample],
    size: Option<(u32, u32)>,
    tools: &RecorderTools,
) -> Result<(), String> {
    let (_, logical_width) = cursor_frame(req, output, combined).ok_or("missing clip layout")?;
    let (width, height) = match size {
        Some(size) => size,
        None => {
            let probe = super::cursor_sidecar::probe(clip, &tools.ffmpeg)?;
            (probe.width, probe.height)
        }
    };
    let factor = f64::from(width) / logical_width;
    super::cursor_sidecar::write(
        clip,
        &super::cursor_sidecar::to_pixels(samples, factor),
        (width, height),
        factor,
        req.cursor,
    )
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

/// After a failed save the remuxed outputs in the cache are the only copy, but
/// the next daemon start keeps only `boltsnap-segment-*` there. Renaming them
/// into that pattern lets recovery save them if the daemon dies first.
fn keep_recoverable(
    ready: BTreeMap<Option<String>, Vec<PathBuf>>,
    cache: &Path,
) -> BTreeMap<Option<String>, Vec<PathBuf>> {
    ready
        .into_iter()
        .map(|(output, paths)| {
            let paths = paths
                .into_iter()
                .map(|path| {
                    let work = path.parent() == Some(cache)
                        && path.file_name().is_some_and(|name| {
                            let name = name.to_string_lossy();
                            name.starts_with("boltsnap-final-")
                                || name.starts_with("boltsnap-combined-")
                        });
                    if !work {
                        return path;
                    }
                    let kept = super::session::segment_path(cache, output.as_deref(), "mp4");
                    match rename_noreplace(&path, &kept) {
                        Ok(()) => kept,
                        Err(error) => {
                            eprintln!(
                                "boltsnap: could not keep {} for recovery: {error}",
                                path.display()
                            );
                            path
                        }
                    }
                })
                .collect();
            (output, paths)
        })
        .collect()
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

/// Concatenate one output's segments into an MP4 in `dir`. `fps` snaps gsr's
/// constant-rate frames back onto their grid; without it times stay as stored.
pub(crate) fn finalize_group(
    output: Option<&str>,
    segments: &[PathBuf],
    fps: Option<u32>,
    dir: &Path,
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
    // gsr writes crash-safe Matroska segments; clips are always MP4.
    if segments.len() == 1 && segments[0].extension().is_some_and(|ext| ext == "mp4") {
        return Ok(segments[0].clone());
    }

    ensure_free_space(dir, source_size(segments)?)?;
    let list = work_path(&tools.segment_dir, "concat", output, "txt");
    let final_path = work_path(dir, "final", output, "mp4");
    if let Err(error) = write_concat_list(&list, segments) {
        remove_work_file(&list);
        return Err(format!("write concat list: {error}"));
    }
    // Only gsr's Matroska segments are constant rate; wf-recorder's frames
    // follow damage and could collide on a fixed grid.
    let grid = segments
        .iter()
        .all(|segment| segment.extension().is_some_and(|ext| ext == "mkv"))
        .then_some(fps)
        .flatten();
    if let Err(error) = run_ffmpeg(&tools.ffmpeg, &build_concat_args(&list, &final_path, grid)) {
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

/// Inputs in layout order and the xstack filter labelled `[v]`.
fn combined_layout(
    ready: &BTreeMap<Option<String>, Vec<PathBuf>>,
    monitors: &[Monitor],
    codec: &str,
) -> Result<(Vec<PathBuf>, String), String> {
    let selected: Vec<&Monitor> = monitors
        .iter()
        .filter(|monitor| ready.contains_key(&Some(monitor.name.clone())))
        .collect();
    if selected.len() != ready.len() {
        return Err("combined recording is missing monitor layout data".into());
    }
    let paths = selected
        .iter()
        .map(|monitor| ready[&Some(monitor.name.clone())][0].clone())
        .collect();
    let filter = build_xstack_filter_for(&selected, canvas_limit(codec))?;
    Ok((paths, filter))
}

fn compose_outputs(
    ready: &BTreeMap<Option<String>, Vec<PathBuf>>,
    monitors: &[Monitor],
    codec: &str,
    dir: &Path,
    tools: &RecorderTools,
) -> Result<PathBuf, String> {
    let (ordered_paths, filter) = combined_layout(ready, monitors, codec)?;
    ensure_free_space(dir, source_size(&ordered_paths)?)?;
    let output = work_path(dir, "combined", None, "mp4");
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
    // Saving moves the bytes unchanged, so the container keeps its extension.
    let extension = media_extension(source);
    loop {
        let destination = crate::paths::unique_saved_path(dir, output, &extension);
        match move_final_file(source, &destination) {
            Ok(()) => {
                move_sidecars(source, &destination);
                return Ok(destination);
            }
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && fs::symlink_metadata(&destination).is_ok() => {}
            Err(error) => {
                return Err(format!(
                    "save recording to {}: {error}",
                    destination.display()
                ));
            }
        }
    }
}

/// The source's extension, lowercased, when it looks like one; otherwise none
/// rather than a claim the bytes do not back.
fn media_extension(source: &Path) -> String {
    source
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            (1..=8).contains(&extension.len())
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .unwrap_or_default()
        .to_ascii_lowercase()
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
                Ok(()) => {
                    move_sidecars(&source, &destination);
                    break destination;
                }
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

/// Cursor sidecars follow their clip; a failure keeps the clip saved.
fn move_sidecars(source: &Path, destination: &Path) {
    if let Err(error) = super::cursor_sidecar::move_sidecars(source, destination) {
        eprintln!("boltsnap: saved {}, but {error}", destination.display());
    }
}

/// Stream-copy the segments into one MP4. Matroska keeps frame times in whole
/// milliseconds (0, 8, 17, 25 ms at 120 FPS), and a player shows a frame at
/// the first refresh after its time, so a frame rounded up misses its refresh
/// and the motion judders. For constant-rate video, a timescale of `grid`
/// frames per second rounds every frame back onto its exact slot.
pub fn build_concat_args(list: &Path, output: &Path, grid: Option<u32>) -> Vec<String> {
    let timescale = grid
        .into_iter()
        .flat_map(|fps| ["-video_track_timescale".to_owned(), fps.to_string()]);
    ["-y", "-f", "concat", "-safe", "0", "-i"]
        .into_iter()
        .map(str::to_owned)
        .chain([list.to_string_lossy().into_owned()])
        .chain(["-c".into(), "copy".into()])
        .chain(timescale)
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
    build_xstack_filter_for(&monitors.iter().collect::<Vec<_>>(), f64::INFINITY)
}

/// Largest frame side hardware H.264 encoders accept.
pub const HARDWARE_H264_MAX_SIDE: u32 = 4096;

/// Largest canvas side the encoder accepts.
fn canvas_limit(codec: &str) -> f64 {
    if ["_nvenc", "_vaapi", "_vulkan"]
        .iter()
        .any(|suffix| codec.ends_with(suffix))
    {
        f64::from(HARDWARE_H264_MAX_SIDE)
    } else {
        f64::INFINITY
    }
}

/// Stacks the outputs at the largest scale, shrunk to fit `limit`. Edges are
/// rounded to even pixels so tiles meet without gaps in 4:2:0.
fn build_xstack_filter_for(monitors: &[&Monitor], limit: f64) -> Result<String, String> {
    if monitors.len() < 2 {
        return Err("combined recording requires at least two outputs".into());
    }
    if monitors.iter().any(|monitor| monitor.scale <= 0.0) {
        return Err("monitor scale must be positive".into());
    }
    let min_x = monitors.iter().map(|monitor| monitor.x).min().unwrap();
    let min_y = monitors.iter().map(|monitor| monitor.y).min().unwrap();
    // Logical rect of each output relative to the layout origin.
    let rects: Vec<_> = monitors
        .iter()
        .map(|m| {
            let (x, y) = (f64::from(m.x - min_x), f64::from(m.y - min_y));
            let (w, h) = (m.width as f64 / m.scale, m.height as f64 / m.scale);
            (x, y, x + w, y + h)
        })
        .collect();
    let right = rects.iter().map(|r| r.2).fold(0.0, f64::max);
    let bottom = rects.iter().map(|r| r.3).fold(0.0, f64::max);
    let scale = monitors
        .iter()
        .map(|monitor| monitor.scale)
        .fold(1.0_f64, f64::max)
        .min(limit / right)
        .min(limit / bottom);
    let px = |logical: f64| ((logical * scale / 2.0).round() * 2.0) as i64;
    let mut filters = String::new();
    let mut inputs = String::new();
    let mut layout = Vec::with_capacity(monitors.len());
    for (index, &(x, y, x1, y1)) in rects.iter().enumerate() {
        let (left, top) = (px(x), px(y));
        let (width, height) = (px(x1) - left, px(y1) - top);
        filters.push_str(&format!("[{index}:v]scale={width}:{height}[v{index}];"));
        inputs.push_str(&format!("[v{index}]"));
        layout.push(format!("{left}_{top}"));
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
    let filter = match hardware_device(codec) {
        Some(device) => {
            args.extend(["-init_hw_device".into(), device.into()]);
            args.extend(["-filter_hw_device".into(), "hw".into()]);
            let stacked = filter.strip_suffix("[v]").unwrap_or(filter);
            format!("{stacked},format=nv12,hwupload[v]")
        }
        None => filter.to_owned(),
    };
    for input in inputs {
        args.push("-i".into());
        args.push(input.to_string_lossy().into_owned());
    }
    args.extend([
        "-filter_complex".into(),
        filter,
        "-map".into(),
        "[v]".into(),
        "-c:v".into(),
        codec.into(),
        "-map".into(),
        "0:a?".into(),
        "-c:a".into(),
        "copy".into(),
    ]);
    args.extend(quality_args(codec));
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Device for encoders that take hardware frames, which need an upload after
/// software filters.
fn hardware_device(codec: &str) -> Option<&'static str> {
    match codec.rsplit('_').next() {
        Some("vulkan") => Some("vulkan=hw"),
        Some("vaapi") => Some("vaapi=hw"),
        _ => None,
    }
}

pub fn quality_args(codec: &str) -> Vec<String> {
    if codec.ends_with("_nvenc") {
        [
            // p1 composes 3840x1080@240 about 2.3x faster than p5 at the
            // same CQ; measured PSNR differed by 0.02 dB.
            "-preset", "p1", "-tune", "hq", "-rc", "vbr", "-cq", "16", "-b:v", "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    } else if codec.ends_with("_vulkan") {
        ["-rc_mode", "cqp", "-qp", "18"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else if codec.ends_with("_vaapi") {
        ["-rc_mode", "CQP", "-qp", "18"]
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
    run_ffmpeg_in(program, args, None)
}

/// `dir` lets filter options name files without filtergraph escaping.
pub(crate) fn run_ffmpeg_in(
    program: &Path,
    args: &[String],
    dir: Option<&Path>,
) -> Result<(), String> {
    let mut busy_retries = 3;
    let output = loop {
        let mut command = Command::new(program);
        // Remux and compose wait for the foreground, never the other way round.
        crate::platform::replay::process::in_background(&mut command);
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let result = command
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

pub(crate) fn require_nonempty(path: &Path) -> Result<(), String> {
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

pub fn check_recording_cache_limit(dir: &Path) -> Result<(), String> {
    check_recording_cache_capacity(dir, 0)
}

pub fn check_recording_cache_capacity(dir: &Path, incoming_bytes: u64) -> Result<(), String> {
    let bytes = fs::read_dir(dir)
        .map_err(|error| format!("read recording cache: {error}"))?
        .try_fold(0_u64, |total, entry| {
            let entry = entry.map_err(|error| format!("read recording cache entry: {error}"))?;
            let metadata = entry
                .metadata()
                .map_err(|error| format!("inspect recording cache entry: {error}"))?;
            let bytes = if metadata.is_file() {
                metadata.len()
            } else {
                0
            };
            Ok::<_, String>(total.saturating_add(bytes))
        })?;
    require_recording_cache_limit(bytes.saturating_add(incoming_bytes))
}

fn require_recording_cache_limit(bytes: u64) -> Result<(), String> {
    if bytes < RECORDING_CACHE_LIMIT_BYTES {
        Ok(())
    } else {
        Err(format!(
            "recording paused because the temporary recording cache reached 2 GiB ({bytes} bytes used); save or dismiss shelf recordings to free space"
        ))
    }
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

/// Free space a remux of `source_bytes` needs next to its source.
pub fn remux_space(source_bytes: u64) -> u64 {
    let margin = (64 * 1024 * 1024_u64).max(source_bytes / 20);
    source_bytes.saturating_add(margin)
}

fn ensure_free_space(dir: &Path, source_bytes: u64) -> Result<(), String> {
    let free = available_space(dir)?;
    let required = remux_space(source_bytes);
    if free > required {
        Ok(())
    } else {
        Err(format!(
            "insufficient disk space: need more than {required} bytes, {free} available"
        ))
    }
}

pub(crate) fn move_final_file(source: &Path, destination: &Path) -> io::Result<()> {
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
    let mut input = File::open(source)?;
    copy_noreplace(&mut input, destination)?;
    if let Err(error) = fs::remove_file(source) {
        eprintln!(
            "boltsnap: saved {}, but could not remove source {}: {error}",
            destination.display(),
            source.display()
        );
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
mod tests;
