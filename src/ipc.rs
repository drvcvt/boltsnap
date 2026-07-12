use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

use crate::record::session::{PublicRecordingState, RecordingAction};

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
    RecordingStatus,
    RecordingWatch,
    ShowRecordingControls,
    RecordingControl {
        action: RecordingAction,
    },
    StartDefaultRecording,
    /// Stop an in-progress recording (same as the indicator's Stop button). Sent by
    /// `boltsnap stop` for a keyboard stop.
    StopRecording,
    StartRecording {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        show_frame: bool,
    },
    /// Start a fullscreen recording of a whole output (Hyprland monitor name) via
    /// `wf-recorder -o`. No overlay; stop is keyboard-only and auto-finalizes.
    StartRecordingOutput {
        name: String,
    },
    StartRecordingOutputs {
        names: Vec<String>,
        combined: bool,
    },
    /// A finished recording's first-frame thumbnail is ready: replace card `id`'s
    /// placeholder with the png at `thumb`. Posted by the off-thread finalize
    /// worker back to the daemon's own socket.
    RecordingThumb {
        id: u64,
        thumb: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingSnapshot {
    pub state: PublicRecordingState,
    pub elapsed_ms: u64,
    pub scope: String,
    pub outputs: Vec<String>,
    pub actions_enabled: bool,
    pub error: Option<String>,
}

impl RecordingSnapshot {
    pub fn idle() -> Self {
        Self {
            state: PublicRecordingState::Idle,
            elapsed_ms: 0,
            scope: "none".into(),
            outputs: Vec::new(),
            actions_enabled: false,
            error: None,
        }
    }

    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"state\":{},\"elapsed_ms\":{},\"scope\":{},\"outputs\":{},\"actions_enabled\":{},\"error\":{}}}\n",
            serde_json::to_string(recording_state_name(self.state)).unwrap(),
            self.elapsed_ms,
            serde_json::to_string(&self.scope).unwrap(),
            serde_json::to_string(&self.outputs).unwrap(),
            self.actions_enabled,
            serde_json::to_string(&self.error).unwrap(),
        )
    }

    fn from_value(value: &Value) -> io::Result<Self> {
        let field = |name| {
            value.get(name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}"))
            })
        };
        Ok(Self {
            state: parse_recording_state(
                field("state")?
                    .as_str()
                    .ok_or_else(|| invalid_data("state must be a string"))?,
            )?,
            elapsed_ms: field("elapsed_ms")?
                .as_u64()
                .ok_or_else(|| invalid_data("elapsed_ms must be an unsigned integer"))?,
            scope: field("scope")?
                .as_str()
                .ok_or_else(|| invalid_data("scope must be a string"))?
                .to_owned(),
            outputs: parse_string_array(field("outputs")?, "outputs")?,
            actions_enabled: field("actions_enabled")?
                .as_bool()
                .ok_or_else(|| invalid_data("actions_enabled must be a boolean"))?,
            error: match field("error")? {
                Value::Null => None,
                Value::String(error) => Some(error.clone()),
                _ => return Err(invalid_data("error must be a string or null")),
            },
        })
    }

    fn as_value(&self) -> Value {
        json!({
            "state": recording_state_name(self.state),
            "elapsed_ms": self.elapsed_ms,
            "scope": self.scope,
            "outputs": self.outputs,
            "actions_enabled": self.actions_enabled,
            "error": self.error,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub ok: bool,
    pub error: Option<String>,
    pub snapshot: Option<RecordingSnapshot>,
}

impl Response {
    pub fn ok(snapshot: Option<RecordingSnapshot>) -> Self {
        Self {
            ok: true,
            error: None,
            snapshot,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            snapshot: None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let header = json!({
            "ok": self.ok,
            "error": self.error,
            "snapshot": self.snapshot.as_ref().map(RecordingSnapshot::as_value),
        });
        let mut buf = Vec::new();
        write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
        buf
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<Self> {
        let (header, payload) = read_frame(reader)?;
        if !payload.is_empty() {
            return Err(invalid_data("response payload must be empty"));
        }
        let value: Value = serde_json::from_slice(&header)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let ok = value
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid_data("response ok must be a boolean"))?;
        let error = match value.get("error") {
            Some(Value::Null) | None => None,
            Some(Value::String(error)) => Some(error.clone()),
            _ => return Err(invalid_data("response error must be a string or null")),
        };
        let snapshot = match value.get("snapshot") {
            Some(Value::Null) | None => None,
            Some(snapshot) => Some(RecordingSnapshot::from_value(snapshot)?),
        };
        Ok(Self {
            ok,
            error,
            snapshot,
        })
    }
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn recording_state_name(state: PublicRecordingState) -> &'static str {
    match state {
        PublicRecordingState::Idle => "idle",
        PublicRecordingState::Recording => "recording",
        PublicRecordingState::Paused => "paused",
        PublicRecordingState::Finalizing => "finalizing",
    }
}

fn parse_recording_state(state: &str) -> io::Result<PublicRecordingState> {
    match state {
        "idle" => Ok(PublicRecordingState::Idle),
        "recording" => Ok(PublicRecordingState::Recording),
        "paused" => Ok(PublicRecordingState::Paused),
        "finalizing" => Ok(PublicRecordingState::Finalizing),
        _ => Err(invalid_data(format!("unknown recording state: {state}"))),
    }
}

fn recording_action_name(action: RecordingAction) -> &'static str {
    match action {
        RecordingAction::Pause => "pause",
        RecordingAction::Resume => "resume",
        RecordingAction::SaveShelf => "save-shelf",
        RecordingAction::SaveDisk => "save-disk",
        RecordingAction::Discard => "discard",
    }
}

fn parse_recording_action(action: &str) -> io::Result<RecordingAction> {
    match action {
        "pause" => Ok(RecordingAction::Pause),
        "resume" => Ok(RecordingAction::Resume),
        "save-shelf" => Ok(RecordingAction::SaveShelf),
        "save-disk" => Ok(RecordingAction::SaveDisk),
        "discard" => Ok(RecordingAction::Discard),
        _ => Err(invalid_data(format!("unknown recording action: {action}"))),
    }
}

fn parse_string_array(value: &Value, name: &str) -> io::Result<Vec<String>> {
    value
        .as_array()
        .ok_or_else(|| invalid_data(format!("{name} must be an array")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid_data(format!("{name} entries must be strings")))
        })
        .collect()
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
            Request::RecordingStatus => {
                let header = json!({ "cmd": "recording_status" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::RecordingWatch => {
                let header = json!({ "cmd": "recording_watch" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::ShowRecordingControls => {
                let header = json!({ "cmd": "recording_show_controls" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::RecordingControl { action } => {
                let header = json!({
                    "cmd": "recording_control",
                    "action": recording_action_name(*action),
                });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StartDefaultRecording => {
                let header = json!({ "cmd": "record_default" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StopRecording => {
                let header = json!({ "cmd": "stop" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StartRecording {
                x,
                y,
                w,
                h,
                show_frame,
            } => {
                let header = json!({
                    "cmd": "record",
                    "x": x,
                    "y": y,
                    "w": w,
                    "h": h,
                    "show_frame": show_frame,
                });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StartRecordingOutput { name } => {
                let header = json!({ "cmd": "record_output", "name": name });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::StartRecordingOutputs { names, combined } => {
                let header = json!({
                    "cmd": "record_outputs",
                    "names": names,
                    "combined": combined,
                });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::RecordingThumb { id, thumb } => {
                let header = json!({
                    "cmd": "recording_thumb",
                    "id": id,
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
            Some("recording_status") => Ok(Request::RecordingStatus),
            Some("recording_watch") => Ok(Request::RecordingWatch),
            Some("recording_show_controls") => Ok(Request::ShowRecordingControls),
            Some("recording_control") => Ok(Request::RecordingControl {
                action: parse_recording_action(
                    v.get("action")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid_data("recording action must be a string"))?,
                )?,
            }),
            Some("record_default") => Ok(Request::StartDefaultRecording),
            Some("stop") => Ok(Request::StopRecording),
            Some("record") => Ok(Request::StartRecording {
                x: v.get("x").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
                y: v.get("y").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
                w: v.get("w").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
                h: v.get("h").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
                show_frame: v
                    .get("show_frame")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true),
            }),
            Some("record_output") => Ok(Request::StartRecordingOutput {
                name: v
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
            }),
            Some("record_outputs") => Ok(Request::StartRecordingOutputs {
                names: parse_string_array(
                    v.get("names")
                        .ok_or_else(|| invalid_data("missing output names"))?,
                    "names",
                )?,
                combined: v
                    .get("combined")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| invalid_data("combined must be a boolean"))?,
            }),
            Some("recording_thumb") => Ok(Request::RecordingThumb {
                id: v.get("id").and_then(|n| n.as_u64()).unwrap_or(0),
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
            show_frame: false,
        };
        let mut cur = Cursor::new(req.encode());
        match Request::read(&mut cur).unwrap() {
            Request::StartRecording {
                x,
                y,
                w,
                h,
                show_frame,
            } => {
                assert_eq!((x, y, w, h), (-100, 40, 800, 600));
                assert!(!show_frame);
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
}
