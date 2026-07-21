use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(test)]
use crate::protocol::MAX_PAYLOAD_BYTES;
pub use crate::protocol::{
    RecordingSnapshot, Replacement, Request, Response, read_frame, write_frame,
};

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("boltsnap.sock");
        }
    }
    std::env::temp_dir().join("boltsnap.sock")
}

/// True if a daemon answers on the socket.
pub fn daemon_alive() -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
            let _ = s.write_all(&Request::Ping.encode());
            // A successful connect+write is enough; we don't require a reply.
            true
        }
        Err(_) => false,
    }
}

fn systemd_start_args() -> [&'static str; 4] {
    ["--user", "start", "--no-block", "boltsnap-daemon.service"]
}

/// Connect to the daemon, asking the user service manager to start it if needed.
fn ensure_daemon() -> io::Result<UnixStream> {
    if let Ok(s) = UnixStream::connect(socket_path()) {
        return Ok(s);
    }

    let managed = Command::new("systemctl")
        .args(systemd_start_args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());

    // Keep working on systems without a usable user service manager.
    if !managed {
        let exe = std::env::current_exe()?;
        Command::new(exe)
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
    }

    // Poll for it to come up (~1s).
    for _ in 0..100 {
        if let Ok(s) = UnixStream::connect(socket_path()) {
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "daemon did not start",
    ))
}

/// Send a request to the shelf daemon, starting it if needed.
pub fn send_to_shelf(req: Request) -> io::Result<()> {
    let mut stream = ensure_daemon()?;
    stream.write_all(&req.encode())?;
    stream.flush()?;
    Ok(())
}

pub fn call_daemon(req: Request) -> io::Result<Response> {
    let mut stream = ensure_daemon()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(&req.encode())?;
    stream.flush()?;
    Response::read(&mut stream)
}

pub fn watch_recording() -> io::Result<UnixStream> {
    let mut stream = ensure_daemon()?;
    stream.write_all(&Request::RecordingWatch.encode())?;
    stream.flush()?;
    Ok(stream)
}

#[cfg(test)]
fn watch_recording_at(path: &std::path::Path) -> io::Result<UnixStream> {
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(&Request::RecordingWatch.encode())?;
    stream.flush()?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Cursor};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    #[test]
    fn recording_control_roundtrips_all_actions() {
        for action in [
            crate::record::session::RecordingAction::Pause,
            crate::record::session::RecordingAction::Resume,
            crate::record::session::RecordingAction::SaveShelf,
            crate::record::session::RecordingAction::SaveDisk,
            crate::record::session::RecordingAction::Discard,
        ] {
            let decoded = Request::read(&mut Cursor::new(
                Request::RecordingControl { action }.encode(),
            ))
            .unwrap();
            assert!(matches!(
                decoded,
                Request::RecordingControl { action: got } if got == action
            ));
        }
    }

    #[test]
    fn recording_requests_roundtrip() {
        for request in [
            Request::RecordingStatus,
            Request::RecordingWatch,
            Request::ShowRecordingControls,
            Request::StartDefaultRecording,
        ] {
            let encoded = request.encode();
            let decoded = Request::read(&mut Cursor::new(encoded)).unwrap();
            assert_eq!(
                std::mem::discriminant(&decoded),
                std::mem::discriminant(&request)
            );
        }

        let request = Request::StartRecordingOutputs {
            names: vec!["DP-3".into(), "DP-1".into()],
            combined: true,
        };
        assert!(matches!(
            Request::read(&mut Cursor::new(request.encode())).unwrap(),
            Request::StartRecordingOutputs { names, combined }
                if names == ["DP-3", "DP-1"] && combined
        ));
    }

    #[test]
    fn recording_snapshot_json_uses_stable_public_names() {
        let line = RecordingSnapshot {
            state: crate::record::session::PublicRecordingState::Paused,
            elapsed_ms: 83_000,
            scope: "both".into(),
            outputs: vec!["DP-3".into(), "DP-1".into()],
            actions_enabled: true,
            error: None,
        }
        .to_json_line();
        assert_eq!(
            line,
            "{\"state\":\"paused\",\"elapsed_ms\":83000,\"scope\":\"both\",\"outputs\":[\"DP-3\",\"DP-1\"],\"actions_enabled\":true,\"error\":null}\n"
        );
    }

    #[test]
    fn response_roundtrips_success_and_error() {
        let snapshot = RecordingSnapshot::idle();
        let success = Response::ok(Some(snapshot.clone()));
        assert_eq!(
            Response::read(&mut Cursor::new(success.encode())).unwrap(),
            success
        );

        let failure = Response::error("cannot pause while idle");
        assert_eq!(
            Response::read(&mut Cursor::new(failure.encode())).unwrap(),
            failure
        );

        let owned = Response::ok_path(PathBuf::from("/tmp/boltsnap-shelf-video.mp4"));
        assert_eq!(
            Response::read(&mut Cursor::new(owned.encode())).unwrap(),
            owned
        );
    }

    #[test]
    fn watch_snapshot_lines_are_newline_framed() {
        let first = RecordingSnapshot::idle().to_json_line();
        let second = RecordingSnapshot {
            state: crate::record::session::PublicRecordingState::Recording,
            elapsed_ms: 1_000,
            scope: "area".into(),
            outputs: vec!["DP-3".into()],
            actions_enabled: true,
            error: None,
        }
        .to_json_line();
        let stream = format!("{first}{second}");
        let lines = stream.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"state\":\"idle\""));
        assert!(lines[1].contains("\"elapsed_ms\":1000"));
    }

    #[test]
    fn watch_stream_delivers_lines_before_server_exit() {
        let path = std::env::temp_dir().join(format!(
            "boltsnap-watch-test-{}-{}.sock",
            std::process::id(),
            crate::paths::timestamp()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let (first_sent_tx, first_sent_rx) = mpsc::channel();
        let (first_received_tx, first_received_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(matches!(
                Request::read(&mut stream).unwrap(),
                Request::RecordingWatch
            ));
            stream.write_all(b"{\"state\":\"idle\"}\n").unwrap();
            stream.flush().unwrap();
            first_sent_tx.send(()).unwrap();
            first_received_rx.recv().unwrap();
            stream.write_all(b"{\"state\":\"recording\"}\n").unwrap();
        });

        let mut reader = BufReader::new(watch_recording_at(&path).unwrap());
        first_sent_rx.recv().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"state\":\"idle\"}\n");
        first_received_tx.send(()).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"state\":\"recording\"}\n");
        server.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{\"cmd\":\"add\"}", &[1, 2, 3, 4]).unwrap();
        let mut cur = Cursor::new(buf);
        let (header, payload) = read_frame(&mut cur).unwrap();
        assert_eq!(header, b"{\"cmd\":\"add\"}");
        assert_eq!(payload, vec![1, 2, 3, 4]);
    }

    #[test]
    fn oversized_frame_header_is_rejected_before_allocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(65_537_u32).to_be_bytes());
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        let error = read_frame(&mut Cursor::new(bytes)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("header"));
    }

    #[test]
    fn oversized_frame_payload_is_rejected_before_allocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(&((MAX_PAYLOAD_BYTES as u32) + 1).to_be_bytes());
        let error = read_frame(&mut Cursor::new(bytes)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("payload"));
    }

    #[test]
    fn recording_geometry_rejects_wrapped_and_zero_dimensions() {
        for header in [
            br#"{"cmd":"record","x":2147483648,"y":0,"w":1,"h":1}"#.as_slice(),
            br#"{"cmd":"record","x":0,"y":0,"w":0,"h":1}"#.as_slice(),
        ] {
            let mut bytes = Vec::new();
            write_frame(&mut bytes, header, &[]).unwrap();
            assert_eq!(
                Request::read(&mut Cursor::new(bytes)).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn request_add_roundtrip() {
        let req = Request::Add {
            source: "area".into(),
            png: vec![9, 8, 7],
            output: Some("DP-3".into()),
        };
        let bytes = req.encode();
        let mut cur = Cursor::new(bytes);
        match Request::read(&mut cur).unwrap() {
            Request::Add {
                source,
                png,
                output,
            } => {
                assert_eq!(source, "area");
                assert_eq!(png, vec![9, 8, 7]);
                assert_eq!(output.as_deref(), Some("DP-3"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_add_without_output_remains_compatible() {
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            br#"{"cmd":"add","source":"legacy"}"#,
            &[1, 2, 3],
        )
        .unwrap();

        match Request::read(&mut Cursor::new(bytes)).unwrap() {
            Request::Add { output, .. } => assert_eq!(output, None),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_add_video_roundtrip() {
        let req = Request::AddVideo {
            source: "eddy".into(),
            path: PathBuf::from("/tmp/eddy clip.mp4"),
            output: Some("DP-2".into()),
            take_ownership: true,
        };
        match Request::read(&mut Cursor::new(req.encode())).unwrap() {
            Request::AddVideo {
                source,
                path,
                output,
                take_ownership,
            } => {
                assert_eq!(source, "eddy");
                assert_eq!(path, PathBuf::from("/tmp/eddy clip.mp4"));
                assert_eq!(output.as_deref(), Some("DP-2"));
                assert!(take_ownership);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_replace_image_roundtrip() {
        let req = Request::Replace {
            id: 12,
            media: Replacement::Image(vec![9, 8, 7]),
        };

        match Request::read(&mut Cursor::new(req.encode())).unwrap() {
            Request::Replace {
                id,
                media: Replacement::Image(png),
            } => {
                assert_eq!(id, 12);
                assert_eq!(png, vec![9, 8, 7]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_replace_video_roundtrip() {
        let req = Request::Replace {
            id: 13,
            media: Replacement::Video {
                path: PathBuf::from("/tmp/edited.mp4"),
                take_ownership: false,
            },
        };

        match Request::read(&mut Cursor::new(req.encode())).unwrap() {
            Request::Replace {
                id,
                media:
                    Replacement::Video {
                        path,
                        take_ownership,
                    },
            } => {
                assert_eq!(id, 13);
                assert_eq!(path, PathBuf::from("/tmp/edited.mp4"));
                assert!(!take_ownership);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_ping_and_reload_roundtrip() {
        let mut cur = Cursor::new(Request::Ping.encode());
        assert!(matches!(Request::read(&mut cur).unwrap(), Request::Ping));
        let mut cur = Cursor::new(Request::StopRecording.encode());
        assert!(matches!(
            Request::read(&mut cur).unwrap(),
            Request::StopRecording
        ));
        let mut cur = Cursor::new(Request::Reload { id: 42 }.encode());
        assert!(matches!(
            Request::read(&mut cur).unwrap(),
            Request::Reload { id: 42 }
        ));
    }

    #[test]
    fn request_start_recording_roundtrip() {
        let req = Request::StartRecording {
            x: -100,
            y: 40,
            w: 800,
            h: 600,
            show_frame: false,
            audio_enabled: false,
        };
        let mut cur = Cursor::new(req.encode());
        match Request::read(&mut cur).unwrap() {
            Request::StartRecording {
                x,
                y,
                w,
                h,
                show_frame,
                audio_enabled,
            } => {
                assert_eq!((x, y, w, h), (-100, 40, 800, 600));
                assert!(!show_frame);
                assert!(!audio_enabled);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn start_recording_missing_frame_field_defaults_true() {
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            br#"{"cmd":"record","x":1,"y":2,"w":3,"h":4}"#,
            &[],
        )
        .unwrap();
        assert!(matches!(
            Request::read(&mut Cursor::new(bytes)).unwrap(),
            Request::StartRecording {
                show_frame: true,
                ..
            }
        ));
    }

    #[test]
    fn start_recording_missing_audio_field_defaults_true() {
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            br#"{"cmd":"record","x":1,"y":2,"w":3,"h":4}"#,
            &[],
        )
        .unwrap();
        assert!(matches!(
            Request::read(&mut Cursor::new(bytes)).unwrap(),
            Request::StartRecording {
                audio_enabled: true,
                ..
            }
        ));
    }

    #[test]
    fn request_start_recording_output_roundtrip() {
        let req = Request::StartRecordingOutput {
            name: "DP-1".into(),
        };
        let mut cur = Cursor::new(req.encode());
        match Request::read(&mut cur).unwrap() {
            Request::StartRecordingOutput { name } => assert_eq!(name, "DP-1"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_recording_thumb_roundtrip() {
        let req = Request::RecordingThumb {
            id: 7,
            thumb: PathBuf::from("/tmp/boltsnap-rec-1.png"),
        };
        let mut cur = Cursor::new(req.encode());
        match Request::read(&mut cur).unwrap() {
            Request::RecordingThumb { id, thumb } => {
                assert_eq!(id, 7);
                assert_eq!(thumb, PathBuf::from("/tmp/boltsnap-rec-1.png"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn socket_path_uses_runtime_dir() {
        let prev = std::env::var("XDG_RUNTIME_DIR").ok();
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/user/test") };
        assert_eq!(socket_path(), PathBuf::from("/run/user/test/boltsnap.sock"));
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) },
            None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
        }
    }

    #[test]
    fn daemon_start_uses_the_user_systemd_service() {
        assert_eq!(
            systemd_start_args(),
            ["--user", "start", "--no-block", "boltsnap-daemon.service"]
        );
    }
}
