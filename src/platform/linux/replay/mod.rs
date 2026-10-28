use std::io::Write;
mod capture_log;
mod card;
mod cursor;
mod monitor;
pub(crate) mod process;
mod socket;
mod watchdog;

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
use super::timing;

struct Session {
    capture: Child,
    worker: Child,
    /// Feed the smooth-cursor plugin in `capture`; stop with the session.
    #[allow(dead_code)]
    cursor: Vec<super::cursor_track::Tracker>,
    name: String,
    output: String,
    /// With `[replay] output = "both"`: where each monitor lies on the
    /// canvas; empty for one monitor.
    layout: Vec<monitor::Placed>,
    /// `[replay] output` as it was when the session started (it may capture
    /// another display, with a notice): what the settings window compares.
    asked: Option<String>,
    output_limit: u64,
    parent_thread: Option<std::sync::mpsc::Sender<()>>,
    /// Why the session captures another monitor than configured; the caller
    /// reports it once.
    notice: Option<String>,
}

impl Session {
    /// Ends the capture and keeps the worker, whose ring stays saveable.
    fn stop_capture(&mut self) {
        process::terminate(&mut self.capture);
        self.cursor.clear();
    }
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

/// The longest a clip export may take, region crops included.
const SAVE_TIMEOUT: Duration = Duration::from_secs(1900);

fn socket_call(name: &str, request: &Value) -> Result<(Value, Vec<u8>), String> {
    let mut stream = UnixStream::connect_addr(&socket::address(name).map_err(|e| e.to_string())?)
        .map_err(|e| format!("replay is not running: {e}"))?;
    socket::configure(&stream).map_err(|e| e.to_string())?;
    wire::write(&mut stream, request).map_err(|e| e.to_string())?;
    let timeout = match request["command"].as_str() {
        Some("save") => SAVE_TIMEOUT,
        Some("start") => Duration::from_secs(120),
        Some("freeze") => Duration::from_secs(15),
        _ => Duration::from_secs(5),
    };
    stream
        .set_read_timeout(Some(timeout))
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
    if response["input_lost"] == true {
        notify_error("capture had stopped; the clip ends where it stopped");
    }
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
    /// Where the selector's monitor lies on a both-displays canvas; a crop
    /// of it moves by this much (zero for one monitor).
    offset: (u32, u32),
}

use crate::selector::clip::ClipState;

pub struct PreviewTarget {
    pub output: String,
    session: String,
    /// With both displays: where each lies on the canvas.
    layout: Vec<monitor::Placed>,
}

impl PreviewTarget {
    /// The monitor the selector opens on: the buffer's own, or with both
    /// displays `None`, the focused one like any recording.
    pub fn monitor(&self) -> Option<&str> {
        self.layout.is_empty().then_some(self.output.as_str())
    }
}

/// The replay control's status, asked directly with short timeouts: never
/// starts the daemon, `None` when nothing answers.
pub fn status_quiet() -> Option<Value> {
    let mut stream = UnixStream::connect_addr(&socket::address("control").ok()?).ok()?;
    socket::configure(&stream).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .ok()?;
    wire::write(&mut stream, &json!({"version":1,"command":"status"})).ok()?;
    wire::read(&mut stream).ok()
}

/// Only fetch routing metadata on the UI startup path, never a decoded frame.
/// Without a buffer to clip from, why not, for the selector's Clip.
pub fn preview_target() -> Result<PreviewTarget, ClipState> {
    preview_target_from(status_quiet().as_ref())
}

fn preview_target_from(status: Option<&Value>) -> Result<PreviewTarget, ClipState> {
    let Some(response) = status else {
        // Nothing answers: no daemon, so no buffer either.
        return Err(ClipState::Off);
    };
    if response["ok"] != true {
        return Err(ClipState::Failed);
    }
    match response["state"].as_str() {
        Some("stopped") => return Err(ClipState::Off),
        Some("starting") => return Err(ClipState::Starting),
        _ => {}
    }
    if response["busy"] == true {
        return Err(ClipState::Busy);
    }
    if response["ready"] != true {
        return Err(ClipState::Stalled);
    }
    let field = |name: &str| response[name].as_str().map(String::from);
    match (field("output"), field("session")) {
        (Some(output), Some(session)) => {
            let layout = match response["layout"].as_array() {
                None => Vec::new(),
                Some(values) => values
                    .iter()
                    .map(monitor::Placed::from_value)
                    .collect::<Option<Vec<_>>>()
                    .ok_or(ClipState::Failed)?,
            };
            Ok(PreviewTarget {
                output,
                session,
                layout,
            })
        }
        _ => Err(ClipState::Failed),
    }
}

impl FrozenSelection {
    /// Freeze the buffer for a selector on monitor `on`; with both displays
    /// the frame is cut to that monitor's part of the canvas.
    pub fn prepare(target: &PreviewTarget, on: Option<&str>) -> Option<Self> {
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
            offset: (0, 0),
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
        if target.layout.is_empty() {
            selection.preview = Some(preview);
            return Some(selection);
        }
        let part = target.layout.iter().find(|p| Some(p.name.as_str()) == on)?;
        if part.x + part.width > selection.width || part.y + part.height > selection.height {
            return None;
        }
        selection.preview = Some(
            image::imageops::crop_imm(&preview, part.x, part.y, part.width, part.height).to_image(),
        );
        selection.output = part.name.clone();
        (selection.width, selection.height) = (part.width, part.height);
        selection.offset = (part.x, part.y);
        Some(selection)
    }

    /// The frozen frame's pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
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
        let (dx, dy) = self.offset;
        call(
            &json!({"version":1,"command":"save","snapshot":self.id,"session":self.session,
            "output":self.output,
            "crop":{"x":crop.x + dx,"y":crop.y + dy,"width":crop.width,"height":crop.height}}),
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

/// An export or an open selection still uses the buffer, even after its input
/// failed. Older workers lack `selection_active`.
fn holds_buffer(status: &Value) -> bool {
    status["busy"] == true || status["selection_active"] == true
}

fn in_use(status: &Value) -> bool {
    status["ready"] == true || holds_buffer(status)
}

fn session_in_use(name: &str) -> bool {
    socket_call(name, &json!({"version":1,"command":"status"}))
        .is_ok_and(|(status, _)| in_use(&status))
}
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
    let video_kbps = settings.video_kbps().to_string();
    let info = super::hypr::json("monitors").ok_or("could not query replay monitor")?;
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
    let gsr = super::gsr::info()?;
    let capture = super::gsr::choose(&settings.encoder, &gsr)?;
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
            // A constant bitrate sized to the buffer keeps RAM and history
            // predictable; constant quality grows with motion and silently
            // shortens the clip once the byte budget evicts instead.
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
            "bf=0;flags=+cgop",
        ])
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
    watchdog::spawn(
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
                // The save whose card shows already; its failure takes it.
                let mut pending = None;
                let result = (|| {
                    let mut request = wire::read(&mut stream).map_err(|e| e.to_string())?;
                    let command = wire::command(&request)?.to_owned();
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
                            // calls ready blocks a restart, and an open
                            // selection or export must end first.
                            let status = worker_alive
                                .then(|| {
                                    socket_call(
                                        &session.name,
                                        &json!({"version":1,"command":"status"}),
                                    )
                                    .ok()
                                })
                                .flatten()
                                .map(|(status, _)| status);
                            if capture_alive
                                && status
                                    .as_ref()
                                    .is_some_and(|status| status["ready"] == true)
                            {
                                return Err("replay is already running".into());
                            }
                            if status.as_ref().is_some_and(holds_buffer) {
                                return Err(
                                    "a replay selection or export is still open; save or cancel it first"
                                        .into(),
                                );
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
                        let mut session = result?;
                        let notice = session.notice.take();
                        *guard = Some(session);
                        drop(guard);
                        if let Some(notice) = notice {
                            notify_error(&notice);
                        }
                        // The tray's checkmark follows, whoever asked.
                        let _ = sender.send(DaemonEvent::ReplayStateChanged);
                        return Ok((json!({"ok":true}), Vec::new()));
                    }
                    if command == "stop" {
                        let session = {
                            let mut owner = owner.lock().unwrap();
                            cancel_start.store(true, Ordering::Relaxed);
                            owner.take()
                        };
                        drop(session);
                        // The tray's checkmark follows, whoever asked.
                        let _ = sender.send(DaemonEvent::ReplayStateChanged);
                        return Ok((json!({"ok":true}), Vec::new()));
                    }
                    let (name, output, output_limit, layout, asked) = {
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
                            session.layout.clone(),
                            session.asked.clone(),
                        )
                    };
                    if request.get("session").is_some()
                        && request["session"].as_str() != Some(&name)
                    {
                        return Err("replay session changed; open the selector again".into());
                    }
                    if command == "save" {
                        timing::mark("replay_save_accepted");
                        crate::record::finalize::check_recording_cache_capacity(
                            &crate::paths::rec_dir(),
                            output_limit,
                        )?;
                        crate::record::finalize::check_recording_reserve(&crate::paths::rec_dir())?;
                        // Both displays: a full-screen save is the monitor
                        // under the pointer, cut from the canvas.
                        if !layout.is_empty() && request["crop"].is_null() {
                            let focused = super::shelf::focused_monitor_name();
                            let monitor = monitor::chosen(&layout, focused.as_deref())
                                .ok_or("replay has no display to clip")?;
                            request["crop"] = monitor.crop_value();
                            request["output"] = json!(monitor.name);
                        }
                        let token = card::token();
                        let thumb = card::picture(&request, super::shelf::card_size().dims());
                        // The card goes to the monitor the clip shows.
                        let shows = request["output"].as_str().unwrap_or(&output);
                        let _ = sender.send(DaemonEvent::ReplayClipPending {
                            token,
                            output: shows.to_owned(),
                            thumb,
                        });
                        pending = Some(token);
                    }
                    if command == "cancel"
                        && let Some(snapshot) = request["snapshot"].as_u64()
                    {
                        card::forget(snapshot);
                    }
                    let (mut response, preview) = socket_call(&name, &request)?;
                    if command == "freeze"
                        && let Some(snapshot) = response["snapshot"].as_u64()
                    {
                        card::remember(snapshot, &preview);
                    }
                    response["output"] = json!(output);
                    response["asked"] = json!(asked);
                    response["session"] = json!(name);
                    if !layout.is_empty() {
                        response["layout"] = layout.iter().map(monitor::Placed::to_value).collect();
                    }
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
                        timing::mark("replay_worker_answered");
                        for stage in ["export_ms", "sync_ms"] {
                            if let Some(ms) = response[stage].as_u64() {
                                timing::reported(stage, ms);
                            }
                        }
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
                            // Older workers still publish Matroska clips.
                            || path.extension().is_none_or(|ext| ext != "mp4" && ext != "mkv")
                        {
                            return Err("clip response is outside the recording cache".into());
                        }
                        let (tx, rx) = std::sync::mpsc::sync_channel(1);
                        sender
                            .send(DaemonEvent::ReplayClipReady {
                                token: pending.take().ok_or("clip without its card")?,
                                path,
                                output,
                                reply: tx,
                            })
                            .map_err(|e| e.to_string())?;
                        rx.recv_timeout(Duration::from_secs(5))
                            .map_err(|_| "shelf did not acknowledge the clip")?;
                        // The shelf answers after it drew the card.
                        timing::mark("replay_card_drawn");
                    }
                    Ok((response, preview))
                })();
                if let (Err(_), Some(token)) = (&result, pending) {
                    let _ = sender.send(DaemonEvent::ReplayClipFailed { token });
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_displays_open_the_selector_where_the_pointer_is() {
        let running = |extra: Value| {
            let mut status = json!({"ok":true,"ready":true,"busy":false,"state":"running",
                "output":"DP-3","session":"worker-1"});
            for (key, value) in extra.as_object().unwrap() {
                status[key] = value.clone();
            }
            preview_target_from(Some(&status))
        };
        // One monitor: the selector opens on the buffer's.
        let one = running(json!({})).ok().unwrap();
        assert_eq!(one.monitor(), Some("DP-3"));
        // Both: the focused one, and each monitor's place on the canvas.
        let both = running(json!({"output":"both","layout":[
            {"name":"DP-3","x":0,"y":0,"width":1920,"height":1080},
            {"name":"DP-1","x":1920,"y":0,"width":1920,"height":1080}]}))
        .ok()
        .unwrap();
        assert_eq!(both.monitor(), None);
        assert_eq!(both.layout[1].x, 1920);
        // A full-screen save takes the focused monitor's part, else the first.
        assert_eq!(monitor::chosen(&both.layout, Some("DP-1")).unwrap().x, 1920);
        assert_eq!(
            monitor::chosen(&both.layout, Some("HDMI-A-1"))
                .unwrap()
                .name,
            "DP-3"
        );
        assert_eq!(monitor::chosen(&both.layout, None).unwrap().name, "DP-3");
        assert_eq!(
            monitor::chosen(&both.layout, Some("DP-1"))
                .unwrap()
                .crop_value(),
            json!({"x":1920,"y":0,"width":1920,"height":1080})
        );
        // A layout the selector cannot read is no target.
        assert_eq!(
            running(json!({"layout":[{"name":"DP-3"}]})).err(),
            Some(ClipState::Failed)
        );
    }

    #[test]
    fn clip_says_why_the_buffer_is_not_there() {
        let reason = |status: Option<Value>| preview_target_from(status.as_ref()).err();
        assert_eq!(reason(None), Some(ClipState::Off));
        assert_eq!(
            reason(Some(json!({"ok":true,"ready":false,"state":"stopped"}))),
            Some(ClipState::Off)
        );
        assert_eq!(
            reason(Some(json!({"ok":true,"ready":false,"state":"starting"}))),
            Some(ClipState::Starting)
        );
        assert_eq!(
            reason(Some(json!({"ok":true,"ready":false,"state":"stalled"}))),
            Some(ClipState::Stalled)
        );
        assert_eq!(
            reason(Some(
                json!({"ok":true,"ready":true,"busy":true,"state":"running"})
            )),
            Some(ClipState::Busy)
        );
        assert_eq!(
            reason(Some(json!({"ok":false,"error":"x"}))),
            Some(ClipState::Failed)
        );
        assert_eq!(
            reason(Some(json!({"ok":true,"ready":true,"state":"running"}))),
            Some(ClipState::Failed),
            "a running buffer without routing is not one to clip from"
        );
        let target = preview_target_from(Some(&json!({"ok":true,"ready":true,"busy":false,
            "state":"running","output":"DP-1","session":"s1"})))
        .unwrap();
        assert_eq!(
            (target.output.as_str(), target.session.as_str()),
            ("DP-1", "s1")
        );
    }

    #[test]
    fn a_held_selection_keeps_a_stalled_session() {
        let status = |ready, busy, selection: Option<bool>| {
            let mut status = json!({"ready": ready, "busy": busy});
            if let Some(selection) = selection {
                status["selection_active"] = json!(selection);
            }
            status
        };
        // G5: input gone, preview done, selection still open.
        assert!(in_use(&status(false, false, Some(true))));
        assert!(holds_buffer(&status(false, false, Some(true))));
        assert!(in_use(&status(true, false, Some(false))));
        assert!(!holds_buffer(&status(true, false, Some(false))));
        assert!(holds_buffer(&status(false, true, None)));
        // A stalled session without a selection may be replaced; an older
        // worker without the field reads as no selection.
        assert!(!in_use(&status(false, false, Some(false))));
        assert!(!in_use(&status(false, false, None)));
    }
}
