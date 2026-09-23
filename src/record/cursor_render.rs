//! Save-time cursor rendering for recordings captured without the pointer.

use super::cursor::{self, Mapping, Placed, Sample, Track, clean_path, sidecar_path};
use crate::config::RecordCursor;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A group's cursor samples in logical units relative to the clip origin.
pub struct Logical {
    pub samples: Vec<Sample>,
}

/// Read the tracks recorded beside `segments`. `None` when no segment has one,
/// e.g. a recording made with the system cursor or a retry after a failed render.
pub fn load(
    segments: &[PathBuf],
    origin: (f64, f64),
    ffmpeg: &Path,
) -> Result<Option<Logical>, String> {
    let tracks: Vec<Option<(Track, u64)>> = segments
        .iter()
        .map(|segment| {
            let track =
                fs::read_to_string(crate::platform::cursor_track::track_path(segment)).ok()?;
            let first = fs::read_to_string(first_frame_path(segment)).ok()?;
            Some((
                cursor::parse_track(&track).ok()?,
                parse_first_frame(&first)?,
            ))
        })
        .collect();
    if tracks.iter().all(Option::is_none) {
        return Ok(None);
    }
    let mut offset = 0.0;
    let mut placed = Vec::new();
    let empty = Track::default();
    for (segment, track) in segments.iter().zip(&tracks) {
        let duration_ms = probe(segment, ffmpeg)?.duration * 1000.0;
        let (track, first_frame_us) = track.as_ref().map_or((&empty, 0), |(t, us)| (t, *us));
        placed.push(Placed {
            track,
            first_frame_us,
            offset_ms: offset,
            duration_ms,
            mapping: Mapping { origin, scale: 1.0 },
        });
        offset += duration_ms;
    }
    Ok(Some(Logical {
        samples: cursor::timeline(&placed),
    }))
}

/// Remove the per-segment cursor files once the clip is final.
pub fn remove_segment_files(segments: &[PathBuf]) {
    for segment in segments {
        let _ = fs::remove_file(crate::platform::cursor_track::track_path(segment));
        let _ = fs::remove_file(first_frame_path(segment));
    }
}

/// gsr writes the first frame's timestamps to `<video>.ts`.
fn first_frame_path(segment: &Path) -> PathBuf {
    let mut name = segment.as_os_str().to_owned();
    name.push(".ts");
    PathBuf::from(name)
}

/// `monotonic_microsec realtime_microsec` header, then the values.
fn parse_first_frame(text: &str) -> Option<u64> {
    text.lines()
        .nth(1)?
        .split_ascii_whitespace()
        .next()?
        .parse()
        .ok()
}

pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}

pub fn probe(path: &Path, ffmpeg: &Path) -> Result<Probe, String> {
    let ffprobe = ffmpeg.with_file_name("ffprobe");
    let output = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height:format=duration",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("run {}: {error}", ffprobe.display()))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("ffprobe {}: missing {name}", path.display()))
    };
    Ok(Probe {
        width: field("width")? as u32,
        height: field("height")? as u32,
        duration: match field("duration") {
            Ok(duration) => duration,
            // A recorder killed after hanging on stop leaves no duration header.
            Err(_) => packet_duration(path, &ffprobe)?,
        },
    })
}

/// Duration from the last video packet plus one frame interval.
fn packet_duration(path: &Path, ffprobe: &Path) -> Result<f64, String> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("run {}: {error}", ffprobe.display()))?;
    let mut times: Vec<f64> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().trim_end_matches(',').parse().ok())
        .collect();
    times.sort_by(f64::total_cmp);
    match times.as_slice() {
        [.., previous, last] => Ok(last + (last - previous)),
        _ => Err(format!("ffprobe {}: no video packets", path.display())),
    }
}

/// Scale logical samples to clip pixels.
pub fn to_pixels(samples: &[Sample], factor: f64) -> Vec<Sample> {
    samples
        .iter()
        .map(|sample| match *sample {
            Sample::At { ms, x, y } => Sample::At {
                ms,
                x: x * factor,
                y: y * factor,
            },
            gone => gone,
        })
        .collect()
}

pub fn preset(mode: RecordCursor) -> Option<cursor::Preset> {
    match mode {
        RecordCursor::System => None,
        RecordCursor::Mellow => Some(cursor::MELLOW),
        RecordCursor::Quick => Some(cursor::QUICK),
    }
}

/// Render the smoothed cursor onto `clean`, writing `clip`, then place the clean
/// video and the raw track beside it. `samples` are clip pixels.
#[allow(clippy::too_many_arguments)]
pub fn render(
    clean: &Path,
    clip: &Path,
    samples: &[Sample],
    size: (u32, u32),
    duration: f64,
    fps: u32,
    mode: RecordCursor,
    scale: f64,
    codec: &str,
    ffmpeg: &Path,
) -> Result<(), String> {
    let preset = preset(mode).ok_or("system cursor needs no rendering")?;
    let frames = (duration * f64::from(fps)).round().max(1.0) as usize;
    let positions = cursor::smooth(samples, fps, frames, preset);
    let arrow = crate::platform::xcursor::arrow(scale);
    let hotspot = (f64::from(arrow.hotspot.0), f64::from(arrow.hotspot.1));
    // FFmpeg runs in the clip directory so filter options need no path escaping.
    let dir = clip.parent().ok_or("clip has no directory")?;
    let id = format!(
        "boltsnap-cursor-{}-{}",
        std::process::id(),
        RENDER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let (script_name, image_name) = (format!("{id}.cmd"), format!("{id}.png"));
    let (script_path, image_path) = (dir.join(&script_name), dir.join(&image_name));
    let result = (|| {
        fs::write(
            &script_path,
            cursor::overlay_script(&positions, fps, hotspot),
        )
        .map_err(|error| format!("write cursor script: {error}"))?;
        arrow
            .image
            .save(&image_path)
            .map_err(|error| format!("write cursor image: {error}"))?;
        super::finalize::run_ffmpeg_in(
            ffmpeg,
            &overlay_args(clean, &image_name, &script_name, fps, codec, clip),
            Some(dir),
        )?;
        super::finalize::require_nonempty(clip)
    })();
    let _ = fs::remove_file(&script_path);
    let png = fs::read(&image_path).ok();
    let _ = fs::remove_file(&image_path);
    if let Err(error) = result {
        let _ = fs::remove_file(clip);
        return Err(error);
    }
    let clean_name = clean_path(clip);
    fs::rename(clean, &clean_name).map_err(|error| format!("keep clean video: {error}"))?;
    let png = png.map(|bytes| base64(&bytes));
    let json = cursor::sidecar_json(
        samples,
        size,
        clean_name.file_name().and_then(|name| name.to_str()),
        png.as_deref().map(|png| (png, hotspot, scale)),
        mode.key(),
    );
    write_atomic(&sidecar_path(clip), json.to_string().as_bytes())
}

static RENDER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `image` and `script` are plain file names in FFmpeg's working directory.
fn overlay_args(
    clean: &Path,
    image: &str,
    script: &str,
    fps: u32,
    codec: &str,
    out: &Path,
) -> Vec<String> {
    let mut args = vec!["-y".to_owned()];
    let upload = super::finalize::hardware_device(codec);
    if let Some(device) = upload {
        args.extend(["-init_hw_device".into(), device.into()]);
        args.extend(["-filter_hw_device".into(), "hw".into()]);
    }
    args.extend(["-i".into(), clean.to_string_lossy().into_owned()]);
    args.extend([
        "-loop".into(),
        "1".into(),
        "-framerate".into(),
        fps.to_string(),
    ]);
    args.extend(["-i".into(), image.to_owned()]);
    let mut filter = format!(
        "[0:v]sendcmd=f={script}[base];[1:v]format=rgba[arrow];\
         [base][arrow]overlay@cursor=x=-100000:y=-100000:eval=frame:shortest=1"
    );
    if upload.is_some() {
        filter.push_str(",format=nv12,hwupload");
    }
    filter.push_str("[v]");
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
    args.extend(super::finalize::quality_args(codec));
    args.push(out.to_string_lossy().into_owned());
    args
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes)
        .and_then(|()| fs::rename(&temp, path))
        .map_err(|error| {
            let _ = fs::remove_file(&temp);
            format!("write {}: {error}", path.display())
        })
}

/// Move a clip's sidecars along with it; `clean_video` is rewritten to the new name.
pub fn move_sidecars(from: &Path, to: &Path) -> Result<(), String> {
    let json = sidecar_path(from);
    let Ok(text) = fs::read_to_string(&json) else {
        return Ok(());
    };
    let mut value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("read cursor track: {error}"))?;
    let clean_from = clean_path(from);
    if clean_from.exists() {
        let clean_to = clean_path(to);
        super::finalize::move_final_file(&clean_from, &clean_to)
            .map_err(|error| format!("move clean video: {error}"))?;
        value["clean_video"] = clean_to
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .into();
    }
    write_atomic(&sidecar_path(to), value.to_string().as_bytes())?;
    let _ = fs::remove_file(json);
    Ok(())
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                TABLE[(n >> shift & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn sidecar_names_follow_the_clip() {
        let clip = Path::new("/v/boltsnap-1-DP-3.mp4");
        assert_eq!(clean_path(clip), Path::new("/v/boltsnap-1-DP-3.clean.mp4"));
        assert_eq!(
            sidecar_path(clip),
            Path::new("/v/boltsnap-1-DP-3.cursor.json")
        );
    }

    #[test]
    fn first_frame_timestamp_uses_the_monotonic_column() {
        assert_eq!(
            parse_first_frame("monotonic_microsec\trealtime_microsec\n29630209157\t1790\n"),
            Some(29_630_209_157)
        );
        assert_eq!(parse_first_frame("header only\n"), None);
    }

    #[test]
    fn overlay_args_upload_for_gpu_encoders() {
        let args = overlay_args(
            Path::new("/c/clean.mp4"),
            "arrow.png",
            "s.cmd",
            240,
            "h264_vulkan",
            Path::new("/c/out.mp4"),
        );
        let joined = args.join(" ");
        assert!(
            joined.starts_with("-y -init_hw_device vulkan=hw -filter_hw_device hw -i /c/clean.mp4")
        );
        assert!(joined.contains("-loop 1 -framerate 240 -i arrow.png"));
        assert!(joined.contains("[0:v]sendcmd=f=s.cmd[base]"));
        assert!(joined.contains(
            "overlay@cursor=x=-100000:y=-100000:eval=frame:shortest=1,format=nv12,hwupload[v]"
        ));
        assert!(joined.ends_with("-map 0:a? -c:a copy -rc_mode cqp -qp 18 /c/out.mp4"));
        let cpu = overlay_args(
            Path::new("c.mp4"),
            "a.png",
            "s.cmd",
            60,
            "libx264",
            Path::new("o.mp4"),
        );
        assert!(
            !cpu.iter()
                .any(|arg| arg.contains("hwupload") || arg == "-init_hw_device")
        );
    }

    #[test]
    fn moving_sidecars_renames_the_clean_video_reference() {
        let dir = std::env::temp_dir().join(format!("boltsnap-sidecars-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("a.mp4");
        let to = dir.join("b.mp4");
        fs::write(clean_path(&from), b"clean").unwrap();
        fs::write(
            sidecar_path(&from),
            r#"{"format":"boltsnap.cursor","clean_video":"a.clean.mp4"}"#,
        )
        .unwrap();
        move_sidecars(&from, &to).unwrap();
        assert!(!sidecar_path(&from).exists() && !clean_path(&from).exists());
        assert_eq!(fs::read(clean_path(&to)).unwrap(), b"clean");
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(sidecar_path(&to)).unwrap()).unwrap();
        assert_eq!(value["clean_video"], "b.clean.mp4");
        move_sidecars(&dir.join("none.mp4"), &to).unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
