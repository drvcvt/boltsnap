use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::os::fd::FromRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::media::Recording;
use crate::replay::crop::Crop;

fn device(command: &mut Command, encoder: &str) -> bool {
    let kind = if encoder.ends_with("_vulkan") {
        Some("vulkan=clip")
    } else if encoder.ends_with("_vaapi") {
        Some("vaapi=clip")
    } else {
        None
    };
    if let Some(kind) = kind {
        command.args(["-init_hw_device", kind, "-filter_hw_device", "clip"]);
    }
    kind.is_some()
}

/// Where a region clip is cut out.
enum Cut {
    /// In NVDEC, with its decoder for the buffer's video and the `-crop`
    /// edges (top x bottom x left x right).
    Decoder(&'static str, String),
    /// In a CPU filter after software decoding.
    Filter(Crop),
}

/// The cut in NVDEC when NVENC encodes: the frames stay on the GPU from
/// decoder to encoder, 2.8 times faster than decoding in software (replay
/// plan §13 C2). Only on 4:2:0 boundaries: an odd edge swaps U and V there.
fn gpu_crop(recording: &Recording, encoder: &str, crop: Crop) -> Option<Cut> {
    if !encoder.ends_with("_nvenc") {
        return None;
    }
    let decoder = match recording
        .streams
        .iter()
        .find(|s| s.parameters.medium() == ffmpeg_next::media::Type::Video)?
        .parameters
        .id()
    {
        ffmpeg_next::codec::Id::H264 => "h264_cuvid",
        ffmpeg_next::codec::Id::HEVC => "hevc_cuvid",
        ffmpeg_next::codec::Id::AV1 => "av1_cuvid",
        _ => return None,
    };
    let edges = nvdec_edges(crop, recording.dimensions().ok()?)?;
    Some(Cut::Decoder(decoder, edges))
}

/// cuvid's `-crop` for `crop` in a `width` x `height` video, when every edge
/// is even.
fn nvdec_edges(crop: Crop, (width, height): (u32, u32)) -> Option<String> {
    let bottom = height.checked_sub(crop.y.checked_add(crop.height)?)?;
    let right = width.checked_sub(crop.x.checked_add(crop.width)?)?;
    let [top, left] = [crop.y, crop.x];
    [top, bottom, left, right]
        .iter()
        .all(|edge| edge % 2 == 0)
        .then(|| format!("{top}x{bottom}x{left}x{right}"))
}

pub fn check_encoder(encoder: &str) -> Result<(), String> {
    let mut command = Command::new("ffmpeg");
    command.args(["-v", "error", "-nostdin"]);
    let upload = device(&mut command, encoder);
    command.args([
        "-f",
        "lavfi",
        "-i",
        "color=size=320x180:rate=60",
        "-vf",
        if upload {
            "format=nv12,hwupload"
        } else {
            "format=nv12"
        },
        "-frames:v",
        "1",
        "-c:v",
        encoder,
        "-bf",
        "0",
        "-f",
        "null",
        "-",
    ]);
    let result = crate::process::output(&mut command, Duration::from_secs(8))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{encoder} cannot initialize: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ))
    }
}

pub fn preview(recording: &Recording) -> Result<Vec<u8>, String> {
    let source = source_file(recording, recording.ring.bytes() as u64 + 65536)?;
    let frame = recording
        .ring
        .video()
        .count()
        .checked_sub(1)
        .ok_or("empty preview")?;
    let filter = format!("select=eq(n\\,{frame})");
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-v",
            "error",
            "-nostdin",
            "-threads",
            "2",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-vf",
            &filter,
            "-frames:v",
            "1",
            "-threads:v",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::from(source));
    let result = crate::process::output_bounded(
        &mut command,
        Duration::from_secs(10),
        crate::replay::wire::MAX_PREVIEW,
        // The selector waits for this frame.
        None,
    )?;
    if !result.status.success() || result.stdout.is_empty() {
        return Err("could not decode replay preview".into());
    }
    Ok(result.stdout)
}

fn source_file(recording: &Recording, limit: u64) -> Result<File, String> {
    let fd = unsafe { libc::memfd_create(c"boltsnap-clip".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "create clip source: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut source = unsafe { File::from_raw_fd(fd) };
    crate::export::to_file(
        recording,
        source.try_clone().map_err(|e| e.to_string())?,
        limit,
    )?;
    source.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(source)
}

pub fn save(
    recording: &Recording,
    crop: Crop,
    encoder: &str,
    path: &Path,
    limit: u64,
) -> Result<crate::export::Written, String> {
    if encoder == crate::replay::crop::NO_ENCODER {
        return Err(
            "region clips need a hardware encoder in FFmpeg (h264_nvenc, h264_vaapi or h264_vulkan)"
                .into(),
        );
    }
    let source = source_file(recording, limit)?;
    let partial = path.with_extension("partial.mp4");
    // Reserve the private file; ffmpeg reopens it by path because MP4 needs
    // to seek back for its index.
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)
        .map_err(|e| format!("create crop output: {e}"))?;
    let gpu = gpu_crop(recording, encoder, crop);
    let retry = match gpu {
        Some(_) => Some((
            source.try_clone().map_err(|e| e.to_string())?,
            output.try_clone().map_err(|e| e.to_string())?,
        )),
        None => None,
    };
    let cut = gpu.unwrap_or(Cut::Filter(crop));
    let mut written = encode(recording, source, output, &partial, encoder, cut, limit);
    // The GPU path failed before it wrote anything (no NVDEC for this codec,
    // a driver error): once more in software, same budget and time limit.
    if let (Err(error), Some((mut source, output))) = (&written, retry)
        && output.metadata().is_ok_and(|m| m.len() == 0)
    {
        eprintln!("boltsnap-replay-worker: GPU crop failed ({error}), cropping in software");
        written = source
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())
            .and_then(|_| {
                encode(
                    recording,
                    source,
                    output,
                    &partial,
                    encoder,
                    Cut::Filter(crop),
                    limit,
                )
            });
    }
    // A failed clip must not keep a full disk full; a finished one that only
    // could not take its name stays.
    let written = written.inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })?;
    crate::export::publish(&partial, path)
        .map_err(|e| format!("{e}; finished clip kept at {}", partial.display()))?;
    Ok(written)
}

fn encode(
    recording: &Recording,
    source: File,
    output: File,
    partial: &Path,
    encoder: &str,
    cut: Cut,
    limit: u64,
) -> Result<crate::export::Written, String> {
    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-threads",
        "2",
    ]);
    let filter = match &cut {
        Cut::Decoder(decoder, edges) => {
            command.args([
                "-hwaccel",
                "cuda",
                "-hwaccel_output_format",
                "cuda",
                "-c:v",
                decoder,
                "-crop",
                edges,
            ]);
            None
        }
        Cut::Filter(crop) => {
            let upload = device(&mut command, encoder);
            let mut chain = format!(
                "crop={}:{}:{}:{},setsar=1,format=nv12",
                crop.width, crop.height, crop.x, crop.y
            );
            if upload {
                chain.push_str(",hwupload");
            }
            Some(chain)
        }
    };
    command.args(["-i", "pipe:0", "-map", "0:v:0", "-map", "0:a:0?"]);
    if let Some(filter) = &filter {
        command.args(["-vf", filter]);
    }
    command.args([
        "-filter_threads",
        "1",
        "-c:v",
        encoder,
        "-threads:v",
        "2",
        "-bf",
        "0",
    ]);
    if encoder.ends_with("_vulkan") || encoder.ends_with("_nvenc") {
        command.args(["-qp", "18"]);
    } else if encoder.ends_with("_vaapi") {
        command.args(["-global_quality", "18"]);
    } else if encoder == "libx264" || encoder == "libx265" {
        command.args(["-preset", "veryfast", "-crf", "18"]);
    }
    // `file:` keeps a colon in the path from selecting another protocol.
    let mut target = std::ffi::OsString::from("file:");
    target.push(partial);
    command
        .args(["-c:a", "copy", "-f", "mp4", "-y"])
        .arg(target)
        .stdin(Stdio::from(source))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let child = crate::process::spawn(&mut command, Some(19))
        .map_err(|e| format!("start crop encoder: {e}"))?;
    let child = Arc::new(Mutex::new(child));
    let watchdog = child.clone();
    let seconds = recording
        .ring
        .bounds()
        .map(|(s, e)| (e - s) as u64 / 1_000_000)?;
    let deadline = Instant::now() + Duration::from_secs(seconds * 3 + 30);
    let (done, stopped) = std::sync::mpsc::channel();
    let watcher = std::thread::spawn(move || {
        if stopped
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_err()
        {
            let _ = watchdog.lock().unwrap().kill();
        }
    });
    let mut exceeded = false;
    let status = loop {
        let mut child = child.lock().unwrap();
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => break Err(e.to_string()),
            Ok(None) => {
                if !exceeded && output.metadata().is_ok_and(|m| m.len() > limit) {
                    exceeded = true;
                    let _ = child.kill();
                }
            }
        }
        drop(child);
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = done.send(());
    let _ = watcher.join();
    if exceeded {
        return Err("crop output exceeds its disk budget".into());
    }
    if !status?.success() {
        return Err("crop encoder failed".into());
    }
    let sync_ms = crate::export::sync(&output, "crop")?;
    let bytes = output
        .metadata()
        .map_err(|e| format!("crop metadata: {e}"))?
        .len();
    if bytes > limit {
        return Err("crop output exceeds its disk budget".into());
    }
    if bytes == 0 {
        return Err("empty crop output".into());
    }
    Ok(crate::export::Written { bytes, sync_ms })
}

#[cfg(test)]
mod tests {
    use super::{Crop, nvdec_edges};

    #[test]
    fn nvdec_crops_only_on_even_edges_inside_the_video() {
        let crop = |x, y, width, height| Crop {
            x,
            y,
            width,
            height,
        };
        // The measured clip: 1450x942 out of 1920x1080 (replay plan §13).
        assert_eq!(
            nvdec_edges(crop(236, 60, 1450, 942), (1920, 1080)).as_deref(),
            Some("60x78x236x234")
        );
        // An odd edge would swap U and V; outside the video is no crop.
        assert_eq!(nvdec_edges(crop(235, 60, 1450, 942), (1920, 1080)), None);
        assert_eq!(nvdec_edges(crop(236, 60, 1450, 942), (1921, 1080)), None);
        assert_eq!(nvdec_edges(crop(600, 0, 1450, 942), (1920, 1080)), None);
    }
}
