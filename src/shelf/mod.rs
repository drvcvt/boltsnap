pub mod layout;
pub mod model;
pub mod paint;
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
    delegate_compositor, delegate_data_device, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler},
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
        wl_data_source::WlDataSource, wl_output, wl_pointer::WlPointer, wl_seat::WlSeat,
        wl_surface::WlSurface,
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
    layer: LayerSurface,
    pointer: Option<WlPointer>,

    ddm: DataDeviceManagerState,
    data_device: Option<DataDevice>,
    drag_source: Option<DragSource>,
    drag_path: Option<std::path::PathBuf>,
    drop_ok: bool,
    icon_surface: Option<WlSurface>,

    model: ShelfModel,
    layout: Layout,
    cfg: LayoutConfig,
    width: u32,
    height: u32,
    hovered: Option<u64>,
    press: Option<PressState>,
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

pub fn run_daemon() -> DynResult<()> {
    // Single-instance: if a daemon already answers, do nothing.
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    let sock = crate::ipc::socket_path();
    let _ = std::fs::remove_file(&sock); // clear stale socket

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<Daemon>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let ddm = DataDeviceManagerState::bind(&globals, &qh)?;
    let pool = SlotPool::new(256 * 256 * 4, &shm)?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("boltsnap"), None);
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
    layer.set_margin(0, 0, 24, 24);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_size(1, 1);
    layer.commit();

    let cfg = LayoutConfig::default();
    let mut daemon = Daemon {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        layer,
        pointer: None,
        ddm,
        data_device: None,
        drag_source: None,
        drag_path: None,
        drop_ok: false,
        icon_surface: None,
        model: ShelfModel::new(),
        layout: Layout::compute(&[], &cfg),
        cfg,
        width: 1,
        height: 1,
        hovered: None,
        press: None,
        exit: false,
        qh: Some(qh.clone()),
    };

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
    /// Recompute layout from the model and resize the layer surface to match.
    fn relayout(&mut self) {
        let sizes: Vec<(u64, u32, u32)> = self
            .model
            .newest_first()
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        self.layout = Layout::compute(&sizes, &self.cfg);
        self.layer.set_size(self.layout.width, self.layout.height);
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
            let thumb = crate::shelf::thumbnail::make_thumbnail(&img.to_rgba8(), 170, 120);
            self.model.replace_thumb(id, thumb);
            self.relayout();
            self.draw(qh);
        }
    }

    /// Ingest a PNG: persist a daemon-owned temp copy, scale a thumbnail, show it.
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
        let thumb = crate::shelf::thumbnail::make_thumbnail(&img, 170, 120);
        self.model.add(path, thumb, source.to_string());
        self.relayout();
        self.draw(qh);
    }

    /// Act on a completed (non-drag) left click.
    fn on_click(&mut self, hit: Hit, redraw: &mut bool) {
        match hit {
            Hit::Body(id) | Hit::Copy(id) => {
                if let Some(t) = self.model.get(id) {
                    let path = t.png_path.clone();
                    if let Err(e) =
                        crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland)
                    {
                        eprintln!("boltsnap daemon: copy failed: {e}");
                    }
                }
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

        // Build a drag icon surface from the thumbnail so it follows the cursor.
        let icon = self.compositor.create_surface(&qh);
        if let Some(t) = self.model.get(id) {
            let (iw, ih) = t.thumb.dimensions();
            let stride = (iw * 4) as i32;
            if let Ok((buf, canvas)) = self.pool.create_buffer(
                iw as i32,
                ih as i32,
                stride,
                wayland_client::protocol::wl_shm::Format::Argb8888,
            ) {
                crate::shelf::paint::clear(canvas);
                crate::shelf::paint::blit_rgba(canvas, iw, ih, &t.thumb, 0, 0);
                let _ = buf.attach_to(&icon);
                icon.commit();
            }
        }

        let source =
            self.ddm
                .create_drag_and_drop_source(&qh, ["image/png", "text/uri-list"], DndAction::Copy);
        let device = self.data_device.as_ref().unwrap();
        source.start_drag(device, self.layer.wl_surface(), Some(&icon), serial);

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
        let (w, h) = (self.width.max(1), self.height.max(1));
        let stride = (w * 4) as i32;
        // Recreate the buffer each draw; SlotPool hands back a fresh slot when the
        // compositor still holds the previous one, so this is double-buffer safe.
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
        // canvas's borrow of self.pool ends here; safe to touch self.layer.

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        let _ = buffer.attach_to(surface);
        self.layer.commit();
    }
}

impl CompositorHandler for Daemon {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {
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
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
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
        if self.data_device.is_none() {
            self.data_device = Some(self.ddm.get_data_device(qh, &seat));
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: Capability) {
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
        let surface = self.layer.wl_surface().clone();
        let mut redraw = false;
        for ev in events {
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
                        Hit::Body(id) | Hit::Edit(id) | Hit::Copy(id) | Hit::Close(id) => id,
                    });
                    if now != self.hovered {
                        self.hovered = now;
                        redraw = true;
                    }
                    // Drag start: left button held, moved past threshold, began on a body.
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
                            Hit::Body(i) | Hit::Edit(i) | Hit::Copy(i) | Hit::Close(i) => i,
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
        // file drops here -> fd closed -> EOF signalled to the reader
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        // No valid target took the drop -> auto-copy fallback so Ctrl+V still works.
        if !self.drop_ok {
            if let Some(path) = self.drag_path.clone() {
                if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland) {
                    eprintln!("boltsnap daemon: fallback copy failed: {e}");
                }
            }
        }
        self.drag_source = None;
        self.drag_path = None;
        self.icon_surface = None;
        self.press = None;
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drop_ok = true;
    }

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drag_source = None;
        self.drag_path = None;
        self.icon_surface = None;
        self.press = None;
    }

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

// A pure drag SOURCE still needs the companion handlers for delegate_data_device.
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
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _x: f64, _y: f64) {
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
delegate_pointer!(Daemon);
delegate_layer!(Daemon);
delegate_data_device!(Daemon);
delegate_registry!(Daemon);
