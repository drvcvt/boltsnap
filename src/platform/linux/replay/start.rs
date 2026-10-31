//! Starting a replay session: the capture, its worker and the wait until
//! the first frame decodes.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use boltsnap::replay::settings::Settings;
use serde_json::{Value, json};

use super::{Session, capture_log, cursor, encode, monitor, process, socket_call};

/// Above `/proc/sys/fs/pipe-max-size` (1 MiB by default) the call fails and
/// the pipe keeps its size.
const CAPTURE_PIPE_BYTES: libc::c_int = 1024 * 1024;

fn companion_path(name: &str) -> PathBuf {
    let adjacent = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(name)));
    adjacent
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

pub(super) fn start(settings: Settings, cancel: Arc<AtomicBool>) -> Result<Session, String> {
    let (ready, result) = std::sync::mpsc::sync_channel(1);
    let (parent_thread, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let session = start_capture(settings, &cancel).map(|mut session| {
            session.parent_thread = Some(parent_thread);
            session
        });
        let running = session.is_ok();
        if ready.send(session).is_ok() && running {
            // PR_SET_PDEATHSIG follows the thread that created the children.
            let _ = finished.recv();
        }
    });
    result.recv().map_err(|_| "replay startup thread exited")?
}

fn start_capture(settings: Settings, cancel: &AtomicBool) -> Result<Session, String> {
    let video_kbps = settings.video_kbps().to_string();
    let info = super::super::hypr::json("monitors").ok_or("could not query replay monitor")?;
    let monitors = crate::record::parse_hyprland_monitors(&info)?;
    let (target, notice) = monitor::replay_capture(
        settings.output.as_deref(),
        &crate::config::Config::load()
            .recording_prefs()
            .default_target,
        &monitors,
    )?;
    let worker = companion_path("boltsnap-replay-worker");
    let capabilities = process::output(
        Command::new(&worker).arg("capabilities"),
        Duration::from_secs(3),
    )
    .map_err(|e| {
        format!("replay worker missing or unavailable; install boltsnap-replay-worker: {e}")
    })?;
    if !capabilities.status.success() {
        return Err("replay worker could not load its media libraries".into());
    }
    let capabilities: Value =
        serde_json::from_slice(&capabilities.stdout).map_err(|e| e.to_string())?;
    let encoders = capabilities["video_encoders"]
        .as_array()
        .ok_or("invalid encoder capabilities")?;
    // gsr ships its own FFmpeg, so it is asked what it can capture with. The
    // system FFmpeg only encodes region clips; probing it for the capture once
    // sent replay to Vulkan when FFmpeg had been built without NVENC.
    let gsr = super::super::gsr::info()?;
    let capture = super::super::gsr::choose(&settings.encoder, &gsr)?;
    if capture.cpu {
        return Err("replay needs a hardware encoder; set replay encoder to one".into());
    }
    let capture_codec = capture.name.as_str();
    let mut candidates = vec![capture.encoder.as_str()];
    if settings.encoder == "auto" {
        candidates.extend(["h264_nvenc", "h264_vaapi", "h264_vulkan"]);
    }
    candidates.dedup();
    let mut encoder = None;
    let mut errors = Vec::new();
    for candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Err("replay start cancelled".into());
        }
        if !encoders.iter().any(|e| e == candidate) {
            continue;
        }
        let probe = process::output(
            Command::new(&worker).args(["check-encoder", candidate]),
            Duration::from_secs(10),
        )?;
        if probe.status.success() {
            encoder = Some(candidate.to_owned());
            break;
        }
        errors.push(
            String::from_utf8_lossy(&probe.stderr)
                .chars()
                .take(512)
                .collect::<String>(),
        );
    }
    // Replay itself works without one; only region clips report the gap.
    let encoder = encoder.unwrap_or_else(|| {
        eprintln!(
            "boltsnap replay: no FFmpeg hardware encoder for region clips: {}",
            errors.join("; ")
        );
        boltsnap::replay::crop::NO_ENCODER.into()
    });
    let directory = crate::paths::rec_dir();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    crate::record::finalize::check_recording_cache_limit(&directory)?;
    crate::record::finalize::check_recording_reserve(&directory)?;
    let mut command = Command::new(companion_path("gpu-screen-recorder"));
    command
        .args(["-w", &target.source()])
        .args([
            "-f",
            &settings.fps.to_string(),
            "-fm",
            "cfr",
            "-k",
            capture_codec,
            // CBR sets the cap: gsr makes the buffer-sized rate maxrate and
            // bufsize. Plain constant quality would grow with motion and
            // silently shorten the clip once the byte budget evicts;
            // `encode::video_opts` turns NVENC into target quality under
            // that cap.
            "-bm",
            "cbr",
            "-q",
            &video_kbps,
            "-tune",
            "performance",
            "-keyint",
            "1",
            "-a",
            "default_output",
            "-ac",
            "aac",
            "-ab",
            "128",
            "-c",
            "nut",
            "-o",
            "/dev/stdout",
            "-fallback-cpu-encoding",
            "no",
            "-ffmpeg-opts",
            "write_index=0;syncpoints=none;strict=experimental;flush_packets=1",
            "-ffmpeg-video-opts",
            &encode::video_opts(&capture.encoder),
        ])
        .envs(encode::env(&capture.encoder).iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Through `capture_log`: inheriting the daemon's stderr would send
        // GPU Screen Recorder's diagnostics to `/dev/null`.
        .stderr(Stdio::piped());
    let cursor = crate::config::Config::load().recording_prefs().cursor;
    let smooth = cursor::smooth(&mut command, cursor, &target)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("replay start cancelled".into());
    }
    let mut capture = process::spawn(&mut command, None)
        .map_err(|e| format!("GPU Screen Recorder unavailable: {e}"))?;
    // gsr holds its own copy of the feed now.
    let cursor = smooth.map_or_else(Vec::new, |(trackers, _feeds)| trackers);
    // One session's log in `replay-capture.log`, the one before in `.old`,
    // each line with its time.
    if let Some(stderr) = capture.stderr.take() {
        let fps = Some(settings.fps);
        match capture_log::open(&crate::paths::cache_dir()) {
            Some(log) => capture_log::follow(stderr, log, fps),
            None => capture_log::follow(stderr, std::io::sink(), fps),
        }
    }
    let input = capture.stdout.take().ok_or("capture pipe missing")?;
    // 64 KiB holds a few ms of video; a keyframe alone is larger. Room for
    // ~100 ms keeps a worker held up by a busy CPU from stalling gsr's encoder.
    unsafe {
        use std::os::fd::AsRawFd;
        libc::fcntl(input.as_raw_fd(), libc::F_SETPIPE_SZ, CAPTURE_PIPE_BYTES);
    }
    let name = format!(
        "worker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let child = process::spawn(
        Command::new(worker)
            .arg("live")
            .arg(&name)
            .arg(directory)
            .arg(settings.seconds.to_string())
            .arg(settings.memory_mib.to_string())
            .arg(&encoder)
            .stdin(Stdio::from(input))
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
        None,
    );
    let worker = match child {
        Ok(mut worker) => {
            // Into the daemon's log, with the time.
            if let Some(stderr) = worker.stderr.take() {
                capture_log::follow(stderr, std::io::stderr(), None);
            }
            worker
        }
        Err(e) => {
            process::terminate(&mut capture);
            return Err(format!("start replay worker: {e}"));
        }
    };
    let mut session = Session {
        capture,
        worker,
        cursor,
        name,
        output: target.output(),
        layout: target.layout(),
        asked: settings.output.clone(),
        output_limit: (settings.memory_mib * 1024 * 1024 / 3) as u64,
        parent_thread: None,
        notice,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return Err("replay start cancelled".into());
        }
        if let Some(exit) = session.capture.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("replay capture failed: {exit}"));
        }
        if let Some(exit) = session.worker.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("replay worker failed: {exit}"));
        }
        if let Ok((status, _)) =
            socket_call(&session.name, &json!({"version":1,"command":"status"}))
        {
            if status["ready"] == true {
                return Ok(session);
            }
            if let Some(error) = status["error"].as_str() {
                return Err(error.into());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("replay did not receive a decodable video frame within sixty seconds".into())
}
