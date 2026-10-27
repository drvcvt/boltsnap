// The daemon is one state (`Daemon`) on one thread. Its parts are sibling
// modules, grouped into folders by concept (docs/plans/shelf/2026-10-27-shelf-split.md).
#[path = "cards/actions.rs"]
mod actions;
#[path = "cards/anim.rs"]
mod anim;
#[path = "cards/cards.rs"]
mod cards;
mod clients;
#[path = "recording/controls.rs"]
mod controls;
#[path = "wayland/dnd.rs"]
mod dnd;
mod events;
#[path = "recording/finish.rs"]
mod finish;
pub(crate) mod font;
#[path = "wayland/handlers.rs"]
mod handlers;
#[path = "wayland/input.rs"]
mod input;
mod limits;
#[path = "cards/pending.rs"]
mod pending;
mod placement;
#[path = "cards/save.rs"]
mod save;
#[path = "recording/session.rs"]
mod session;
mod settings;
#[path = "recording/start.rs"]
mod start;
mod surface;
#[path = "cards/thumbnail.rs"]
mod thumbnail;
mod tray;
#[path = "recording/watch.rs"]
mod watch;

use anim::{AnimKind, CardAnim};
use clients::spawn_client_reader;
use input::PressState;
use limits::ClientLimits;
// Reached as `super::…` by the siblings that stayed as they were.
use actions::{MAX_CACHED_CARDS, MAX_CACHED_IMAGE_BYTES, PreparedAdd};
pub(crate) use cards::{EDITORS, card_size};
use clients::reject_busy;
pub(crate) use events::DaemonEvent;
use finish::watcher_connected;
pub(crate) use placement::{focused_monitor_name, focused_monitor_origin};

use std::os::unix::net::{UnixListener, UnixStream};

use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::data_device_manager::data_device::DataDevice;
use smithay_client_toolkit::data_device_manager::data_source::DragSource;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::seat::pointer::ThemedPointer;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, LayerShell, LayerSurface};
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use crate::DynResult;
use crate::record::finalize::SaveDestination;
use crate::record::session::{RecordingSession, SessionPhase, StopChildrenJob};
use crate::shelf::layout::{Layout, LayoutConfig};
use crate::shelf::model::{CardKind, ShelfModel, Thumb};

// Reached by the tests through `use super::*`.
#[cfg(test)]
use crate::record::finalize::FinalizedClip;
#[cfg(test)]
use crate::shelf::layout::Hit;
#[cfg(test)]
use crate::shelf::model::FileLifetime;
#[cfg(test)]
use crate::shelf::recording::PopupButton;
#[cfg(test)]
use actions::{shelf_cache_evictions_with_limits, shelf_commit_allowed};
#[cfg(test)]
use cards::{prepare_add, visible_cards};
#[cfg(test)]
use clients::{import_shelf_video, spawn_client_writer, validate_recording_dimensions};
#[cfg(test)]
use controls::{
    ControlsOpen, DISCARD_CONFIRM, controls_open_plan, discard_confirmed, esc_resumes,
    popup_key_button, recording_controls_visible,
};
#[cfg(test)]
use dnd::{drag_mime_types, send_file};
#[cfg(test)]
use placement::{
    card_output, focused_output_from_hyprland_json, monitor_for_geometry, parse_focus_snapshot,
};
#[cfg(test)]
use smithay_client_toolkit::seat::keyboard::Keysym;
#[cfg(test)]
use start::{default_start_plan, requested_audio, resolve_output_name, transformed_mode_size};
#[cfg(test)]
use wayland_client::protocol::wl_output;

pub struct Daemon {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    /// The shelf's buffers, kept across frames (`buffers.rs`).
    buffers: crate::platform::buffers::Buffers,
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
    pending_image_traces: Vec<u64>,
    /// Themed pointer so we can set an explicit cursor over the shelf instead of
    /// inheriting whatever shape the previously-focused window left (e.g. a
    /// terminal's I-beam).
    pointer: Option<ThemedPointer>,
    keyboard: Option<WlKeyboard>,

    ddm: DataDeviceManagerState,
    data_device: Option<DataDevice>,
    drag_source: Option<DragSource>,
    /// File and kind of the card being dragged, until the drag ends.
    drag_card: Option<(std::path::PathBuf, CardKind)>,
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
    /// `--save-dir`; otherwise the Save button follows the config at save time.
    save_dir_cli: Option<std::path::PathBuf>,
    /// Card id + start time of the transient ✓ "saved" flash on its Save button.
    save_flash: Option<(u64, std::time::Instant)>,
    /// When the shelf's last animation frame callback was requested.
    frame_requested: Option<std::time::Instant>,
    saves: save::CardSaves,
    /// Replay clips whose card shows while the worker still writes them.
    pending: pending::PendingClips,
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
    /// Controls text at weights 500 and 600.
    popup_fonts: (ab_glyph::FontVec, ab_glyph::FontVec),
    popup_layout: crate::shelf::paint::controls::ControlsLayout,
    /// The controls paused the recording themselves; Esc resumes it.
    auto_paused: bool,
    /// Show the controls once the pause they started has stopped the recorders.
    controls_pending: bool,
    /// First Discard press; a second one within `DISCARD_CONFIRM` discards.
    discard_armed: Option<std::time::Instant>,
    /// `ui_font` the popup font was loaded for; `None` before the first load.
    popup_font_family: Option<Option<String>>,
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
    shelf_prefs: crate::config::ShelfPrefs,
    settings_writer: settings::Writer,
    /// Writes sent to `settings_writer` that have not finished yet.
    pending_writes: usize,
    settings_watchers: Vec<UnixStream>,
    /// The state last sent to `settings_watchers`.
    last_settings: Option<boltsnap::settings::State>,
    tray: Option<crate::tray::TrayPublisher>,
    /// Whether the replay buffer is capturing, for the tray's on/off entry.
    replay_running: Option<super::replay::Running>,
    thumbnails: thumbnail::Worker,
    watch: watch::Watch,
    /// Why the recording paused itself, for the controls' title.
    pause_title: Option<&'static str>,
}

pub(crate) enum AfterStop {
    Pause,
    Recover(String),
    Save(SaveDestination),
    Discard,
}

/// Best-effort desktop notification (no-op if `notify-send` is missing). Spawned
/// detached so it never blocks the daemon's event loop. Used to surface recording
/// failures that would otherwise only appear in the journal.
pub(crate) fn notify(body: &str) {
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

fn dispatch_timeout(animating: bool, phase: Option<SessionPhase>) -> Option<std::time::Duration> {
    if animating {
        Some(std::time::Duration::from_millis(16))
    } else if phase == Some(SessionPhase::Recording) {
        Some(std::time::Duration::from_millis(250))
    } else {
        None
    }
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

/// A daemon started by the compositor has stderr on /dev/null, which loses
/// recorder and finalize errors. Append them to `daemon.log` in the cache dir
/// instead; the previous run's log is kept as `daemon.log.old`.
fn log_to_file_when_detached() {
    use std::os::fd::AsRawFd;
    if unsafe { libc::isatty(libc::STDERR_FILENO) } == 1 {
        return;
    }
    let dir = crate::paths::cache_dir();
    let path = dir.join("daemon.log");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::rename(&path, dir.join("daemon.log.old"));
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        unsafe {
            libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO);
        }
    }
}

pub fn run_daemon(save_dir_cli: Option<std::path::PathBuf>) -> DynResult<()> {
    // Single instance: the lock decides before any log rotation or cleanup, so
    // a racing second start cannot delete the winner's socket or media.
    let Some(_lock) = crate::ipc::lock_daemon()? else {
        return Ok(());
    };
    // A daemon from before the lock existed may still answer.
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    log_to_file_when_detached();
    let sock = crate::ipc::socket_path();
    crate::ipc::remove_stale_socket(&sock)?;

    // No daemon was running, so the shelf is empty: any leftover shelf tempfiles
    // are orphans from a previous run/crash. Remove them so the RAM-only shelf
    // doesn't leak PNGs to disk indefinitely (this is what filled /tmp).
    let cleaned = crate::paths::clean_orphan_shelf_temps();
    if cleaned > 0 {
        eprintln!("boltsnap daemon: cleaned {cleaned} orphaned shelf tempfile(s)");
    }
    let orphans = crate::record::recover::find(&crate::paths::rec_dir());
    let keep: Vec<_> = orphans.iter().flat_map(|o| o.segments.clone()).collect();
    let cleaned_rec = crate::paths::clean_orphan_rec_files(&keep);
    if cleaned_rec > 0 {
        eprintln!("boltsnap daemon: cleaned {cleaned_rec} orphaned recording file(s)");
    }
    crate::record::recover::save_in_background(orphans);
    if let Err(error) = crate::record::audio::cleanup_stale_mixes() {
        eprintln!("boltsnap daemon: clean up stale recording audio: {error}");
    }
    // Encoder discovery takes about 0.5 s; keep it off the first recording start.
    if crate::paths::has_cmd(crate::platform::gsr::PROGRAM) {
        std::thread::spawn(|| {
            if let Err(error) = crate::platform::gsr::info() {
                eprintln!("boltsnap daemon: gpu-screen-recorder encoder discovery: {error}");
            }
        });
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
    crate::config::on_broken(notify);
    let config = crate::config::Config::load();
    let (recording_prefs, shelf_prefs) = (config.recording_prefs(), config.shelf_prefs());
    cards::use_card_size(shelf_prefs.card);
    let (event_tx, event_rx) = calloop::channel::channel::<DaemonEvent>();
    let settings_writer = settings::Writer::spawn(event_tx.clone());
    let changed_tx = event_tx.clone();
    if let Err(error) = settings::watch_config(crate::config::config_path(), move || {
        let _ = changed_tx.send(DaemonEvent::ConfigChanged);
    }) {
        eprintln!("boltsnap daemon: config edits apply on the next action only: {error}");
    }
    let thumbnails = thumbnail::Worker::spawn(event_tx.clone(), cards::card_size);
    let mut daemon = Daemon {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        buffers: Default::default(),
        compositor,
        layer_shell,
        layer: None,
        output_name: None,
        shelf_configured: false,
        shelf_pending_draw: false,
        pending_image_traces: Vec::new(),
        pointer: None,
        keyboard: None,
        ddm,
        data_device: None,
        drag_source: None,
        drag_card: None,
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
        save_dir_cli,
        save_flash: None,
        frame_requested: None,
        saves: save::CardSaves::default(),
        pending: pending::PendingClips::default(),
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
        popup_layout: crate::shelf::paint::controls::controls_layout(&font::fallback_popup_font()),
        popup_fonts: (font::fallback_popup_font(), font::fallback_popup_font()),
        auto_paused: false,
        controls_pending: false,
        discard_armed: None,
        popup_font_family: None,
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
        recording_prefs,
        shelf_prefs,
        settings_writer,
        pending_writes: 0,
        settings_watchers: Vec::new(),
        last_settings: None,
        tray: None,
        replay_running: None,
        thumbnails,
        watch: watch::Watch::default(),
        pause_title: None,
    };

    // Populate output metadata (names) so we can place the shelf on the focused
    // monitor instead of wherever the compositor defaults to.
    event_queue.roundtrip(&mut daemon)?;
    // No-animation layer rule must be in place before the surface maps.
    prep_shelf_compositor_rules();

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
    let client_limits = ClientLimits::default();
    handle
        .insert_source(source, move |_readiness, listener, daemon: &mut Daemon| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        spawn_client_reader(stream, daemon.event_tx.clone(), &client_limits)
                    }
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

    let _replay = super::replay::serve(daemon.event_tx.clone())?;
    daemon.replay_running = Some(_replay.running());
    daemon.publish_tray_snapshot();

    while !daemon.exit {
        // Only active animations and recorders need periodic work. Pausing and
        // finalizing workers wake calloop through event_tx when they finish.
        let timeout = dispatch_timeout(
            daemon.animating(),
            daemon.recording.as_ref().map(|s| s.phase),
        );
        let timeout = [
            daemon.save_flash_remaining(),
            daemon.discard_confirm_remaining(),
        ]
        .into_iter()
        .fold(timeout, |a, b| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        });
        event_loop
            .dispatch(timeout, &mut daemon)
            .map_err(|e| format!("dispatch: {e}"))?;
        if daemon.animation_timer_due() {
            daemon.tick_animations(&qh);
        }
        daemon.expire_save_flash(&qh);
        daemon.expire_discard_confirm();
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

#[cfg(test)]
mod tests;
