pub mod layout;
pub mod model;
pub mod paint;
pub mod thumbnail;

use std::os::unix::net::UnixListener;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerHandler},
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
    protocol::{wl_output, wl_pointer::WlPointer, wl_seat::WlSeat, wl_surface::WlSurface},
};

use crate::DynResult;
use crate::shelf::layout::{Layout, LayoutConfig};
use crate::shelf::model::ShelfModel;

pub struct Daemon {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    pointer: Option<WlPointer>,

    model: ShelfModel,
    layout: Layout,
    cfg: LayoutConfig,
    width: u32,
    height: u32,
    hovered: Option<u64>,
    exit: bool,
}

pub fn run_daemon() -> DynResult<()> {
    // Single-instance: if a daemon already answers, do nothing.
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    let sock = crate::ipc::socket_path();
    let _ = std::fs::remove_file(&sock); // clear stale socket
    // The real listener is wired into calloop in Task D4; bind/drop here just
    // proves the path is free during the D1 scaffold.
    let _listener = UnixListener::bind(&sock)?;
    drop(_listener);

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Daemon>(&conn)?;
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
        exit: false,
    };

    loop {
        event_queue.blocking_dispatch(&mut daemon)?;
        if daemon.exit {
            break;
        }
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

    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let stride = (w * 4) as i32;
        // Recreate the buffer each draw; SlotPool hands back a fresh slot when
        // the compositor still holds the previous one, so this is double-buffer safe.
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
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
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
        _events: &[PointerEvent],
    ) {
        // filled in Milestone E
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

    fn dnd_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
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
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _x: f64, _y: f64) {}
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

delegate_data_device!(Daemon);
delegate_compositor!(Daemon);
delegate_output!(Daemon);
delegate_shm!(Daemon);
delegate_seat!(Daemon);
delegate_pointer!(Daemon);
delegate_layer!(Daemon);
delegate_registry!(Daemon);
