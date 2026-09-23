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

/// What to decode and where the cursor-free reference comes from.
pub struct Job<'a> {
    /// FFmpeg input arguments that yield one video stream, e.g. `-i clean.mp4`
    /// or several inputs plus a composing filter and `-map`.
    pub decode: Vec<String>,
    /// File whose first audio stream is copied into the outputs.
    pub audio: &'a Path,
    pub size: (u32, u32),
    pub clip: &'a Path,
    /// `Some` renames an existing cursor-free video to the clean sidecar;
    /// `None` encodes the decoded frames a second time as the clean sidecar.
    pub existing_clean: Option<&'a Path>,
}

/// Draw the smoothed cursor into every frame and write `clip` plus its clean
/// video and raw track. Frames go through pipes as yuv420p, so the cost is
/// linear in the clip length. `samples` are clip pixels.
#[allow(clippy::too_many_arguments)]
pub fn render(
    job: Job,
    samples: &[Sample],
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
    let color = probe_color(job.existing_clean.unwrap_or(job.audio), ffmpeg);
    let sprite = cursor::Sprite::new(
        arrow.image.as_raw(),
        arrow.image.width() as usize,
        arrow.image.height() as usize,
        color.bt709,
        color.full,
    );
    let clean_out = clean_path(job.clip);
    let encode_clean = job.existing_clean.is_none().then_some(clean_out.as_path());
    let result = pipe(
        &job,
        encode_clean,
        fps,
        codec,
        &color,
        ffmpeg,
        |index, frame| {
            let position = positions.get(index).copied().flatten();
            if let Some((x, y)) = position {
                cursor::blend_yuv420(
                    frame,
                    job.size.0 as usize,
                    job.size.1 as usize,
                    &sprite,
                    (x - hotspot.0).round() as i64,
                    (y - hotspot.1).round() as i64,
                );
            }
        },
    );
    if let Err(error) = result {
        let _ = fs::remove_file(job.clip);
        if encode_clean.is_some() {
            let _ = fs::remove_file(&clean_out);
        }
        return Err(error);
    }
    if let Some(existing) = job.existing_clean {
        fs::rename(existing, &clean_out).map_err(|error| format!("keep clean video: {error}"))?;
    }
    let mut png = Vec::new();
    arrow
        .image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| format!("encode cursor image: {error}"))?;
    let json = cursor::sidecar_json(
        samples,
        job.size,
        clean_out.file_name().and_then(|name| name.to_str()),
        Some((&base64(&png), hotspot, scale)),
        mode.key(),
    );
    write_atomic(&sidecar_path(job.clip), json.to_string().as_bytes())
}

struct Color {
    bt709: bool,
    full: bool,
    /// Output tags, so players treat the result like the source.
    tags: Vec<String>,
}

fn probe_color(path: &Path, ffmpeg: &Path) -> Color {
    let text = Command::new(ffmpeg.with_file_name("ffprobe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=color_range,color_space",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("unknown")
            .to_owned()
    };
    let full = value("color_range") == "pc";
    let bt709 = !matches!(value("color_space").as_str(), "bt470bg" | "smpte170m");
    let (space, range) = (
        if bt709 { "bt709" } else { "smpte170m" },
        if full { "pc" } else { "tv" },
    );
    Color {
        bt709,
        full,
        tags: [
            "-colorspace",
            space,
            "-color_primaries",
            space,
            "-color_trc",
            space,
            "-color_range",
            range,
        ]
        .map(str::to_owned)
        .to_vec(),
    }
}

/// Decode to raw frames, let `draw` modify each, encode the result (and the
/// untouched frames when `clean` is set).
#[allow(clippy::too_many_arguments)]
fn pipe(
    job: &Job,
    clean: Option<&Path>,
    fps: u32,
    codec: &str,
    color: &Color,
    ffmpeg: &Path,
    mut draw: impl FnMut(usize, &mut [u8]),
) -> Result<(), String> {
    use std::io::Read;
    use std::process::{Child, Stdio};
    let (width, height) = (job.size.0 as usize, job.size.1 as usize);
    let log = |name: &str| job.clip.with_extension(format!("{name}.log"));
    let spawn = |args: Vec<String>, name: &str, input: bool| -> Result<Child, String> {
        let log_file = fs::File::create(log(name)).map_err(|e| format!("create log: {e}"))?;
        Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(args)
            .stdin(if input { Stdio::piped() } else { Stdio::null() })
            .stdout(if input { Stdio::null() } else { Stdio::piped() })
            .stderr(log_file)
            .spawn()
            .map_err(|error| format!("start ffmpeg: {error}"))
    };
    let mut decode_args = job.decode.clone();
    decode_args.extend(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"].map(str::to_owned));
    let mut decoder = spawn(decode_args, "decode", false)?;
    let mut encoders = vec![(
        spawn(
            encode_args(job, job.clip, fps, codec, color),
            "encode",
            true,
        )?,
        true,
    )];
    if let Some(clean) = clean {
        encoders.push((
            spawn(encode_args(job, clean, fps, codec, color), "clean", true)?,
            false,
        ));
    }
    let source = decoder.stdout.take().ok_or("decoder output missing")?;
    let sinks = encoders
        .iter_mut()
        .map(|(child, drawn)| Ok((child.stdin.take().ok_or("encoder input missing")?, *drawn)))
        .collect::<Result<Vec<_>, String>>()?;
    let frame_len = cursor::yuv420_len(width, height);
    // Decoder, drawing and encoders overlap through small frame queues; a
    // single thread alternating 3 MB reads and writes over 64 KB pipes kept
    // each ffmpeg waiting on the other.
    let mut index = 0;
    let streamed = std::thread::scope(|scope| -> Result<(), String> {
        let (frames_tx, frames) = std::sync::mpsc::sync_channel::<Vec<u8>>(2);
        let reader = scope.spawn(move || -> Result<(), String> {
            let mut source = source;
            enlarge_pipe(&source);
            loop {
                let mut frame = vec![0u8; frame_len];
                match source.read_exact(&mut frame) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                        return Ok(());
                    }
                    Err(error) => return Err(format!("read decoded frames: {error}")),
                }
                if frames_tx.send(frame).is_err() {
                    return Ok(());
                }
            }
        });
        let writers: Vec<_> = sinks
            .into_iter()
            .map(|(sink, drawn)| {
                let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(2);
                let writer = scope.spawn(move || -> Result<(), String> {
                    use std::io::Write;
                    let mut sink = sink;
                    enlarge_pipe(&sink);
                    for frame in rx {
                        sink.write_all(&frame)
                            .map_err(|error| format!("write frames to encoder: {error}"))?;
                    }
                    Ok(())
                });
                (tx, drawn, writer)
            })
            .collect();
        let mut result = Ok(());
        'frames: for mut frame in frames {
            for (tx, drawn, _) in &writers {
                if !*drawn && tx.send(frame.clone()).is_err() {
                    break 'frames;
                }
            }
            draw(index, &mut frame);
            index += 1;
            let mut frame = Some(frame);
            for (tx, drawn, _) in &writers {
                if *drawn && tx.send(frame.take().unwrap_or_default()).is_err() {
                    break 'frames;
                }
            }
        }
        for (tx, _, writer) in writers {
            drop(tx);
            if let Err(error) = writer
                .join()
                .unwrap_or(Err("encoder writer panicked".into()))
            {
                result = result.and(Err(error));
            }
        }
        let read = reader.join().unwrap_or(Err("frame reader panicked".into()));
        result.and(read)
    });
    if streamed.is_err() {
        let _ = decoder.kill();
    }
    let mut failures = Vec::new();
    for (name, child) in std::iter::once(("decode", &mut decoder)).chain(
        encoders
            .iter_mut()
            .enumerate()
            .map(|(i, (child, _))| (if i == 0 { "encode" } else { "clean" }, child)),
    ) {
        match child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                let detail = fs::read_to_string(log(name)).unwrap_or_default();
                failures.push(format!(
                    "ffmpeg {name} exited with {status}: {}",
                    detail.trim()
                ));
            }
            Err(error) => failures.push(format!("wait for ffmpeg {name}: {error}")),
        }
        let _ = fs::remove_file(log(name));
    }
    streamed?;
    if index == 0 {
        failures.push("no frames decoded".into());
    }
    if !failures.is_empty() {
        return Err(failures.join("; "));
    }
    super::finalize::require_nonempty(job.clip)?;
    clean.map_or(Ok(()), super::finalize::require_nonempty)
}

/// Raise a pipe's capacity to 1 MiB (the default unprivileged maximum) so each
/// side can run ahead of the other. Failure keeps the 64 KiB default.
fn enlarge_pipe(pipe: &impl std::os::fd::AsRawFd) {
    unsafe {
        libc::fcntl(pipe.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20);
    }
}

fn encode_args(job: &Job, out: &Path, fps: u32, codec: &str, color: &Color) -> Vec<String> {
    let mut args = vec!["-y".to_owned()];
    let upload = super::finalize::hardware_device(codec);
    if let Some(device) = upload {
        args.extend(["-init_hw_device".into(), device.into()]);
        args.extend(["-filter_hw_device".into(), "hw".into()]);
    }
    args.extend(
        [
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-video_size",
            &format!("{}x{}", job.size.0, job.size.1),
            "-framerate",
            &fps.to_string(),
            "-i",
            "-",
            "-i",
        ]
        .map(str::to_owned),
    );
    args.push(job.audio.to_string_lossy().into_owned());
    args.extend(["-map", "0:v", "-map", "1:a?", "-c:a", "copy"].map(str::to_owned));
    if upload.is_some() {
        args.extend(["-vf".into(), "format=nv12,hwupload".into()]);
    }
    args.extend(["-c:v".into(), codec.into()]);
    args.extend(super::finalize::quality_args(codec));
    args.extend(color.tags.iter().cloned());
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
    fn encoder_reads_raw_frames_copies_audio_and_uploads_for_gpu_codecs() {
        let clip = Path::new("/c/out.mp4");
        let job = Job {
            decode: Vec::new(),
            audio: Path::new("/c/clean.mp4"),
            size: (1920, 1080),
            clip,
            existing_clean: None,
        };
        let color = Color {
            bt709: true,
            full: false,
            tags: vec!["-color_range".into(), "tv".into()],
        };
        let joined = encode_args(&job, clip, 240, "h264_vulkan", &color).join(" ");
        assert!(joined.starts_with("-y -init_hw_device vulkan=hw -filter_hw_device hw -f rawvideo -pix_fmt yuv420p -video_size 1920x1080 -framerate 240 -i - -i /c/clean.mp4"));
        assert!(joined.contains("-map 0:v -map 1:a? -c:a copy -vf format=nv12,hwupload -c:v h264_vulkan -rc_mode cqp -qp 18 -color_range tv /c/out.mp4"));
        let cpu = encode_args(&job, clip, 60, "libx264", &color);
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

    /// Renders a synthetic clip; `BOLTSNAP_RENDER_TEST="WxH SECONDS FPS CODEC"`.
    #[test]
    #[ignore = "encodes video; set BOLTSNAP_RENDER_TEST"]
    fn synthetic_render_is_linear_and_places_the_cursor() {
        let spec = std::env::var("BOLTSNAP_RENDER_TEST").unwrap();
        let [size, seconds, fps, codec]: [&str; 4] =
            spec.split(' ').collect::<Vec<_>>().try_into().unwrap();
        let (w, h) = size.split_once('x').unwrap();
        let (w, h): (u32, u32) = (w.parse().unwrap(), h.parse().unwrap());
        let (seconds, fps): (f64, u32) = (seconds.parse().unwrap(), fps.parse().unwrap());
        let dir = std::env::temp_dir().join(format!("boltsnap-render-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let clean = dir.join("clean-in.mp4");
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("color=c=0x303030:s={w}x{h}:r={fps}:d={seconds}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=d={seconds}"))
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
            ])
            .args(["-color_range", "tv", "-colorspace", "bt709", "-c:a", "aac"])
            .arg(&clean)
            .status()
            .unwrap();
        assert!(status.success());
        // Cursor moves right 1 px per frame; the spring lags, so check a frame
        // after it stopped moving.
        let frames = (seconds * f64::from(fps)) as usize;
        let mut samples: Vec<Sample> = (0..frames / 2)
            .map(|i| Sample::At {
                ms: i as f64 * 1000.0 / f64::from(fps),
                x: 100.0 + i as f64 % f64::from(w - 200),
                y: f64::from(h) / 2.0,
            })
            .collect();
        samples.push(Sample::At {
            ms: frames as f64 / 2.0 * 1000.0 / f64::from(fps),
            x: 200.0,
            y: 100.0,
        });
        let clip = dir.join("clip.mp4");
        let started = std::time::Instant::now();
        render(
            Job {
                decode: ["-i", &clean.to_string_lossy(), "-map", "0:v:0"]
                    .map(str::to_owned)
                    .to_vec(),
                audio: &clean,
                size: (w, h),
                clip: &clip,
                existing_clean: Some(&clean),
            },
            &samples,
            seconds,
            fps,
            RecordCursor::Quick,
            1.0,
            codec,
            Path::new("ffmpeg"),
        )
        .unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        println!(
            "rendered {frames} frames {w}x{h} with {codec} in {elapsed:.2}s ({:.0} fps)",
            frames as f64 / elapsed
        );
        let probe_clip = probe(&clip, Path::new("ffmpeg")).unwrap();
        assert_eq!((probe_clip.width, probe_clip.height), (w, h));
        assert!(
            (probe_clip.duration - seconds).abs() < 0.1,
            "{}",
            probe_clip.duration
        );
        // Last frame: the arrow's hotspot sits at (200, 100) after settling.
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-sseof", "-0.1", "-i"])
            .arg(&clip)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .output()
            .unwrap();
        let luma = |x: u32, y: u32| out.stdout[(y * w + x) as usize];
        let arrow = crate::platform::xcursor::arrow(1.0);
        let (hx, hy) = arrow.hotspot;
        let lit = (0..arrow.image.height())
            .flat_map(|y| (0..arrow.image.width()).map(move |x| (x, y)))
            .filter(|(x, y)| arrow.image.get_pixel(*x, *y)[3] == 255)
            .filter(|(x, y)| {
                let (fx, fy) = (200 - hx + x, 100 - hy + y);
                luma(fx, fy).abs_diff(0x30) > 20
            })
            .count();
        assert!(lit > 20, "cursor pixels found: {lit}");
        assert!(luma(w - 5, h - 5).abs_diff(0x30) < 6);
        assert!(clean_path(&clip).exists() && sidecar_path(&clip).exists());
        let _ = fs::remove_dir_all(dir);
    }
}
