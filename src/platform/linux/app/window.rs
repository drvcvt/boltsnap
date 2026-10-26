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
const MIN_SIZE: (u32, u32) = (360, 320);

pub(super) struct Win {
    pub(super) registry: RegistryState,
    pub(super) seats: SeatState,
    pub(super) outputs: OutputState,
    pub(super) compositor: CompositorState,
    pub(super) xdg: XdgShell,
    pub(super) shm: Shm,
    pool: SlotPool,
    /// The buffers the compositor knows, reused as it releases them.
    buffers: crate::platform::buffers::Buffers,
    window: Window,
    configured: bool,
    size: (u32, u32),
    scale: u32,
    pub(super) app: App,
    /// Drawn into frame after frame; new only for another size or scale.
    canvas: Option<Canvas>,
    /// The frame before, to tell the compositor only what changed. The two
    /// swap after every frame, so nothing is copied for it.
    last: Option<Canvas>,
    /// The compositor takes tiny-skia's byte order as it is.
    rgba: bool,
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
    pub(super) saver: Option<super::save::Saver>,
    /// What the pages show on top: a refused write, or why nothing saves.
    pub(super) notice: Option<boltsnap::app::pages::Notice>,
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
        let rgba = parts.shm.formats().contains(&wl_shm::Format::Abgr8888);
        let window = open_window(&parts.compositor, &parts.xdg, qh);
        Ok(Self {
            registry: parts.registry,
            seats: parts.seats,
            outputs: parts.outputs,
            compositor: parts.compositor,
            xdg: parts.xdg,
            shm: parts.shm,
            pool,
            buffers: Default::default(),
            window,
            configured: false,
            size: OPEN_SIZE,
            scale: 1,
            app,
            canvas: None,
            last: None,
            rgba,
            frame_pending: false,
            dirty: true,
            pointer: None,
            keyboard: None,
            mods: Mods::default(),
            clicks: Clicks::default(),
            handle,
            qh: qh.clone(),
            paste,
            saver: None,
            notice: None,
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
        self.changed();
    }

    /// The app has something new to show.
    pub(super) fn changed(&mut self) {
        self.dirty = true;
        self.redraw();
    }

    /// Closing: commit what is being edited and wait for every write.
    pub(super) fn finish(&mut self) {
        for effect in self.app.finish(Instant::now()) {
            self.effect(effect);
        }
        if let Some(saver) = self.saver.take() {
            saver.finish();
        }
    }

    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::Close => self.exit = true,
            Effect::Changed(block) => {
                if let (Some(write), Some(saver)) = (self.app.write(block), &self.saver) {
                    saver.send(write);
                }
            }
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
        let (bw, bh) = (w * scale, h * scale);
        let fits = |canvas: &Option<Canvas>| {
            canvas
                .as_ref()
                .is_some_and(|c| (c.pixmap.width(), c.pixmap.height()) == (bw, bh))
        };
        if !fits(&self.last) {
            self.last = None;
        }
        if !fits(&self.canvas) {
            self.canvas = Canvas::new(w, h, scale);
        }
        let Some(canvas) = self.canvas.as_mut() else {
            return;
        };
        let now = Instant::now();
        self.app.draw(canvas, now);
        self.dirty = self.app.animating(now);
        let data = canvas.pixmap.data();
        let surface = self.window.wl_surface();
        // Every draw asks for the next frame: it paces input redraws to the
        // display, and keeps animations going while there are any.
        let last = self
            .last
            .as_ref()
            .map_or(&[][..], |last| last.pixmap.data());
        let Some((x, y, dw, dh)) = changed(last, data, bw as usize) else {
            // Nothing new to show; an animation still running keeps asking.
            if self.dirty {
                surface.frame(&self.qh, surface.clone());
                surface.commit();
                self.frame_pending = true;
            }
            return;
        };
        let format = if self.rgba {
            wl_shm::Format::Abgr8888
        } else {
            wl_shm::Format::Argb8888
        };
        let Some((buffer, pixels)) =
            self.buffers
                .next(&mut self.pool, (bw as i32, bh as i32), format)
        else {
            return;
        };
        if self.rgba {
            pixels.copy_from_slice(data);
        } else {
            crate::selector::render::pixmap_to_argb8888(&canvas.pixmap, pixels);
        }
        surface.set_buffer_scale(scale as i32);
        // Only what changed is uploaded again.
        surface.damage_buffer(x, y, dw, dh);
        surface.frame(&self.qh, surface.clone());
        if buffer.attach_to(surface).is_err() {
            return;
        }
        self.window.commit();
        self.frame_pending = true;
        // This frame is the one to compare the next against; the next draws
        // over the older one.
        if self.last.is_some() {
            std::mem::swap(&mut self.canvas, &mut self.last);
        } else {
            self.last = self.canvas.take();
        }
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

/// The rectangle (x, y, w, h in buffer pixels) around every pixel of `now`
/// that differs from `last`; all of it when there is no last frame, `None`
/// when nothing changed.
fn changed(last: &[u8], now: &[u8], width: usize) -> Option<(i32, i32, i32, i32)> {
    let stride = width * 4;
    let height = now.len() / stride.max(1);
    if last.len() != now.len() {
        return Some((0, 0, width as i32, height as i32));
    }
    let rows = |row: usize| {
        (
            &last[row * stride..][..stride],
            &now[row * stride..][..stride],
        )
    };
    let top = (0..height).find(|&row| rows(row).0 != rows(row).1)?;
    let bottom = (top..height)
        .rev()
        .find(|&row| rows(row).0 != rows(row).1)?;
    let (mut left, mut right) = (width, 0);
    for row in top..=bottom {
        let (a, b) = rows(row);
        if a == b {
            continue;
        }
        let pixel = |i: usize| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4];
        if let Some(first) = (0..left.min(width)).find(|&i| pixel(i)) {
            left = first;
        }
        if let Some(last) = (right..width).rev().find(|&i| pixel(i)) {
            right = last;
        }
    }
    Some((
        left as i32,
        top as i32,
        (right.max(left) - left + 1) as i32,
        (bottom - top + 1) as i32,
    ))
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
        // A configure is answered with a whole buffer at the latest size, on
        // the display's beat: dragging a window edge sends one per pointer
        // event, and drawing each at once floods the compositor.
        self.last = None;
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

#[cfg(test)]
mod tests {
    use super::changed;

    #[test]
    fn damage_is_the_rectangle_around_what_changed() {
        let (w, h) = (8, 6);
        let last = vec![0u8; w * h * 4];
        assert_eq!(changed(&[], &last, w), Some((0, 0, 8, 6)), "no last frame");
        assert_eq!(changed(&last, &last, w), None, "nothing changed");
        let mut now = last.clone();
        for (x, y) in [(2, 1), (5, 3)] {
            now[(y * w + x) * 4 + 1] = 9;
        }
        assert_eq!(changed(&last, &now, w), Some((2, 1, 4, 3)));
    }
}
