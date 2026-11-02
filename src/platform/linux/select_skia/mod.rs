//! Wayland screenshot and recording selector using SCTK layer surfaces and tiny-skia.

use crate::selector::clip::ClipState;
use crate::selector::motion::{BarMotion, Targets};
use crate::selector::{edit, render, scene::SceneCache};
use boltsnap::ui::text::Fonts;
use std::time::Instant;

use std::thread;

use image::RgbaImage;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::RegistryState,
    seat::{SeatState, pointer::ThemedPointer},
    shell::wlr_layer::LayerShell,
    shm::{Shm, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard::WlKeyboard, wl_output},
};

use crate::DynResult;

mod input;
mod present;
mod snap;
mod startup;
mod wayland;

use input::{Interaction, MIN_SEL, contains};
use present::OutputView;
use startup::Start;

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
    let start = Start {
        instant,
        record_mode: false,
        show_frame: true,
        audio_enabled: true,
    };
    let mut sel = run_selector(start, Some(capture_handle), Err(ClipState::Off))?;
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
    let start = Start {
        instant: false,
        record_mode: true,
        show_frame: initial_show_frame,
        audio_enabled: initial_audio_enabled,
    };
    let mut sel = run_selector(start, None, super::replay::preview_target())?;
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
    start: Start,
    capture_handle: Option<thread::JoinHandle<Result<CapturedDesktop, String>>>,
    replay: Result<super::replay::PreviewTarget, ClipState>,
) -> DynResult<Selector> {
    // With a buffer, Clip waits for its frozen frame; without, it says why.
    let clip = replay.as_ref().err().copied().unwrap_or(ClipState::Loading);
    let replay = replay.ok();
    let font_handle = thread::spawn(ui_fonts);
    let timing = super::timing::Span::new("selector_setup");

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Selector>(&conn)?;
    super::timing::mark("selector_wayland_connected");
    let mut sel = Selector::new(&globals, event_queue.handle(), start, clip, replay.as_ref())?;

    // Discover outputs (names) so we can target the focused monitor. This
    // roundtrip overlaps with the capture thread (screenshot mode).
    event_queue.roundtrip(&mut sel)?;

    // Record mode: nothing to join; the backdrop is built from a transparent
    // base on the first configure.
    if let Some(handle) = capture_handle {
        sel.adopt_capture(handle)?;
    }
    let targets = if start.record_mode {
        vec![sel.record_output(replay.as_ref())?]
    } else {
        sel.capture_outputs()?
    };
    sel.chrome = chrome(
        font_handle.join().map_err(|_| "font worker panicked")?,
        sel.clip,
    );
    if let Some(video) = sel.video {
        sel.set_video(video);
    }
    sel.open_views(targets);
    if !sel.target_layout_valid() {
        return Err("capture monitor layout changed during capture".into());
    }
    drop(timing);

    let mut events = calloop::EventLoop::<Selector>::try_new()?;
    calloop_wayland_source::WaylandSource::new(conn, event_queue)
        .insert(events.handle())
        .map_err(|e| format!("selector event source: {e}"))?;
    if let Some(target) = replay {
        startup::watch_replay_preview(&events, target, sel.replay_output.clone())?;
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
    /// The screen's pixels as a recording or clip sees them, for the size
    /// the toolbar shows.
    video: Option<(u32, u32)>,
    /// Whether Clip has its buffer, and if not, why.
    clip: ClipState,
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
    /// `selector_magnifier = "always"`: the magnifier shows without Alt.
    magnifier_always: bool,
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
    /// Ctrl's rectangles to snap to (`snap.rs`).
    snap: snap::Snap,
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

/// The chrome in `fonts`; with replay off the toolbar has no Clip at all
/// (decided before the first frame and kept while the selector is open).
fn chrome(fonts: Fonts, clip: ClipState) -> render::Chrome {
    let mut chrome = render::Chrome::new(fonts);
    if clip == ClipState::Off {
        chrome.toolbar = chrome.toolbar.without_clip();
    }
    chrome
}

/// The toolbar's targets: what is under the pointer and held, the toggles,
/// and whether Clip has a buffer.
fn bar_targets(
    hovered: Option<usize>,
    pressed: Option<usize>,
    show_frame: bool,
    audio: bool,
    clip: ClipState,
    size: Option<(u32, u32)>,
) -> Targets {
    use render::toolbar::{AUDIO, CLIP, FRAME};
    Targets {
        hovered,
        pressed,
        on: std::array::from_fn(|i| (i == FRAME && show_frame) || (i == AUDIO && audio)),
        enabled: std::array::from_fn(|i| i != CLIP || clip == ClipState::Ready),
        reason: clip.reason(),
        size,
    }
}

/// The output's current mode in pixels, turned as the screen is.
fn video_size(info: &smithay_client_toolkit::output::OutputInfo) -> Option<(u32, u32)> {
    use wl_output::Transform;
    let mode = info.modes.iter().find(|mode| mode.current)?;
    let (w, h) = (
        u32::try_from(mode.dimensions.0).ok()?,
        u32::try_from(mode.dimensions.1).ok()?,
    );
    let turned = matches!(
        info.transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    );
    (w > 0 && h > 0).then_some(if turned { (h, w) } else { (w, h) })
}

impl Selector {
    /// The magnifier follows the pointer: with Alt held, or always when
    /// set; never while recording a region, and only while the pointer is
    /// over the selector (it starts at (0, 0) and stays where it left).
    fn magnifier_on(&self) -> bool {
        (self.alt_held || self.magnifier_always) && !self.record_mode && self.pointer_in
    }

    /// Hand the toolbar what it should show now: there once a selection is
    /// drawn (not while it is first dragged out), following it as it changes.
    fn sync_bar(&mut self, now: Instant) {
        if !self.record_mode {
            return;
        }
        let layout = match self.mode {
            // No toolbar under a rectangle that is only being offered.
            Mode::Editing { rect } if self.snap_shown().is_none() => render::toolbar::layout(
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
        let size = match self.mode {
            Mode::Editing { rect } => self.clip_size(rect),
            _ => None,
        };
        let targets = bar_targets(
            hovered,
            self.pressed,
            self.show_frame,
            self.audio_enabled,
            self.clip,
            size,
        );
        self.motion.update(layout, &targets, now);
    }

    /// The pixels a recording or clip of `rect` has, as the clip's crop
    /// computes them; `None` when it would be too small.
    fn clip_size(&self, rect: edit::Rect) -> Option<(u32, u32)> {
        let crop = boltsnap::replay::crop::Crop::from_selection(
            [rect.x, rect.y, rect.w, rect.h],
            (self.surf_w, self.surf_h),
            self.video?,
        )
        .ok()?;
        Some((crop.width, crop.height))
    }

    /// The screen's pixels, and room for the widest size they allow.
    fn set_video(&mut self, video: (u32, u32)) {
        self.video = Some(video);
        self.chrome.toolbar = self.chrome.toolbar.with_size(&self.chrome.fonts, video);
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

#[cfg(test)]
mod shots;
