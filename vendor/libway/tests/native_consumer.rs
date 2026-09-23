#![cfg(feature = "gpu")]
mod support;
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use support::*;
#[test]
#[ignore = "requires LIBWAY_TEST_NATIVE_WORKER and one DRM node; synthetic compositor only"]
fn native_recording_uses_separate_video_and_cursor_connections() {
    let worker = std::env::var_os("LIBWAY_TEST_NATIVE_WORKER").expect("explicit worker");
    let dir = std::env::temp_dir().join(format!(
        "libway-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let server = Server::listen(
        Config {
            cursor: Some(CursorFault::IdleImage),
            dma: true,
            fault: Fault::IdleSecond,
            mode_sizes: vec![(320, 180)],
            native_logical_size: true,
            ..Default::default()
        },
        &dir.join("display"),
    );
    let log = std::fs::File::create(dir.join("worker.log")).unwrap();
    let mut child = Command::new(worker)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("XDG_RUNTIME_DIR", &dir)
        .env("WAYLAND_DISPLAY", "display")
        .args(["cursor-record-fixture", "TEST-0", "60", "libx264", "-"])
        .arg(dir.join("recording.mp4"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(8) && !dir.join("recording.mp4").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "worker exited {status}: {}",
                std::fs::read_to_string(dir.join("worker.log")).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_secs(2));
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let stop = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if stop.elapsed() > Duration::from_secs(12) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("worker stop timed out: {}", dir.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let log = std::fs::read_to_string(dir.join("worker.log")).unwrap();
    assert!(status.success(), "{log}");
    let result = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-select_streams",
            "v:0",
            "-of",
            "csv=p=0",
        ])
        .arg(dir.join("recording.mp4"))
        .output()
        .unwrap();
    assert!(result.status.success(), "{log}");
    let frames = String::from_utf8(result.stdout)
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap();
    assert!(frames >= 30, "{frames} frames: {log}");
    // Validate both stream durations and the initial packet cadence. This catches
    // MP4 empty_moov stretching the first video frame when audio starts later.
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,duration",
            "-of",
            "csv=p=0",
        ])
        .arg(dir.join("recording.mp4"))
        .output()
        .unwrap();
    assert!(probe.status.success());
    let streams = String::from_utf8(probe.stdout).unwrap();
    let duration = |kind: &str| -> f64 {
        streams
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{kind},")))
            .expect("audio and video required")
            .parse()
            .unwrap()
    };
    assert!(
        (duration("video") - duration("audio")).abs() < 0.06,
        "{streams}"
    );
    let probe = Command::new("ffprobe")
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
        .arg(dir.join("recording.mp4"))
        .output()
        .unwrap();
    assert!(probe.status.success());
    let times = String::from_utf8(probe.stdout)
        .unwrap()
        .lines()
        .map(|line| line.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    for pair in times.windows(2) {
        assert!((pair[1] - pair[0] - 1.0 / 60.0).abs() < 0.00001, "{pair:?}");
    }
    server.settle();
    assert_eq!(
        server
            .metrics
            .frames
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        server
            .metrics
            .cursor_sessions
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    eprintln!(
        "synthetic native recording: {} frames; artifacts {}",
        frames,
        dir.display()
    );
}
