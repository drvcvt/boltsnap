//! New region selector on raw SCTK (wlr-layer-shell) + tiny-skia, behind `--new`.
//! Parallel to `src/select.rs` (egui); same public signature so it is a drop-in.

mod font;
mod render;

use std::thread;

use image::RgbaImage;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{
            BTN_LEFT, CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
            ThemedPointer,
        },
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
        wl_keyboard::WlKeyboard, wl_output, wl_pointer::WlPointer, wl_seat::WlSeat,
        wl_surface::WlSurface,
    },
};

use crate::DynResult;

/// Drop-in replacement for `crate::select::run_select_with_parallel_capture`:
/// same signature, same parallel-capture overlap. Opens a fullscreen
/// wlr-layer-shell overlay on the focused output, renders the frozen screenshot
/// with a draggable selection via tiny-skia, and returns the cropped image on
/// confirm (or `None` on Esc/cancel).
pub fn run_select_with_parallel_capture<F>(capture: F) -> DynResult<Option<RgbaImage>>
where
    F: FnOnce() -> Result<RgbaImage, String> + Send + 'static,
{
    // Start the screenshot grab so it overlaps with Wayland init below.
    let capture_handle = thread::spawn(capture);

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Selector>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let pool = SlotPool::new(256, &shm)?;

    let mut sel = Selector {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        compositor,
        layer_shell,
        layer: None,
        pointer: None,
        keyboard: None,
        image: None,
        base: None,
        surf_w: 0,
        surf_h: 0,
        configured: false,
        drag_start: None,
        drag_now: None,
        result: None,
        done: false,
        qh: qh.clone(),
        frame_pending: false,
        needs_redraw: false,
    };

    // Discover outputs (names) so we can target the focused monitor. This
    // roundtrip overlaps with the capture thread.
    event_queue.roundtrip(&mut sel)?;

    // Now block on the capture result; the grab ran during setup.
    let image = capture_handle
        .join()
        .map_err(|_| "capture worker panicked".to_string())??;
    sel.image = Some(image);

    // Create the fullscreen overlay on the focused output. Always pass a
    // concrete output: Hyprland may fail to map a layer surface with a null
    // output (see the shelf null-output note).
    let output = sel.focused_output();
    let surface = sel.compositor.create_surface(&qh);
    let layer = sel.layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("boltsnap-select"),
        output.as_ref(),
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, 0); // fill the output; real size arrives in configure
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    sel.layer = Some(layer);

    while !sel.done {
        event_queue.blocking_dispatch(&mut sel)?;
    }

    Ok(sel.result.take())
}

struct Selector {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    layer_shell: LayerShell,
    layer: Option<LayerSurface>,
    pointer: Option<ThemedPointer>,
    keyboard: Option<WlKeyboard>,
    /// Full-resolution captured screenshot (cropped from on confirm).
    image: Option<RgbaImage>,
    /// Display base layer, sized to the surface (built on first configure).
    base: Option<tiny_skia::Pixmap>,
    surf_w: u32,
    surf_h: u32,
    configured: bool,
    drag_start: Option<(f64, f64)>,
    drag_now: Option<(f64, f64)>,
    result: Option<RgbaImage>,
    done: bool,
    /// QueueHandle for requesting frame callbacks from `draw`.
    qh: QueueHandle<Selector>,
    /// A frame callback is in flight (committed, awaiting the compositor).
    frame_pending: bool,
    /// A redraw is owed (selection changed) and runs on the next frame callback.
    needs_redraw: bool,
}

impl Selector {
    /// Pick a concrete `wl_output` on the focused monitor (Hyprland), falling
    /// back to the first output. Mirrors the shelf's `target_output`.
    fn focused_output(&self) -> Option<wl_output::WlOutput> {
        let outputs: Vec<_> = self.output_state.outputs().collect();
        if outputs.len() <= 1 {
            return outputs.into_iter().next();
        }
        let name = crate::shelf::focused_monitor_name();
        name.as_ref()
            .and_then(|n| {
                outputs
                    .iter()
                    .find(|o| {
                        self.output_state.info(o).and_then(|i| i.name).as_deref() == Some(n.as_str())
                    })
                    .cloned()
            })
            .or_else(|| outputs.into_iter().next())
    }

    /// Request a redraw, throttled to the compositor's frame clock. Draws now if
    /// no frame callback is pending; otherwise marks a redraw owed so the next
    /// `frame` callback coalesces it. Keeps a fast drag from flooding commits.
    fn request_redraw(&mut self) {
        self.needs_redraw = true;
        if !self.frame_pending {
            self.draw();
        }
    }

    /// Render the current frame (screenshot + dim + optional selection) into a
    /// fresh wl_shm buffer and commit it.
    fn draw(&mut self) {
        if !self.configured {
            return;
        }
        let (Some(layer), Some(base)) = (self.layer.as_ref(), self.base.as_ref()) else {
            return;
        };
        // `base` is built once from the captured image at the surface size on the
        // first configure, so base.{width,height} == (surf_w, surf_h). A fullscreen
        // layer overlay is not resized by the compositor, so the buffer below and
        // `base` always agree; if that ever changes, rebuild `base` on resize.
        let (w, h) = (self.surf_w.max(1), self.surf_h.max(1));
        let sel = match (self.drag_start, self.drag_now) {
            (Some(a), Some(b)) => {
                let x = a.0.min(b.0) as f32;
                let y = a.1.min(b.1) as f32;
                let sw = (a.0.max(b.0) - a.0.min(b.0)) as f32;
                let sh = (a.1.max(b.1) - a.1.min(b.1)) as f32;
                Some((x, y, sw, sh))
            }
            _ => None,
        };

        let mut frame = base.clone();
        render::render_overlay(&mut frame, sel);

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
        render::pixmap_to_argb8888(&frame, canvas);
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        // Throttle to the compositor's frame clock: request a callback and hold
        // further redraws until it fires (see `request_redraw` / `frame`). Caps
        // commits to the refresh rate and lets the SlotPool reuse one buffer
        // instead of growing unboundedly during a fast drag.
        surface.frame(&self.qh, surface.clone());
        let _ = buffer.attach_to(surface);
        layer.commit();
        self.frame_pending = true;
        self.needs_redraw = false;
    }

    /// Confirm the current drag: crop the full-res image and finish, or reset
    /// the drag if it was sub-pixel.
    fn confirm(&mut self) {
        let (Some(a), Some(b)) = (self.drag_start, self.drag_now) else {
            return;
        };
        let Some(img) = self.image.as_ref() else {
            return;
        };
        let (iw, ih) = (img.width(), img.height());
        match render::rect_to_image(a, b, self.surf_w, self.surf_h, iw, ih) {
            Some((x, y, w, h)) => {
                self.result = Some(image::imageops::crop_imm(img, x, y, w, h).to_image());
                self.done = true;
            }
            None => {
                self.drag_start = None;
                self.drag_now = None;
                self.request_redraw();
            }
        }
    }
}

impl CompositorHandler for Selector {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {
        // A committed frame was presented; allow the next draw and run any
        // redraw that was coalesced while we waited for this callback.
        self.frame_pending = false;
        if self.needs_redraw {
            self.draw();
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Selector {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Selector {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        // Surface went away without a confirm = cancel.
        self.done = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let w = if configure.new_size.0 != 0 { configure.new_size.0 } else { self.surf_w.max(1) };
        let h = if configure.new_size.1 != 0 { configure.new_size.1 } else { self.surf_h.max(1) };
        self.surf_w = w;
        self.surf_h = h;
        if self.base.is_none() {
            if let Some(img) = self.image.as_ref() {
                self.base = Some(render::base_pixmap_from_image(img, w, h));
            }
        }
        self.configured = true;
        self.draw();
    }
}

impl SeatHandler for Selector {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
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
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, cap: Capability) {
        if cap == Capability::Keyboard {
            self.keyboard = None;
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for Selector {
    fn pointer_frame(&mut self, conn: &Connection, _: &QueueHandle<Self>, _: &WlPointer, events: &[PointerEvent]) {
        let mut redraw = false;
        for ev in events {
            // Show a crosshair over the overlay instead of inheriting the prior
            // window's cursor or the compositor's busy "watch" cursor.
            if matches!(ev.kind, PointerEventKind::Enter { .. }) {
                if let Some(p) = self.pointer.as_ref() {
                    let _ = p.set_cursor(conn, CursorIcon::Crosshair);
                }
            }
            let (x, y) = ev.position;
            match ev.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    self.drag_start = Some((x, y));
                    self.drag_now = Some((x, y));
                    redraw = true;
                }
                PointerEventKind::Motion { .. } => {
                    if self.drag_start.is_some() {
                        self.drag_now = Some((x, y));
                        redraw = true;
                    }
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if self.drag_start.is_some() {
                        self.drag_now = Some((x, y));
                        self.confirm();
                        return; // confirm() may have set done / drawn already
                    }
                }
                _ => {}
            }
        }
        if redraw {
            self.request_redraw();
        }
    }
}

impl KeyboardHandler for Selector {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: &WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: &WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, event: KeyEvent) {
        if event.keysym == Keysym::Escape {
            self.result = None;
            self.done = true;
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: Modifiers, _: u32) {}
}

impl ShmHandler for Selector {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Selector {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Selector);
delegate_output!(Selector);
delegate_shm!(Selector);
delegate_seat!(Selector);
delegate_keyboard!(Selector);
delegate_pointer!(Selector);
delegate_layer!(Selector);
delegate_registry!(Selector);
