//! The xdg toplevel that shows [`App`]: buffers, scale, frame pacing, and
//! what the app asks of the window.

use std::time::Instant;

use boltsnap::app::App;
use boltsnap::ui::{Canvas, Clicks, Effect, Input, Mods};
use calloop::LoopHandle;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{SeatState, pointer::ThemedPointer},
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_keyboard::WlKeyboard, wl_output, wl_shm, wl_surface},
};

/// Logical size before the compositor picks one, and the smallest it may.
pub(super) const OPEN_SIZE: (u32, u32) = (760, 520);
const MIN_SIZE: (u32, u32) = (560, 400);

pub(super) struct Win {
    pub(super) registry: RegistryState,
    pub(super) seats: SeatState,
    pub(super) outputs: OutputState,
    pub(super) compositor: CompositorState,
    pub(super) xdg: XdgShell,
    pub(super) shm: Shm,
    pool: SlotPool,
    window: Window,
    configured: bool,
    size: (u32, u32),
    scale: u32,
    pub(super) app: App,
    /// A frame callback is in flight; the next draw waits for it.
    frame_pending: bool,
    dirty: bool,
    pub(super) pointer: Option<ThemedPointer>,
    pub(super) keyboard: Option<WlKeyboard>,
    pub(super) mods: Mods,
    pub(super) clicks: Clicks,
    pub(super) handle: LoopHandle<'static, Win>,
    qh: QueueHandle<Win>,
    paste: calloop::channel::Sender<String>,
    pub(super) exit: bool,
}

fn open_window(compositor: &CompositorState, xdg: &XdgShell, qh: &QueueHandle<Win>) -> Window {
    let window = xdg.create_window(
        compositor.create_surface(qh),
        WindowDecorations::RequestServer,
        qh,
    );
    window.set_title("Boltsnap");
    window.set_app_id("boltsnap");
    window.set_min_size(Some(MIN_SIZE));
    window.commit();
    window
}

pub(super) struct Parts {
    pub(super) registry: RegistryState,
    pub(super) seats: SeatState,
    pub(super) outputs: OutputState,
    pub(super) compositor: CompositorState,
    pub(super) xdg: XdgShell,
    pub(super) shm: Shm,
}

impl Win {
    pub(super) fn new(
        parts: Parts,
        app: App,
        qh: &QueueHandle<Win>,
        handle: LoopHandle<'static, Win>,
        paste: calloop::channel::Sender<String>,
    ) -> crate::DynResult<Self> {
        let pool = SlotPool::new((OPEN_SIZE.0 * OPEN_SIZE.1 * 4) as usize, &parts.shm)?;
        let window = open_window(&parts.compositor, &parts.xdg, qh);
        Ok(Self {
            registry: parts.registry,
            seats: parts.seats,
            outputs: parts.outputs,
            compositor: parts.compositor,
            xdg: parts.xdg,
            shm: parts.shm,
            pool,
            window,
            configured: false,
            size: OPEN_SIZE,
            scale: 1,
            app,
            frame_pending: false,
            dirty: true,
            pointer: None,
            keyboard: None,
            mods: Mods::default(),
            clicks: Clicks::default(),
            handle,
            qh: qh.clone(),
            paste,
            exit: false,
        })
    }

    pub(super) fn surface(&self) -> &wl_surface::WlSurface {
        self.window.wl_surface()
    }

    /// Hand `input` to the app, act on what it asks, and redraw.
    pub(super) fn input(&mut self, input: Input) {
        for effect in self.app.handle(input, Instant::now()) {
            self.effect(effect);
        }
        self.dirty = true;
        self.redraw();
    }

    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::Close => self.exit = true,
            Effect::Copy(text) => {
                // Served from a thread until another client owns the
                // clipboard; it ends with the window like any app's copy.
                std::thread::spawn(move || {
                    use wl_clipboard_rs::copy::{MimeType, Options, Source};
                    let mut options = Options::new();
                    options.foreground(true);
                    let source = Source::Bytes(text.into_bytes().into_boxed_slice());
                    if let Err(error) = options.copy(source, MimeType::Text) {
                        eprintln!("boltsnap settings: copy failed: {error}");
                    }
                });
            }
            Effect::Paste => {
                let paste = self.paste.clone();
                std::thread::spawn(move || {
                    use std::io::Read;
                    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents};
                    let Ok((mut pipe, _)) =
                        get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text)
                    else {
                        return;
                    };
                    let mut text = String::new();
                    if pipe.read_to_string(&mut text).is_ok() {
                        let _ = paste.send(text.lines().next().unwrap_or("").to_owned());
                    }
                });
            }
        }
    }

    /// Draw now, unless a frame callback is due first; then it draws.
    pub(super) fn redraw(&mut self) {
        if self.configured && !self.frame_pending && self.dirty {
            self.draw();
        }
    }

    fn draw(&mut self) {
        let (w, h, scale) = (self.size.0, self.size.1, self.scale);
        let Some(mut canvas) = Canvas::new(w, h, scale) else {
            return;
        };
        let now = Instant::now();
        self.app.draw(&mut canvas, now);
        let (bw, bh) = (w * scale, h * scale);
        let Ok((buffer, pixels)) = self.pool.create_buffer(
            bw as i32,
            bh as i32,
            bw as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        crate::selector::render::pixmap_to_argb8888(&canvas.pixmap, pixels);
        let surface = self.window.wl_surface();
        surface.set_buffer_scale(scale as i32);
        surface.damage_buffer(0, 0, bw as i32, bh as i32);
        // Every draw asks for the next frame: it paces input redraws to the
        // display, and keeps animations going while there are any.
        surface.frame(&self.qh, surface.clone());
        if buffer.attach_to(surface).is_err() {
            return;
        }
        self.window.commit();
        self.frame_pending = true;
        self.dirty = self.app.animating(now);
    }

    /// Another `boltsnap settings` wants the window: show its page and map a
    /// fresh toplevel, which Hyprland focuses (an activation request only
    /// marks the window urgent; settings plan, Phase 4 step 1).
    pub(super) fn raise(&mut self, page: &str) {
        if !page.is_empty() {
            self.app.show_named(page, Instant::now());
        }
        self.window = open_window(&self.compositor, &self.xdg, &self.qh);
        self.configured = false;
        self.frame_pending = false;
        self.dirty = true;
    }
}

impl WindowHandler for Win {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let w = configure.new_size.0.map_or(self.size.0, |w| w.get());
        let h = configure.new_size.1.map_or(self.size.1, |h| h.get());
        self.size = (w.max(1), h.max(1));
        self.app
            .resize((self.size.0 as f32, self.size.1 as f32), Instant::now());
        self.configured = true;
        // A configure needs an answer with a buffer of the new size now.
        self.frame_pending = false;
        self.dirty = true;
        self.redraw();
    }
}

impl CompositorHandler for Win {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if surface == self.window.wl_surface() {
            self.scale = factor.max(1) as u32;
            self.dirty = true;
            self.redraw();
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if surface == self.window.wl_surface() {
            self.frame_pending = false;
            self.redraw();
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Win {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Win {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Win {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers!(OutputState, SeatState);
}

delegate_compositor!(Win);
delegate_output!(Win);
delegate_shm!(Win);
delegate_seat!(Win);
delegate_keyboard!(Win);
delegate_pointer!(Win);
delegate_xdg_shell!(Win);
delegate_xdg_window!(Win);
delegate_registry!(Win);
