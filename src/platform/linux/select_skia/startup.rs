//! The selector's setup before its first frame: the Wayland state, the
//! frozen desktop, which outputs get an overlay and where, the overlays
//! themselves, and the replay preview that arrives later.

use std::thread;
use std::time::Instant;

use boltsnap::ui::text::Fonts;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::RegistryState,
    seat::SeatState,
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell},
    },
    shm::{Shm, slot::SlotPool},
};
use wayland_client::{QueueHandle, globals::GlobalList, protocol::wl_output};

use super::present::OutputView;
use super::{CapturedDesktop, Mode, Selector, bar_targets, chrome, video_size};
use crate::DynResult;
use crate::selector::clip::ClipState;
use crate::selector::motion::BarMotion;
use crate::selector::scene::SceneCache;
use crate::selector::{desktop, render};

/// How the selector opens.
#[derive(Clone, Copy)]
pub(super) struct Start {
    /// Skip the editable phase: release in Drawing confirms immediately.
    pub instant: bool,
    /// Record mode: no frozen screenshot, the selection rectangle is the result.
    pub record_mode: bool,
    pub show_frame: bool,
    pub audio_enabled: bool,
}

/// An output and the part of the desktop its overlay shows.
type Target = (wl_output::WlOutput, desktop::Viewport);

impl Selector {
    /// The Wayland state bound from `globals`, nothing shown yet.
    pub(super) fn new(
        globals: &GlobalList,
        qh: QueueHandle<Self>,
        start: Start,
        clip: ClipState,
        replay: Option<&super::super::replay::PreviewTarget>,
    ) -> DynResult<Self> {
        let compositor = CompositorState::bind(globals, &qh)?;
        let shm = Shm::bind(globals, &qh)?;
        let layer_shell = LayerShell::bind(globals, &qh)?;
        let pool = SlotPool::new(256, &shm)?;
        let targets = bar_targets(
            None,
            None,
            start.show_frame,
            start.audio_enabled,
            clip,
            None,
        );
        Ok(Self {
            registry_state: RegistryState::new(globals),
            seat_state: SeatState::new(globals, &qh),
            output_state: OutputState::new(globals, &qh),
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
            chrome: chrome(Fonts::fallback(), clip),
            motion: BarMotion::new(&targets, Instant::now()),
            pressed: None,
            pointer_in: false,
            clip,
            video: None,
            surf_w: 0,
            surf_h: 0,
            configured: false,
            mode: Mode::Idle,
            interaction: None,
            cursor: (0.0, 0.0),
            alt_held: false,
            magnifier_always: crate::config::Config::load().magnifier_always(),
            result: None,
            done: false,
            qh,
            needs_redraw: false,
            instant: start.instant,
            record_mode: start.record_mode,
            show_frame: start.show_frame,
            audio_enabled: start.audio_enabled,
            result_rect: None,
            result_clip: false,
            replay_output: replay.map(|r| r.output.clone()),
            frozen: None,
            clip_reported: false,
            output_origin: None,
            // With both displays the selector opens where the pointer is.
            target_name: replay.and_then(|r| r.monitor().map(String::from)),
            failure: None,
            snap: Default::default(),
        })
    }

    /// Screenshot mode: block on the capture; the grab ran during setup.
    pub(super) fn adopt_capture(
        &mut self,
        handle: thread::JoinHandle<Result<CapturedDesktop, String>>,
    ) -> DynResult<()> {
        let image = handle
            .join()
            .map_err(|_| "capture worker panicked".to_string())??;
        self.target_name = image.preferred_output;
        self.captured_monitors = image.monitors;
        self.image = Some(image.image);
        Ok(())
    }

    /// Record mode: the one output to record on, its whole surface.
    pub(super) fn record_output(
        &mut self,
        replay: Option<&super::super::replay::PreviewTarget>,
    ) -> DynResult<Target> {
        let output = if let Some(name) = &self.target_name {
            self.output_state.outputs().find(|o| {
                self.output_state
                    .info(o)
                    .is_some_and(|i| i.name.as_ref() == Some(name))
            })
        } else {
            self.focused_output()
        }
        .ok_or("capture monitor disconnected")?;
        let info = self.output_state.info(&output);
        self.output_origin = info
            .as_ref()
            .map(|i| i.logical_position.unwrap_or(i.location));
        self.video = info.as_ref().and_then(video_size);
        // Both displays: the frozen frame comes cut to this monitor.
        if replay.is_some_and(|r| r.monitor().is_none()) {
            self.replay_output = info.and_then(|i| i.name);
        }
        Ok((output, (0, 0, 1, 1)))
    }

    /// Screenshot mode: every captured monitor's output and its part of the
    /// frozen desktop.
    pub(super) fn capture_outputs(&mut self) -> DynResult<Vec<Target>> {
        let regions: Vec<_> = self
            .captured_monitors
            .iter()
            .map(|m| (m.origin.0, m.origin.1, m.logical_size.0, m.logical_size.1))
            .collect();
        let (left, top, w, h) = desktop::bounds(&regions).ok_or("invalid desktop layout")?;
        self.surf_w = w;
        self.surf_h = h;
        self.output_origin = Some((left, top));
        let targets = self
            .captured_monitors
            .iter()
            .map(|m| {
                let output = self
                    .output_state
                    .outputs()
                    .find(|o| {
                        self.output_state
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
            .collect::<Result<Vec<_>, &str>>()?;
        Ok(targets)
    }

    /// One overlay layer surface per target, committed without a buffer
    /// (unmapped until the first frame).
    pub(super) fn open_views(&mut self, targets: Vec<Target>) {
        for (output, viewport) in targets {
            let surface = self.compositor.create_surface(&self.qh);
            let layer = self.layer_shell.create_layer_surface(
                &self.qh,
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
            self.views.push(OutputView {
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
    }
}

/// Clip's frozen frame, prepared on its own thread; when it comes, the
/// selector shows it as its backdrop and Clip becomes ready (or failed).
pub(super) fn watch_replay_preview(
    events: &calloop::EventLoop<Selector>,
    target: super::super::replay::PreviewTarget,
    on: Option<String>,
) -> DynResult<()> {
    let (sender, receiver) =
        calloop::channel::channel::<Option<super::super::replay::FrozenSelection>>();
    events
        .handle()
        .insert_source(receiver, |event, _, sel| {
            let calloop::channel::Event::Msg(frozen) = event else {
                return;
            };
            let failed = |sel: &mut Selector| {
                sel.clip = ClipState::Failed;
                sel.request_redraw();
            };
            let Some(mut frozen) = frozen else {
                return failed(sel);
            };
            if sel.replay_output.as_deref() != Some(frozen.output.as_str()) {
                return failed(sel);
            }
            sel.image = frozen.preview.take();
            sel.base = Some(match sel.image.as_ref() {
                Some(image) => {
                    render::base_pixmap_from_image(image, sel.surf_w.max(1), sel.surf_h.max(1))
                }
                None => return failed(sel),
            });
            sel.overlay = sel.base.as_ref().map(SceneCache::new);
            for view in &mut sel.views {
                view.committed_generation = None;
                for buffer in &mut view.buffers {
                    buffer.generation = None;
                }
            }
            let size = frozen.size();
            sel.frozen = Some(frozen);
            if sel.video != Some(size) {
                sel.set_video(size);
            }
            sel.clip = ClipState::Ready;
            sel.clip_reported = false;
            sel.request_redraw();
        })
        .map_err(|e| format!("replay preview source: {e}"))?;
    thread::spawn(move || {
        let _ = sender.send(super::super::replay::FrozenSelection::prepare(
            &target,
            on.as_deref(),
        ));
    });
    Ok(())
}
