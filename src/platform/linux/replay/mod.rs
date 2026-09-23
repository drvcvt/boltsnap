use std::io::Write;
pub(crate) mod process;
mod socket;

use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boltsnap::replay::{settings::Settings, wire};
use serde_json::{Value, json};

use super::shelf::DaemonEvent;

struct Session {
    capture: Child,
    worker: Child,
    name: String,
    output: String,
    output_limit: u64,
    parent_thread: Option<std::sync::mpsc::Sender<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        process::terminate(&mut self.capture);
        process::terminate(&mut self.worker);
        self.parent_thread.take();
    }
}

pub struct Service {
    session: Arc<Mutex<Option<Session>>>,
    stopping: Arc<AtomicBool>,
    cancel_start: Arc<AtomicBool>,
}

/// A read-only view of whether a capture session is held, for the tray's
/// on/off state. Cloneable and cheap so the daemon can ask on every menu build.
#[derive(Clone)]
pub struct Running(Arc<Mutex<Option<Session>>>);

impl Running {
    pub fn get(&self) -> bool {
        self.0.lock().is_ok_and(|session| session.is_some())
    }
}

impl Service {
    pub fn running(&self) -> Running {
        Running(self.session.clone())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        self.cancel_start.store(true, Ordering::Relaxed);
        let _ = socket_call("control", &json!({"version":1,"command":"status"}));
        self.session.lock().unwrap().take();
    }
}

fn socket_call(name: &str, request: &Value) -> Result<(Value, Vec<u8>), String> {
    let mut stream = UnixStream::connect_addr(&socket::address(name).map_err(|e| e.to_string())?)
        .map_err(|e| format!("replay is not running: {e}"))?;
    socket::configure(&stream).map_err(|e| e.to_string())?;
    wire::write(&mut stream, request).map_err(|e| e.to_string())?;
    let timeout = match request["command"].as_str() {
        Some("save") => 1900,
        Some("start") => 120,
        Some("freeze") => 15,
        _ => 5,
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(timeout)))
        .map_err(|e| e.to_string())?;
    let response = wire::read(&mut stream).map_err(|e| format!("replay response: {e}"))?;
    if response["ok"].as_bool() != Some(true) {
        return Err(response["error"]
            .as_str()
            .unwrap_or("replay request failed")
            .into());
    }
    let preview = wire::read_preview(&mut stream, &response).map_err(|e| e.to_string())?;
    Ok((response, preview))
}

pub fn call(request: &Value) -> Result<Value, String> {
    crate::ipc::call_daemon(crate::ipc::Request::RecordingStatus)
        .map_err(|e| format!("start shelf: {e}"))?;
    socket_call("control", request).map(|(response, _)| response)
}

pub fn cli(args: &[String]) -> crate::DynResult<()> {
    let [command] = args else {
        return Err("usage: boltsnap replay start|stop|status|save".into());
    };
    if !matches!(command.as_str(), "start" | "stop" | "status" | "save") {
        return Err("usage: boltsnap replay start|stop|status|save".into());
    }
    let response =
        call(&json!({"version":1,"command":command})).inspect_err(|error| notify_error(error))?;
    println!("{response}");
    Ok(())
}

pub fn notify_error(error: &str) {
    let _ = crate::paths::spawn_reaped(
        Command::new("notify-send")
            .args(["Boltsnap replay", error])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    );
}

pub struct FrozenSelection {
    id: u64,
    session: String,
    pub output: String,
    pub preview: Option<image::RgbaImage>,
    width: u32,
    height: u32,
}

pub struct PreviewTarget {
    pub output: String,
    session: String,
}

/// Only fetch routing metadata on the UI startup path, never a decoded frame.
pub fn preview_target() -> Option<PreviewTarget> {
    let mut stream = UnixStream::connect_addr(&socket::address("control").ok()?).ok()?;
    socket::configure(&stream).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .ok()?;
    wire::write(&mut stream, &json!({"version":1,"command":"status"})).ok()?;
    let response = wire::read(&mut stream).ok()?;
    if response["ok"] != true || response["ready"] != true || response["busy"] == true {
        return None;
    }
    Some(PreviewTarget {
        output: response["output"].as_str()?.into(),
        session: response["session"].as_str()?.into(),
    })
}

impl FrozenSelection {
    pub fn prepare(target: &PreviewTarget) -> Option<Self> {
        let (response, bytes) = socket_call(
            "control",
            &json!({"version":1,"command":"freeze","session":target.session}),
        )
        .ok()?;
        let mut selection = Self {
            id: response["snapshot"].as_u64()?,
            session: response["session"].as_str()?.into(),
            output: response["output"].as_str()?.into(),
            preview: None,
            width: u32::try_from(response["width"].as_u64()?).ok()?,
            height: u32::try_from(response["height"].as_u64()?).ok()?,
        };
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(selection.width);
        limits.max_image_height = Some(selection.height);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let preview = reader.decode().ok()?.into_rgba8();
        if preview.dimensions() != (selection.width, selection.height) {
            return None;
        }
        selection.preview = Some(preview);
        Some(selection)
    }

    pub fn save(
        &self,
        rect: crate::selector::edit::Rect,
        (width, height): (u32, u32),
    ) -> Result<(), String> {
        let crop = boltsnap::replay::crop::Crop::from_selection(
            [rect.x, rect.y, rect.w, rect.h],
            (width, height),
            (self.width, self.height),
        )?;
        call(
            &json!({"version":1,"command":"save","snapshot":self.id,"session":self.session,
            "crop":{"x":crop.x,"y":crop.y,"width":crop.width,"height":crop.height}}),
        )?;
        Ok(())
    }
}

impl Drop for FrozenSelection {
    fn drop(&mut self) {
        // Cancellation must not start a daemon or wait for the worker's reply
        // when the user presses REC/Escape. The supervisor processes the request
        // even after this connection closes.
        if let Ok(address) = socket::address("control")
            && let Ok(mut stream) = UnixStream::connect_addr(&address)
        {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
            let _ = wire::write(
                &mut stream,
                &json!({"version":1,"command":"cancel","snapshot":self.id,"session":self.session}),
            );
        }
    }
}

/// How often the watchdog looks at a running session, and how many consecutive
/// unready looks it takes to call the capture dead. The recorder can lose its
/// video stream while its process and its audio stream keep going, and a buffer
/// in that state can never produce a clip again. Nothing downstream notices, so
/// the service has to: it replaces the session the way a manual Start would.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const WATCHDOG_STRIKES: u32 = 3;

fn watchdog(
    owner: Arc<Mutex<Option<Session>>>,
    stopping: Arc<AtomicBool>,
    cancel_start: Arc<AtomicBool>,
    starting: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut strikes = 0;
        while !stopping.load(Ordering::Relaxed) {
            std::thread::sleep(WATCHDOG_INTERVAL);
            // Never interrupt a start in flight, and never take the buffer away
            // from a selection or an export that is still using it.
            if starting.load(Ordering::Relaxed) {
                strikes = 0;
                continue;
            }
            let Some(name) = owner.lock().unwrap().as_ref().map(|s| s.name.clone()) else {
                strikes = 0;
                continue;
            };
            let healthy = socket_call(&name, &json!({"version":1,"command":"status"}))
                .is_ok_and(|(status, _)| status["ready"] == true || status["busy"] == true);
            if healthy {
                strikes = 0;
                continue;
            }
            strikes += 1;
            if strikes < WATCHDOG_STRIKES {
                continue;
            }
            strikes = 0;
            let settings = match crate::config::Config::replay_settings() {
                Ok(settings) => settings,
                Err(_) => continue,
            };
            let mut guard = owner.lock().unwrap();
            // Only replace the session the strikes were counted against.
            if guard.as_ref().is_none_or(|s| s.name != name) {
                continue;
            }
            let dead = guard.take();
            starting.store(true, Ordering::Relaxed);
            cancel_start.store(false, Ordering::Relaxed);
            drop(guard);
            // Terminating the old pair can take a moment; the lock is already
            // released so status and stop stay answerable meanwhile.
            drop(dead);
            let restarted = start(settings, cancel_start.clone());
            let mut guard = owner.lock().unwrap();
            starting.store(false, Ordering::Relaxed);
            if cancel_start.load(Ordering::Relaxed) {
                continue;
            }
            match restarted {
                Ok(session) => *guard = Some(session),
                Err(error) => {
                    drop(guard);
                    notify_error(&format!(
                        "replay capture stopped and could not restart: {error}"
                    ));
                }
            }
        }
    });
}

/// Where GPU Screen Recorder's diagnostics go. Inheriting the daemon's stderr
/// sends them to `/dev/null`, and then a capture that stops mid-session leaves
/// nothing to look at. One file, truncated per start, so it cannot grow.
fn capture_log() -> Stdio {
    std::fs::File::create(crate::paths::cache_dir().join("replay-capture.log"))
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

fn companion_path(name: &str) -> PathBuf {
    let adjacent = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(name)));
    adjacent
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn start(settings: Settings, cancel: Arc<AtomicBool>) -> Result<Session, String> {
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
    let info = process::output(
        Command::new("hyprctl").args(["-j", "monitors"]),
        Duration::from_secs(3),
    )?;
    if !info.status.success() {
        return Err("could not query replay monitor".into());
    }
    let monitors = crate::record::parse_hyprland_monitors(&info.stdout)?;
    let output = settings.output.or_else(|| {
        match crate::config::Config::load()
            .recording_prefs()
            .default_target
        {
            crate::config::RecordDefaultTarget::Output(name) => Some(name),
            _ => None,
        }
    });
    let monitor = monitors
        .iter()
        .find(|m| output.as_ref().map_or(m.focused, |o| &m.name == o))
        .ok_or("configured replay monitor is unavailable")?;
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
    let candidates = if settings.encoder == "auto" {
        vec!["h264_nvenc", "h264_vaapi", "h264_vulkan"]
    } else {
        vec![settings.encoder.as_str()]
    };
    let mut selected = None;
    let mut errors = Vec::new();
    for encoder in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Err("replay start cancelled".into());
        }
        if !encoders.iter().any(|e| e == encoder) {
            continue;
        }
        let probe = process::output(
            Command::new(&worker).args(["check-encoder", encoder]),
            Duration::from_secs(10),
        )?;
        if probe.status.success() {
            selected = Some(encoder.to_owned());
            break;
        }
        errors.push(
            String::from_utf8_lossy(&probe.stderr)
                .chars()
                .take(512)
                .collect::<String>(),
        );
    }
    let encoder = selected.ok_or_else(|| {
        format!(
            "no usable requested hardware encoder: {}",
            errors.join("; ")
        )
    })?;
    let capture_codec = match encoder.as_str() {
        "h264_nvenc" | "h264_vaapi" => "h264",
        "hevc_nvenc" | "hevc_vaapi" => "hevc",
        "av1_nvenc" | "av1_vaapi" => "av1",
        "h264_vulkan" | "hevc_vulkan" | "av1_vulkan" => &encoder,
        other => {
            return Err(format!(
                "{other} is installed, but has no compatible GPU Screen Recorder adapter"
            ));
        }
    };
    let directory = crate::paths::rec_dir();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    crate::record::finalize::check_recording_cache_limit(&directory)?;
    crate::record::finalize::check_recording_reserve(&directory)?;
    let mut command = Command::new(companion_path("gpu-screen-recorder"));
    command
        .args([
            "-w",
            &monitor.name,
            "-f",
            &settings.fps.to_string(),
            "-fm",
            "cfr",
            "-k",
            capture_codec,
            // The frozen selector is decoded from this stream too. Keep text
            // legible; the cheaper medium preset visibly degrades the preview.
            "-q",
            "very_high",
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
            "bf=0;flags=+cgop",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(capture_log());
    if cancel.load(Ordering::Relaxed) {
        return Err("replay start cancelled".into());
    }
    let mut capture = process::spawn(&mut command, None)
        .map_err(|e| format!("GPU Screen Recorder unavailable: {e}"))?;
    let input = capture.stdout.take().ok_or("capture pipe missing")?;
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
            .stderr(Stdio::inherit()),
        None,
    );
    let worker = match child {
        Ok(worker) => worker,
        Err(e) => {
            process::terminate(&mut capture);
            return Err(format!("start replay worker: {e}"));
        }
    };
    let mut session = Session {
        capture,
        worker,
        name,
        output: monitor.name.clone(),
        output_limit: (settings.memory_mib * 1024 * 1024 / 3) as u64,
        parent_thread: None,
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

pub(crate) fn serve(sender: calloop::channel::Sender<DaemonEvent>) -> Result<Service, String> {
    let listener = socket::bind("control").map_err(|e| format!("replay control socket: {e}"))?;
    let service = Service {
        session: Arc::new(Mutex::new(None)),
        stopping: Arc::new(AtomicBool::new(false)),
        cancel_start: Arc::new(AtomicBool::new(false)),
    };
    let owner = service.session.clone();
    let stopping = service.stopping.clone();
    let cancel_start = service.cancel_start.clone();
    let starting = Arc::new(AtomicBool::new(false));
    watchdog(
        service.session.clone(),
        service.stopping.clone(),
        service.cancel_start.clone(),
        starting.clone(),
    );
    std::thread::spawn(move || {
        let active = Arc::new(AtomicUsize::new(0));
        for connection in listener.incoming() {
            if stopping.load(Ordering::Relaxed) {
                break;
            }
            let Ok(mut stream) = connection else {
                continue;
            };
            if socket::configure(&stream).is_err() || active.load(Ordering::Relaxed) >= 8 {
                continue;
            }
            active.fetch_add(1, Ordering::Relaxed);
            let active = active.clone();
            let owner = owner.clone();
            let sender = sender.clone();
            let starting = starting.clone();
            let cancel_start = cancel_start.clone();
            std::thread::spawn(move || {
                let result = (|| {
                    let request = wire::read(&mut stream).map_err(|e| e.to_string())?;
                    let command = wire::command(&request)?;
                    if command == "start" {
                        let mut guard = owner.lock().unwrap();
                        if starting.load(Ordering::Relaxed) {
                            return Err("replay is already starting or stopping".into());
                        }
                        if let Some(session) = guard.as_mut() {
                            let capture_alive = session
                                .capture
                                .try_wait()
                                .map_err(|e| e.to_string())?
                                .is_none();
                            let worker_alive = session
                                .worker
                                .try_wait()
                                .map_err(|e| e.to_string())?
                                .is_none();
                            // Two live processes are not proof of a live
                            // capture. The recorder can keep its audio stream
                            // going after its video stops, and then the buffer
                            // is stalled for good: no clip can ever come out of
                            // it, and refusing to start would leave no way back
                            // except Stop. Only a session the worker still
                            // calls ready blocks a restart.
                            let capturing = capture_alive
                                && worker_alive
                                && socket_call(
                                    &session.name,
                                    &json!({"version":1,"command":"status"}),
                                )
                                .is_ok_and(|(status, _)| status["ready"] == true);
                            if capturing {
                                return Err("replay is already running".into());
                            }
                            guard.take();
                        }
                        let settings = crate::config::Config::replay_settings()?;
                        starting.store(true, Ordering::Relaxed);
                        cancel_start.store(false, Ordering::Relaxed);
                        drop(guard);
                        let result = start(settings, cancel_start.clone());
                        let mut guard = owner.lock().unwrap();
                        starting.store(false, Ordering::Relaxed);
                        if cancel_start.load(Ordering::Relaxed) {
                            return Err("replay start cancelled".into());
                        }
                        *guard = Some(result?);
                        return Ok((json!({"ok":true}), Vec::new()));
                    }
                    if command == "stop" {
                        let session = {
                            let mut owner = owner.lock().unwrap();
                            cancel_start.store(true, Ordering::Relaxed);
                            owner.take()
                        };
                        drop(session);
                        return Ok((json!({"ok":true}), Vec::new()));
                    }
                    let (name, output, output_limit) = {
                        let owner = owner.lock().unwrap();
                        if command == "status" && owner.is_none() {
                            return Ok((
                                json!({"ok":true,"ready":false,"state":if starting.load(Ordering::Relaxed) {"starting"} else {"stopped"}}),
                                Vec::new(),
                            ));
                        }
                        let session = owner
                            .as_ref()
                            .ok_or("replay is stopped; start it from the tray first")?;
                        (
                            session.name.clone(),
                            session.output.clone(),
                            session.output_limit,
                        )
                    };
                    if request.get("session").is_some()
                        && request["session"].as_str() != Some(&name)
                    {
                        return Err("replay session changed; open the selector again".into());
                    }
                    if command == "save" {
                        crate::record::finalize::check_recording_cache_capacity(
                            &crate::paths::rec_dir(),
                            output_limit,
                        )?;
                        crate::record::finalize::check_recording_reserve(&crate::paths::rec_dir())?;
                    }
                    let (mut response, preview) = socket_call(&name, &request)?;
                    response["output"] = json!(output);
                    response["session"] = json!(name);
                    if command == "status" {
                        // A running session whose video has stopped arriving is
                        // not the same as a healthy one; say so, because every
                        // control that needs the buffer silently goes dead.
                        response["state"] = json!(if response["ready"] == true {
                            "running"
                        } else {
                            "stalled"
                        });
                    }
                    if command == "save" {
                        let bytes = response["path_bytes"]
                            .as_array()
                            .ok_or("clip response has no native path")?
                            .iter()
                            .map(|v| {
                                v.as_u64()
                                    .and_then(|n| u8::try_from(n).ok())
                                    .ok_or("invalid clip path byte")
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
                        if path.parent() != Some(crate::paths::rec_dir().as_path())
                            || path.extension().is_none_or(|ext| ext != "mkv")
                        {
                            return Err("clip response is outside the recording cache".into());
                        }
                        let (tx, rx) = std::sync::mpsc::sync_channel(1);
                        sender
                            .send(DaemonEvent::ReplayClipReady {
                                path,
                                output,
                                reply: tx,
                            })
                            .map_err(|e| e.to_string())?;
                        rx.recv_timeout(Duration::from_secs(5))
                            .map_err(|_| "shelf did not acknowledge the clip")?;
                    }
                    Ok((response, preview))
                })();
                let (value, preview) = result.unwrap_or_else(|error: String| {
                    (json!({"ok":false,"error":error}), Vec::new())
                });
                if wire::write(&mut stream, &value)
                    .and_then(|_| stream.write_all(&preview))
                    .is_err()
                    && let (Some(id), Some(name)) =
                        (value["snapshot"].as_u64(), value["session"].as_str())
                {
                    let _ =
                        socket_call(name, &json!({"version":1,"command":"cancel","snapshot":id}));
                }
                active.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    if crate::config::Config::replay_settings().is_ok_and(|s| s.autostart) {
        std::thread::spawn(|| {
            if let Err(error) = socket_call("control", &json!({"version":1,"command":"start"})) {
                eprintln!("boltsnap replay autostart: {error}");
            }
        });
    }
    Ok(service)
}
