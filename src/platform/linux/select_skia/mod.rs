//! New region selector on raw SCTK (wlr-layer-shell) + tiny-skia, behind `--new`.
//! Parallel to `src/select.rs` (egui); same public signature so it is a drop-in.

use crate::selector::{
    edit, render,
    scene::{Scene, SceneCache},
};

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
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
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

/// Draw the overlay's labels in the desktop UI font, the same one the shelf
/// resolves, so selector and shelf read as one surface set. Idempotent.
pub fn install_ui_font() {
    let _timing = super::timing::Span::new("font");
    let family = crate::config::Config::load().ui_font;
    // The bar sets its labels in Medium and its active modules in DemiBold.
    let (medium, demibold) = crate::platform::linux::shelf::font::load_ui_font_weights(
        family.as_deref(),
        (500.0, 600.0),
    );
    render::set_ui_font(medium, demibold);
}

/// Frozen image and the output layout it was captured from. The selector must
/// never display these pixels on a freshly queried, potentially different output.
pub struct CapturedOutput {
    pub image: RgbaImage,
    pub name: String,
    pub origin: (i32, i32),
    pub logical_size: (u32, u32),
    pub transform: wl_output::Transform,
}

pub fn run_select_with_parallel_capture<F>(
    capture: F,
    instant: bool,
) -> DynResult<Option<(RgbaImage, Option<String>)>>
where
    F: FnOnce() -> Result<CapturedOutput, String> + Send + 'static,
{
    // Start the screenshot grab so it overlaps with Wayland init below.
    let capture_handle = thread::spawn(capture);
    let mut sel = run_selector(instant, false, true, true, Some(capture_handle), None)?;
    Ok(sel
        .result
        .take()
        .map(|image| (image, sel.target_name.take())))
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
    pub clip: bool,
    pub surface_size: (u32, u32),
    pub output_origin: Option<(i32, i32)>,
    pub frozen: Option<super::replay::FrozenSelection>,
}

pub fn run_select_record(
    initial_show_frame: bool,
    initial_audio_enabled: bool,
) -> DynResult<RecordSelectionResult> {
    let mut sel = run_selector(
        false,
        true,
        initial_show_frame,
        initial_audio_enabled,
        None,
        super::replay::preview_target(),
    )?;
    Ok(RecordSelectionResult {
        rect: sel.result_rect.take(),
        show_frame: sel.show_frame,
        audio_enabled: sel.audio_enabled,
        clip: sel.result_clip,
        surface_size: (sel.surf_w, sel.surf_h),
        output_origin: sel.output_origin,
        frozen: sel.frozen.take(),
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
    capture_handle: Option<thread::JoinHandle<Result<CapturedOutput, String>>>,
    replay: Option<super::replay::PreviewTarget>,
) -> DynResult<Selector> {
    let font_handle = thread::spawn(install_ui_font);
    let timing = super::timing::Span::new("selector_setup");

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Selector>(&conn)?;
    super::timing::mark("selector_wayland_connected");
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
        overlay: None,
        buffers: Vec::new(),
        committed_generation: None,
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
        result_clip: false,
        replay_output: replay.as_ref().map(|r| r.output.clone()),
        frozen: None,
        clip_reported: false,
        output_origin: None,
        target_name: replay.as_ref().map(|r| r.output.clone()),
        captured_layout: None,
        target_output: None,
        failure: None,
    };

    // Discover outputs (names) so we can target the focused monitor. This
    // roundtrip overlaps with the capture thread (screenshot mode).
    event_queue.roundtrip(&mut sel)?;

    // Screenshot mode: block on the capture result; the grab ran during setup.
    // Record mode: nothing to join — the backdrop is built from a transparent
    // base on the first configure.
    if let Some(handle) = capture_handle {
        let image = handle
            .join()
            .map_err(|_| "capture worker panicked".to_string())??;
        sel.target_name = Some(image.name);
        sel.captured_layout = Some((image.origin, image.logical_size, image.transform));
        sel.image = Some(image.image);
    }

    // Create the fullscreen overlay on the focused output. Always pass a
    // concrete output: Hyprland may fail to map a layer surface with a null
    // output (see the shelf null-output note).
    let output = if let Some(name) = &sel.target_name {
        Some(
            sel.output_state
                .outputs()
                .find(|output| {
                    sel.output_state
                        .info(output)
                        .is_some_and(|info| info.name.as_deref() == Some(name))
                })
                .ok_or("capture monitor disconnected")?,
        )
    } else {
        sel.focused_output()
    };
    sel.output_origin = output
        .as_ref()
        .and_then(|o| sel.output_state.info(o))
        .map(|info| info.logical_position.unwrap_or(info.location));
    sel.target_output = output;
    if !sel.target_layout_valid() {
        return Err("capture monitor layout changed during capture".into());
    }
    font_handle.join().map_err(|_| "font worker panicked")?;
    let surface = sel.compositor.create_surface(&qh);
    let layer = sel.layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("boltsnap-select"),
        sel.target_output.as_ref(),
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, 0); // fill the output; real size arrives in configure
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    sel.layer = Some(layer);
    drop(timing);

    let mut events = calloop::EventLoop::<Selector>::try_new()?;
    calloop_wayland_source::WaylandSource::new(conn, event_queue)
        .insert(events.handle())
        .map_err(|e| format!("selector event source: {e}"))?;
    if let Some(target) = replay {
        let (sender, receiver) =
            calloop::channel::channel::<Option<super::replay::FrozenSelection>>();
        events
            .handle()
            .insert_source(receiver, |event, _, sel| {
                if let calloop::channel::Event::Msg(Some(mut frozen)) = event {
                    if sel.replay_output.as_deref() != Some(frozen.output.as_str()) {
                        return;
                    }
                    sel.image = frozen.preview.take();
                    sel.base = Some(match sel.image.as_ref() {
                        Some(image) => render::base_pixmap_from_image(
                            image,
                            sel.surf_w.max(1),
                            sel.surf_h.max(1),
                        ),
                        None => return,
                    });
                    sel.overlay = sel.base.as_ref().map(SceneCache::new);
                    sel.committed_generation = None;
                    for buffer in &mut sel.buffers {
                        buffer.generation = None;
                    }
                    sel.frozen = Some(frozen);
                    sel.clip_reported = false;
                    sel.request_redraw();
                }
            })
            .map_err(|e| format!("replay preview source: {e}"))?;
        thread::spawn(move || {
            let _ = sender.send(super::replay::FrozenSelection::prepare(&target));
        });
    }
    while !sel.done {
        events.dispatch(None, &mut sel)?;
        // wl_buffer.release is handled inside SCTK, independently of frame callbacks.
        if !sel.done && sel.needs_redraw && !sel.frame_pending {
            sel.draw();
        }
    }

    if let Some(error) = &sel.failure {
        return Err(error.clone().into());
    }
    Ok(sel)
}

struct FrameBuffer {
    buffer: Buffer,
    generation: Option<u64>,
    size: (u32, u32),
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
    overlay: Option<SceneCache>,
    buffers: Vec<FrameBuffer>,
    committed_generation: Option<u64>,
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
    /// Alt is held → show the magnifier (consumed in Task 11).
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
    result_clip: bool,
    replay_output: Option<String>,
    frozen: Option<super::replay::FrozenSelection>,
    /// Whether a press on an unavailable Clip has already been reported.
    clip_reported: bool,
    output_origin: Option<(i32, i32)>,
    target_name: Option<String>,
    captured_layout: Option<((i32, i32), (u32, u32), wl_output::Transform)>,
    target_output: Option<wl_output::WlOutput>,
    failure: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordControlHit {
    Clip,
    Audio,
    Frame,
    Record,
    Background,
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
    let toolbar = render::record_toolbar(sel, surf_w, surf_h)?;
    toolbar
        .controls
        .iter()
        .position(|&rect| contains(rect, point))
        .map(|i| {
            [
                RecordControlHit::Record,
                RecordControlHit::Clip,
                RecordControlHit::Frame,
                RecordControlHit::Audio,
            ][i]
        })
        .or_else(|| contains(toolbar.bounds, point).then_some(RecordControlHit::Background))
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
/// click-to-confirm — tolerates pointer jitter on a confirm click.
const DRAG_SLOP: f64 = 3.0;

impl Selector {
    /// Pick a concrete `wl_output` on the focused monitor (Hyprland), falling
    /// back to the first output. Mirrors the shelf's `target_output`.
    fn target_layout_valid(&self) -> bool {
        let Some(output) = &self.target_output else {
            return true;
        };
        let Some(info) = self.output_state.info(output) else {
            return false;
        };
        self.captured_layout
            .is_none_or(|(origin, size, transform)| {
                info.transform == transform
                    && info.logical_position.unwrap_or(info.location) == origin
                    && info
                        .logical_size
                        .is_none_or(|s| s == (size.0 as i32, size.1 as i32))
            })
    }

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
    /// reusable wl_shm buffer and commit only changed regions.
    fn draw(&mut self) {
        if !self.configured || self.done {
            return;
        }
        let (Some(layer), Some(base), Some(overlay)) = (
            self.layer.as_ref(),
            self.base.as_ref(),
            self.overlay.as_mut(),
        ) else {
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

        let hovered = sel
            .and_then(|s| render::record_toolbar(s, w, h))
            .and_then(|t| {
                t.controls
                    .iter()
                    .position(|&rect| contains(rect, self.cursor))
            });
        overlay.update(
            base,
            Scene {
                selection: sel,
                editing: matches!(self.mode, Mode::Editing { .. }),
                record: self.record_mode,
                toggles: if self.record_mode {
                    (self.frozen.is_some(), self.show_frame, self.audio_enabled)
                } else {
                    (false, false, false)
                },
                hovered: self.record_mode.then_some(hovered).flatten(),
                magnifier: (self.alt_held && !self.record_mode).then_some(self.cursor),
            },
        );
        if self.committed_generation == Some(overlay.generation()) {
            self.needs_redraw = false;
            return;
        }
        // Retire differently sized slots only after release. They count toward
        // the same three-slot cap even during rapid compositor resizes.
        self.buffers
            .retain(|slot| slot.size == (w, h) || slot.buffer.canvas(&mut self.pool).is_none());
        let free = self
            .buffers
            .iter()
            .position(|b| b.size == (w, h) && b.buffer.canvas(&mut self.pool).is_some());
        let index = match free {
            Some(index) => index,
            None if self.buffers.len() < 3 => {
                let created = self.pool.create_buffer(
                    w as i32,
                    h as i32,
                    (w * 4) as i32,
                    wayland_client::protocol::wl_shm::Format::Argb8888,
                );
                let (buffer, _) = match created {
                    Ok(v) => v,
                    Err(error) => {
                        self.failure = Some(format!("selector buffer: {error}"));
                        self.done = true;
                        return;
                    }
                };
                self.buffers.push(FrameBuffer {
                    buffer,
                    generation: None,
                    size: (w, h),
                });
                self.buffers.len() - 1
            }
            None => {
                self.needs_redraw = true;
                return;
            }
        };
        let slot = &mut self.buffers[index];
        let repair = overlay.damage_since(slot.generation);
        let Some(canvas) = slot.buffer.canvas(&mut self.pool) else {
            self.needs_redraw = true;
            return;
        };
        overlay.copy_regions(canvas, &repair);
        slot.generation = Some(overlay.generation());
        let surface = layer.wl_surface();
        if let Err(error) = slot.buffer.attach_to(surface) {
            self.failure = Some(format!("selector attach: {error}"));
            self.done = true;
            return;
        }
        for (x0, y0, x1, y1) in overlay.damage_since(self.committed_generation) {
            surface.damage_buffer(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32);
        }
        surface.frame(&self.qh, surface.clone());
        layer.commit();
        if self.committed_generation.is_none() {
            super::timing::mark("selector_first_commit");
        }
        self.committed_generation = Some(overlay.generation());
        self.frame_pending = true;
        self.needs_redraw = false;
    }

    /// Confirm the selection. In record mode, store the rect (surface px) and
    /// finish. Otherwise crop the full-res capture to `rect`, or return to Idle
    /// if the rect is sub-pixel.
    fn confirm_rect(&mut self, rect: edit::Rect) {
        super::timing::mark("selection_confirmed");
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
                super::timing::mark("crop_ready");
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
        // The compositor is ready for another frame (not presentation proof). Run any
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
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if !self.target_layout_valid() {
            self.failure = Some("capture monitor layout changed".into());
            self.done = true;
        }
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.target_output.as_ref() == Some(&output) {
            self.failure = Some("capture monitor disconnected".into());
            self.done = true;
        }
    }
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
        if !self.target_layout_valid() {
            self.failure = Some("capture monitor layout changed".into());
            self.done = true;
            return;
        }
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
        if self
            .base
            .as_ref()
            .is_none_or(|base| base.width() != w || base.height() != h)
        {
            self.base = Some(match self.image.as_ref() {
                Some(img) => render::base_pixmap_from_image(img, w, h),
                // Record mode (no screenshot): a transparent base so the dim
                // overlay shows a translucent backdrop with a clear selection.
                None => render::transparent_base(w, h),
            });
            self.overlay = self.base.as_ref().map(SceneCache::new);
            for slot in &mut self.buffers {
                slot.generation = None;
            }
            self.committed_generation = None;
        }
        if !self.configured {
            super::timing::mark("selector_configured");
        }
        self.configured = true;
        self.request_redraw();
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
            let previous_cursor = self.cursor;
            self.cursor = (x, y);
            if self.record_mode
                && let Mode::Editing { rect } = self.mode
            {
                let sel = (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                redraw |= record_control_hit(sel, self.surf_w, self.surf_h, previous_cursor)
                    != record_control_hit(sel, self.surf_w, self.surf_h, self.cursor);
            }
            match ev.kind {
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    match self.mode {
                        Mode::Editing { rect } => {
                            // Toolbar clicks do not alter the selection.
                            if self.record_mode {
                                let sel =
                                    (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                                match record_control_hit(sel, self.surf_w, self.surf_h, (x, y)) {
                                    Some(RecordControlHit::Clip) => {
                                        if self.frozen.is_some() {
                                            self.result_clip = true;
                                            self.confirm_rect(rect);
                                        } else if !self.clip_reported {
                                            // A control that does nothing when
                                            // pressed is worse than one that
                                            // says why. Once per selection, so
                                            // repeated presses do not stack up.
                                            self.clip_reported = true;
                                            crate::platform::replay::notify_error(
                                                if self.replay_output.is_some() {
                                                    "Replay preview is unavailable or still loading. REC remains available."
                                                } else {
                                                    "No replay buffer to clip from. Start it from the tray menu (Replay buffer \u{2192} Start) or with `boltsnap replay start`."
                                                },
                                            );
                                        }
                                        return;
                                    }
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
                                    Some(RecordControlHit::Background) => return,
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
        let toolbar = render::record_toolbar(sel, 400, 300).unwrap();
        let audio = toolbar.controls[3];
        let center = (audio.0 + audio.2 / 2.0, audio.1 + audio.3 / 2.0);
        assert_eq!(
            record_control_hit(sel, 400, 300, center),
            Some(RecordControlHit::Audio)
        );
        assert_eq!(
            record_control_hit(
                sel,
                400,
                300,
                (
                    toolbar.bounds.0 + toolbar.bounds.2 / 2.0,
                    (toolbar.bounds.1 + toolbar.controls[0].1) / 2.0,
                )
            ),
            Some(RecordControlHit::Background)
        );
    }
}
