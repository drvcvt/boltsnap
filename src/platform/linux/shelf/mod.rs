mod font;

use std::os::unix::net::{UnixListener, UnixStream};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    data_device_manager::{
        DataDeviceManagerState, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{DataSourceHandler, DragSource},
    },
    delegate_compositor, delegate_data_device, delegate_keyboard, delegate_layer, delegate_output,
    delegate_pointer, delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{
            BTN_LEFT, BTN_RIGHT, CursorIcon, PointerEvent, PointerEventKind, PointerHandler,
            ThemeSpec, ThemedPointer,
        },
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_source::WlDataSource, wl_keyboard::WlKeyboard, wl_output, wl_pointer::WlPointer,
        wl_seat::WlSeat, wl_surface::WlSurface,
    },
};

use crate::DynResult;
use crate::record::audio::AudioCapture;
use crate::record::finalize::{
    FinalizeFailure, FinalizeRequest, FinalizedClip, SaveDestination, check_recording_cache_limit,
    check_recording_reserve, finalize_recording, promote_recording,
};
use crate::record::session::{
    CaptureScope, PublicRecordingState, RecorderTools, RecordingAction, RecordingSession,
    SessionPhase, StopChildrenJob, StopChildrenResult, StoppedSegment, spawn_segment, start_plan,
};
use crate::shelf::layout::{Hit, Layout, LayoutConfig, ThumbRect};
use crate::shelf::model::{CardKind, FileLifetime, ShelfModel, Thumb};
use crate::shelf::recording::{POPUP_H, POPUP_W, PopupButton};

pub struct Daemon {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    layer_shell: LayerShell,
    /// The shelf surface. Recreated when the focused output changes so each
    /// capture's thumbnail appears on the monitor the user is actually on.
    layer: Option<LayerSurface>,
    /// Name of the output the current `layer` lives on (Hyprland monitor name).
    output_name: Option<String>,
    /// `wlr-layer-shell` forbids attaching a buffer before the first configure.
    shelf_configured: bool,
    shelf_pending_draw: bool,
    /// Themed pointer so we can set an explicit cursor over the shelf instead of
    /// inheriting whatever shape the previously-focused window left (e.g. a
    /// terminal's I-beam).
    pointer: Option<ThemedPointer>,
    keyboard: Option<WlKeyboard>,

    ddm: DataDeviceManagerState,
    data_device: Option<DataDevice>,
    drag_source: Option<DragSource>,
    drag_path: Option<std::path::PathBuf>,
    drop_ok: bool,
    icon_surface: Option<WlSurface>,
    /// Dedicated pool kept alive for the whole drag so the shelf's `pool` can't
    /// reuse the icon's slot and turn it into a "ghost".
    drag_icon_pool: Option<SlotPool>,
    /// The icon's `Buffer` wrapper, retained for the whole drag. Dropping it lets
    /// SCTK destroy the underlying `wl_buffer` mid-drag (the icon then vanishes,
    /// especially when the cursor crosses to another output) — so we hold it
    /// until the drag ends.
    drag_icon_buffer: Option<Buffer>,

    model: ShelfModel,
    layout: Layout,
    cfg: LayoutConfig,
    width: u32,
    height: u32,
    hovered: Option<u64>,
    /// Directory the Save button writes to (config/flag/default).
    save_dir: std::path::PathBuf,
    /// Card id + start time of the transient ✓ "saved" flash on its Save button.
    save_flash: Option<(u64, std::time::Instant)>,
    saving_cards: std::collections::HashSet<u64>,
    press: Option<PressState>,
    /// In-flight per-card appear/dismiss animations.
    anims: Vec<CardAnim>,
    exit: bool,
    /// Queue handle stashed for use inside calloop callbacks (which get only `&mut Daemon`).
    qh: Option<QueueHandle<Daemon>>,
    recording: Option<RecordingSession>,
    marker: Option<LayerSurface>,
    marker_pool: Option<SlotPool>,
    marker_region: Option<Region>,
    marker_configured: bool,
    popup: Option<LayerSurface>,
    popup_pool: Option<SlotPool>,
    popup_font: ab_glyph::FontVec,
    popup_configured: bool,
    watchers: Vec<UnixStream>,
    last_recording_snapshot: crate::ipc::RecordingSnapshot,
    last_recording_space_check: std::time::Instant,
    event_tx: calloop::channel::Sender<DaemonEvent>,
    focused_output: Option<String>,
    focus_query_pending: bool,
    pending_controls: Vec<UnixStream>,
    pending_default_recordings: Vec<UnixStream>,
    pending_tray_default_recording: bool,
    recording_prefs: crate::config::RecordingPrefs,
    persisted_recording_prefs: crate::config::RecordingPrefs,
    prefs_generation: u64,
    persisted_prefs_generation: u64,
    prefs_writer: PrefsWriter,
    tray: Option<crate::tray::TrayPublisher>,
}

pub(crate) enum AfterStop {
    Pause,
    Recover(String),
    Save(SaveDestination),
    Discard,
}

pub(crate) enum DaemonEvent {
    ClientRequest {
        request: crate::ipc::Request,
        stream: UnixStream,
    },
    ChildrenStopped {
        after: AfterStop,
        result: StopChildrenResult,
    },
    Finalized(Result<Vec<FinalizedClip>, FinalizeFailure>),
    CardPromoted {
        id: u64,
        result: Result<std::path::PathBuf, String>,
    },
    FocusResolved(Result<Vec<crate::record::Monitor>, String>),
    Tray(crate::tray::TrayAction),
    PrefsPersisted {
        generation: u64,
        prefs: crate::config::RecordingPrefs,
        error: Option<String>,
    },
}

#[derive(Clone)]
struct PrefsWrite {
    generation: u64,
    prefs: crate::config::RecordingPrefs,
}

struct PrefsWriter {
    latest: std::sync::Arc<crate::tray::LatestValue<PrefsWrite>>,
    wake: std::sync::mpsc::SyncSender<()>,
}

impl PrefsWriter {
    fn spawn(event_tx: calloop::channel::Sender<DaemonEvent>) -> Self {
        let latest = std::sync::Arc::new(crate::tray::LatestValue::<PrefsWrite>::new());
        let worker_latest = std::sync::Arc::clone(&latest);
        let (wake, wake_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while wake_rx.recv().is_ok() {
                let Some(write) = worker_latest.take() else {
                    continue;
                };
                let error = crate::config::save_recording_prefs(&write.prefs)
                    .err()
                    .map(|error| error.to_string());
                let _ = event_tx.send(DaemonEvent::PrefsPersisted {
                    generation: write.generation,
                    prefs: write.prefs,
                    error,
                });
            }
        });
        Self { latest, wake }
    }

    fn submit(&self, write: PrefsWrite) {
        self.latest.replace(write);
        let _ = self.wake.try_send(());
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PrefsCompletion {
    Persisted,
    Stale,
    Rollback,
}

fn prefs_completion(
    current_generation: u64,
    persisted_generation: u64,
    completed_generation: u64,
    failed: bool,
) -> PrefsCompletion {
    if failed {
        if completed_generation == current_generation {
            PrefsCompletion::Rollback
        } else {
            PrefsCompletion::Stale
        }
    } else if completed_generation > persisted_generation {
        PrefsCompletion::Persisted
    } else {
        PrefsCompletion::Stale
    }
}

/// In-flight left-button press, used to distinguish a click from a drag.
struct PressState {
    id: u64,
    hit: Hit,
    x: f64,
    y: f64,
    serial: u32,
    dragging: bool,
}

/// How long the ✓ "saved" flash stays on the Save button.
const SAVE_FLASH_MS: u128 = 700;
/// Card scale at the far end of the appear/dismiss animation.
const ANIM_SCALE_MIN: f32 = 0.88;

fn shelf_commit_allowed(drag_active: bool) -> bool {
    !drag_active
}

const MAX_VISIBLE_CARDS: usize = 5;

fn visible_cards(model: &ShelfModel) -> impl Iterator<Item = &Thumb> {
    model.newest_first().take(MAX_VISIBLE_CARDS)
}

fn ease_out_cubic(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

fn overflow_opacity(appear_progress: Option<f32>, disappear_progress: Option<f32>) -> Option<f32> {
    appear_progress
        .map(|progress| 1.0 - progress.clamp(0.0, 1.0))
        .or_else(|| disappear_progress.map(|progress| progress.clamp(0.0, 1.0)))
}

fn animated_card_layout(
    items: &[(u64, u32, u32, f32)],
    cfg: &LayoutConfig,
    appear_progress: Option<f32>,
) -> Layout {
    if items.is_empty() {
        return Layout::compute(&[], cfg);
    }
    let scroll = appear_progress
        .filter(|_| items.len() > MAX_VISIBLE_CARDS)
        .map(|progress| {
            ((items[0].2 + cfg.gap) as f32 * (1.0 - progress.clamp(0.0, 1.0))).round() as u32
        });
    let widest = items
        .iter()
        .take(MAX_VISIBLE_CARDS)
        .map(|(_, w, ..)| *w)
        .max()
        .unwrap_or(0);
    let mut thumbs = Vec::with_capacity(items.len());
    let mut y = cfg.pad;
    for (i, (id, w, h, collapse)) in items.iter().enumerate() {
        if i > 0 {
            let gap_collapse = collapse.max(if i == 1 { items[0].3 } else { 0.0 });
            y += (cfg.gap as f32 * (1.0 - gap_collapse)).round() as u32;
        }
        let height = (*h as f32 * (1.0 - collapse)).round() as u32;
        thumbs.push(ThumbRect {
            id: *id,
            x: cfg.pad,
            y: if i > 0 {
                y.saturating_sub(scroll.unwrap_or(0))
            } else {
                y
            },
            w: *w,
            h: height,
        });
        y += height;
    }
    let height = if items.len() > MAX_VISIBLE_CARDS {
        cfg.pad * 2
            + items
                .iter()
                .take(MAX_VISIBLE_CARDS)
                .map(|(_, _, h, _)| *h)
                .sum::<u32>()
            + cfg.gap * (MAX_VISIBLE_CARDS as u32 - 1)
    } else {
        y + cfg.pad
    };
    Layout {
        width: cfg.pad * 2 + widest,
        height,
        thumbs,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimKind {
    Appear,
    Disappear,
}

impl AnimKind {
    /// Per-kind duration in ms. The dismiss runs a touch longer than the appear
    /// so it reads as smooth rather than snappy.
    fn dur(self) -> u128 {
        match self {
            AnimKind::Appear => 150,
            AnimKind::Disappear => 240,
        }
    }
}

/// A running per-card animation. `start` is when it began; progress is
/// `elapsed / kind.dur()`, eased, until it reaches 1.0 and is retired.
struct CardAnim {
    id: u64,
    kind: AnimKind,
    start: std::time::Instant,
}

/// Best-effort desktop notification (no-op if `notify-send` is missing). Spawned
/// detached so it never blocks the daemon's event loop. Used to surface recording
/// failures that would otherwise only appear in the journal.
fn notify(body: &str) {
    if crate::paths::has_cmd("notify-send") {
        let _ = crate::paths::spawn_reaped(
            std::process::Command::new("notify-send")
                .arg("boltsnap")
                .arg(body)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        );
    }
}

fn prepare_recording_cache() -> Result<(), String> {
    let dir = crate::paths::rec_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("create recording cache: {error}"))?;
    check_recording_cache_limit(&dir)?;
    check_recording_reserve(&dir)
}

fn validate_recording_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        Err("recording width and height must be greater than zero".into())
    } else {
        Ok(())
    }
}

fn recording_action_label(action: RecordingAction) -> &'static str {
    match action {
        RecordingAction::Pause => "pause",
        RecordingAction::Resume => "resume",
        RecordingAction::SaveShelf => "save to shelf",
        RecordingAction::SaveDisk => "save to disk",
        RecordingAction::Discard => "discard",
    }
}

fn spawn_client_reader(mut stream: UnixStream, tx: calloop::channel::Sender<DaemonEvent>) {
    std::thread::spawn(move || {
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        match crate::ipc::Request::read(&mut stream) {
            Ok(request) => {
                let _ = tx.send(DaemonEvent::ClientRequest { request, stream });
            }
            Err(error) => eprintln!("boltsnap daemon: bad request: {error}"),
        }
    });
}

fn spawn_client_writer(mut stream: UnixStream, bytes: Vec<u8>) {
    std::thread::spawn(move || {
        use std::io::Write;
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
        let _ = stream.write_all(&bytes);
    });
}

fn parse_focus_snapshot(json: &[u8]) -> Result<Vec<crate::record::Monitor>, String> {
    crate::record::parse_hyprland_monitors(json)
}

fn focused_output_from_hyprland_json(json: &[u8]) -> Option<String> {
    parse_focus_snapshot(json)
        .ok()?
        .into_iter()
        .find(|monitor| monitor.focused)
        .map(|monitor| monitor.name)
}

fn query_focus_snapshot() -> Result<Vec<crate::record::Monitor>, String> {
    use std::process::{Command, Stdio};
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none()
        || !crate::paths::has_cmd("hyprctl")
    {
        return Err("Hyprland monitor query is unavailable".into());
    }
    let mut child = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("start Hyprland monitor query: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| format!("read Hyprland monitor query: {error}"))?;
                if !status.success() {
                    return Err(format!("Hyprland monitor query exited with {status}"));
                }
                return parse_focus_snapshot(&output.stdout);
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Hyprland monitor query timed out".into());
            }
        }
    }
}

fn spawn_focus_query(tx: calloop::channel::Sender<DaemonEvent>) {
    std::thread::spawn(move || {
        let _ = tx.send(DaemonEvent::FocusResolved(query_focus_snapshot()));
    });
}

fn default_start_plan(
    prefs: &crate::config::RecordingPrefs,
    query: &Result<Vec<crate::record::Monitor>, String>,
    wayland: &[crate::record::Monitor],
) -> Result<crate::record::session::StartPlan, String> {
    let mut monitors: Vec<_> = match query {
        Ok(fresh) => fresh
            .iter()
            .filter(|monitor| wayland.iter().any(|output| output.name == monitor.name))
            .cloned()
            .collect(),
        Err(_) => wayland
            .iter()
            .cloned()
            .map(|mut monitor| {
                monitor.focused = false;
                monitor
            })
            .collect(),
    };
    if query.is_err() {
        let needs_focused_fallback = match &prefs.default_target {
            crate::config::RecordDefaultTarget::Focused => true,
            crate::config::RecordDefaultTarget::Output(name) => {
                !monitors.iter().any(|monitor| monitor.name == *name)
            }
            crate::config::RecordDefaultTarget::Both => false,
        };
        if needs_focused_fallback && let Some(first) = monitors.first_mut() {
            first.focused = true;
        }
    }
    start_plan(prefs, &monitors).map_err(|error| match query {
        Ok(_) => error,
        Err(query_error) => format!("{query_error}: {error}"),
    })
}

fn resolve_output_name<'a>(requested: Option<&str>, available: &'a [String]) -> Option<&'a str> {
    requested
        .and_then(|requested| {
            available
                .iter()
                .find(|name| name.as_str() == requested)
                .map(String::as_str)
        })
        .or_else(|| available.first().map(String::as_str))
}

fn transformed_mode_size(
    (width, height): (i32, i32),
    transform: wl_output::Transform,
) -> (i32, i32) {
    if matches!(
        transform,
        wl_output::Transform::_90
            | wl_output::Transform::_270
            | wl_output::Transform::Flipped90
            | wl_output::Transform::Flipped270
    ) {
        (height, width)
    } else {
        (width, height)
    }
}

fn recording_controls_visible(phase: SessionPhase) -> bool {
    phase != SessionPhase::Discarding
}

fn monitor_for_geometry<'a>(
    monitors: &'a [crate::record::Monitor],
    geo: &crate::record::Geometry,
) -> Option<&'a crate::record::Monitor> {
    let x = i64::from(geo.x) + i64::from(geo.w / 2);
    let y = i64::from(geo.y) + i64::from(geo.h / 2);
    monitors.iter().find(|monitor| {
        let scale = monitor.scale.max(1.0);
        let width = (f64::from(monitor.width) / scale).round() as i64;
        let height = (f64::from(monitor.height) / scale).round() as i64;
        let left = i64::from(monitor.x);
        let top = i64::from(monitor.y);
        x >= left && x < left + width && y >= top && y < top + height
    })
}

fn spawn_recording_thumbnail(id: u64, video: std::path::PathBuf) {
    let thumb = crate::paths::rec_file("rec-thumb", "png");
    std::thread::spawn(move || {
        let _ = std::process::Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(&video)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(&thumb)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = crate::ipc::send_to_shelf(crate::ipc::Request::RecordingThumb { id, thumb });
    });
}

/// Name of the focused Hyprland monitor, via `hyprctl monitors -j`. `None` off
/// Hyprland (then the compositor places the shelf on its default output).
pub(crate) fn focused_monitor_name() -> Option<String> {
    use std::process::Command;
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none()
        || !crate::paths::has_cmd("hyprctl")
    {
        return None;
    }
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    focused_output_from_hyprland_json(&out.stdout)
}

/// Logical layout origin (`x`, `y`) of the focused Hyprland monitor, via
/// `hyprctl monitors -j`. Used to map a selection rect (overlay-output-local
/// logical px) into compositor-global coords for recording. `None` off Hyprland.
pub(crate) fn focused_monitor_origin() -> Option<(i32, i32)> {
    use std::process::Command;
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none()
        || !crate::paths::has_cmd("hyprctl")
    {
        return None;
    }
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    for m in v.as_array()? {
        if m.get("focused").and_then(|f| f.as_bool()) == Some(true) {
            let x = m.get("x").and_then(|n| n.as_i64())? as i32;
            let y = m.get("y").and_then(|n| n.as_i64())? as i32;
            return Some((x, y));
        }
    }
    None
}

/// Disable Hyprland's open/close animation for the shelf layer so thumbnails
/// appear and vanish instantly.
///
/// NOTE: `hyprctl keyword layerrule ...` is a SILENT no-op when Hyprland runs
/// the non-legacy (hyprlua) parser — it replies "keyword can't work with
/// non-legacy parsers" but exits 0. So a runtime push can't be relied on. We
/// best-effort push it here for legacy-parser setups, but the durable fix is a
/// static rule in the user's config (documented in the README): a windowrule
/// `noanim` for class `boltsnap-select` and a layerrule `noanim` for namespace
/// `boltsnap`. No-op off Hyprland.
fn prep_shelf_compositor_rules() {
    use std::process::{Command, Stdio};
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && crate::paths::has_cmd("hyprctl")
    {
        let _ = Command::new("hyprctl")
            .args(["keyword", "layerrule", "noanim, boltsnap"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn run_daemon(save_dir_cli: Option<std::path::PathBuf>) -> DynResult<()> {
    // Single-instance: if a daemon already answers, do nothing.
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    let sock = crate::ipc::socket_path();
    let _ = std::fs::remove_file(&sock); // clear stale socket

    // No daemon was running, so the shelf is empty: any leftover shelf tempfiles
    // are orphans from a previous run/crash. Remove them so the RAM-only shelf
    // doesn't leak PNGs to disk indefinitely (this is what filled /tmp).
    let cleaned = crate::paths::clean_orphan_shelf_temps();
    if cleaned > 0 {
        eprintln!("boltsnap daemon: cleaned {cleaned} orphaned shelf tempfile(s)");
    }
    let cleaned_rec = crate::paths::clean_orphan_rec_files();
    if cleaned_rec > 0 {
        eprintln!("boltsnap daemon: cleaned {cleaned_rec} orphaned recording file(s)");
    }
    if let Err(error) = crate::record::audio::cleanup_stale_mixes() {
        eprintln!("boltsnap daemon: clean up stale recording audio: {error}");
    }

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Daemon>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let ddm = DataDeviceManagerState::bind(&globals, &qh)?;
    let pool = SlotPool::new(256 * 256 * 4, &shm)?;

    let cfg = LayoutConfig::default();
    let config = crate::config::Config::load();
    let save_dir = crate::config::resolve_save_dir(save_dir_cli.as_deref(), &config);
    let recording_prefs = config.recording_prefs();
    let (event_tx, event_rx) = calloop::channel::channel::<DaemonEvent>();
    let prefs_writer = PrefsWriter::spawn(event_tx.clone());
    let mut daemon = Daemon {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        layer_shell,
        layer: None,
        output_name: None,
        shelf_configured: false,
        shelf_pending_draw: false,
        pointer: None,
        keyboard: None,
        ddm,
        data_device: None,
        drag_source: None,
        drag_path: None,
        drag_icon_pool: None,
        drag_icon_buffer: None,
        drop_ok: false,
        icon_surface: None,
        model: ShelfModel::new(),
        layout: Layout::compute(&[], &cfg),
        cfg,
        width: 1,
        height: 1,
        hovered: None,
        save_dir,
        save_flash: None,
        saving_cards: std::collections::HashSet::new(),
        press: None,
        anims: Vec::new(),
        exit: false,
        qh: Some(qh.clone()),
        recording: None,
        marker: None,
        marker_pool: None,
        marker_region: None,
        marker_configured: false,
        popup: None,
        popup_pool: None,
        popup_font: font::fallback_popup_font(),
        popup_configured: false,
        watchers: Vec::new(),
        last_recording_snapshot: crate::ipc::RecordingSnapshot::idle(),
        last_recording_space_check: std::time::Instant::now(),
        event_tx,
        focused_output: None,
        focus_query_pending: false,
        pending_controls: Vec::new(),
        pending_default_recordings: Vec::new(),
        pending_tray_default_recording: false,
        persisted_recording_prefs: recording_prefs.clone(),
        recording_prefs,
        prefs_generation: 0,
        persisted_prefs_generation: 0,
        prefs_writer,
        tray: None,
    };

    // Populate output metadata (names) so we can place the shelf on the focused
    // monitor instead of wherever the compositor defaults to.
    event_queue.roundtrip(&mut daemon)?;
    // No-animation layer rule must be in place before the surface maps.
    prep_shelf_compositor_rules();
    daemon.place_on_output(None, &qh);

    // Unified event loop: Wayland fd + the unix-socket listener fd.
    use calloop::generic::Generic;
    use calloop::{EventLoop, Interest, Mode, PostAction};
    use calloop_wayland_source::WaylandSource;

    let listener = UnixListener::bind(&sock)?;
    listener.set_nonblocking(true)?;

    let mut event_loop: EventLoop<Daemon> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    WaylandSource::new(conn.clone(), event_queue)
        .insert(handle.clone())
        .map_err(|e| format!("insert wayland source: {e}"))?;

    handle
        .insert_source(event_rx, |event, _, daemon: &mut Daemon| {
            if let calloop::channel::Event::Msg(event) = event {
                daemon.handle_daemon_event(event);
            }
        })
        .map_err(|e| format!("insert recording worker source: {e}"))?;

    let source = Generic::new(listener, Interest::READ, Mode::Level);
    handle
        .insert_source(source, |_readiness, listener, daemon: &mut Daemon| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => spawn_client_reader(stream, daemon.event_tx.clone()),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        eprintln!("boltsnap daemon: accept error: {e}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| format!("insert listener source: {e}"))?;

    daemon.refresh_focus();

    while !daemon.exit {
        // Tick ~60fps while a card is animating; otherwise a 250ms timeout, which
        // also lets a counting recording catch whole-second boundaries for its
        // MM:SS readout without burning CPU for the (possibly long) recording.
        let timeout = if daemon.animating() {
            std::time::Duration::from_millis(16)
        } else {
            std::time::Duration::from_millis(250)
        };
        event_loop
            .dispatch(timeout, &mut daemon)
            .map_err(|e| format!("dispatch: {e}"))?;
        if daemon.animating() {
            daemon.tick_animations(&qh);
        }
        // Publish whole-second progress and notice an unexpected recorder exit.
        daemon.tick_recording(&qh);
    }
    if let Some(mut session) = daemon.recording.take() {
        let children = std::mem::take(&mut session.active);
        if !children.is_empty() {
            let job = StopChildrenJob { children };
            let _ = job.interrupt();
            let _ = job.wait();
        }
        if let Some(audio) = session.audio.take()
            && let Err(error) = audio.cleanup()
        {
            eprintln!("boltsnap daemon: clean up recording audio on shutdown: {error}");
        }
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

fn requested_audio(
    prefs: &crate::config::RecordingPrefs,
) -> Option<crate::config::RecordAudioSource> {
    prefs.audio_enabled.then_some(prefs.audio_source)
}

fn spawn_initial_segment(
    scope: &CaptureScope,
    codec: &str,
    tools: &RecorderTools,
    prefs: &crate::config::RecordingPrefs,
) -> Result<
    (
        Vec<crate::record::session::ActiveRecorder>,
        Option<AudioCapture>,
    ),
    String,
> {
    let mut audio = requested_audio(prefs)
        .map(crate::record::audio::prepare_audio)
        .transpose()?;
    match spawn_segment(
        scope,
        codec,
        audio.as_ref().map(AudioCapture::source),
        tools,
    ) {
        Ok(active) => Ok((active, audio)),
        Err(error) => {
            let Some(audio) = audio.take() else {
                return Err(error);
            };
            match audio.cleanup() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}; clean up recording audio: {cleanup}")),
            }
        }
    }
}

fn cleanup_audio_async(audio: Option<AudioCapture>) {
    if let Some(audio) = audio {
        std::thread::spawn(move || {
            if let Err(error) = audio.cleanup() {
                eprintln!("boltsnap daemon: clean up recording audio: {error}");
            }
        });
    }
}

impl Daemon {
    fn cached_monitors(&self) -> Vec<crate::record::Monitor> {
        self.output_state
            .outputs()
            .filter_map(|output| self.output_state.info(&output))
            .filter_map(|info| {
                let name = info.name?;
                let (x, y) = info.logical_position.unwrap_or(info.location);
                let logical_size = info.logical_size.filter(|(w, h)| *w > 0 && *h > 0);
                let mode_size = info
                    .modes
                    .iter()
                    .find(|mode| mode.current)
                    .map(|mode| transformed_mode_size(mode.dimensions, info.transform))
                    .filter(|(w, h)| *w > 0 && *h > 0);
                let fallback_scale = f64::from(info.scale_factor.max(1));
                let scale = match (mode_size, logical_size) {
                    (Some((width, _)), Some((logical_width, _))) => {
                        f64::from(width) / f64::from(logical_width)
                    }
                    _ => fallback_scale,
                };
                let (width, height) = mode_size.or_else(|| {
                    logical_size.map(|(width, height)| {
                        (
                            (f64::from(width) * scale).round() as i32,
                            (f64::from(height) * scale).round() as i32,
                        )
                    })
                })?;
                Some(crate::record::Monitor {
                    focused: self.focused_output.as_deref() == Some(name.as_str()),
                    description: info
                        .description
                        .unwrap_or_else(|| format!("{} {}", info.make, info.model)),
                    name,
                    x,
                    y,
                    width: width as u32,
                    height: height as u32,
                    scale,
                })
            })
            .collect()
    }

    fn tray_snapshot(&self) -> crate::tray::TraySnapshot {
        crate::tray::TraySnapshot {
            prefs: self.recording_prefs.clone(),
            monitors: self.cached_monitors(),
            state: self.recording_snapshot().state,
        }
    }

    fn start_tray(&mut self) {
        self.tray = Some(crate::tray::TrayPublisher::spawn(
            self.tray_snapshot(),
            self.event_tx.clone(),
        ));
    }

    fn publish_tray_snapshot(&self) {
        let snapshot = self.tray_snapshot();
        if let Some(tray) = &self.tray {
            tray.publish(snapshot);
        }
    }

    fn apply_show_frame_pref(&mut self, show: bool) {
        if let Some(session) = self.recording.as_mut() {
            session.show_frame = show;
        }
        if !show {
            self.remove_marker();
        } else if let Some(geo) = self.recording.as_ref().and_then(|session| {
            (session.phase == SessionPhase::Recording)
                .then_some(&session.scope)
                .and_then(|scope| match scope {
                    CaptureScope::Area(geo) => Some(*geo),
                    CaptureScope::Outputs(_) => None,
                })
        }) && let Some(qh) = self.qh.clone()
        {
            self.create_marker(&geo, &qh);
        }
    }

    fn persist_tray_prefs(&mut self, prefs: crate::config::RecordingPrefs) {
        let show_frame_changed = self.recording_prefs.show_frame != prefs.show_frame;
        self.recording_prefs = prefs.clone();
        self.prefs_generation += 1;
        if show_frame_changed {
            self.apply_show_frame_pref(prefs.show_frame);
        }
        self.prefs_writer.submit(PrefsWrite {
            generation: self.prefs_generation,
            prefs,
        });
        self.publish_tray_snapshot();
    }

    fn handle_tray_action(&mut self, action: crate::tray::TrayAction) {
        use crate::tray::TrayAction;

        match action {
            TrayAction::StartRegion => {
                if self.recording.is_some() {
                    notify("A recording is already in progress");
                } else {
                    let result = std::env::current_exe()
                        .map_err(|error| format!("find boltsnap executable: {error}"))
                        .and_then(|exe| {
                            crate::paths::spawn_reaped(
                                std::process::Command::new(exe)
                                    .arg("record")
                                    .stdin(std::process::Stdio::null())
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::null()),
                            )
                            .map(|_| ())
                            .map_err(|error| format!("start region selector: {error}"))
                        });
                    if let Err(error) = result {
                        notify(&error);
                    }
                }
            }
            TrayAction::StartDefault => {
                if self.recording.is_some() {
                    notify("A recording is already in progress");
                } else {
                    self.pending_tray_default_recording = true;
                    self.refresh_focus();
                }
            }
            TrayAction::SetDefaultTarget(target) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.default_target = target;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetBothMode(mode) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.both_mode = mode;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetAudioSource(source) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.audio_source = source;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetShowFrame(show) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.show_frame = show;
                self.persist_tray_prefs(prefs);
            }
            TrayAction::SetDiskAddToShelf(add) => {
                let mut prefs = self.recording_prefs.clone();
                prefs.disk_add_to_shelf = add;
                self.persist_tray_prefs(prefs);
            }
        }
    }

    fn target_output(
        &self,
        requested: Option<&str>,
    ) -> (Option<String>, Option<wl_output::WlOutput>) {
        let outputs: Vec<_> = self.output_state.outputs().collect();
        let available: Vec<_> = outputs
            .iter()
            .filter_map(|output| self.output_state.info(output).and_then(|info| info.name))
            .collect();
        let requested = resolve_output_name(requested, &available);
        let output = requested
            .and_then(|n| {
                outputs
                    .iter()
                    .find(|o| self.output_state.info(o).and_then(|i| i.name).as_deref() == Some(n))
                    .cloned()
            })
            // Fall back to the first available output when the focused monitor
            // can't be resolved. Hyprland may fail to map a layer surface with a
            // null output, so always pass a concrete one when we have it.
            .or_else(|| outputs.into_iter().next());
        let name = output
            .as_ref()
            .and_then(|output| self.output_state.info(output))
            .and_then(|info| info.name);
        (name, output)
    }

    /// Ensure the shelf surface lives on the currently focused output. Recreates
    /// the layer surface (dropping the old one, which unmaps it) when the focused
    /// monitor changed since last time. Returns true when a fresh surface was
    /// created and must wait for its initial configure before drawing.
    fn place_on_output(&mut self, requested: Option<&str>, qh: &QueueHandle<Self>) -> bool {
        let requested = requested.or(self.output_name.as_deref());
        let (name, output) = self.target_output(requested);
        if self.layer.is_some() && name == self.output_name {
            return false;
        }
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("boltsnap"),
            output.as_ref(),
        );
        layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
        layer.set_margin(0, 0, 24, 24);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        layer.set_size(self.layout.width.max(1), self.layout.height.max(1));
        layer.commit();
        self.layer = Some(layer); // old layer dropped here -> unmaps from prior output
        self.output_name = name;
        self.shelf_configured = false;
        self.shelf_pending_draw = false;
        true
    }

    /// Recompute layout from the model and resize the layer surface to match.
    fn relayout(&mut self) {
        let sizes: Vec<(u64, u32, u32)> = visible_cards(&self.model)
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        self.layout = Layout::compute(&sizes, &self.cfg);
        // We are a self-sized layer surface: we dictate our size via set_size and
        // the compositor echoes it back in configure. Track it directly so draw()
        // paints at the new size immediately — relying only on the configure
        // round-trip deadlocked the surface at the startup 1x1 (the post-set_size
        // commit carried a stale-sized buffer, so Hyprland never reconfigured).
        self.width = self.layout.width.max(1);
        self.height = self.layout.height.max(1);
        if let Some(layer) = self.layer.as_ref() {
            layer.set_size(self.layout.width, self.layout.height);
        }
    }

    /// Handle one request already read by a worker, keeping IPC reads off calloop.
    fn handle_client_request(
        &mut self,
        req: crate::ipc::Request,
        mut stream: std::os::unix::net::UnixStream,
    ) {
        use std::io::Write;
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return,
        };
        match req {
            crate::ipc::Request::Ping => {
                spawn_client_writer(stream, b"PONG".to_vec());
            }
            crate::ipc::Request::Add {
                source,
                png,
                output,
            } => {
                self.add_png(&png, &source, output.as_deref(), &qh);
            }
            crate::ipc::Request::RecordingStatus => {
                self.write_response(
                    stream,
                    crate::ipc::Response::ok(Some(self.recording_snapshot())),
                );
            }
            crate::ipc::Request::RecordingWatch => {
                let _ = stream.set_read_timeout(None);
                let _ = stream.set_nonblocking(true);
                let line = self.recording_snapshot().to_json_line();
                if matches!(stream.write(line.as_bytes()), Ok(written) if written == line.len()) {
                    self.watchers.push(stream);
                }
            }
            crate::ipc::Request::ShowRecordingControls => {
                if !self
                    .recording
                    .as_ref()
                    .is_some_and(|session| recording_controls_visible(session.phase))
                {
                    self.write_response(
                        stream,
                        crate::ipc::Response::error("recording controls are unavailable"),
                    );
                } else {
                    self.pending_controls.push(stream);
                    self.refresh_focus();
                }
            }
            crate::ipc::Request::RecordingControl { action } => {
                let response = match self.handle_recording_action(action, &qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::StartDefaultRecording => {
                if self.recording.is_some() {
                    self.write_response(
                        stream,
                        crate::ipc::Response::error("a recording is already in progress"),
                    );
                } else {
                    self.pending_default_recordings.push(stream);
                    self.refresh_focus();
                }
            }
            crate::ipc::Request::StartRecording {
                x,
                y,
                w,
                h,
                show_frame,
                audio_enabled,
            } => {
                // The selector already persisted this value in its process. Feed it
                // through the ordered writer too so an older in-flight tray write
                // cannot restore a stale frame preference afterward.
                self.persisted_recording_prefs.show_frame = show_frame;
                self.persisted_recording_prefs.audio_enabled = audio_enabled;
                let mut prefs = self.recording_prefs.clone();
                prefs.show_frame = show_frame;
                prefs.audio_enabled = audio_enabled;
                self.persist_tray_prefs(prefs);
                let response = match self.start_recording(x, y, w, h, show_frame, &qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::StartRecordingOutput { name } => {
                let response = match self.start_named_recording_outputs(
                    vec![name],
                    crate::config::RecordBothMode::Separate,
                ) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::StartRecordingOutputs { names, combined } => {
                let response = match self.start_named_recording_outputs(
                    names,
                    if combined {
                        crate::config::RecordBothMode::Combined
                    } else {
                        crate::config::RecordBothMode::Separate
                    },
                ) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
            crate::ipc::Request::RecordingThumb { id, thumb } => {
                self.update_recording_thumb(id, thumb, &qh);
            }
            crate::ipc::Request::StopRecording => {
                let response = match self.handle_recording_action(RecordingAction::SaveShelf, &qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                };
                self.write_response(stream, response);
            }
        }
    }

    fn write_response(&self, stream: UnixStream, response: crate::ipc::Response) {
        spawn_client_writer(stream, response.encode());
    }

    /// Ingest a PNG: persist a daemon-owned temp copy, scale a thumbnail, show it
    /// on the currently focused monitor.
    fn add_png(&mut self, png: &[u8], source: &str, output: Option<&str>, qh: &QueueHandle<Self>) {
        let img = match image::load_from_memory(png) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("boltsnap daemon: bad PNG: {e}");
                return;
            }
        };
        // Daemon-owned temp file for drag URI delivery and clipboard fallback.
        let path = crate::paths::temp_png("shelf");
        if let Err(e) = std::fs::write(&path, png) {
            eprintln!("boltsnap daemon: temp write failed: {e}");
            return;
        }
        let thumb = crate::shelf::thumbnail::make_card_thumbnail(
            &img,
            crate::shelf::thumbnail::CARD_W,
            crate::shelf::thumbnail::CARD_H,
        );
        // A compositor can omit the terminal drag event and leave redraws
        // blocked. A fresh capture supersedes that stale interaction.
        if self.drag_source.is_some() {
            self.clear_drag();
        }
        let id = self.model.add(path, thumb, source.to_string());
        self.start_anim(id, AnimKind::Appear);
        self.relayout();
        self.place_on_output(output, qh);
        self.draw(qh);
    }

    /// Copy the card under the cursor to the clipboard: an image as `image/png`,
    /// a video as a `text/uri-list` file reference (path only, never bytes).
    fn copy_card(&mut self, id: u64) {
        if let Some(t) = self.model.get(id) {
            let path = t.png_path.clone();
            if t.kind == crate::shelf::model::CardKind::Video {
                // Video: copy a file REFERENCE (path), not bytes — instant for any
                // clip length and pasteable as a file. No auto-clipboard elsewhere.
                if let Err(e) = crate::clipboard::copy_uri_to_clipboard(&path) {
                    eprintln!("boltsnap daemon: video copy failed: {e}");
                }
                return;
            }
            if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland) {
                eprintln!("boltsnap daemon: copy failed: {e}");
            }
        }
    }

    /// Save the full-res file of card `id` and flash a ✓ on its Save button.
    /// Image cards go to `<save_dir>/boltsnap-<ts>.png`; video cards go to
    /// `<record_dir>/boltsnap-<ts>.<ext>` (ext from the source file, usually mp4).
    fn save_card(&mut self, id: u64) {
        let (src, kind) = match self.model.get(id) {
            Some(t) if t.lifetime == FileLifetime::Permanent => {
                eprintln!("boltsnap daemon: already saved {}", t.png_path.display());
                notify("Already saved");
                self.save_flash = Some((id, std::time::Instant::now()));
                return;
            }
            Some(t) => (t.png_path.clone(), t.kind),
            None => return,
        };
        if kind == CardKind::Video {
            if !self.saving_cards.insert(id) {
                return;
            }
            let dir = crate::config::resolve_record_dir(&crate::config::Config::load());
            let tx = self.event_tx.clone();
            std::thread::spawn(move || {
                let result = promote_recording(&src, &dir, None);
                let _ = tx.send(DaemonEvent::CardPromoted { id, result });
            });
            return;
        }

        let stamp = crate::paths::local_timestamp();
        let name = crate::paths::boltsnap_filename_ext(&stamp, "png");
        let dir = self.save_dir.clone();
        let dest = dir.join(name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("boltsnap daemon: save mkdir failed: {e}");
            return;
        }
        match std::fs::copy(&src, &dest) {
            Ok(_) => {
                eprintln!("boltsnap daemon: saved {}", dest.display());
                self.save_flash = Some((id, std::time::Instant::now()));
            }
            Err(e) => eprintln!("boltsnap daemon: save failed: {e}"),
        }
    }

    /// Act on a completed (non-drag) left click.
    fn on_click(&mut self, hit: Hit, redraw: &mut bool) {
        match hit {
            Hit::Body(id) => self.copy_card(id),
            Hit::Save(id) => {
                self.save_card(id);
                *redraw = true;
            }
            Hit::Close(id) => {
                // Animate the card out; tick_animations removes it (and its temp
                // file) and relayouts when the dismiss animation completes.
                if self.hovered == Some(id) {
                    self.hovered = None;
                }
                self.start_anim(id, AnimKind::Disappear);
                *redraw = true;
            }
        }
    }

    /// Start a Wayland drag for thumbnail `id`, offering image/png (or video/mp4)
    /// + a file URI, with the thumbnail itself as the drag icon. Returns true if
    /// the drag was actually started.
    fn begin_drag(&mut self, id: u64, serial: u32) -> bool {
        let (path, kind) = match self.model.get(id) {
            Some(t) => (t.png_path.clone(), t.kind),
            None => return false,
        };
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return false,
        };
        if self.data_device.is_none() {
            return false;
        }
        let origin = match self.layer.as_ref() {
            Some(l) => l.wl_surface().clone(),
            None => return false,
        };

        // wl_data_device assigns the dnd_icon ROLE to the surface inside
        // start_drag, and a surface must not carry a buffer on its role-assigning
        // commit. So: create the source + surface, call start_drag FIRST (assigns
        // the role), and only THEN attach + damage + commit the icon buffer.
        // Doing it the other way (buffer committed before the role existed) made
        // the icon flash and then vanish inconsistently — and reliably broke when
        // the drag crossed to another output, where the compositor re-maps the
        // icon and found it in a bad state.
        let source = match kind {
            crate::shelf::model::CardKind::Image => self.ddm.create_drag_and_drop_source(
                &qh,
                ["image/png", "text/uri-list"],
                DndAction::Copy,
            ),
            crate::shelf::model::CardKind::Video => self.ddm.create_drag_and_drop_source(
                &qh,
                ["video/mp4", "text/uri-list"],
                DndAction::Copy,
            ),
        };
        let icon = self.compositor.create_surface(&qh);
        {
            let device = self.data_device.as_ref().unwrap();
            source.start_drag(device, &origin, Some(&icon), serial);
        }

        // Build the icon in its OWN pool, and retain BOTH the pool and the Buffer
        // for the whole drag (dropping the Buffer destroys the wl_buffer, which is
        // what made the icon disappear mid-drag / on monitor change).
        if let Some(t) = self.model.get(id) {
            let (iw, ih) = t.thumb.dimensions();
            let stride = (iw * 4) as i32;
            let bytes = crate::shelf::paint::build_drag_icon(&t.thumb, iw, ih, 10.0, 0.85);
            if let Ok(mut pool) = SlotPool::new((iw * ih * 4) as usize, &self.shm) {
                if let Ok((buf, canvas)) = pool.create_buffer(
                    iw as i32,
                    ih as i32,
                    stride,
                    wayland_client::protocol::wl_shm::Format::Argb8888,
                ) {
                    // SlotPool rounds the canvas length up to a 64-byte boundary,
                    // so `canvas` can be a few bytes longer than the tightly-packed
                    // icon (`iw*ih*4`). The wl_buffer uses our exact stride, so the
                    // trailing slack is unused — copy only the real bytes (a plain
                    // copy_from_slice panics on the length mismatch).
                    canvas[..bytes.len()].copy_from_slice(&bytes);
                    let _ = buf.attach_to(&icon);
                    icon.damage_buffer(0, 0, iw as i32, ih as i32);
                    icon.commit();
                    self.drag_icon_buffer = Some(buf);
                }
                self.drag_icon_pool = Some(pool);
            }
        }

        self.drag_path = Some(path);
        self.drop_ok = false;
        self.icon_surface = Some(icon);
        self.drag_source = Some(source);
        true
    }

    /// Reset all in-flight drag state in one place (kept in sync between the
    /// `cancelled` and `dnd_finished` paths), then repaint the shelf once.
    fn clear_drag(&mut self) {
        self.drag_source = None;
        self.drag_path = None;
        self.icon_surface = None;
        self.drag_icon_buffer = None;
        self.drag_icon_pool = None;
        self.drop_ok = false;
        self.press = None;
        // draw() is suppressed while a drag is active; now that it's cleared,
        // repaint once to restore a clean shelf frame.
        if let Some(qh) = self.qh.clone() {
            self.draw(&qh);
        }
    }

    /// Begin an appear/dismiss animation for card `id`, replacing any existing
    /// animation on that card.
    fn start_anim(&mut self, id: u64, kind: AnimKind) {
        self.anims.retain(|a| a.id != id);
        self.anims.push(CardAnim {
            id,
            kind,
            start: std::time::Instant::now(),
        });
    }

    /// (scale, opacity) for card `id` from its in-flight animation, or (1.0, 1.0)
    /// when it is settled.
    fn anim_factor(&self, id: u64) -> (f32, f32) {
        for a in &self.anims {
            if a.id != id {
                continue;
            }
            let t = (a.start.elapsed().as_millis() as f32 / a.kind.dur() as f32).clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            let span = 1.0 - ANIM_SCALE_MIN;
            return match a.kind {
                AnimKind::Appear => (ANIM_SCALE_MIN + span * eased, eased),
                // Shrink toward nothing in step with the collapsing slot (so the
                // leaving card doesn't overlap the cards sliding up), and fade.
                AnimKind::Disappear => (1.0 - eased, 1.0 - eased),
            };
        }
        if self
            .model
            .newest_first()
            .nth(MAX_VISIBLE_CARDS)
            .is_some_and(|card| card.id == id)
            && let Some(opacity) = overflow_opacity(
                self.active_appear_progress(),
                self.active_disappear_progress(),
            )
        {
            return (1.0, opacity);
        }
        (1.0, 1.0)
    }

    fn active_appear_progress(&self) -> Option<f32> {
        let newest = self.model.newest_first().next()?.id;
        self.anims
            .iter()
            .rev()
            .find(|anim| anim.id == newest && anim.kind == AnimKind::Appear)
            .map(|anim| {
                ease_out_cubic(anim.start.elapsed().as_millis() as f32 / anim.kind.dur() as f32)
            })
    }

    fn active_disappear_progress(&self) -> Option<f32> {
        self.anims
            .iter()
            .rev()
            .find(|anim| {
                anim.kind == AnimKind::Disappear
                    && visible_cards(&self.model).any(|card| card.id == anim.id)
            })
            .map(|anim| {
                ease_out_cubic(anim.start.elapsed().as_millis() as f32 / anim.kind.dur() as f32)
            })
    }

    fn animating(&self) -> bool {
        !self.anims.is_empty() || self.save_flash.is_some()
    }

    /// Advance animations one frame: retire finished ones (removing dismissed
    /// cards + their temp files), relayout if anything changed, and redraw.
    fn tick_animations(&mut self, qh: &QueueHandle<Self>) {
        let done: Vec<(u64, AnimKind)> = self
            .anims
            .iter()
            .filter(|a| a.start.elapsed().as_millis() >= a.kind.dur())
            .map(|a| (a.id, a.kind))
            .collect();
        self.anims
            .retain(|a| a.start.elapsed().as_millis() < a.kind.dur());
        let mut removed = false;
        for (id, kind) in done {
            if kind == AnimKind::Disappear
                && let Some(t) = self.model.remove(id)
            {
                let _ = t.delete_file_on_dismiss();
                if self.hovered == Some(id) {
                    self.hovered = None;
                }
                removed = true;
            }
        }
        if removed {
            self.relayout();
        }
        if let Some((_, started)) = self.save_flash {
            if started.elapsed().as_millis() >= SAVE_FLASH_MS {
                self.save_flash = None;
            }
        }
        self.draw(qh);
    }

    /// Layout for the current frame: disappearing slots collapse, while a sixth
    /// card remains in the fixed-height viewport just long enough to scroll and
    /// fade below it as the new card enters at the top.
    fn animated_layout(&self) -> Layout {
        let cfg = &self.cfg;
        let appear_progress = self.active_appear_progress();
        let disappear_progress = self.active_disappear_progress();
        let limit = MAX_VISIBLE_CARDS
            + usize::from(
                (appear_progress.is_some() || disappear_progress.is_some())
                    && self.model.len() > MAX_VISIBLE_CARDS,
            );
        let items: Vec<(u64, u32, u32, f32)> = self
            .model
            .newest_first()
            .take(limit)
            .map(|t| {
                let collapse = self
                    .anims
                    .iter()
                    .find(|a| a.id == t.id && a.kind == AnimKind::Disappear)
                    .map(|a| {
                        let p = (a.start.elapsed().as_millis() as f32 / a.kind.dur() as f32)
                            .clamp(0.0, 1.0);
                        ease_out_cubic(p)
                    })
                    .unwrap_or(0.0);
                (t.id, t.thumb.width(), t.thumb.height(), collapse)
            })
            .collect();
        animated_card_layout(&items, cfg, appear_progress)
    }

    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        if !self.shelf_configured {
            self.shelf_pending_draw = true;
            return;
        }
        // Committing the origin surface during the implicit grab makes some
        // wlroots compositors cancel the drag as it crosses to another output.
        // clear_drag() redraws after the terminal event; a fresh pointer press
        // clears a compositor-stuck drag before processing the new interaction.
        if !shelf_commit_allowed(self.drag_source.is_some()) {
            return;
        }
        // Render against an animated layout that collapses any disappearing card's
        // slot, and resize the (bottom-anchored) surface to match so the cards
        // above slide down smoothly during the dismiss instead of snapping at the
        // end. When nothing is animating this equals the settled layout.
        let render = self.animated_layout();
        let (w, h) = (render.width.max(1), render.height.max(1));
        let size_changed = w != self.width || h != self.height;
        self.width = w;
        self.height = h;
        // Per-card (scale, opacity) for in-flight appear/dismiss animations. Built
        // before create_buffer because `canvas` mutably borrows self.pool, which
        // would otherwise block the immutable self borrow anim_factor needs.
        let anims: Vec<(u64, f32, f32)> = render
            .thumbs
            .iter()
            .filter_map(|r| {
                let (s, o) = self.anim_factor(r.id);
                if (s - 1.0).abs() < f32::EPSILON && (o - 1.0).abs() < f32::EPSILON {
                    None
                } else {
                    Some((r.id, s, o))
                }
            })
            .collect();
        let layer = match self.layer.as_ref() {
            Some(l) => l,
            None => return,
        };
        if size_changed {
            layer.set_size(w, h);
        }
        let stride = (w * 4) as i32;
        let (buffer, canvas) = match self.pool.create_buffer(
            w as i32,
            h as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };

        crate::shelf::paint::draw_shelf(
            canvas,
            w,
            h,
            &render,
            &self.model,
            self.hovered,
            &self.cfg,
            &anims,
            self.save_flash.map(|(id, _)| id),
        );

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
    }

    // ----- Recording lifecycle -------------------------------------------

    fn start_recording(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        show_frame: bool,
        qh: &QueueHandle<Self>,
    ) -> Result<(), String> {
        if self.recording.is_some() {
            return Err("a recording is already in progress".into());
        }
        validate_recording_dimensions(w, h)?;
        if !crate::paths::has_cmd("wf-recorder") {
            return Err("wf-recorder is not installed".into());
        }
        prepare_recording_cache()?;
        let geo = crate::record::Geometry { x, y, w, h };
        let scope = CaptureScope::Area(geo);
        let codec = crate::config::resolve_record_codec(None, &crate::config::Config::load());
        let tools = RecorderTools::default();
        let monitors = self.cached_monitors();
        let monitors = monitor_for_geometry(&monitors, &geo)
            .cloned()
            .into_iter()
            .collect();
        let (active, audio) = spawn_initial_segment(&scope, &codec, &tools, &self.recording_prefs)?;
        self.recording = Some(RecordingSession::new(
            scope,
            monitors,
            codec.clone(),
            crate::config::RecordBothMode::Separate,
            show_frame,
            audio,
            active,
            std::time::Instant::now(),
        ));
        if show_frame {
            self.create_marker(&geo, qh);
        }
        self.publish_recording_snapshot();
        eprintln!("boltsnap daemon: recording {} -> {codec:?}", geo.to_arg());
        Ok(())
    }

    fn start_default_recording(
        &mut self,
        focus_snapshot: &Result<Vec<crate::record::Monitor>, String>,
    ) -> Result<(), String> {
        let plan = default_start_plan(
            &self.recording_prefs,
            focus_snapshot,
            &self.cached_monitors(),
        )?;
        let notice = plan.notice.clone();
        self.start_recording_outputs(plan.outputs, plan.both_mode)?;
        if let Some(notice) = notice {
            notify(&notice);
        }
        Ok(())
    }

    fn start_named_recording_outputs(
        &mut self,
        names: Vec<String>,
        both_mode: crate::config::RecordBothMode,
    ) -> Result<(), String> {
        let connected = self.cached_monitors();
        let monitors = names
            .iter()
            .map(|name| {
                connected
                    .iter()
                    .find(|monitor| monitor.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("recording output {name} is disconnected"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.start_recording_outputs(monitors, both_mode)
    }

    fn start_recording_outputs(
        &mut self,
        monitors: Vec<crate::record::Monitor>,
        both_mode: crate::config::RecordBothMode,
    ) -> Result<(), String> {
        if self.recording.is_some() {
            return Err("a recording is already in progress".into());
        }
        if !crate::paths::has_cmd("wf-recorder") {
            return Err("wf-recorder is not installed".into());
        }
        prepare_recording_cache()?;
        if monitors.is_empty() {
            return Err("no recording output was selected".into());
        }
        let names = monitors
            .iter()
            .map(|monitor| monitor.name.clone())
            .collect::<Vec<_>>();
        let scope = CaptureScope::Outputs(names.clone());
        let codec = crate::config::resolve_record_codec(None, &crate::config::Config::load());
        let tools = RecorderTools::default();
        let (active, audio) = spawn_initial_segment(&scope, &codec, &tools, &self.recording_prefs)?;
        self.recording = Some(RecordingSession::new(
            scope,
            monitors,
            codec,
            both_mode,
            false,
            audio,
            active,
            std::time::Instant::now(),
        ));
        self.publish_recording_snapshot();
        Ok(())
    }

    /// Build the click-through marker layer surface (a `Top` layer with an EMPTY
    /// input region so clicks pass through to the apps being recorded). Returns
    /// the surface, its draw pool, and the region (retained so it isn't destroyed).
    ///
    /// The surface is created on the focused output (margins are per-output, so a
    /// null output would mis-place the marker on a non-origin monitor). The rect
    /// is in compositor-GLOBAL coords; we convert it to OUTPUT-LOCAL margins by
    /// subtracting that output's layout origin — the same origin `record_flow`
    /// added to globalize the selection, so the subtraction is exact.
    fn create_marker(&mut self, geo: &crate::record::Geometry, qh: &QueueHandle<Self>) {
        use crate::shelf::recording::MARKER_INFLATE;
        let inflate = MARKER_INFLATE as i32;
        let mw = geo.w + 2 * MARKER_INFLATE;
        let mh = geo.h + 2 * MARKER_INFLATE;

        let monitor = self
            .cached_monitors()
            .into_iter()
            .find(|monitor| monitor_for_geometry(std::slice::from_ref(monitor), geo).is_some());
        let (output_name, ox, oy) = monitor
            .map(|monitor| (Some(monitor.name), monitor.x, monitor.y))
            .unwrap_or((None, 0, 0));
        let surface = self.compositor.create_surface(qh);
        let (_name, output) = self.target_output(output_name.as_deref());
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some("boltsnap-recording"),
            output.as_ref(),
        );
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        // Convert the GLOBAL rect to OUTPUT-LOCAL margins by subtracting the
        // focused monitor's layout origin (the inverse of record_flow's globalize),
        // then anchor top-left and push to the rect via margins (inflated by border).
        let local_x = geo.x - ox;
        let local_y = geo.y - oy;
        layer.set_margin((local_y - inflate).max(0), 0, 0, (local_x - inflate).max(0));
        layer.set_size(mw, mh);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);

        // EMPTY input region == clicks pass through. `Region::new` with no `add`
        // produces an empty region; retain it (it self-destroys on drop).
        let region = Region::new(&self.compositor).ok();
        if let Some(reg) = region.as_ref() {
            layer.set_input_region(Some(reg.wl_region()));
        } else {
            eprintln!("boltsnap daemon: could not create empty input region for marker");
        }
        layer.commit();

        self.marker = Some(layer);
        self.marker_pool = SlotPool::new((mw * mh * 4) as usize, &self.shm).ok();
        self.marker_region = region;
        self.marker_configured = false;
    }

    fn create_popup(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        self.popup_font = font::load_popup_font();
        let pool = SlotPool::new((POPUP_W * POPUP_H * 4) as usize, &self.shm)
            .map_err(|error| format!("allocate recording controls: {error}"))?;
        let surface = self.compositor.create_surface(qh);
        let (_name, output) = self.target_output(self.focused_output.as_deref());
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("boltsnap-recording-controls"),
            output.as_ref(),
        );
        layer.set_size(POPUP_W, POPUP_H);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        layer.commit();
        self.popup = Some(layer);
        self.popup_pool = Some(pool);
        self.popup_configured = false;
        Ok(())
    }

    /// Draw the click-through marker border into its surface.
    fn draw_marker(&mut self) {
        if !self.marker_configured {
            return;
        }
        let geo = match self.recording.as_ref().map(|session| &session.scope) {
            Some(CaptureScope::Area(geo)) => *geo,
            _ => return,
        };
        let (layer, pool) = match (self.marker.as_ref(), self.marker_pool.as_mut()) {
            (Some(l), Some(p)) => (l, p),
            _ => return,
        };
        use crate::shelf::recording::MARKER_INFLATE;
        let mw = geo.w + 2 * MARKER_INFLATE;
        let mh = geo.h + 2 * MARKER_INFLATE;
        let stride = (mw * 4) as i32;
        let (buffer, canvas) = match pool.create_buffer(
            mw as i32,
            mh as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        crate::shelf::paint::draw_marker_border(
            canvas,
            mw,
            mh,
            crate::shelf::recording::MARKER_BORDER,
            crate::shelf::recording::MARKER_RADIUS,
        );
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, mw as i32, mh as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
    }

    fn draw_popup(&mut self) {
        if !self.popup_configured {
            return;
        }
        let (state, enabled, elapsed) = match self.recording.as_ref() {
            Some(session) => (
                session.public_state(),
                session.actions_enabled(),
                crate::shelf::recording::fmt_elapsed(
                    session.elapsed_at(std::time::Instant::now()).as_secs(),
                ),
            ),
            None => return,
        };
        let (layer, pool) = match (self.popup.as_ref(), self.popup_pool.as_mut()) {
            (Some(l), Some(p)) => (l, p),
            _ => return,
        };
        let stride = (POPUP_W * 4) as i32;
        let (buffer, canvas) = match pool.create_buffer(
            POPUP_W as i32,
            POPUP_H as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        crate::shelf::paint::draw_recording_popup(
            canvas,
            POPUP_W,
            POPUP_H,
            state,
            enabled,
            &elapsed,
            &self.popup_font,
        );
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, POPUP_W as i32, POPUP_H as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
    }

    fn tick_recording(&mut self, _qh: &QueueHandle<Self>) {
        let mut recovery = self.recording.as_mut().and_then(|session| {
            if session.phase != SessionPhase::Recording {
                return None;
            }
            session
                .active
                .iter_mut()
                .find_map(|recorder| match recorder.child.try_wait() {
                    Ok(Some(status)) => Some(format!("wf-recorder exited unexpectedly: {status}")),
                    Err(error) => Some(format!("check wf-recorder: {error}")),
                    Ok(None) => None,
                })
        });
        if recovery.is_none()
            && self.recording.as_ref().is_some_and(|session| {
                session.phase == SessionPhase::Recording
                    && self.last_recording_space_check.elapsed()
                        >= std::time::Duration::from_secs(1)
            })
        {
            self.last_recording_space_check = std::time::Instant::now();
            let dir = crate::paths::rec_dir();
            recovery = check_recording_cache_limit(&dir)
                .and_then(|()| check_recording_reserve(&dir))
                .err();
        }
        if let Some(error) = recovery {
            if let Some(session) = self.recording.as_mut() {
                let _ = session.begin_pause(std::time::Instant::now());
            }
            self.remove_marker();
            let children = self
                .recording
                .as_mut()
                .map(|session| std::mem::take(&mut session.active))
                .unwrap_or_default();
            self.stop_children(AfterStop::Recover(error), children);
        }
        let snapshot = self.recording_snapshot();
        if snapshot != self.last_recording_snapshot {
            self.draw_popup();
            self.publish_recording_snapshot();
        }
    }

    fn refresh_focus(&mut self) {
        if self.focus_query_pending {
            return;
        }
        self.focus_query_pending = true;
        spawn_focus_query(self.event_tx.clone());
    }

    fn finish_pending_controls(&mut self) {
        if self.pending_controls.is_empty() {
            return;
        }
        let response = match self.qh.clone() {
            Some(qh) => {
                self.remove_popup();
                match self.show_recording_controls(&qh) {
                    Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                    Err(error) => crate::ipc::Response::error(error),
                }
            }
            None => crate::ipc::Response::error("recording controls are unavailable"),
        };
        let encoded = response.encode();
        for stream in std::mem::take(&mut self.pending_controls) {
            spawn_client_writer(stream, encoded.clone());
        }
    }

    fn finish_pending_default_recordings(
        &mut self,
        focus_snapshot: &Result<Vec<crate::record::Monitor>, String>,
    ) {
        for stream in std::mem::take(&mut self.pending_default_recordings) {
            let response = match self.start_default_recording(focus_snapshot) {
                Ok(()) => crate::ipc::Response::ok(Some(self.recording_snapshot())),
                Err(error) => crate::ipc::Response::error(error),
            };
            self.write_response(stream, response);
        }
    }

    fn reject_pending_controls(&mut self, error: &str) {
        let encoded = crate::ipc::Response::error(error).encode();
        for stream in std::mem::take(&mut self.pending_controls) {
            spawn_client_writer(stream, encoded.clone());
        }
    }

    fn show_recording_controls(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        if !self
            .recording
            .as_ref()
            .is_some_and(|session| recording_controls_visible(session.phase))
        {
            return Err("recording controls are unavailable".into());
        }
        if self.popup.is_none() {
            self.create_popup(qh)?;
        } else {
            self.draw_popup();
        }
        Ok(())
    }

    fn handle_recording_action(
        &mut self,
        action: RecordingAction,
        qh: &QueueHandle<Self>,
    ) -> Result<(), String> {
        let session = self
            .recording
            .as_ref()
            .ok_or_else(|| "there is no active recording".to_string())?;
        if !session.can_accept(action) {
            return Err(format!(
                "cannot {} while {:?}",
                recording_action_label(action),
                session.phase
            ));
        }

        match action {
            RecordingAction::Pause => {
                let session = self.recording.as_mut().unwrap();
                session.begin_pause(std::time::Instant::now())?;
                let children = std::mem::take(&mut session.active);
                self.remove_marker();
                self.stop_children(AfterStop::Pause, children);
            }
            RecordingAction::Resume => {
                let (scope, codec, audio_source) = {
                    let session = self.recording.as_ref().unwrap();
                    (
                        session.scope.clone(),
                        session.codec.clone(),
                        session
                            .audio
                            .as_ref()
                            .map(|audio| audio.source().to_owned()),
                    )
                };
                let active = match spawn_segment(
                    &scope,
                    &codec,
                    audio_source.as_deref(),
                    &RecorderTools::default(),
                ) {
                    Ok(active) => active,
                    Err(error) => {
                        if let Some(session) = self.recording.as_mut() {
                            session.last_error = Some(error.clone());
                        }
                        self.publish_recording_snapshot();
                        return Err(error);
                    }
                };
                self.recording
                    .as_mut()
                    .unwrap()
                    .resume(active, std::time::Instant::now())?;
                let marker = self.recording.as_ref().and_then(|session| {
                    if session.show_frame {
                        match &session.scope {
                            CaptureScope::Area(geo) => Some(*geo),
                            CaptureScope::Outputs(_) => None,
                        }
                    } else {
                        None
                    }
                });
                if let Some(geo) = marker {
                    self.create_marker(&geo, qh);
                }
            }
            RecordingAction::SaveShelf | RecordingAction::SaveDisk => {
                let destination = if action == RecordingAction::SaveShelf {
                    SaveDestination::Shelf
                } else {
                    SaveDestination::Disk(crate::config::resolve_record_dir(
                        &crate::config::Config::load(),
                    ))
                };
                let session = self.recording.as_mut().unwrap();
                session.begin_finalize(std::time::Instant::now())?;
                let children = std::mem::take(&mut session.active);
                self.remove_marker();
                self.draw_popup();
                if children.is_empty() {
                    self.start_finalize_worker(destination);
                } else {
                    self.stop_children(AfterStop::Save(destination), children);
                }
            }
            RecordingAction::Discard => {
                let session = self.recording.as_mut().unwrap();
                session.begin_discard(std::time::Instant::now())?;
                let children = std::mem::take(&mut session.active);
                self.reject_pending_controls("recording is being discarded");
                self.remove_marker();
                self.remove_popup();
                if children.is_empty() {
                    self.finish_discard(Vec::new());
                } else {
                    self.stop_children(AfterStop::Discard, children);
                }
            }
        }
        self.draw_popup();
        self.publish_recording_snapshot();
        Ok(())
    }

    fn stop_children(
        &mut self,
        after: AfterStop,
        children: Vec<crate::record::session::ActiveRecorder>,
    ) {
        if children.is_empty() {
            self.handle_daemon_event(DaemonEvent::ChildrenStopped {
                after,
                result: StopChildrenResult::Ready(Vec::new()),
            });
            return;
        }
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let job = StopChildrenJob { children };
            let interrupt_error = job.interrupt().err();
            let result = match (interrupt_error, job.wait()) {
                (None, result) => result,
                (Some(interrupt), StopChildrenResult::Ready(kept)) => StopChildrenResult::Failed {
                    kept,
                    error: interrupt,
                },
                (Some(interrupt), StopChildrenResult::Failed { kept, error }) => {
                    StopChildrenResult::Failed {
                        kept,
                        error: format!("{interrupt}; {error}"),
                    }
                }
            };
            let _ = tx.send(DaemonEvent::ChildrenStopped { after, result });
        });
    }

    fn handle_daemon_event(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::ClientRequest { request, stream } => {
                self.handle_client_request(request, stream);
            }
            DaemonEvent::ChildrenStopped { after, result } => {
                let (segments, error) = match result {
                    StopChildrenResult::Ready(segments) => (segments, None),
                    StopChildrenResult::Failed { kept, error } => (kept, Some(error)),
                };
                match after {
                    AfterStop::Pause | AfterStop::Recover(_) => {
                        let recovery = match after {
                            AfterStop::Recover(error) => Some(error),
                            _ => error,
                        };
                        if let Some(session) = self.recording.as_mut() {
                            let _ = session.finish_pause(segments);
                            if let Some(error) = recovery {
                                session.last_error = Some(error.clone());
                                notify(&format!("Recording paused: {error}"));
                            }
                        }
                        self.draw_popup();
                    }
                    AfterStop::Save(destination) => {
                        if let Some(error) = error {
                            if let Some(session) = self.recording.as_mut() {
                                session.add_completed(segments);
                                let _ = session.finalize_failed(error.clone());
                            }
                            notify(&format!("Recording could not be finalized: {error}"));
                            self.draw_popup();
                        } else {
                            if let Some(session) = self.recording.as_mut() {
                                session.add_completed(segments);
                            }
                            self.start_finalize_worker(destination);
                        }
                    }
                    AfterStop::Discard => self.finish_discard(segments),
                }
            }
            DaemonEvent::Finalized(Ok(clips)) => {
                let audio = self
                    .recording
                    .as_mut()
                    .and_then(|session| session.audio.take());
                self.recording = None;
                cleanup_audio_async(audio);
                self.remove_marker();
                self.remove_popup();
                self.add_finalized_cards(clips);
            }
            DaemonEvent::Finalized(Err(failure)) => {
                self.add_finalized_cards(failure.completed);
                if let Some(session) = self.recording.as_mut() {
                    session.completed = failure.recoverable_segments;
                    let _ = session.finalize_failed(failure.error.clone());
                }
                notify(&format!(
                    "Recording could not be finalized: {}",
                    failure.error
                ));
                self.draw_popup();
            }
            DaemonEvent::CardPromoted { id, result } => {
                self.saving_cards.remove(&id);
                match result {
                    Ok(path) => {
                        eprintln!("boltsnap daemon: saved {}", path.display());
                        if self.model.promote(id, path) {
                            self.save_flash = Some((id, std::time::Instant::now()));
                            if let Some(qh) = self.qh.clone() {
                                self.draw(&qh);
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("boltsnap daemon: video save failed: {error}");
                        notify(&format!("Video could not be saved: {error}"));
                    }
                }
            }
            DaemonEvent::FocusResolved(snapshot) => {
                let connected = self.cached_monitors();
                if let Some(output) = snapshot.as_ref().ok().and_then(|monitors| {
                    monitors
                        .iter()
                        .find(|monitor| monitor.focused)
                        .filter(|monitor| {
                            connected.iter().any(|output| output.name == monitor.name)
                        })
                        .map(|monitor| monitor.name.clone())
                }) {
                    self.focused_output = Some(output);
                }
                self.focus_query_pending = false;
                self.finish_pending_default_recordings(&snapshot);
                if std::mem::take(&mut self.pending_tray_default_recording)
                    && let Err(error) = self.start_default_recording(&snapshot)
                {
                    notify(&error);
                }
                self.finish_pending_controls();
                if self.tray.is_none() {
                    self.start_tray();
                } else {
                    self.publish_tray_snapshot();
                }
            }
            DaemonEvent::Tray(action) => self.handle_tray_action(action),
            DaemonEvent::PrefsPersisted {
                generation,
                prefs,
                error,
            } => match prefs_completion(
                self.prefs_generation,
                self.persisted_prefs_generation,
                generation,
                error.is_some(),
            ) {
                PrefsCompletion::Persisted => {
                    self.persisted_prefs_generation = generation;
                    self.persisted_recording_prefs = prefs;
                }
                PrefsCompletion::Rollback => {
                    let show_frame_changed = self.recording_prefs.show_frame
                        != self.persisted_recording_prefs.show_frame;
                    self.recording_prefs = self.persisted_recording_prefs.clone();
                    if show_frame_changed {
                        self.apply_show_frame_pref(self.recording_prefs.show_frame);
                    }
                    notify(&format!(
                        "Recording settings could not be saved: {}",
                        error.unwrap_or_else(|| "unknown error".into())
                    ));
                    self.publish_tray_snapshot();
                }
                PrefsCompletion::Stale => {}
            },
        }
        self.publish_recording_snapshot();
    }

    fn start_finalize_worker(&mut self, destination: SaveDestination) {
        let Some(session) = self.recording.as_mut() else {
            return;
        };
        let request = FinalizeRequest {
            segments: std::mem::take(&mut session.completed),
            monitors: session.monitors.clone(),
            both_mode: session.both_mode,
            codec: session.codec.clone(),
            destination,
        };
        let tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = finalize_recording(request, &RecorderTools::default());
            let _ = tx.send(DaemonEvent::Finalized(result));
        });
    }

    fn finish_discard(&mut self, segments: Vec<StoppedSegment>) {
        if let Some(session) = self.recording.as_mut() {
            session.add_completed(segments);
        }
        let (paths, audio) = match self.recording.take() {
            Some(session) => (
                session
                    .completed
                    .into_values()
                    .flatten()
                    .collect::<Vec<_>>(),
                session.audio,
            ),
            None => (Vec::new(), None),
        };
        cleanup_audio_async(audio);
        self.remove_marker();
        self.remove_popup();
        std::thread::spawn(move || {
            let cache = crate::paths::rec_dir();
            for path in paths {
                if path.starts_with(&cache) {
                    let _ = std::fs::remove_file(path);
                }
            }
        });
    }

    fn add_finalized_cards(&mut self, clips: Vec<FinalizedClip>) {
        let Some(qh) = self.qh.clone() else {
            return;
        };
        let mut added = false;
        for clip in clips {
            if clip.permanent && !self.recording_prefs.disk_add_to_shelf {
                continue;
            }
            let mut image = image::RgbaImage::new(
                crate::shelf::thumbnail::CARD_W,
                crate::shelf::thumbnail::CARD_H,
            );
            for pixel in image.pixels_mut() {
                *pixel = image::Rgba([32, 32, 40, 255]);
            }
            let placeholder = crate::shelf::thumbnail::make_card_thumbnail(
                &image,
                crate::shelf::thumbnail::CARD_W,
                crate::shelf::thumbnail::CARD_H,
            );
            let source = clip
                .output
                .as_deref()
                .map(|output| format!("record:{output}"))
                .unwrap_or_else(|| "record".into());
            let id = self.model.add_kind_with_lifetime(
                clip.path.clone(),
                placeholder,
                source,
                CardKind::Video,
                if clip.permanent {
                    FileLifetime::Permanent
                } else {
                    FileLifetime::Temporary
                },
            );
            self.start_anim(id, AnimKind::Appear);
            spawn_recording_thumbnail(id, clip.path);
            added = true;
        }
        if added {
            self.relayout();
            self.place_on_output(None, &qh);
            self.draw(&qh);
        }
    }

    fn recording_snapshot(&self) -> crate::ipc::RecordingSnapshot {
        let Some(session) = self.recording.as_ref() else {
            return crate::ipc::RecordingSnapshot::idle();
        };
        let (scope, outputs) = match &session.scope {
            CaptureScope::Area(_) => (
                "area".to_string(),
                session
                    .monitors
                    .iter()
                    .map(|monitor| monitor.name.clone())
                    .collect(),
            ),
            CaptureScope::Outputs(outputs) => (
                if outputs.len() > 1 { "both" } else { "output" }.to_string(),
                outputs.clone(),
            ),
        };
        let elapsed_ms = session.elapsed_at(std::time::Instant::now()).as_millis() as u64;
        crate::ipc::RecordingSnapshot {
            state: session.public_state(),
            elapsed_ms: elapsed_ms / 1000 * 1000,
            scope,
            outputs,
            actions_enabled: session.actions_enabled(),
            error: session.last_error.clone(),
        }
    }

    fn publish_recording_snapshot(&mut self) {
        use std::io::Write;
        let snapshot = self.recording_snapshot();
        if snapshot == self.last_recording_snapshot {
            return;
        }
        let state_changed = snapshot.state != self.last_recording_snapshot.state;
        let line = snapshot.to_json_line();
        self.watchers.retain_mut(
            |watcher| matches!(watcher.write(line.as_bytes()), Ok(n) if n == line.len()),
        );
        self.last_recording_snapshot = snapshot;
        if state_changed {
            self.publish_tray_snapshot();
        }
    }

    fn remove_marker(&mut self) {
        self.marker = None;
        self.marker_pool = None;
        self.marker_region = None;
        self.marker_configured = false;
    }

    fn remove_popup(&mut self) {
        self.popup = None;
        self.popup_pool = None;
        self.popup_configured = false;
    }

    /// Swap a recording card's placeholder for its real first-frame thumbnail,
    /// posted via `RecordingThumb`. Missing/unreadable png → keep the placeholder.
    fn update_recording_thumb(
        &mut self,
        id: u64,
        thumb: std::path::PathBuf,
        qh: &QueueHandle<Self>,
    ) {
        if let Ok(img) = image::open(&thumb) {
            let card = crate::shelf::thumbnail::make_card_thumbnail(
                &img.to_rgba8(),
                crate::shelf::thumbnail::CARD_W,
                crate::shelf::thumbnail::CARD_H,
            );
            if self.model.replace_thumb(id, card) {
                self.draw(qh);
            }
        }
        // The first-frame png was only needed to build the thumbnail.
        let _ = std::fs::remove_file(&thumb);
    }

    fn is_popup_surface(&self, surface: &WlSurface) -> bool {
        self.popup
            .as_ref()
            .map(|popup| popup.wl_surface() == surface)
            .unwrap_or(false)
    }

    /// True if `surface` is the active recording's click-through marker surface.
    fn is_marker_surface(&self, surface: &WlSurface) -> bool {
        self.marker
            .as_ref()
            .map(|marker| marker.wl_surface() == surface)
            .unwrap_or(false)
    }

    fn on_popup_click(&mut self, pos: (f64, f64)) {
        let Some(session) = self.recording.as_ref() else {
            return;
        };
        let action = match crate::shelf::recording::popup_hit(
            session.public_state(),
            session.actions_enabled(),
            pos.0,
            pos.1,
        ) {
            Some(PopupButton::PauseResume)
                if session.public_state() == PublicRecordingState::Paused =>
            {
                RecordingAction::Resume
            }
            Some(PopupButton::PauseResume) => RecordingAction::Pause,
            Some(PopupButton::SaveShelf) => RecordingAction::SaveShelf,
            Some(PopupButton::SaveDisk) => RecordingAction::SaveDisk,
            Some(PopupButton::Discard) => RecordingAction::Discard,
            None => return,
        };
        if let Some(qh) = self.qh.clone() {
            let _ = self.handle_recording_action(action, &qh);
        }
    }
}

impl CompositorHandler for Daemon {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Daemon {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_tray_snapshot();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_tray_snapshot();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_tray_snapshot();
    }
}

impl LayerShellHandler for Daemon {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.is_popup_surface(layer.wl_surface()) {
            self.remove_popup();
            return;
        }
        if self.is_marker_surface(layer.wl_surface()) {
            self.remove_marker();
            return;
        }
        if self
            .layer
            .as_ref()
            .map(|shelf| shelf.wl_surface() == layer.wl_surface())
            .unwrap_or(false)
        {
            self.exit = true;
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if self.is_popup_surface(layer.wl_surface()) {
            self.popup_configured = true;
            self.draw_popup();
            return;
        }
        if self.is_marker_surface(layer.wl_surface()) {
            self.marker_configured = true;
            self.draw_marker();
            return;
        }
        let is_shelf = self
            .layer
            .as_ref()
            .map(|shelf| shelf.wl_surface() == layer.wl_surface())
            .unwrap_or(false);
        if !is_shelf {
            return;
        }
        // wlr-layer-shell: a configure dimension of 0 means "client, pick your own
        // size". For our self-sized, bottom-left-anchored shelf, Hyprland replies
        // to set_size() with new_size=(0,0); keeping the stale value here left the
        // surface at the startup 1x1 (mapped but invisible). Fall back to layout.
        self.width = if configure.new_size.0 != 0 {
            configure.new_size.0
        } else {
            self.layout.width.max(1)
        };
        self.height = if configure.new_size.1 != 0 {
            configure.new_size.1
        } else {
            self.layout.height.max(1)
        };
        let should_draw = self.shelf_pending_draw || self.model.newest_first().next().is_some();
        self.shelf_configured = true;
        self.shelf_pending_draw = false;
        if should_draw {
            self.draw(qh);
        }
    }
}

impl SeatHandler for Daemon {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            let cursor_surface = self.compositor.create_surface(qh);
            if let Ok(tp) = self.seat_state.get_pointer_with_theme(
                qh,
                &seat,
                self.shm.wl_shm(),
                cursor_surface,
                ThemeSpec::default(),
            ) {
                self.pointer = Some(tp);
            }
        }
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(k) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(k);
            }
        }
        if self.data_device.is_none() {
            self.data_device = Some(self.ddm.get_data_device(qh, &seat));
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard {
            self.keyboard = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for Daemon {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        let shelf_surface = self.layer.as_ref().map(|l| l.wl_surface().clone());
        let mut redraw = false;
        for ev in events {
            if matches!(ev.kind, PointerEventKind::Press { .. }) && self.drag_source.is_some() {
                self.clear_drag();
            }
            if self.is_popup_surface(&ev.surface) {
                if matches!(ev.kind, PointerEventKind::Enter { .. }) {
                    if let Some(p) = self.pointer.as_ref() {
                        let _ = p.set_cursor(conn, CursorIcon::Default);
                    }
                }
                if let PointerEventKind::Press { button, .. } = ev.kind {
                    if button == BTN_LEFT {
                        self.on_popup_click(ev.position);
                    }
                }
                continue;
            }
            // Everything else is shelf input.
            if shelf_surface.as_ref() != Some(&ev.surface) {
                continue;
            }
            // Set the normal arrow when the pointer enters the shelf, instead of
            // inheriting the previously-focused window's cursor (e.g. an I-beam).
            if matches!(ev.kind, PointerEventKind::Enter { .. }) {
                if let Some(p) = self.pointer.as_ref() {
                    let _ = p.set_cursor(conn, CursorIcon::Default);
                }
            }
            let (x, y) = ev.position;
            match ev.kind {
                PointerEventKind::Leave { .. } => {
                    if self.hovered.is_some() {
                        self.hovered = None;
                        redraw = true;
                    }
                }
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let now = self.layout.hit(x, y, &self.cfg).map(|h| match h {
                        Hit::Body(id) | Hit::Save(id) | Hit::Close(id) => id,
                    });
                    if now != self.hovered {
                        self.hovered = now;
                        redraw = true;
                    }
                    let start_drag = self
                        .press
                        .as_ref()
                        .map(|p| {
                            !p.dragging
                                && matches!(p.hit, Hit::Body(_))
                                && (x - p.x).powi(2) + (y - p.y).powi(2) > 36.0
                        })
                        .unwrap_or(false);
                    if start_drag {
                        let (id, serial) = {
                            let p = self.press.as_ref().unwrap();
                            (p.id, p.serial)
                        };
                        // Only mark the press as dragging if the drag actually
                        // started; otherwise the eventual Release would wrongly be
                        // swallowed instead of treated as a click.
                        if self.begin_drag(id, serial) {
                            if let Some(p) = self.press.as_mut() {
                                p.dragging = true;
                            }
                        }
                    }
                }
                PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                    if let Some(hit) = self.layout.hit(x, y, &self.cfg) {
                        let id = match hit {
                            Hit::Body(i) | Hit::Save(i) | Hit::Close(i) => i,
                        };
                        self.press = Some(PressState {
                            id,
                            hit,
                            x,
                            y,
                            serial,
                            dragging: false,
                        });
                    }
                }
                PointerEventKind::Press { button, .. } if button == BTN_RIGHT => {
                    if let Some(hit) = self.layout.hit(x, y, &self.cfg) {
                        let id = match hit {
                            Hit::Body(i) | Hit::Save(i) | Hit::Close(i) => i,
                        };
                        self.copy_card(id);
                    }
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if let Some(p) = self.press.take() {
                        if !p.dragging {
                            self.on_click(p.hit, &mut redraw);
                        }
                    }
                }
                _ => {}
            }
        }
        if redraw {
            if let Some(qh) = self.qh.clone() {
                self.draw(&qh);
            }
        }
    }
}

impl KeyboardHandler for Daemon {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape {
            self.remove_popup();
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        _: Modifiers,
        _: u32,
    ) {
    }
}

impl ShmHandler for Daemon {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl DataSourceHandler for Daemon {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        fd: WritePipe,
    ) {
        use std::io::Write;
        use std::os::fd::OwnedFd;
        let is_ours = self
            .drag_source
            .as_ref()
            .map(|d| d.inner() == source)
            .unwrap_or(false);
        if !is_ours {
            return;
        }
        let Some(path) = self.drag_path.clone() else {
            return;
        };
        // Write the requested data on a DETACHED THREAD, never on the calloop
        // event-loop thread. The drop target reads the pipe at its own pace
        // (Electron apps like Discord request the full multi-MB PNG and drain it
        // lazily), and the pipe buffer is only ~64 KiB. A synchronous
        // `write_all` here would block the entire daemon until the reader drains
        // or the pipe breaks — freezing the shelf ("card is there but dead",
        // clicks/remove/drag all stop, and the drag never reaches dnd_finished so
        // it can never recover). Offloading keeps the daemon responsive.
        let fd = OwnedFd::from(fd);
        std::thread::spawn(move || {
            let mut file = std::fs::File::from(fd);
            match mime.as_str() {
                "text/uri-list" => {
                    let uri = crate::clipboard::uri_list_for(&path);
                    let _ = file.write_all(uri.as_bytes());
                }
                _ => {
                    if let Ok(bytes) = std::fs::read(&path) {
                        let _ = file.write_all(&bytes);
                    }
                }
            }
        });
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        // A cancel without a successful drop = the user dropped onto nothing (or
        // the target rejected it): fall back to copying the image to the clipboard
        // so the drag is never wasted.
        if !self.drop_ok {
            if let Some(path) = self.drag_path.clone() {
                // Only screenshots fall back to a clipboard image copy; a video's
                // raw bytes aren't meaningful on the clipboard (mirrors copy_card).
                let is_video = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
                if is_video {
                    eprintln!(
                        "boltsnap daemon: drag cancelled for a video card; not copying to clipboard (use Save)"
                    );
                } else if let Err(e) =
                    crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland)
                {
                    eprintln!("boltsnap daemon: fallback copy failed: {e}");
                }
            }
        }
        self.clear_drag();
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drop_ok = true;
    }

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.clear_drag();
    }

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl DataDeviceHandler for Daemon {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _x: f64,
        _y: f64,
        _: &WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
    }
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for Daemon {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl ProvidesRegistryState for Daemon {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Daemon);
delegate_output!(Daemon);
delegate_shm!(Daemon);
delegate_seat!(Daemon);
delegate_keyboard!(Daemon);
delegate_pointer!(Daemon);
delegate_layer!(Daemon);
delegate_data_device!(Daemon);
delegate_registry!(Daemon);

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    #[test]
    fn active_drag_defers_shelf_commits() {
        assert!(!shelf_commit_allowed(true));
        assert!(shelf_commit_allowed(false));
    }

    #[test]
    fn five_card_viewport_reveals_cached_card_after_removal() {
        let mut model = ShelfModel::new();
        let ids: Vec<_> = (0..6)
            .map(|n| {
                model.add(
                    std::path::PathBuf::from(format!("/tmp/card-{n}.png")),
                    image::RgbaImage::new(1, 1),
                    "area".into(),
                )
            })
            .collect();

        let visible: Vec<_> = visible_cards(&model).map(|card| card.id).collect();
        assert_eq!(visible, ids[1..].iter().rev().copied().collect::<Vec<_>>());
        assert!(model.get(ids[0]).is_some(), "oldest card stays cached");

        model.remove(ids[5]);
        let visible: Vec<_> = visible_cards(&model).map(|card| card.id).collect();
        assert_eq!(visible, ids[..5].iter().rev().copied().collect::<Vec<_>>());
    }

    #[test]
    fn overflow_insertion_scrolls_oldest_below_viewport() {
        let cfg = LayoutConfig::default();
        let items: Vec<_> = (1..=6).rev().map(|id| (id, 190, 132, 0.0)).collect();

        let start = animated_card_layout(&items, &cfg, Some(0.0));
        assert_eq!(start.height, 744);
        assert_eq!(
            start.thumbs.iter().map(|card| card.y).collect::<Vec<_>>(),
            [22, 22, 164, 306, 448, 590]
        );

        let end = animated_card_layout(&items, &cfg, Some(1.0));
        assert_eq!(end.height, 744);
        assert_eq!(
            end.thumbs.iter().map(|card| card.y).collect::<Vec<_>>(),
            [22, 164, 306, 448, 590, 732]
        );
    }

    #[test]
    fn dismissal_pulls_cached_card_back_into_viewport() {
        let cfg = LayoutConfig::default();
        let start: Vec<_> = (1..=6).rev().map(|id| (id, 190, 132, 0.0)).collect();
        let start = animated_card_layout(&start, &cfg, None);
        assert_eq!(start.height, 744);
        assert_eq!(start.thumbs[5].y, 732);

        let end: Vec<_> = (1..=6)
            .rev()
            .map(|id| (id, 190, 132, f32::from(id == 5)))
            .collect();
        let end = animated_card_layout(&end, &cfg, None);
        assert_eq!(end.height, 744);
        assert_eq!(end.thumbs[5].y, 590);
    }

    #[test]
    fn overflow_card_crossfades_at_viewport_edge() {
        assert_eq!(overflow_opacity(Some(0.25), None), Some(0.75));
        assert_eq!(overflow_opacity(None, Some(0.25)), Some(0.25));
        assert_eq!(overflow_opacity(None, None), None);
    }

    #[test]
    fn requested_audio_respects_toggle_and_keeps_source_choice() {
        let mut prefs = crate::config::RecordingPrefs {
            audio_enabled: false,
            audio_source: crate::config::RecordAudioSource::System,
            ..Default::default()
        };
        assert_eq!(requested_audio(&prefs), None);
        prefs.audio_enabled = true;
        assert_eq!(
            requested_audio(&prefs),
            Some(crate::config::RecordAudioSource::System)
        );
    }

    fn receive_event(rx: &calloop::channel::Channel<DaemonEvent>) -> DaemonEvent {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match rx.try_recv() {
                Ok(event) => return event,
                Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    panic!("timed out waiting for daemon event")
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    panic!("daemon event channel disconnected")
                }
            }
        }
    }

    #[test]
    fn partial_client_does_not_block_a_complete_client_or_drop_its_payload() {
        let (tx, rx) = calloop::channel::channel();
        let (slow_server, mut slow_client) = UnixStream::pair().unwrap();
        let png = vec![0x5a; 256 * 1024];
        let mut encoded = Vec::new();
        crate::ipc::write_frame(&mut encoded, br#"{"cmd":"add","source":"area"}"#, &png).unwrap();
        let split = encoded.len() / 2;
        slow_client.write_all(&encoded[..split]).unwrap();
        spawn_client_reader(slow_server, tx.clone());

        let (fast_server, mut fast_client) = UnixStream::pair().unwrap();
        spawn_client_reader(fast_server, tx);
        fast_client
            .write_all(&crate::ipc::Request::Ping.encode())
            .unwrap();

        assert!(matches!(
            receive_event(&rx),
            DaemonEvent::ClientRequest {
                request: crate::ipc::Request::Ping,
                ..
            }
        ));

        slow_client.write_all(&encoded[split..]).unwrap();
        match receive_event(&rx) {
            DaemonEvent::ClientRequest {
                request:
                    crate::ipc::Request::Add {
                        source,
                        png: received,
                        ..
                    },
                ..
            } => {
                assert_eq!(source, "area");
                assert_eq!(received, png);
            }
            _ => panic!("expected complete PNG request"),
        }
    }

    #[test]
    fn unread_reply_does_not_block_another_reply() {
        let (mut blocked_server, _blocked_client) = UnixStream::pair().unwrap();
        blocked_server.set_nonblocking(true).unwrap();
        let fill = [0u8; 4096];
        loop {
            match blocked_server.write(&fill) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("fill reply socket: {error}"),
            }
        }
        blocked_server.set_nonblocking(false).unwrap();
        spawn_client_writer(blocked_server, vec![1]);

        let (fast_server, mut fast_client) = UnixStream::pair().unwrap();
        fast_client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        spawn_client_writer(fast_server, b"PONG".to_vec());
        let mut reply = [0u8; 4];
        fast_client.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"PONG");
    }

    #[test]
    fn area_uses_cached_monitor_layout() {
        let monitors = vec![
            crate::record::Monitor {
                name: "DP-1".into(),
                description: String::new(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                focused: false,
            },
            crate::record::Monitor {
                name: "HDMI-A-1".into(),
                description: String::new(),
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.0,
                focused: true,
            },
        ];
        let geo = crate::record::Geometry {
            x: 2100,
            y: 120,
            w: 640,
            h: 480,
        };

        assert_eq!(
            monitor_for_geometry(&monitors, &geo).map(|monitor| monitor.name.as_str()),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn focused_output_and_popup_target_use_current_hyprland_focus() {
        let json = br#"[
            {"name":"DP-1","description":"Main","x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false},
            {"name":"HDMI-A-1","description":"AOC","x":1920,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true}
        ]"#;
        let available = vec!["DP-1".to_string(), "HDMI-A-1".to_string()];

        let focused = focused_output_from_hyprland_json(json);
        assert_eq!(focused.as_deref(), Some("HDMI-A-1"));
        assert_eq!(
            resolve_output_name(focused.as_deref(), &available),
            Some("HDMI-A-1")
        );
        assert_eq!(
            resolve_output_name(Some("disconnected"), &available),
            Some("DP-1")
        );
    }

    fn monitor(name: &str, focused: bool) -> crate::record::Monitor {
        crate::record::Monitor {
            name: name.into(),
            description: name.into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused,
        }
    }

    #[test]
    fn focus_query_snapshot_keeps_current_order_and_focus() {
        let json = br#"[
            {"name":"DP-1","description":"AOC","x":1920,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false},
            {"name":"DP-3","description":"Main","x":0,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true}
        ]"#;
        let snapshot = parse_focus_snapshot(json).unwrap();
        assert_eq!(
            snapshot
                .iter()
                .map(|monitor| (monitor.name.as_str(), monitor.focused))
                .collect::<Vec<_>>(),
            vec![("DP-1", false), ("DP-3", true)]
        );
    }

    #[test]
    fn failed_focus_query_never_reuses_stale_focus_for_default_start() {
        let prefs = crate::config::RecordingPrefs::default();
        let stale_wayland = vec![monitor("DP-3", false), monitor("DP-1", true)];
        let plan =
            default_start_plan(&prefs, &Err("query timed out".into()), &stale_wayland).unwrap();
        assert_eq!(plan.outputs[0].name, "DP-3");

        let disconnected = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Output("DP-OLD".into()),
            ..prefs
        };
        let plan = default_start_plan(
            &disconnected,
            &Err("query timed out".into()),
            &stale_wayland,
        )
        .unwrap();
        assert_eq!(plan.outputs[0].name, "DP-3");
    }

    #[test]
    fn default_start_uses_fresh_snapshot_order_instead_of_wayland_order() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Both,
            ..crate::config::RecordingPrefs::default()
        };
        let wayland = vec![monitor("DP-3", true), monitor("DP-1", false)];
        let fresh = Ok(vec![monitor("DP-1", false), monitor("DP-3", true)]);
        let plan = default_start_plan(&prefs, &fresh, &wayland).unwrap();
        assert_eq!(
            plan.outputs
                .iter()
                .map(|monitor| monitor.name.as_str())
                .collect::<Vec<_>>(),
            vec!["DP-1", "DP-3"]
        );
    }

    #[test]
    fn hotplug_mismatch_cannot_focus_a_disconnected_output() {
        let prefs = crate::config::RecordingPrefs::default();
        let wayland = vec![monitor("DP-1", true)];
        let fresh = Ok(vec![monitor("DP-OLD", true), monitor("DP-1", false)]);
        assert!(default_start_plan(&prefs, &fresh, &wayland).is_err());

        let named = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Output("DP-1".into()),
            ..prefs
        };
        let plan = default_start_plan(&named, &fresh, &wayland).unwrap();
        assert_eq!(plan.outputs[0].name, "DP-1");
        assert!(!plan.outputs[0].focused);
    }

    #[test]
    fn failed_focus_query_can_still_start_both_connected_wayland_outputs() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Both,
            ..crate::config::RecordingPrefs::default()
        };
        let stale_wayland = vec![monitor("DP-3", true), monitor("DP-1", false)];
        let plan = default_start_plan(&prefs, &Err("query failed".into()), &stale_wayland).unwrap();
        assert_eq!(plan.outputs.len(), 2);
        assert!(plan.outputs.iter().all(|monitor| !monitor.focused));
    }

    #[test]
    fn rotated_output_bounds_select_the_marker_monitor() {
        let (width, height) = transformed_mode_size((2560, 1440), wl_output::Transform::_90);
        assert_eq!((width, height), (1440, 2560));
        let monitors = vec![crate::record::Monitor {
            name: "DP-ROTATED".into(),
            description: String::new(),
            x: 1920,
            y: 0,
            width: width as u32,
            height: height as u32,
            scale: 1.0,
            focused: true,
        }];
        let geo = crate::record::Geometry {
            x: 2000,
            y: 2200,
            w: 100,
            h: 100,
        };

        assert_eq!(
            monitor_for_geometry(&monitors, &geo).map(|monitor| monitor.name.as_str()),
            Some("DP-ROTATED")
        );
    }

    #[test]
    fn delayed_focus_cannot_reopen_discarding_controls_but_finalizing_stays_visible() {
        assert!(!recording_controls_visible(SessionPhase::Discarding));
        assert!(recording_controls_visible(SessionPhase::Finalizing));
    }

    #[test]
    fn daemon_rejects_zero_sized_recording_geometry() {
        assert!(validate_recording_dimensions(0, 1080).is_err());
        assert!(validate_recording_dimensions(1920, 0).is_err());
        assert!(validate_recording_dimensions(1920, 1080).is_ok());
    }

    #[test]
    fn preference_completions_never_rollback_a_newer_request() {
        assert_eq!(prefs_completion(2, 0, 1, false), PrefsCompletion::Persisted);
        assert_eq!(prefs_completion(2, 1, 1, true), PrefsCompletion::Stale);
        assert_eq!(prefs_completion(2, 1, 2, true), PrefsCompletion::Rollback);
        assert_eq!(prefs_completion(2, 2, 1, false), PrefsCompletion::Stale);
    }

    #[test]
    fn preference_writer_mailbox_coalesces_to_the_latest_generation() {
        let latest = crate::tray::LatestValue::new();
        latest.replace(PrefsWrite {
            generation: 1,
            prefs: crate::config::RecordingPrefs::default(),
        });
        let newest = crate::config::RecordingPrefs {
            show_frame: false,
            ..Default::default()
        };
        latest.replace(PrefsWrite {
            generation: 2,
            prefs: newest,
        });

        let write = latest.take().unwrap();
        assert_eq!(write.generation, 2);
        assert!(!write.prefs.show_frame);
        assert!(latest.take().is_none());
    }
}
