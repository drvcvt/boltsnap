use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

#[derive(Debug)]
pub enum Request {
    Add {
        source: String,
        png: Vec<u8>,
    },
    Reload {
        id: u64,
    },
    Ping,
    /// Stop an in-progress recording (same as the indicator's Stop button). Sent by
    /// `boltsnap stop` for a keyboard stop.
    StopRecording,
    StartRecording {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    /// A finished recording is ready to ingest: the temp `.mp4` and a first-frame
    /// `.png` thumbnail (both as paths; the daemon owns/reads them). Posted by the
    /// daemon's off-thread Confirm worker back to its own socket.
    RecordingDone {
        video: PathBuf,
        thumb: PathBuf,
    },
}

/// Frame = [u32 BE header_len][u32 BE payload_len][header bytes][payload bytes].
pub fn write_frame<W: Write>(w: &mut W, header: &[u8], payload: &[u8]) -> io::Result<()> {
    w.write_all(&(header.len() as u32).to_be_bytes())?;
    w.write_all(&(payload.len() as u32).to_be_bytes())?;
    w.write_all(header)?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut len4 = [0u8; 4];
    r.read_exact(&mut len4)?;
    let hlen = u32::from_be_bytes(len4) as usize;
    r.read_exact(&mut len4)?;
    let plen = u32::from_be_bytes(len4) as usize;
    let mut header = vec![0u8; hlen];
    r.read_exact(&mut header)?;
    let mut payload = vec![0u8; plen];
    r.read_exact(&mut payload)?;
    Ok((header, payload))
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Request::Add { source, png } => {
                let header = json!({ "cmd": "add", "source": source });
                write_frame(&mut buf, header.to_string().as_bytes(), png).unwrap();
            }
            Request::Reload { id } => {
                let header = json!({ "cmd": "reload", "id": id });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::Ping => {
                let header = json!({ "cmd": "ping" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StopRecording => {
                let header = json!({ "cmd": "stop" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StartRecording { x, y, w, h } => {
                let header = json!({ "cmd": "record", "x": x, "y": y, "w": w, "h": h });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::RecordingDone { video, thumb } => {
                let header = json!({
                    "cmd": "recording_done",
                    "video": video.to_string_lossy(),
                    "thumb": thumb.to_string_lossy(),
                });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
        }
        buf
    }

    pub fn read<R: Read>(r: &mut R) -> io::Result<Request> {
        let (header, payload) = read_frame(r)?;
        let v: Value = serde_json::from_slice(&header)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        match v.get("cmd").and_then(|c| c.as_str()) {
            Some("add") => Ok(Request::Add {
                source: v
                    .get("source")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                png: payload,
            }),
            Some("reload") => Ok(Request::Reload {
                id: v.get("id").and_then(|i| i.as_u64()).unwrap_or(0),
            }),
            Some("ping") => Ok(Request::Ping),
            Some("stop") => Ok(Request::StopRecording),
            Some("record") => Ok(Request::StartRecording {
                x: v.get("x").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
                y: v.get("y").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
                w: v.get("w").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
                h: v.get("h").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            }),
            Some("recording_done") => Ok(Request::RecordingDone {
                video: PathBuf::from(v.get("video").and_then(|s| s.as_str()).unwrap_or("")),
                thumb: PathBuf::from(v.get("thumb").and_then(|s| s.as_str()).unwrap_or("")),
            }),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown cmd: {other:?}"),
            )),
        }
    }
}

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

/// Connect to the daemon, self-spawning `boltsnap daemon` if none is running.
fn ensure_daemon() -> io::Result<UnixStream> {
    if let Ok(s) = UnixStream::connect(socket_path()) {
        return Ok(s);
    }
    // No daemon: spawn one detached.
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
    fn request_add_roundtrip() {
        let req = Request::Add {
            source: "area".into(),
            png: vec![9, 8, 7],
        };
        let bytes = req.encode();
        let mut cur = Cursor::new(bytes);
        match Request::read(&mut cur).unwrap() {
            Request::Add { source, png } => {
                assert_eq!(source, "area");
                assert_eq!(png, vec![9, 8, 7]);
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
        };
        let mut cur = Cursor::new(req.encode());
        match Request::read(&mut cur).unwrap() {
            Request::StartRecording { x, y, w, h } => {
                assert_eq!((x, y, w, h), (-100, 40, 800, 600));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_recording_done_roundtrip() {
        let req = Request::RecordingDone {
            video: PathBuf::from("/tmp/boltsnap-rec-1.mp4"),
            thumb: PathBuf::from("/tmp/boltsnap-rec-1.png"),
        };
        let mut cur = Cursor::new(req.encode());
        match Request::read(&mut cur).unwrap() {
            Request::RecordingDone { video, thumb } => {
                assert_eq!(video, PathBuf::from("/tmp/boltsnap-rec-1.mp4"));
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
}
