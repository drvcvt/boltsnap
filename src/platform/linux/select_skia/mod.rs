//! New region selector on raw SCTK (wlr-layer-shell) + tiny-skia, behind `--new`.
//! Parallel to `src/select.rs` (egui); same public signature so it is a drop-in.

use crate::selector::{edit, render};

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
pub fn run_select_with_parallel_capture<F>(
    capture: F,
    instant: bool,
) -> DynResult<Option<RgbaImage>>
where
    F: FnOnce() -> Result<RgbaImage, String> + Send + 'static,
{
    // Start the screenshot grab so it overlaps with Wayland init below.
    let capture_handle = thread::spawn(capture);
    let mut sel = run_selector(instant, false, true, true, Some(capture_handle))?;
    Ok(sel.result.take())
}

/// Record-mode selector: opens the SAME overlay with the same draw/resize/move
/// editing, but does NOT capture a screenshot. It shows a translucent dim
/// backdrop (the live screen reads through the selection) and a red REC pill, and
/// on confirm returns the selection `Rect` (logical surface px) instead of an
/// image. `None` on Esc/cancel. The caller maps the rect to compositor-global
/// coords and starts the recording.
pub struct RecordSelectionResult {
    pub rect: Option<edit::Rect>,
    pub show_frame: bool,
    pub audio_enabled: bool,
}

pub fn run_select_record(
    initial_show_frame: bool,
    initial_audio_enabled: bool,
) -> DynResult<RecordSelectionResult> {
    let mut sel = run_selector(false, true, initial_show_frame, initial_audio_enabled, None)?;
    Ok(RecordSelectionResult {
        rect: sel.result_rect.take(),
        show_frame: sel.show_frame,
        audio_enabled: sel.audio_enabled,
    })
}

/// Shared driver for both selector modes. Binds the Wayland globals, builds the
/// fullscreen layer overlay on the focused output, and runs the event loop until
/// confirm/cancel. `capture_handle`, when `Some`, is joined to obtain the frozen
/// screenshot (screenshot mode); when `None` (record mode) the backdrop is a
/// plain translucent dim instead of a frozen frame.
fn run_selector(
    instant: bool,
    record_mode: bool,
    show_frame: bool,
    audio_enabled: bool,
    capture_handle: Option<thread::JoinHandle<Result<RgbaImage, String>>>,
) -> DynResult<Selector> {
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
        mode: Mode::Idle,
        interaction: None,
        cursor: (0.0, 0.0),
        alt_held: false,
        result: None,
        done: false,
        qh: qh.clone(),
        frame_pending: false,
        needs_redraw: false,
        instant,
        record_mode,
        show_frame,
        audio_enabled,
        result_rect: None,
    };

    // Discover outputs (names) so we can target the focused monitor. This
    // roundtrip overlaps with the capture thread (screenshot mode).
    event_queue.roundtrip(&mut sel)?;

    // Screenshot mode: block on the capture result; the grab ran during setup.
    // Record mode: nothing to join â€” the backdrop is built from a transparent
    // base on the first configure.
    if let Some(handle) = capture_handle {
        let image = handle
            .join()
            .map_err(|_| "capture worker panicked".to_string())??;
        sel.image = Some(image);
    }

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

    Ok(sel)
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
    /// Current interaction phase.
    mode: Mode,
    /// Active press interaction while in Editing (resize/move), with whether the
    /// pointer has moved enough to count as a drag (vs a click-to-confirm).
    interaction: Option<Interaction>,
    /// Last pointer position (so the Alt magnifier can follow the cursor).
    cursor: (f64, f64),
    /// Alt is held â†’ show the magnifier (consumed in Task 11).
    alt_held: bool,
    result: Option<RgbaImage>,
    done: bool,
    /// QueueHandle for requesting frame callbacks from `draw`.
    qh: QueueHandle<Selector>,
    /// A frame callback is in flight (committed, awaiting the compositor).
    frame_pending: bool,
    /// A redraw is owed (selection changed) and runs on the next frame callback.
    needs_redraw: bool,
    /// Skip the editable phase: release in Drawing confirms immediately.
    instant: bool,
    /// Record mode: no screenshot capture, a translucent dim backdrop, a red REC
    /// pill, and confirm yields the selection rect (into `result_rect`) instead
    /// of cropping an image.
    record_mode: bool,
    /// Whether the recording-area border should remain visible while recording.
    show_frame: bool,
    /// Whether the next recording should include the configured audio source.
    audio_enabled: bool,
    /// The confirmed selection rect (surface px), set on confirm in record mode.
    result_rect: Option<edit::Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordControlHit {
    Audio,
    Frame,
    Record,
}

fn contains(rect: (f64, f64, f64, f64), point: (f64, f64)) -> bool {
    point.0 >= rect.0 && point.0 < rect.0 + rect.2 && point.1 >= rect.1 && point.1 < rect.1 + rect.3
}

fn record_control_hit(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    point: (f64, f64),
) -> Option<RecordControlHit> {
    if render::record_audio_button_rect(sel, surf_w, surf_h)
        .is_some_and(|rect| contains(rect, point))
    {
        Some(RecordControlHit::Audio)
    } else if render::record_frame_checkbox_rect(sel, surf_w, surf_h)
        .is_some_and(|rect| contains(rect, point))
    {
        Some(RecordControlHit::Frame)
    } else if render::rec_pill_rect(sel, surf_w, surf_h).is_some_and(|rect| contains(rect, point)) {
        Some(RecordControlHit::Record)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum Mode {
    /// No selection yet; waiting for a press.
    Idle,
    /// First drag in progress, from `anchor` to `now`.
    Drawing { anchor: (f64, f64), now: (f64, f64) },
    /// Committed, editable selection.
    Editing { rect: edit::Rect },
}

#[derive(Clone, Copy)]
enum Interaction {
    /// Resizing via a handle.
    Resize { handle: edit::Handle },
    /// Moving the whole rect; `grab` is cursor-minus-origin at press time.
    Move { grab: (f64, f64) },
    /// Pressed inside; becomes a Move once the cursor leaves a small slop radius,
    /// otherwise a confirm on release. `press` is the press position.
    ClickInside { press: (f64, f64) },
}

/// Handle hit radius (px) and minimum selection size (px).
const HANDLE_R: f64 = 9.0;
const MIN_SEL: f64 = 4.0;
/// Pixels of motion before a press-inside counts as a drag (move) rather than a
/// click-to-confirm â€” tolerates pointer jitter on a confirm click.
const DRAG_SLOP: f64 = 3.0;

impl Selector {
    /// Pick a concrete `wl_output` on the focused monitor (Hyprland), falling
    /// back to the first output. Mirrors the shelf's `target_output`.
    fn focused_output(&self) -> Option<wl_output::WlOutput> {
        let outputs: Vec<_> = self.output_state.outputs().collect();
        if outputs.len() <= 1 {
            return outputs.into_iter().next();
        }
        let name = crate::platform::shelf::focused_monitor_name();
        name.as_ref()
            .and_then(|n| {
                outputs
                    .iter()
                    .find(|o| {
                        self.output_state.info(o).and_then(|i| i.name).as_deref()
                            == Some(n.as_str())
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
        let sel = match self.mode {
            Mode::Idle => None,
            Mode::Drawing { anchor, now } => {
                let r = edit::Rect::from_corners(anchor, now);
                Some((r.x as f32, r.y as f32, r.w as f32, r.h as f32))
            }
            Mode::Editing { rect } => {
                Some((rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32))
            }
        };

        let mut frame = base.clone();
        render::dim_and_restore(&mut frame, sel);
        if let Some(s) = sel {
            render::draw_border(&mut frame, s);
            if matches!(self.mode, Mode::Editing { .. }) {
                render::draw_handles(&mut frame, s);
            }
            if self.record_mode {
                render::draw_rec_pill(&mut frame, s, self.surf_w, self.surf_h);
                render::draw_record_frame_checkbox(
                    &mut frame,
                    s,
                    self.surf_w,
                    self.surf_h,
                    self.show_frame,
                );
                render::draw_record_audio_button(
                    &mut frame,
                    s,
                    self.surf_w,
                    self.surf_h,
                    self.audio_enabled,
                );
            } else {
                render::draw_badge(&mut frame, s, self.surf_w, self.surf_h);
            }
        }
        // The magnifier samples the frozen screenshot; there is none in record
        // mode (transparent base), so it is only useful for screenshots.
        if self.alt_held && !self.record_mode {
            render::draw_magnifier(&mut frame, base, self.cursor, self.surf_w, self.surf_h);
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

    /// Confirm the selection. In record mode, store the rect (surface px) and
    /// finish. Otherwise crop the full-res capture to `rect`, or return to Idle
    /// if the rect is sub-pixel.
    fn confirm_rect(&mut self, rect: edit::Rect) {
        if self.record_mode {
            // Reject a sub-pixel selection rather than confirming an empty rect.
            if rect.w < MIN_SEL || rect.h < MIN_SEL {
                self.mode = Mode::Idle;
                self.request_redraw();
                return;
            }
            self.result_rect = Some(rect);
            self.done = true;
            return;
        }
        let Some(img) = self.image.as_ref() else {
            self.done = true;
            return;
        };
        let (iw, ih) = (img.width(), img.height());
        match render::rect_to_image(
            (rect.x, rect.y),
            (rect.right(), rect.bottom()),
            self.surf_w,
            self.surf_h,
            iw,
            ih,
        ) {
            Some((x, y, w, h)) => {
                self.result = Some(image::imageops::crop_imm(img, x, y, w, h).to_image());
                self.done = true;
            }
            None => {
                self.mode = Mode::Idle;
                self.request_redraw();
            }
        }
    }
}

impl CompositorHandler for Selector {
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
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {
        // A committed frame was presented; allow the next draw and run any
        // redraw that was coalesced while we waited for this callback.
        self.frame_pending = false;
        if self.needs_redraw {
            self.draw();
        }
    }
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
        let w = if configure.new_size.0 != 0 {
            configure.new_size.0
        } else {
            self.surf_w.max(1)
        };
        let h = if configure.new_size.1 != 0 {
            configure.new_size.1
        } else {
            self.surf_h.max(1)
        };
        self.surf_w = w;
        self.surf_h = h;
        if self.base.is_none() {
            self.base = Some(match self.image.as_ref() {
                Some(img) => render::base_pixmap_from_image(img, w, h),
                // Record mode (no screenshot): a transparent base so the dim
                // overlay shows a translucent backdrop with a clear selection.
                None => render::transparent_base(w, h),
            });
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

impl PointerHandler for Selector {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        let mut redraw = false;
        for ev in events {
            if matches!(ev.kind, PointerEventKind::Enter { .. }) {
                if let Some(p) = self.pointer.as_ref() {
                    let _ = p.set_cursor(conn, CursorIcon::Crosshair);
                }
            }
            let (x, y) = ev.position;
            self.cursor = (x, y);
            match ev.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    match self.mode {
                        Mode::Editing { rect } => {
                            // Record mode: the REC pill is a Start button. A press
                            // inside it confirms the selection (begins recording)
                            // rather than being treated as a click outside the rect
                            // (which would reset the selection).
                            if self.record_mode {
                                let sel =
                                    (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                                match record_control_hit(sel, self.surf_w, self.surf_h, (x, y)) {
                                    Some(RecordControlHit::Audio) => {
                                        self.audio_enabled = !self.audio_enabled;
                                        self.interaction = None;
                                        self.request_redraw();
                                        return;
                                    }
                                    Some(RecordControlHit::Frame) => {
                                        self.show_frame = !self.show_frame;
                                        self.interaction = None;
                                        self.request_redraw();
                                        return;
                                    }
                                    Some(RecordControlHit::Record) => {
                                        self.confirm_rect(rect);
                                        return;
                                    }
                                    None => {}
                                }
                            }
                            match edit::hit_region(rect, (x, y), HANDLE_R) {
                                edit::Region::Handle(h) => {
                                    self.interaction = Some(Interaction::Resize { handle: h });
                                }
                                edit::Region::Inside => {
                                    self.interaction =
                                        Some(Interaction::ClickInside { press: (x, y) });
                                }
                                edit::Region::Outside => {
                                    self.mode = Mode::Drawing {
                                        anchor: (x, y),
                                        now: (x, y),
                                    };
                                    self.interaction = None;
                                }
                            }
                        }
                        _ => {
                            self.mode = Mode::Drawing {
                                anchor: (x, y),
                                now: (x, y),
                            };
                            self.interaction = None;
                        }
                    }
                    redraw = true;
                }
                PointerEventKind::Motion { .. } => match self.mode {
                    Mode::Drawing { anchor, .. } => {
                        self.mode = Mode::Drawing {
                            anchor,
                            now: (x, y),
                        };
                        redraw = true;
                    }
                    Mode::Editing { rect } => {
                        match self.interaction {
                            Some(Interaction::Resize { handle }) => {
                                let nr = edit::resize_rect(
                                    rect,
                                    handle,
                                    (x, y),
                                    MIN_SEL,
                                    self.surf_w as f64,
                                    self.surf_h as f64,
                                );
                                self.mode = Mode::Editing { rect: nr };
                                redraw = true;
                            }
                            Some(Interaction::Move { grab }) => {
                                let target = edit::Rect {
                                    x: x - grab.0,
                                    y: y - grab.1,
                                    w: rect.w,
                                    h: rect.h,
                                };
                                let nr = edit::move_rect(
                                    target,
                                    0.0,
                                    0.0,
                                    self.surf_w as f64,
                                    self.surf_h as f64,
                                );
                                self.mode = Mode::Editing { rect: nr };
                                redraw = true;
                            }
                            Some(Interaction::ClickInside { press }) => {
                                // Promote to a move only once the cursor leaves the
                                // slop radius, so a jittery click still confirms.
                                if (x - press.0).powi(2) + (y - press.1).powi(2)
                                    > DRAG_SLOP * DRAG_SLOP
                                {
                                    self.interaction = Some(Interaction::Move {
                                        grab: (x - rect.x, y - rect.y),
                                    });
                                    redraw = true;
                                }
                            }
                            None => {}
                        }
                        if self.alt_held {
                            redraw = true;
                        }
                    }
                    Mode::Idle => {
                        if self.alt_held {
                            redraw = true;
                        }
                    }
                },
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    match self.mode {
                        Mode::Drawing { anchor, now } => {
                            let rect = edit::Rect::from_corners(anchor, now);
                            if rect.w < MIN_SEL || rect.h < MIN_SEL {
                                self.mode = Mode::Idle;
                            } else if self.instant {
                                self.confirm_rect(rect);
                                return;
                            } else {
                                self.mode = Mode::Editing { rect };
                            }
                            self.interaction = None;
                            redraw = true;
                        }
                        Mode::Editing { rect } => {
                            // A press-inside with no drag is a confirm click.
                            if matches!(self.interaction, Some(Interaction::ClickInside { .. })) {
                                self.confirm_rect(rect);
                                return;
                            }
                            self.interaction = None;
                        }
                        Mode::Idle => {}
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
        match event.keysym {
            Keysym::Escape => {
                self.result = None;
                self.done = true;
            }
            // Space or Enter confirms the editable selection (Space by habit).
            Keysym::space | Keysym::Return | Keysym::KP_Enter => {
                if let Mode::Editing { rect } = self.mode {
                    self.confirm_rect(rect);
                }
            }
            _ => {}
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
        modifiers: Modifiers,
        _: u32,
    ) {
        if self.alt_held != modifiers.alt {
            self.alt_held = modifiers.alt;
            self.request_redraw();
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_control_hit_does_not_confirm() {
        let sel = (80.0, 80.0, 200.0, 120.0);
        let audio = render::record_audio_button_rect(sel, 400, 300).unwrap();
        let center = (audio.0 + audio.2 / 2.0, audio.1 + audio.3 / 2.0);
        assert_eq!(
            record_control_hit(sel, 400, 300, center),
            Some(RecordControlHit::Audio)
        );
    }
}
