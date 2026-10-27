//! Wayland screenshot and recording selector using SCTK layer surfaces and tiny-skia.

use crate::selector::motion::{BarMotion, Targets};
use crate::selector::{desktop, edit, render, scene::SceneCache};
use boltsnap::ui::text::Fonts;
use std::time::Instant;

use std::thread;

use image::RgbaImage;
use smithay_client_toolkit::{
    compositor::CompositorState,
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{SeatState, pointer::ThemedPointer},
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
    protocol::{wl_keyboard::WlKeyboard, wl_output},
};

use crate::DynResult;

mod input;
mod present;

use input::{Interaction, MIN_SEL, contains};
use present::OutputView;

/// The desktop UI font at the app's two weights, as the settings window and
/// the shelf resolve it, so the selector reads as part of the same product.
pub fn ui_fonts() -> Fonts {
    let _timing = super::timing::Span::new("font");
    let family = crate::config::Config::load().ui_font;
    let (medium, demibold) = crate::platform::linux::shelf::font::load_ui_font_weights(
        family.as_deref(),
        (500.0, 600.0),
    );
    Fonts::new(medium, demibold)
}

/// Frozen image and the output layout it was captured from. The selector must
/// never display these pixels on a freshly queried, potentially different output.
pub struct CapturedDesktop {
    pub image: RgbaImage,
    pub monitors: Vec<CapturedMonitor>,
    pub preferred_output: Option<String>,
}

pub struct CapturedMonitor {
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
    F: FnOnce() -> Result<CapturedDesktop, String> + Send + 'static,
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
/// layer overlays on the captured outputs (or one recording output), running until
/// confirm/cancel. `capture_handle`, when `Some`, is joined to obtain the frozen
/// screenshot (screenshot mode); when `None` (record mode) the backdrop is a
/// plain translucent dim instead of a frozen frame.
fn run_selector(
    instant: bool,
    record_mode: bool,
    show_frame: bool,
    audio_enabled: bool,
    capture_handle: Option<thread::JoinHandle<Result<CapturedDesktop, String>>>,
    replay: Option<super::replay::PreviewTarget>,
) -> DynResult<Selector> {
    let font_handle = thread::spawn(ui_fonts);
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
        views: Vec::new(),
        captured_monitors: Vec::new(),
        pointer: None,
        keyboard: None,
        image: None,
        base: None,
        overlay: None,
        // The embedded face until the desktop's is loaded (before any frame).
        chrome: render::Chrome::new(Fonts::fallback()),
        motion: BarMotion::new(
            &bar_targets(None, None, show_frame, audio_enabled, false),
            Instant::now(),
        ),
        pressed: None,
        pointer_in: false,
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
        sel.target_name = image.preferred_output;
        sel.captured_monitors = image.monitors;
        sel.image = Some(image.image);
    }

    let outputs: Vec<_> = if record_mode {
        let output = if let Some(name) = &sel.target_name {
            sel.output_state.outputs().find(|o| {
                sel.output_state
                    .info(o)
                    .is_some_and(|i| i.name.as_ref() == Some(name))
            })
        } else {
            sel.focused_output()
        }
        .ok_or("capture monitor disconnected")?;
        sel.output_origin = sel
            .output_state
            .info(&output)
            .map(|i| i.logical_position.unwrap_or(i.location));
        vec![(output, (0, 0, 1, 1))]
    } else {
        let regions: Vec<_> = sel
            .captured_monitors
            .iter()
            .map(|m| (m.origin.0, m.origin.1, m.logical_size.0, m.logical_size.1))
            .collect();
        let (left, top, w, h) = desktop::bounds(&regions).ok_or("invalid desktop layout")?;
        sel.surf_w = w;
        sel.surf_h = h;
        sel.output_origin = Some((left, top));
        sel.captured_monitors
            .iter()
            .map(|m| {
                let output = sel
                    .output_state
                    .outputs()
                    .find(|o| {
                        sel.output_state
                            .info(o)
                            .is_some_and(|i| i.name.as_ref() == Some(&m.name))
                    })
                    .ok_or("capture monitor disconnected")?;
                Ok((
                    output,
                    (
                        (i64::from(m.origin.0) - i64::from(left)) as u32,
                        (i64::from(m.origin.1) - i64::from(top)) as u32,
                        m.logical_size.0,
                        m.logical_size.1,
                    ),
                ))
            })
            .collect::<Result<Vec<_>, &str>>()?
    };
    sel.chrome = render::Chrome::new(font_handle.join().map_err(|_| "font worker panicked")?);
    for (output, viewport) in outputs {
        let surface = sel.compositor.create_surface(&qh);
        let layer = sel.layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("boltsnap-select"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        sel.views.push(OutputView {
            layer,
            output,
            viewport,
            configured: false,
            buffers: Vec::new(),
            committed_generation: None,
            frame_pending: false,
            needs_redraw: true,
        });
    }
    if !sel.target_layout_valid() {
        return Err("capture monitor layout changed during capture".into());
    }
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
                    for view in &mut sel.views {
                        view.committed_generation = None;
                        for buffer in &mut view.buffers {
                            buffer.generation = None;
                        }
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
        if !sel.done && sel.needs_redraw {
            sel.draw();
        }
    }

    if let Some(error) = &sel.failure {
        return Err(error.clone().into());
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
    views: Vec<OutputView>,
    captured_monitors: Vec<CapturedMonitor>,
    pointer: Option<ThemedPointer>,
    keyboard: Option<WlKeyboard>,
    /// Full-resolution captured screenshot (cropped from on confirm).
    image: Option<RgbaImage>,
    /// Display base layer, sized to the surface (built on first configure).
    base: Option<tiny_skia::Pixmap>,
    overlay: Option<SceneCache>,
    /// The face and the toolbar's measured widths.
    chrome: render::Chrome,
    /// The toolbar's springs.
    motion: BarMotion,
    /// The toolbar control held down, until the button comes up.
    pressed: Option<usize>,
    /// The pointer is over a selector surface (Wayland says when it leaves).
    pointer_in: bool,
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
    failure: Option<String>,
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

/// The toolbar's targets: what is under the pointer and held, the toggles,
/// and whether Clip has a buffer.
fn bar_targets(
    hovered: Option<usize>,
    pressed: Option<usize>,
    show_frame: bool,
    audio: bool,
    clip: bool,
) -> Targets {
    use render::toolbar::{AUDIO, CLIP, FRAME};
    Targets {
        hovered,
        pressed,
        on: std::array::from_fn(|i| (i == FRAME && show_frame) || (i == AUDIO && audio)),
        enabled: std::array::from_fn(|i| i != CLIP || clip),
    }
}

impl Selector {
    /// Hand the toolbar what it should show now: there once a selection is
    /// drawn (not while it is first dragged out), following it as it changes.
    fn sync_bar(&mut self, now: Instant) {
        if !self.record_mode {
            return;
        }
        let layout = match self.mode {
            Mode::Editing { rect } => render::toolbar::layout(
                (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
                boltsnap::ui::Rect::new(0.0, 0.0, self.surf_w as f32, self.surf_h as f32),
                &self.chrome.toolbar,
            ),
            _ => None,
        };
        let hovered = layout
            .as_ref()
            .filter(|_| self.pointer_in)
            .and_then(|l| l.hit((self.cursor.0 as f32, self.cursor.1 as f32)));
        let targets = bar_targets(
            hovered,
            self.pressed,
            self.show_frame,
            self.audio_enabled,
            self.frozen.is_some(),
        );
        self.motion.update(layout, &targets, now);
    }

    /// Captured pixels must remain attached to the same logical output layout.
    fn target_layout_valid(&self) -> bool {
        self.views.iter().enumerate().all(|(index, view)| {
            let Some(info) = self.output_state.info(&view.output) else {
                return false;
            };
            self.captured_monitors.get(index).is_none_or(|m| {
                info.name.as_ref() == Some(&m.name)
                    && info.transform == m.transform
                    && info.logical_position.unwrap_or(info.location) == m.origin
                    && info.logical_size.is_none_or(|size| {
                        size == (m.logical_size.0 as i32, m.logical_size.1 as i32)
                    })
            })
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
        if let Some(view) = self.views.iter().find(|v| v.contains(self.cursor)) {
            self.target_name = self.output_state.info(&view.output).and_then(|i| i.name);
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

impl OutputHandler for Selector {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if !self.record_mode && !self.views.is_empty() {
            self.failure = Some("monitor connected during selection; retry capture".into());
            self.done = true;
        }
    }
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
        if self.views.iter().any(|v| v.output == output) {
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
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if !self.target_layout_valid() {
            self.failure = Some("capture monitor layout changed".into());
            self.done = true;
            return;
        }
        let Some(view) = self.views.iter_mut().find(|v| &v.layer == layer) else {
            return;
        };
        let w = if configure.new_size.0 == 0 {
            view.viewport.2
        } else {
            configure.new_size.0
        };
        let h = if configure.new_size.1 == 0 {
            view.viewport.3
        } else {
            configure.new_size.1
        };
        if !self.record_mode && (w, h) != (view.viewport.2, view.viewport.3) {
            self.failure = Some("capture monitor size changed".into());
            self.done = true;
            return;
        }
        if self.record_mode {
            self.surf_w = w;
            self.surf_h = h;
            view.viewport = (0, 0, w, h);
        }
        view.configured = true;
        if self.views.iter().any(|v| !v.configured) {
            return;
        }
        let (w, h) = (self.surf_w, self.surf_h);
        if self
            .base
            .as_ref()
            .is_none_or(|base| base.width() != w || base.height() != h)
        {
            self.base = Some(match self.image.as_ref() {
                Some(img) => render::base_pixmap_from_image(img, w, h),
                None => render::transparent_base(w, h),
            });
            self.overlay = self.base.as_ref().map(SceneCache::new);
            for view in &mut self.views {
                for slot in &mut view.buffers {
                    slot.generation = None;
                }
                view.committed_generation = None;
            }
        }
        if !self.configured {
            super::timing::mark("selector_configured");
        }
        self.configured = true;
        self.request_redraw();
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
mod shots;
