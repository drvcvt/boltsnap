use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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
) -> Result<u64, String> {
    let source = source_file(recording, limit)?;
    let partial = path.with_extension("partial.mkv");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)
        .map_err(|e| format!("create crop output: {e}"))?;
    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-threads",
        "2",
    ]);
    let upload = device(&mut command, encoder);
    let mut filter = format!(
        "crop={}:{}:{}:{},setsar=1,format=nv12",
        crop.width, crop.height, crop.x, crop.y
    );
    if upload {
        filter.push_str(",hwupload");
    }
    command.args([
        "-i",
        "pipe:0",
        "-map",
        "0:v:0",
        "-map",
        "0:a:0?",
        "-vf",
        &filter,
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
    command
        .args(["-c:a", "copy", "-f", "matroska", "pipe:1"])
        .stdin(Stdio::from(source))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = crate::process::spawn(&mut command, Some(19))
        .map_err(|e| format!("start crop encoder: {e}"))?;
    let mut encoded = child.stdout.take().ok_or("crop output pipe missing")?;
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
    let copied = (|| {
        let mut bytes = [0; 64 * 1024];
        let mut total = 0_u64;
        loop {
            let count = encoded.read(&mut bytes).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or("crop size overflow")?;
            if total > limit {
                return Err("crop output exceeds its disk budget".into());
            }
            output
                .write_all(&bytes[..count])
                .map_err(|e| format!("write crop: {e}"))?;
        }
        output.sync_all().map_err(|e| format!("sync crop: {e}"))?;
        Ok(total)
    })();
    if copied.is_err() {
        let _ = child.lock().unwrap().kill();
    }
    let status = loop {
        match child.lock().unwrap().try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => break Err(e.to_string()),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let _ = done.send(());
    let _ = watcher.join();
    let bytes =
        copied.map_err(|e: String| format!("{e}; partial output: {}", partial.display()))?;
    if !status?.success() || bytes == 0 {
        return Err(format!(
            "crop encoder failed; partial output: {}",
            partial.display()
        ));
    }
    crate::export::publish(&partial, path)?;
    Ok(bytes)
}
