pub mod layout;
pub mod model;
pub mod paint;
pub mod preview;
pub mod thumbnail;

use std::os::unix::net::UnixListener;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
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
        pointer::{BTN_LEFT, BTN_RIGHT, PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
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
use crate::shelf::model::ShelfModel;

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
    pointer: Option<WlPointer>,
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

    model: ShelfModel,
    layout: Layout,
    cfg: LayoutConfig,
    width: u32,
    height: u32,
    hovered: Option<u64>,
    press: Option<PressState>,
    /// The open enlarge ("lightbox") view, if any, on its own overlay surface.
    preview: Option<PreviewState>,
    exit: bool,
    /// Queue handle stashed for use inside calloop callbacks (which get only `&mut Daemon`).
    qh: Option<QueueHandle<Daemon>>,
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

/// The open enlarge view: its own overlay layer-surface + a dedicated pool, plus
/// the full-resolution image to render. Dropping it unmaps the surface.
struct PreviewState {
    surface: LayerSurface,
    pool: SlotPool,
    image: image::RgbaImage,
}

/// Name of the focused Hyprland monitor, via `hyprctl monitors -j`. `None` off
/// Hyprland (then the compositor places the shelf on its default output).
fn focused_monitor_name() -> Option<String> {
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
    paint::draw_shelf(&mut canvas, w, h, &layout, &model, Some(id), &cfg);

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

pub fn run_daemon() -> DynResult<()> {
    // Single-instance: if a daemon already answers, do nothing.
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    let sock = crate::ipc::socket_path();
    let _ = std::fs::remove_file(&sock); // clear stale socket

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Daemon>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let ddm = DataDeviceManagerState::bind(&globals, &qh)?;
    let pool = SlotPool::new(256 * 256 * 4, &shm)?;

    let cfg = LayoutConfig::default();
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
        pointer: None,
        keyboard: None,
        ddm,
        data_device: None,
        drag_source: None,
        drag_path: None,
        drag_icon_pool: None,
        drop_ok: false,
        icon_surface: None,
        model: ShelfModel::new(),
        layout: Layout::compute(&[], &cfg),
        cfg,
        width: 1,
        height: 1,
        hovered: None,
        press: None,
        preview: None,
        exit: false,
        qh: Some(qh.clone()),
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
        event_loop
            .dispatch(std::time::Duration::from_millis(250), &mut daemon)
            .map_err(|e| format!("dispatch: {e}"))?;
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

impl Daemon {
    /// Ensure the shelf surface lives on the currently focused output. Recreates
    /// the layer surface (dropping the old one, which unmaps it) when the focused
    /// monitor changed since last time. Cheap no-op when already correct.
    fn place_on_focused_output(&mut self, qh: &QueueHandle<Self>) {
        let name = focused_monitor_name();
        if self.layer.is_some() && name == self.output_name {
            return;
        }
        let output = name
            .as_ref()
            .and_then(|n| {
                self.output_state.outputs().find(|o| {
                    self.output_state.info(o).and_then(|i| i.name).as_deref() == Some(n.as_str())
                })
            })
            // Fall back to the first available output when the focused monitor
            // can't be resolved (e.g. the daemon was started without
            // HYPRLAND_INSTANCE_SIGNATURE so focused_monitor_name() is None).
            // Hyprland never maps a layer surface created with a null output, so
            // the shelf would silently never appear — always pass a concrete one.
            .or_else(|| self.output_state.outputs().next());
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
    }

    /// Recompute layout from the model and resize the layer surface to match.
    fn relayout(&mut self) {
        let sizes: Vec<(u64, u32, u32)> = self
            .model
            .newest_first()
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        self.layout = Layout::compute(&sizes, &self.cfg);
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
        self.model.add(path, thumb, source.to_string());
        self.relayout();
        self.place_on_focused_output(qh); // follow the user to the active monitor
        self.draw(qh);
    }

    /// Copy the full image of the card under the cursor to the clipboard.
    fn copy_card(&mut self, id: u64) {
        if let Some(t) = self.model.get(id) {
            let path = t.png_path.clone();
            if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland) {
                eprintln!("boltsnap daemon: copy failed: {e}");
            }
        }
    }

    /// Open the centered enlarge ("lightbox") view for thumbnail `id` on its own
    /// overlay surface, leaving the shelf surface underneath untouched.
    fn open_preview(&mut self, id: u64) {
        if self.preview.is_some() {
            return;
        }
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        let image = match image::open(&path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("boltsnap daemon: preview open failed: {e}");
                return;
            }
        };
        let qh = match self.qh.clone() {
            Some(q) => q,
            None => return,
        };
        let surface = self.compositor.create_surface(&qh);
        let layer = self.layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("boltsnap-preview"),
            None,
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0); // fill the output; the real size arrives in configure
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        let pool = match SlotPool::new(256, &self.shm) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("boltsnap daemon: preview pool failed: {e}");
                return;
            }
        };
        self.preview = Some(PreviewState {
            surface: layer,
            pool,
            image,
        });
    }

    /// Close the enlarge view; dropping the surface unmaps it.
    fn close_preview(&mut self) {
        self.preview = None;
    }

    /// Act on a completed (non-drag) left click.
    fn on_click(&mut self, hit: Hit, redraw: &mut bool) {
        match hit {
            Hit::Body(id) => {
                self.open_preview(id);
            }
            Hit::Close(id) => {
                if let Some(t) = self.model.remove(id) {
                    let _ = std::fs::remove_file(&t.png_path);
                    if self.hovered == Some(id) {
                        self.hovered = None;
                    }
                    self.relayout();
                    *redraw = true;
                }
            }
            Hit::Edit(id) => {
                self.spawn_editor(id);
            }
        }
    }

    /// Start a Wayland drag for thumbnail `id`, offering image/png + a file URI,
    /// with the thumbnail itself as the drag icon.
    fn begin_drag(&mut self, id: u64, serial: u32) {
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return,
        };
        if self.data_device.is_none() {
            return;
        }
        let origin = match self.layer.as_ref() {
            Some(l) => l.wl_surface().clone(),
            None => return,
        };

        // Build a drag icon from the thumbnail, in its OWN pool kept alive for the
        // whole drag — the shelf's `pool` would otherwise reuse the slot mid-drag
        // and leave a "ghost". Crisp Lanczos scale, ~85% opacity, rounded corners.
        let icon = self.compositor.create_surface(&qh);
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
                    icon.commit();
                }
                self.drag_icon_pool = Some(pool);
            }
        }

        let source = self.ddm.create_drag_and_drop_source(
            &qh,
            ["image/png", "text/uri-list"],
            DndAction::Copy,
        );
        let device = self.data_device.as_ref().unwrap();
        source.start_drag(device, &origin, Some(&icon), serial);

        self.drag_path = Some(path);
        self.drop_ok = false;
        self.icon_surface = Some(icon);
        self.drag_source = Some(source);
    }

    /// Open the annotation editor for thumbnail `id` in a child process; when it
    /// saves (overwriting the temp PNG in place), ask the daemon to reload that
    /// thumbnail via its own socket.
    fn spawn_editor(&mut self, id: u64) {
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

    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        let layer = match self.layer.as_ref() {
            Some(l) => l,
            None => return,
        };
        let (w, h) = (self.width.max(1), self.height.max(1));
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
        );

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
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
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // Is this the enlarge-view surface? Render the lightbox into it and stop.
        if let Some(pv) = self.preview.as_mut() {
            if pv.surface.wl_surface() == layer.wl_surface() {
                let w = configure.new_size.0.max(1);
                let h = configure.new_size.1.max(1);
                let stride = (w * 4) as i32;
                let img = pv.image.clone();
                if let Ok((buffer, canvas)) = pv.pool.create_buffer(
                    w as i32,
                    h as i32,
                    stride,
                    wayland_client::protocol::wl_shm::Format::Argb8888,
                ) {
                    crate::shelf::preview::render_lightbox(canvas, w, h, &img, 48);
                    let surface = pv.surface.wl_surface();
                    surface.damage_buffer(0, 0, w as i32, h as i32);
                    let _ = buffer.attach_to(surface);
                    surface.commit();
                }
                return;
            }
        }
        if configure.new_size.0 != 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 != 0 {
            self.height = configure.new_size.1;
        }
        self.draw(qh);
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
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
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
        _: &Connection,
        _qh: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        let surface = match self.layer.as_ref() {
            Some(l) => l.wl_surface().clone(),
            None => return,
        };
        let mut redraw = false;
        for ev in events {
            // A click anywhere on the enlarge view closes it.
            let on_preview = self
                .preview
                .as_ref()
                .map(|pv| ev.surface == *pv.surface.wl_surface())
                .unwrap_or(false);
            if on_preview {
                if matches!(ev.kind, PointerEventKind::Press { .. }) {
                    self.close_preview();
                }
                continue;
            }
            if ev.surface != surface {
                continue;
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
                        Hit::Body(id) | Hit::Edit(id) | Hit::Close(id) => id,
                    });
                    if now != self.hovered {
                        self.hovered = now;
                        redraw = true;
                    }
                    if let Some(p) = self.press.as_mut() {
                        if !p.dragging {
                            let dx = x - p.x;
                            let dy = y - p.y;
                            if (dx * dx + dy * dy) > 36.0 && matches!(p.hit, Hit::Body(_)) {
                                p.dragging = true;
                                let (id, serial) = (p.id, p.serial);
                                self.begin_drag(id, serial);
                            }
                        }
                    }
                }
                PointerEventKind::Press { button, serial, .. } if button == BTN_LEFT => {
                    if let Some(hit) = self.layout.hit(x, y, &self.cfg) {
                        let id = match hit {
                            Hit::Body(i) | Hit::Edit(i) | Hit::Close(i) => i,
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
                            Hit::Body(i) | Hit::Edit(i) | Hit::Close(i) => i,
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
        // Esc closes the enlarge view.
        if self.preview.is_some() && event.keysym == Keysym::Escape {
            self.close_preview();
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
        let mut file = std::fs::File::from(OwnedFd::from(fd));
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
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        if !self.drop_ok {
            if let Some(path) = self.drag_path.clone() {
                if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland)
                {
                    eprintln!("boltsnap daemon: fallback copy failed: {e}");
                }
            }
        }
        self.drag_source = None;
        self.drag_path = None;
        self.icon_surface = None;
        self.drag_icon_pool = None;
        self.press = None;
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drop_ok = true;
    }

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drag_source = None;
        self.drag_path = None;
        self.icon_surface = None;
        self.drag_icon_pool = None;
        self.press = None;
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
