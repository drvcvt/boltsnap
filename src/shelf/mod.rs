pub mod layout;
pub mod model;
pub mod paint;
pub mod recording;
pub mod thumbnail;

use std::os::unix::net::UnixListener;

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
use crate::shelf::layout::{Hit, Layout, LayoutConfig};
use crate::shelf::model::{CardKind, ShelfModel};
use crate::shelf::recording::{IndButton, RecPhase, Recording};

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
    press: Option<PressState>,
    /// In-flight per-card appear/dismiss animations.
    anims: Vec<CardAnim>,
    exit: bool,
    /// Queue handle stashed for use inside calloop callbacks (which get only `&mut Daemon`).
    qh: Option<QueueHandle<Daemon>>,
    /// Active area recording (wf-recorder child + overlay surfaces), if any. Only
    /// one recording at a time.
    recording: Option<Recording>,
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

/// Duration of a card appear/dismiss animation.
const ANIM_MS: u128 = 150;
/// How long the ✓ "saved" flash stays on the Save button.
const SAVE_FLASH_MS: u128 = 700;
/// Card scale at the far end of the animation (start of appear / end of dismiss).
const ANIM_SCALE_MIN: f32 = 0.88;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimKind {
    Appear,
    Disappear,
}

/// A running per-card animation. `start` is when it began; progress is
/// `elapsed / ANIM_MS`, eased, until it reaches 1.0 and is retired.
struct CardAnim {
    id: u64,
    kind: AnimKind,
    start: std::time::Instant,
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
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    for m in v.as_array()? {
        if m.get("focused").and_then(|f| f.as_bool()) == Some(true) {
            return m.get("name").and_then(|n| n.as_str()).map(str::to_string);
        }
    }
    None
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

/// Render the shelf with one sample thumbnail (hovered, so the buttons show) to
/// a PNG via the exact production draw path. Lets styling be inspected without a
/// compositor. Converts the premultiplied BGRA canvas back to straight RGBA.
pub fn debug_render(out: &std::path::Path) -> DynResult<()> {
    use image::{Rgba, RgbaImage};
    let (tw, th) = (thumbnail::CARD_W, thumbnail::CARD_H);
    let mut sample = RgbaImage::new(tw, th);
    for (x, y, p) in sample.enumerate_pixels_mut() {
        // a colourful gradient so corners/border are easy to see
        *p = Rgba([(x * 255 / tw) as u8, (y * 200 / th) as u8, 170, 255]);
    }
    let mut model = ShelfModel::new();
    let id = model.add(
        std::path::PathBuf::from("/tmp/x.png"),
        sample,
        "area".into(),
    );
    let cfg = LayoutConfig::default();
    let sizes: Vec<(u64, u32, u32)> = model
        .newest_first()
        .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
        .collect();
    let layout = Layout::compute(&sizes, &cfg);
    let (w, h) = (layout.width, layout.height);
    let mut canvas = vec![0u8; (w * h * 4) as usize];
    paint::draw_shelf(
        &mut canvas,
        w,
        h,
        &layout,
        &model,
        Some(id),
        &cfg,
        &[],
        None,
    );

    // BGRA premultiplied -> composite over mid-gray so the rounded corners
    // (transparent) and the white border are clearly visible when inspected.
    let bg = 64u32;
    let mut img = RgbaImage::new(w, h);
    for (i, px) in canvas.chunks_exact(4).enumerate() {
        let (pb, pg, pr, a) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
        // premultiplied source over opaque gray: out = src + bg*(1-a)
        let inv = 255 - a;
        let r = (pr + bg * inv / 255).min(255) as u8;
        let g = (pg + bg * inv / 255).min(255) as u8;
        let b = (pb + bg * inv / 255).min(255) as u8;
        let x = (i as u32) % w;
        let y = (i as u32) / w;
        img.put_pixel(x, y, Rgba([r, g, b, 255]));
    }
    img.save(out)?;
    eprintln!("debug-render: wrote {}x{} to {}", w, h, out.display());
    Ok(())
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
        press: None,
        anims: Vec::new(),
        exit: false,
        qh: Some(qh.clone()),
        recording: None,
    };

    // Populate output metadata (names) so we can place the shelf on the focused
    // monitor instead of wherever the compositor defaults to.
    event_queue.roundtrip(&mut daemon)?;
    // No-animation layer rule must be in place before the surface maps.
    prep_shelf_compositor_rules();
    daemon.place_on_focused_output(&qh);

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

    let source = Generic::new(listener, Interest::READ, Mode::Level);
    handle
        .insert_source(source, |_readiness, listener, daemon: &mut Daemon| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => daemon.handle_client(stream),
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
        // Refresh the recording indicator's MM:SS and reap a finished/failed child.
        daemon.tick_recording(&qh);
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

impl Daemon {
    fn target_output(&self) -> (Option<String>, Option<wl_output::WlOutput>) {
        let outputs: Vec<_> = self.output_state.outputs().collect();
        if outputs.len() <= 1 {
            return (None, outputs.into_iter().next());
        }
        let name = focused_monitor_name();
        let output = name
            .as_ref()
            .and_then(|n| {
                outputs
                    .iter()
                    .find(|o| {
                        self.output_state.info(o).and_then(|i| i.name).as_deref()
                            == Some(n.as_str())
                    })
                    .cloned()
            })
            // Fall back to the first available output when the focused monitor
            // can't be resolved. Hyprland may fail to map a layer surface with a
            // null output, so always pass a concrete one when we have it.
            .or_else(|| outputs.into_iter().next());
        (name, output)
    }

    /// Ensure the shelf surface lives on the currently focused output. Recreates
    /// the layer surface (dropping the old one, which unmaps it) when the focused
    /// monitor changed since last time. Returns true when a fresh surface was
    /// created and must wait for its initial configure before drawing.
    fn place_on_focused_output(&mut self, qh: &QueueHandle<Self>) -> bool {
        let (name, output) = self.target_output();
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
        let sizes: Vec<(u64, u32, u32)> = self
            .model
            .newest_first()
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

    /// Handle one client connection: read a single request and act on it.
    fn handle_client(&mut self, mut stream: std::os::unix::net::UnixStream) {
        use std::io::Write;
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let req = match crate::ipc::Request::read(&mut stream) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("boltsnap daemon: bad request: {e}");
                return;
            }
        };
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return,
        };
        match req {
            crate::ipc::Request::Ping => {
                let _ = stream.write_all(b"PONG");
            }
            crate::ipc::Request::Add { source, png } => {
                self.add_png(&png, &source, &qh);
            }
            crate::ipc::Request::Reload { id } => {
                self.reload(id, &qh);
            }
            crate::ipc::Request::StartRecording { x, y, w, h } => {
                self.start_recording(x, y, w, h, &qh);
            }
            crate::ipc::Request::RecordingDone { video, thumb } => {
                self.ingest_recording(video, thumb, &qh);
            }
        }
    }

    /// Re-read a thumbnail's PNG from disk (after the editor overwrote it) and refresh.
    fn reload(&mut self, id: u64, qh: &QueueHandle<Self>) {
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        if let Ok(img) = image::open(&path) {
            let thumb = crate::shelf::thumbnail::make_card_thumbnail(
                &img.to_rgba8(),
                crate::shelf::thumbnail::CARD_W,
                crate::shelf::thumbnail::CARD_H,
            );
            self.model.replace_thumb(id, thumb);
            self.relayout();
            self.draw(qh);
        }
    }

    /// Ingest a PNG: persist a daemon-owned temp copy, scale a thumbnail, show it
    /// on the currently focused monitor.
    fn add_png(&mut self, png: &[u8], source: &str, qh: &QueueHandle<Self>) {
        let img = match image::load_from_memory(png) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("boltsnap daemon: bad PNG: {e}");
                return;
            }
        };
        // daemon-owned temp file for the editor + drag uri-list
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
        let id = self.model.add(path, thumb, source.to_string());
        self.start_anim(id, AnimKind::Appear);
        self.relayout();
        self.place_on_focused_output(qh); // follow the user to the active monitor
        self.draw(qh);
    }

    /// Copy the full image of the card under the cursor to the clipboard.
    /// For video cards this is a no-op (raw video bytes are not meaningful on the
    /// clipboard; use drag or Save instead).
    fn copy_card(&mut self, id: u64) {
        if let Some(t) = self.model.get(id) {
            if t.kind == crate::shelf::model::CardKind::Video {
                eprintln!(
                    "boltsnap daemon: right-click copy not supported for video cards \
                     (use drag or Save)"
                );
                return;
            }
            let path = t.png_path.clone();
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
            Some(t) => (t.png_path.clone(), t.kind),
            None => return,
        };
        let stamp = crate::paths::local_timestamp();
        let (dir, dest) = match kind {
            crate::shelf::model::CardKind::Image => {
                let name = crate::paths::boltsnap_filename_ext(&stamp, "png");
                (self.save_dir.clone(), self.save_dir.join(name))
            }
            crate::shelf::model::CardKind::Video => {
                let ext = src
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("mp4");
                let name = crate::paths::boltsnap_filename_ext(&stamp, ext);
                let rec_dir =
                    crate::config::resolve_record_dir(&crate::config::Config::load());
                (rec_dir.clone(), rec_dir.join(name))
            }
        };
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
            Hit::Body(id) => {
                // video cards open in eddy too (eddy gaining video playback)
                self.open_in_eddy(id);
            }
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
                    canvas.copy_from_slice(&bytes);
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

    /// Open thumbnail `id` in eddy (the viewer + annotation editor) in a child
    /// process; when it saves (overwriting the temp PNG in place), ask the daemon
    /// to reload that thumbnail via its own socket.
    fn open_in_eddy(&mut self, id: u64) {
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        std::thread::spawn(move || {
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(_) => return,
            };
            let status = std::process::Command::new(exe)
                .arg("edit")
                .arg(&path)
                .arg("-o")
                .arg(&path)
                .arg("--no-copy")
                .status();
            if matches!(status, Ok(s) if s.success()) {
                let _ = crate::ipc::send_to_shelf(crate::ipc::Request::Reload { id });
            }
        });
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
            let t = (a.start.elapsed().as_millis() as f32 / ANIM_MS as f32).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3); // ease-out cubic
            let span = 1.0 - ANIM_SCALE_MIN;
            return match a.kind {
                AnimKind::Appear => (ANIM_SCALE_MIN + span * eased, eased),
                AnimKind::Disappear => (1.0 - span * eased, 1.0 - eased),
            };
        }
        (1.0, 1.0)
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
            .filter(|a| a.start.elapsed().as_millis() >= ANIM_MS)
            .map(|a| (a.id, a.kind))
            .collect();
        self.anims
            .retain(|a| a.start.elapsed().as_millis() < ANIM_MS);
        let mut removed = false;
        for (id, kind) in done {
            if kind == AnimKind::Disappear
                && let Some(t) = self.model.remove(id)
            {
                let _ = std::fs::remove_file(&t.png_path);
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

    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        if !self.shelf_configured {
            self.shelf_pending_draw = true;
            return;
        }
        // NOTE: we deliberately do NOT skip drawing while a drag is active.
        // An earlier version early-returned here whenever `drag_source.is_some()`
        // to avoid origin-surface commits during the grab — but if a drag never
        // delivers a terminal event (e.g. the compositor rejects start_drag in
        // some modes and sends neither dnd_finished nor cancelled), `drag_source`
        // stays Some forever and the shelf is permanently frozen (dead to clicks,
        // remove, preview). That latch is far worse than the redraw it prevented,
        // and the real freeze cause (a synchronous pipe write) is fixed by
        // threading send_request, so this suppression is no longer needed.
        let layer = match self.layer.as_ref() {
            Some(l) => l,
            None => return,
        };
        let (w, h) = (self.width.max(1), self.height.max(1));
        // Per-card (scale, opacity) for in-flight appear/dismiss animations. Built
        // before create_buffer because `canvas` mutably borrows self.pool, which
        // would otherwise block the immutable self borrow anim_factor needs.
        let anims: Vec<(u64, f32, f32)> = self
            .layout
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
            &self.layout,
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

    // ----- Area recording lifecycle --------------------------------------

    /// True while a recording is actively counting up (Recording phase). Drives
    /// the loop's per-second indicator refresh; the Stopped phase doesn't tick.
    fn recording_counting(&self) -> bool {
        matches!(
            self.recording.as_ref().map(|r| r.phase),
            Some(RecPhase::Recording)
        )
    }

    /// Begin recording the global-coords region (x,y,w,h): spawn wf-recorder,
    /// raise a click-through marker around the rect and a control indicator near
    /// the shelf. One recording at a time.
    fn start_recording(&mut self, x: i32, y: i32, w: u32, h: u32, qh: &QueueHandle<Self>) {
        if self.recording.is_some() {
            eprintln!("boltsnap daemon: a recording is already in progress");
            return;
        }
        if !crate::paths::has_cmd("wf-recorder") {
            eprintln!("boltsnap daemon: wf-recorder not found — cannot record");
            return;
        }
        let geo = crate::record::Geometry { x, y, w, h };
        let path = crate::paths::temp_file("rec", "mp4");
        let codec = crate::config::resolve_record_codec(None, &crate::config::Config::load());

        let child = match std::process::Command::new("wf-recorder")
            .args(crate::record::wf_recorder_args(&geo, &codec, &path))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("boltsnap daemon: failed to spawn wf-recorder: {e}");
                return;
            }
        };

        // Control indicator near the shelf. If even this tiny pool can't be made,
        // abort cleanly (SIGINT + reap the just-spawned child) rather than recording
        // headlessly with no way to stop it.
        let (indicator, indicator_pool) = match self.create_indicator(qh) {
            Some(v) => v,
            None => {
                eprintln!("boltsnap daemon: could not allocate indicator buffer; aborting record");
                let mut child = child;
                unsafe {
                    libc::kill(child.id() as i32, libc::SIGINT);
                }
                let _ = child.wait();
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        // Click-through region marker (border just OUTSIDE the recorded rect).
        let (marker, marker_pool, marker_region) = self.create_marker(&geo, qh);

        self.recording = Some(Recording {
            child: Some(child),
            path,
            started: std::time::Instant::now(),
            geo,
            marker,
            marker_pool,
            marker_region,
            marker_configured: false,
            indicator,
            indicator_pool,
            indicator_configured: false,
            phase: RecPhase::Recording,
            last_drawn_secs: None,
        });
        eprintln!("boltsnap daemon: recording {} -> {:?}", geo.to_arg(), codec);
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
    fn create_marker(
        &mut self,
        geo: &crate::record::Geometry,
        qh: &QueueHandle<Self>,
    ) -> (Option<LayerSurface>, Option<SlotPool>, Option<Region>) {
        use crate::shelf::recording::MARKER_INFLATE;
        let inflate = MARKER_INFLATE as i32;
        let mw = geo.w + 2 * MARKER_INFLATE;
        let mh = geo.h + 2 * MARKER_INFLATE;

        let surface = self.compositor.create_surface(qh);
        let (_name, output) = self.target_output();
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
        let (ox, oy) = focused_monitor_origin().unwrap_or((0, 0));
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

        let pool = SlotPool::new((mw * mh * 4) as usize, &self.shm).ok();
        (Some(layer), pool, region)
    }

    /// Build the recording control indicator layer surface, anchored bottom-left
    /// above the shelf. `None` if its (tiny) draw pool can't be allocated.
    fn create_indicator(&mut self, qh: &QueueHandle<Self>) -> Option<(LayerSurface, SlotPool)> {
        use crate::shelf::recording::{IND_H, IND_W};
        let pool = SlotPool::new((IND_W * IND_H * 4) as usize, &self.shm).ok()?;
        let surface = self.compositor.create_surface(qh);
        let (_name, output) = self.target_output();
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some("boltsnap-recording"),
            output.as_ref(),
        );
        layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
        // Sit above the shelf (shelf is at bottom-left margin 24 with its own
        // height). Offset by an estimate so the two don't overlap.
        let above = (self.height as i32).max(60) + 48;
        layer.set_margin(0, 0, above, 24);
        layer.set_size(IND_W, IND_H);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        layer.commit();
        Some((layer, pool))
    }

    /// Draw the click-through marker border into its surface.
    fn draw_marker(&mut self) {
        let rec = match self.recording.as_mut() {
            Some(r) => r,
            None => return,
        };
        if !rec.marker_configured {
            return;
        }
        let (layer, pool) = match (rec.marker.as_ref(), rec.marker_pool.as_mut()) {
            (Some(l), Some(p)) => (l, p),
            _ => return,
        };
        use crate::shelf::recording::MARKER_INFLATE;
        let mw = rec.geo.w + 2 * MARKER_INFLATE;
        let mh = rec.geo.h + 2 * MARKER_INFLATE;
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
        );
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, mw as i32, mh as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
    }

    /// Draw the indicator (●+MM:SS+Stop, or Confirm/Cancel) into its surface.
    fn draw_indicator(&mut self) {
        use crate::shelf::recording::{IND_H, IND_W};
        let rec = match self.recording.as_mut() {
            Some(r) => r,
            None => return,
        };
        if !rec.indicator_configured {
            return;
        }
        let phase = rec.phase;
        let elapsed = crate::shelf::recording::fmt_elapsed(rec.started.elapsed().as_secs());
        let stride = (IND_W * 4) as i32;
        let (buffer, canvas) = match rec.indicator_pool.create_buffer(
            IND_W as i32,
            IND_H as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        crate::shelf::paint::draw_indicator(canvas, IND_W, IND_H, phase, &elapsed);
        let surface = rec.indicator.wl_surface();
        surface.damage_buffer(0, 0, IND_W as i32, IND_H as i32);
        let _ = buffer.attach_to(surface);
        rec.indicator.commit();
    }

    /// Per-loop recording housekeeping: refresh the MM:SS readout on whole-second
    /// boundaries, and detect a wf-recorder that died/exited on its own.
    fn tick_recording(&mut self, _qh: &QueueHandle<Self>) {
        let teardown = {
            let rec = match self.recording.as_mut() {
                Some(r) => r,
                None => return,
            };
            // While recording, did the child exit unexpectedly (crash/error)?
            let mut dead = false;
            if rec.phase == RecPhase::Recording {
                if let Some(child) = rec.child.as_mut() {
                    if let Ok(Some(status)) = child.try_wait() {
                        eprintln!("boltsnap daemon: wf-recorder exited early: {status}");
                        dead = true;
                    }
                }
            }
            dead
        };
        if teardown {
            self.cancel_recording_cleanup();
            return;
        }
        // Repaint the elapsed time only when the whole second changed.
        if self.recording_counting() {
            let secs = self
                .recording
                .as_ref()
                .map(|r| r.started.elapsed().as_secs());
            let changed = self
                .recording
                .as_ref()
                .map(|r| r.last_drawn_secs != secs)
                .unwrap_or(false);
            if changed {
                if let Some(r) = self.recording.as_mut() {
                    r.last_drawn_secs = secs;
                }
                self.draw_indicator();
            }
        }
    }

    /// Stop the active recording: SIGINT wf-recorder (so it FINALIZES the mp4 —
    /// `Child::kill` only sends SIGKILL, which would truncate/corrupt it), drop
    /// the marker, and switch the indicator to Confirm/Cancel. The child is NOT
    /// reaped here; Confirm/Cancel `wait()`s it off-thread once the user decides
    /// (so Confirm's ffmpeg sees the fully-finalized file).
    fn stop_recording(&mut self) {
        let rec = match self.recording.as_mut() {
            Some(r) if r.phase == RecPhase::Recording => r,
            _ => return,
        };
        if let Some(child) = rec.child.as_ref() {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
        }
        // Drop the click-through marker frame; a finished recording needn't be framed.
        rec.marker = None;
        rec.marker_pool = None;
        rec.marker_region = None;
        rec.phase = RecPhase::Stopped;
        self.draw_indicator();
    }

    /// Confirm the stopped recording: on a DETACHED thread, `wait()` for
    /// wf-recorder to finish finalizing the mp4, extract a first-frame thumbnail
    /// with ffmpeg, then post `RecordingDone` back to our own socket. Nothing here
    /// blocks the calloop loop. The indicator stays up until `ingest_recording`.
    fn confirm_recording(&mut self) {
        let rec = match self.recording.as_mut() {
            Some(r) if r.phase == RecPhase::Stopped => r,
            _ => return,
        };
        let child = rec.child.take();
        let video = rec.path.clone();
        let thumb = crate::paths::temp_file("rec-thumb", "png");
        std::thread::spawn(move || {
            // Wait for wf-recorder to finish writing the mp4 (SIGINT already sent).
            if let Some(mut c) = child {
                let _ = c.wait();
            }
            // First-frame thumbnail (best-effort; ingest tolerates a missing png).
            let _ = std::process::Command::new("ffmpeg")
                .args(["-y", "-i"])
                .arg(&video)
                .args(["-frames:v", "1", "-update", "1"])
                .arg(&thumb)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let _ = crate::ipc::send_to_shelf(crate::ipc::Request::RecordingDone { video, thumb });
        });
    }

    /// Tear down the active recording without keeping its output: SIGINT the
    /// child if we still own it (so a still-LIVE wf-recorder actually finalizes
    /// rather than being orphaned), then reap it and unlink the temp mp4 on a
    /// DETACHED thread, and drop both overlays immediately on the calloop thread.
    ///
    /// This MUST NOT block the loop: it is reached both from the Cancel button
    /// and from `LayerShellHandler::closed`, where the recorder may still be live
    /// (a synchronous `wait()` there would freeze the daemon until it exits on its
    /// own). The crash-path caller (`tick_recording`) only invokes this AFTER
    /// `try_wait()` already saw the child dead, so the off-thread `wait()` returns
    /// at once there.
    fn cancel_recording_cleanup(&mut self) {
        if let Some(mut rec) = self.recording.take() {
            let child = rec.child.take();
            let path = rec.path.clone();
            // SIGINT only when we still own the child (don't signal a pid we may
            // have already waited). Mirror stop_recording: SIGINT, never SIGKILL.
            if let Some(c) = child.as_ref() {
                unsafe {
                    libc::kill(c.id() as i32, libc::SIGINT);
                }
            }
            std::thread::spawn(move || {
                if let Some(mut c) = child {
                    let _ = c.wait();
                }
                let _ = std::fs::remove_file(&path);
            });
        }
        // `rec` dropped here -> marker + indicator surfaces (and the region) unmap.
    }

    /// Ingest a finished recording posted via `RecordingDone`: load the first-frame
    /// thumbnail (placeholder if missing), add a Video card, tear down the
    /// indicator. The temp mp4 path becomes the card's file.
    fn ingest_recording(
        &mut self,
        video: std::path::PathBuf,
        thumb: std::path::PathBuf,
        qh: &QueueHandle<Self>,
    ) {
        // Build the card thumbnail from the extracted first frame; fall back to a
        // neutral placeholder so a missing/unreadable png never loses the video.
        let frame = match image::open(&thumb) {
            Ok(img) => img.to_rgba8(),
            Err(_) => {
                let mut ph = image::RgbaImage::new(
                    crate::shelf::thumbnail::CARD_W,
                    crate::shelf::thumbnail::CARD_H,
                );
                for p in ph.pixels_mut() {
                    *p = image::Rgba([32, 32, 40, 255]);
                }
                ph
            }
        };
        // The first-frame png was only needed to build the thumbnail.
        let _ = std::fs::remove_file(&thumb);

        let card_thumb = crate::shelf::thumbnail::make_card_thumbnail(
            &frame,
            crate::shelf::thumbnail::CARD_W,
            crate::shelf::thumbnail::CARD_H,
        );
        let id = self
            .model
            .add_kind(video, card_thumb, "record".into(), CardKind::Video);
        // Tear down the indicator + any remaining overlay; recording is done.
        self.recording = None;
        self.start_anim(id, AnimKind::Appear);
        self.relayout();
        self.place_on_focused_output(qh);
        self.draw(qh);
    }

    /// True if `surface` is the active recording's indicator surface.
    fn is_indicator_surface(&self, surface: &WlSurface) -> bool {
        self.recording
            .as_ref()
            .map(|r| r.indicator.wl_surface() == surface)
            .unwrap_or(false)
    }

    /// True if `surface` is the active recording's click-through marker surface.
    fn is_marker_surface(&self, surface: &WlSurface) -> bool {
        self.recording
            .as_ref()
            .and_then(|r| r.marker.as_ref())
            .map(|m| m.wl_surface() == surface)
            .unwrap_or(false)
    }

    /// Handle a left-click at indicator-local `pos`: hit-test the current phase's
    /// buttons and act (Stop / Confirm / Cancel).
    fn on_indicator_click(&mut self, pos: (f64, f64)) {
        let phase = match self.recording.as_ref() {
            Some(r) => r.phase,
            None => return,
        };
        match crate::shelf::recording::ind_hit(phase, pos.0, pos.1) {
            Some(IndButton::Stop) => self.stop_recording(),
            Some(IndButton::Confirm) => self.confirm_recording(),
            Some(IndButton::Cancel) => self.cancel_recording_cleanup(),
            None => {}
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
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Daemon {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // A recording overlay being closed by the compositor: tear the recording
        // down (reap the child, drop overlays) instead of exiting the daemon.
        if self.is_indicator_surface(layer.wl_surface())
            || self.is_marker_surface(layer.wl_surface())
        {
            self.cancel_recording_cleanup();
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
        // Route recording overlays first: they are self-sized like the shelf, so
        // mark them configured and draw their (fixed-size) content.
        if self.is_indicator_surface(layer.wl_surface()) {
            if let Some(r) = self.recording.as_mut() {
                r.indicator_configured = true;
            }
            self.draw_indicator();
            return;
        }
        if self.is_marker_surface(layer.wl_surface()) {
            if let Some(r) = self.recording.as_mut() {
                r.marker_configured = true;
            }
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
            // Route clicks on the recording indicator to its Stop/Confirm/Cancel
            // buttons. (The marker has an empty input region, so it never gets
            // pointer events.)
            if self.is_indicator_surface(&ev.surface) {
                if matches!(ev.kind, PointerEventKind::Enter { .. }) {
                    if let Some(p) = self.pointer.as_ref() {
                        let _ = p.set_cursor(conn, CursorIcon::Default);
                    }
                }
                if let PointerEventKind::Press { button, .. } = ev.kind {
                    if button == BTN_LEFT {
                        self.on_indicator_click(ev.position);
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
        _: KeyEvent,
    ) {
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
                    let abs = std::fs::canonicalize(&path).unwrap_or(path);
                    let uri = format!("file://{}\r\n", abs.display());
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
