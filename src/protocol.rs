use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

const MAX_HEADER_BYTES: usize = 64 * 1024;
pub(crate) const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicRecordingState {
    Idle,
    Recording,
    Paused,
    Finalizing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingAction {
    Pause,
    Resume,
    SaveShelf,
    SaveDisk,
    Discard,
}

#[derive(Debug)]
pub enum Replacement {
    Image(Vec<u8>),
    Video { path: PathBuf, take_ownership: bool },
}

#[derive(Debug)]
pub enum Request {
    Add {
        source: String,
        png: Vec<u8>,
        output: Option<String>,
    },
    AddVideo {
        source: String,
        path: PathBuf,
        output: Option<String>,
        take_ownership: bool,
    },
    Replace {
        id: u64,
        media: Replacement,
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
        audio_enabled: bool,
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
    pub path: Option<PathBuf>,
}

impl Response {
    pub fn ok(snapshot: Option<RecordingSnapshot>) -> Self {
        Self {
            ok: true,
            error: None,
            snapshot,
            path: None,
        }
    }

    pub fn ok_path(path: PathBuf) -> Self {
        Self {
            ok: true,
            error: None,
            snapshot: None,
            path: Some(path),
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            snapshot: None,
            path: None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let header = json!({
            "ok": self.ok,
            "error": self.error,
            "snapshot": self.snapshot.as_ref().map(RecordingSnapshot::as_value),
            "path": self.path.as_ref().map(|path| path.to_string_lossy()),
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
        let path = match value.get("path") {
            Some(Value::Null) | None => None,
            Some(Value::String(path)) => Some(PathBuf::from(path)),
            _ => return Err(invalid_data("response path must be a string or null")),
        };
        Ok(Self {
            ok,
            error,
            snapshot,
            path,
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
    if hlen > MAX_HEADER_BYTES {
        return Err(invalid_data(format!(
            "IPC header exceeds {MAX_HEADER_BYTES} bytes"
        )));
    }
    if plen > MAX_PAYLOAD_BYTES {
        return Err(invalid_data(format!(
            "IPC payload exceeds {MAX_PAYLOAD_BYTES} bytes"
        )));
    }
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
            Request::Add {
                source,
                png,
                output,
            } => {
                let mut header = json!({ "cmd": "add", "source": source });
                if let Some(output) = output {
                    header["output"] = json!(output);
                }
                write_frame(&mut buf, header.to_string().as_bytes(), png).unwrap();
            }
            Request::AddVideo {
                source,
                path,
                output,
                take_ownership,
            } => {
                let mut header = json!({
                    "cmd": "add_video",
                    "source": source,
                    "path": path.to_string_lossy(),
                    "take_ownership": take_ownership,
                });
                if let Some(output) = output {
                    header["output"] = json!(output);
                }
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::Replace {
                id,
                media: Replacement::Image(png),
            } => {
                let header = json!({ "cmd": "replace", "id": id, "media": "image" });
                write_frame(&mut buf, header.to_string().as_bytes(), png).unwrap();
            }
            Request::Replace {
                id,
                media:
                    Replacement::Video {
                        path,
                        take_ownership,
                    },
            } => {
                let header = json!({
                    "cmd": "replace",
                    "id": id,
                    "media": "video",
                    "path": path.to_string_lossy(),
                    "take_ownership": take_ownership,
                });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
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
                audio_enabled,
            } => {
                let header = json!({
                    "cmd": "record",
                    "x": x,
                    "y": y,
                    "w": w,
                    "h": h,
                    "show_frame": show_frame,
                    "audio_enabled": audio_enabled,
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
                output: v
                    .get("output")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            }),
            Some("add_video") => Ok(Request::AddVideo {
                source: v
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                path: PathBuf::from(v.get("path").and_then(Value::as_str).unwrap_or("")),
                output: v
                    .get("output")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                take_ownership: v
                    .get("take_ownership")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            Some("replace") => {
                let id = v.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                match v.get("media").and_then(|m| m.as_str()) {
                    Some("image") => Ok(Request::Replace {
                        id,
                        media: Replacement::Image(payload),
                    }),
                    Some("video") => Ok(Request::Replace {
                        id,
                        media: Replacement::Video {
                            path: PathBuf::from(
                                v.get("path").and_then(|p| p.as_str()).unwrap_or(""),
                            ),
                            take_ownership: v
                                .get("take_ownership")
                                .and_then(Value::as_bool)
                                .unwrap_or(true),
                        },
                    }),
                    other => Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown replacement media: {other:?}"),
                    )),
                }
            }
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
            Some("record") => {
                let coordinate = |name| {
                    i32::try_from(
                        v.get(name)
                            .and_then(Value::as_i64)
                            .ok_or_else(|| invalid_data(format!("{name} must be an integer")))?,
                    )
                    .map_err(|_| invalid_data(format!("{name} is out of range")))
                };
                let dimension = |name| {
                    let value =
                        u32::try_from(v.get(name).and_then(Value::as_u64).ok_or_else(|| {
                            invalid_data(format!("{name} must be an unsigned integer"))
                        })?)
                        .map_err(|_| invalid_data(format!("{name} is out of range")))?;
                    if value == 0 {
                        Err(invalid_data(format!("{name} must be greater than zero")))
                    } else {
                        Ok(value)
                    }
                };
                Ok(Request::StartRecording {
                    x: coordinate("x")?,
                    y: coordinate("y")?,
                    w: dimension("w")?,
                    h: dimension("h")?,
                    show_frame: v
                        .get("show_frame")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(true),
                    audio_enabled: v
                        .get("audio_enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                })
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn recording_actions_roundtrip_without_platform_state() {
        for action in [
            RecordingAction::Pause,
            RecordingAction::Resume,
            RecordingAction::SaveShelf,
            RecordingAction::SaveDisk,
            RecordingAction::Discard,
        ] {
            let request = Request::RecordingControl { action };
            assert!(matches!(
                Request::read(&mut Cursor::new(request.encode())).unwrap(),
                Request::RecordingControl { action: decoded } if decoded == action
            ));
        }
    }

    #[test]
    fn response_roundtrips_without_platform_transport() {
        let response = Response::ok(Some(RecordingSnapshot::idle()));
        assert_eq!(
            Response::read(&mut Cursor::new(response.encode())).unwrap(),
            response
        );
    }

    #[test]
    fn binary_frame_roundtrips_without_platform_transport() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, br#"{"cmd":"add"}"#, &[1, 2, 3, 4]).unwrap();
        let (header, payload) = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(header, br#"{"cmd":"add"}"#);
        assert_eq!(payload, [1, 2, 3, 4]);
    }

    #[test]
    fn oversized_payload_is_rejected_before_allocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(&((MAX_PAYLOAD_BYTES as u32) + 1).to_be_bytes());
        let error = read_frame(&mut Cursor::new(bytes)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
