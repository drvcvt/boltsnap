use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ffmpeg_next::Packet;
use serde_json::{Value, json};

use crate::media::Recording;
use crate::replay::{crop::Crop, ring::Ring, wire};

struct Frozen {
    id: u64,
    recording: Recording,
    created: Instant,
}

/// How long a frozen selection waits for Save or Cancel.
const SELECTION_TTL: Duration = Duration::from_secs(120);

struct State {
    recording: Option<Recording>,
    frozen: Option<Frozen>,
    next_id: u64,
    busy: bool,
    error: Option<String>,
    updated: Instant,
}

fn capture(state: &Mutex<State>, duration: i64, budget: usize) -> Result<(), String> {
    let (mut input, streams, video_index) = crate::media::open(Path::new("-"))?;
    state.lock().unwrap().recording = Some(Recording {
        streams,
        ring: Ring::new(duration, budget)?,
        packets_read: 0,
    });
    loop {
        let mut packet = Packet::empty();
        packet
            .read(&mut input)
            .map_err(|e| format!("replay input ended: {e}"))?;
        let mut state = state.lock().unwrap();
        let recording = state.recording.as_mut().ok_or("missing live recording")?;
        let index = packet.stream();
        let stream = recording.streams.get(index).ok_or("unknown input stream")?;
        recording.ring.push(
            crate::ring::entry(packet, stream.time_base)?,
            index == video_index,
        )?;
        recording.packets_read += 1;
        if index == video_index {
            state.updated = Instant::now();
        }
    }
}

/// The video stopped arriving, or the input failed.
fn input_lost(state: &State) -> bool {
    state.error.is_some() || state.updated.elapsed() > Duration::from_secs(3)
}

/// What the buffer holds stays saveable after its input failed or stalled,
/// which is when the last seconds matter most; the clip then ends where
/// capture stopped. Only a buffer that never filled has nothing to give.
fn snapshot(state: &State) -> Result<Recording, String> {
    let Some(recording) = state.recording.as_ref() else {
        return Err(state
            .error
            .clone()
            .unwrap_or_else(|| "replay is warming up".into()));
    };
    recording
        .snapshot()
        .map_err(|error| state.error.clone().unwrap_or(error))
}

fn response(stream: &mut UnixStream, result: Result<Value, String>) {
    let value = match result {
        Ok(mut value) => {
            value["ok"] = json!(true);
            value
        }
        Err(error) => json!({"ok": false, "error": error}),
    };
    let _ = wire::write(stream, &value);
}

pub fn run(
    name: &str,
    directory: PathBuf,
    seconds: u64,
    memory: usize,
    encoder: String,
) -> Result<(), String> {
    let listener = crate::socket::bind(name).map_err(|e| format!("replay socket: {e}"))?;
    let state = Arc::new(Mutex::new(State {
        recording: None,
        frozen: None,
        next_id: 0,
        busy: false,
        error: None,
        updated: Instant::now(),
    }));
    let input_state = state.clone();
    let budget = memory / 3;
    std::thread::spawn(move || {
        if let Err(error) = capture(&input_state, seconds as i64 * 1_000_000, budget) {
            input_state.lock().unwrap().error = Some(error);
        }
    });
    loop {
        {
            let mut state = state.lock().unwrap();
            if state
                .frozen
                .as_ref()
                .is_some_and(|f| f.created.elapsed() > SELECTION_TTL)
            {
                state.frozen = None;
            }
        }
        let mut descriptor = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 1000) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.to_string());
        }
        if ready == 0 {
            continue;
        }
        let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
        if crate::socket::configure(&stream).is_err() {
            continue;
        }
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        let request = match wire::read(&mut stream) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let command = match wire::command(&request) {
            Ok(c) => c,
            Err(error) => {
                response(&mut stream, Err(error));
                continue;
            }
        };
        let mut state_guard = state.lock().unwrap();
        let result = match command {
            "status" => {
                let bounds = state_guard
                    .recording
                    .as_ref()
                    .and_then(|r| r.ring.bounds().ok());
                // An open selection still holds a saveable snapshot, even
                // once the input has failed and `ready` is false.
                let selection_active = state_guard
                    .frozen
                    .as_ref()
                    .is_some_and(|f| f.created.elapsed() <= SELECTION_TTL);
                Ok(
                    json!({"ready": state_guard.error.is_none() && bounds.is_some() && state_guard.updated.elapsed() < Duration::from_secs(3),
                    "duration_us": bounds.map(|(s,e)| e-s).unwrap_or(0), "busy": state_guard.busy,
                    // Bytes reaching the budget before `seconds` means the
                    // bitrate, not the duration, bounds the history.
                    "bytes": state_guard.recording.as_ref().map_or(0, |r| r.ring.bytes()),
                    "budget_bytes": budget,
                    "selection_active": selection_active,
                    "error": state_guard.error, "encoder": encoder, "seconds": seconds}),
                )
            }
            "freeze" if !state_guard.busy && state_guard.frozen.is_none() => {
                let prepared = snapshot(&state_guard).and_then(|recording| {
                    let (width, height) = recording.dimensions()?;
                    let (start, end) = recording.ring.bounds()?;
                    let tail = recording.tail_snapshot()?;
                    Ok((recording, tail, width, height, start, end))
                });
                match prepared {
                    Ok((recording, tail, width, height, start, end)) => {
                        state_guard.next_id += 1;
                        let id = state_guard.next_id;
                        state_guard.frozen = Some(Frozen {
                            id,
                            recording,
                            created: Instant::now(),
                        });
                        state_guard.busy = true;
                        let state = state.clone();
                        std::thread::spawn(move || {
                            let preview = crate::crop::preview(&tail);
                            drop(tail);
                            let mut guard = state.lock().unwrap();
                            guard.busy = false;
                            if guard.frozen.as_ref().is_none_or(|f| f.id != id) {
                                drop(guard);
                                response(&mut stream, Err("replay selection cancelled".into()));
                                return;
                            }
                            drop(guard);
                            let sent = match preview {
                                Ok(preview) => {
                                    let metadata = json!({"ok":true,"snapshot":id,"width":width,"height":height,
                                        "start_us":start,"end_us":end,"duration_us":end-start,"preview_bytes":preview.len()});
                                    wire::write(&mut stream, &metadata)
                                        .and_then(|_| stream.write_all(&preview))
                                        .is_ok()
                                }
                                Err(error) => {
                                    response(&mut stream, Err(error));
                                    false
                                }
                            };
                            if !sent {
                                let mut guard = state.lock().unwrap();
                                if guard.frozen.as_ref().is_some_and(|f| f.id == id) {
                                    guard.frozen = None;
                                }
                            }
                        });
                        continue;
                    }
                    Err(error) => Err(error),
                }
            }
            "cancel" => {
                if state_guard
                    .frozen
                    .as_ref()
                    .is_some_and(|f| Some(f.id) == request["snapshot"].as_u64())
                {
                    state_guard.frozen = None;
                }
                Ok(json!({}))
            }
            "save" if !state_guard.busy => {
                let selected = if let Some(id) = request["snapshot"].as_u64() {
                    if state_guard.frozen.as_ref().is_some_and(|f| f.id == id) {
                        Ok(state_guard.frozen.take().unwrap().recording)
                    } else {
                        Err("replay selection expired; open the selector again".into())
                    }
                } else if state_guard.frozen.is_some() {
                    Err("a replay selection is already open".into())
                } else {
                    snapshot(&state_guard)
                };
                match selected.and_then(|recording| {
                    let (w, h) = recording.dimensions()?;
                    let crop = if request["crop"].is_null() {
                        None
                    } else {
                        Some(Crop::from_value(&request["crop"], w, h)?)
                    };
                    Ok((recording, crop))
                }) {
                    Ok((recording, crop)) => {
                        let input_lost = input_lost(&state_guard);
                        state_guard.busy = true;
                        state_guard.next_id += 1;
                        let path = directory.join(format!(
                            "clip-{}-{}-{}.mp4",
                            std::process::id(),
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos(),
                            state_guard.next_id
                        ));
                        let state = state.clone();
                        let encoder = encoder.clone();
                        std::thread::spawn(move || {
                            // Writing a clip waits for the game, not the reverse.
                            crate::process::background_thread();
                            let result = (|| {
                                let (start, end) = recording.ring.bounds()?;
                                let began = std::time::Instant::now();
                                let written = match crop {
                                    Some(crop) => crate::crop::save(
                                        &recording,
                                        crop,
                                        &encoder,
                                        &path,
                                        budget as u64,
                                    )?,
                                    None => crate::export::remux(&recording, &path, budget as u64)?,
                                };
                                Ok(
                                    json!({"path": path.to_string_lossy(), "path_bytes":path.as_os_str().as_bytes(), "bytes": written.bytes, "duration_us": end-start, "input_lost": input_lost,
                                        "export_ms": began.elapsed().as_millis(), "sync_ms": written.sync_ms}),
                                )
                            })();
                            drop(recording);
                            state.lock().unwrap().busy = false;
                            response(&mut stream, result);
                        });
                        continue;
                    }
                    Err(error) => Err(error),
                }
            }
            "stop" => {
                response(&mut stream, Ok(json!({})));
                return Ok(());
            }
            "freeze" | "save" => Err("another replay selection or export is active".into()),
            _ => Err("unknown replay command".into()),
        };
        drop(state_guard);
        response(&mut stream, result);
    }
}
