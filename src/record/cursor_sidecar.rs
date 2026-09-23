//! `X.cursor.json` for smooth-cursor clips: the raw pointer track in clip
//! pixels. The cursor itself is already in the video (drawn by the gsr plugin).

use super::cursor::{self, Mapping, Placed, Sample, Track, sidecar_path};
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

/// Write `X.cursor.json` beside `clip`: `samples` in clip pixels, the arrow the
/// plugin drew at `scale` clip pixels per logical unit.
pub fn write(
    clip: &Path,
    samples: &[Sample],
    size: (u32, u32),
    scale: f64,
    mode: RecordCursor,
) -> Result<(), String> {
    let arrow = super::cursor_motion::arrow(super::cursor_motion::cursor_size() * scale as f32);
    let mut png = Vec::new();
    image::RgbaImage::from_raw(arrow.width, arrow.height, arrow.rgba)
        .ok_or("cursor image size")?
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| format!("encode cursor image: {error}"))?;
    let json = cursor::sidecar_json(
        samples,
        size,
        Some((&base64(&png), arrow.hotspot, scale)),
        mode.key(),
    );
    write_atomic(&sidecar_path(clip), json.to_string().as_bytes())
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

/// Move a clip's cursor track along with it.
pub fn move_sidecars(from: &Path, to: &Path) -> Result<(), String> {
    let json = sidecar_path(from);
    if !json.exists() {
        return Ok(());
    }
    super::finalize::move_final_file(&json, &sidecar_path(to))
        .map_err(|error| format!("move cursor track: {error}"))
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
    fn written_track_moves_with_its_clip() {
        let dir = std::env::temp_dir().join(format!("boltsnap-sidecars-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("a.mp4");
        let to = dir.join("b.mp4");
        let samples = [Sample::At {
            ms: 0.0,
            x: 3.0,
            y: 4.0,
        }];
        write(&from, &samples, (1920, 1080), 1.5, RecordCursor::Mellow).unwrap();
        move_sidecars(&from, &to).unwrap();
        assert!(!sidecar_path(&from).exists());
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(sidecar_path(&to)).unwrap()).unwrap();
        assert_eq!(value["format"], "boltsnap.cursor");
        assert_eq!(value["cursor_in_video"], true);
        assert!(value.get("clean_video").is_none());
        assert_eq!(value["images"]["arrow"]["scale"], 1.5);
        assert_eq!(value["render"]["preset"], "mellow");
        move_sidecars(&dir.join("none.mp4"), &to).unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
