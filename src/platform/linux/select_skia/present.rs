//! Drawing the scene into wl_shm buffers on the compositor's frame clock.

use smithay_client_toolkit::{
    compositor::CompositorHandler,
    shell::{WaylandSurface, wlr_layer::LayerSurface},
    shm::slot::Buffer,
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_output, wl_surface::WlSurface},
};

use std::time::Instant;

use super::{Mode, Selector, contains};
use crate::selector::{desktop, edit, scene::Scene};

pub(super) struct FrameBuffer {
    pub(super) buffer: Buffer,
    pub(super) generation: Option<u64>,
    pub(super) size: (u32, u32),
}

pub(super) struct OutputView {
    pub(super) layer: LayerSurface,
    pub(super) output: wl_output::WlOutput,
    pub(super) viewport: desktop::Viewport,
    pub(super) configured: bool,
    pub(super) buffers: Vec<FrameBuffer>,
    pub(super) committed_generation: Option<u64>,
    pub(super) frame_pending: bool,
    pub(super) needs_redraw: bool,
}

impl OutputView {
    pub(super) fn contains(&self, point: (f64, f64)) -> bool {
        let (x, y, w, h) = self.viewport;
        contains((x as f64, y as f64, w as f64, h as f64), point)
    }
}

impl Selector {
    /// Request a redraw, throttled to the compositor's frame clock. Draws now if
    /// no frame callback is pending; otherwise marks a redraw owed so the next
    /// `frame` callback coalesces it. Keeps a fast drag from flooding commits.
    pub(super) fn request_redraw(&mut self) {
        // What the toolbar should show changes now, not at the next frame.
        self.sync_bar(Instant::now());
        self.needs_redraw = true;
        for view in &mut self.views {
            view.needs_redraw = true;
        }
        self.draw();
    }

    /// Render the current frame (screenshot + dim + optional selection) into a
    /// reusable wl_shm buffer and commit only changed regions.
    pub(super) fn draw(&mut self) {
        if !self.configured || self.done {
            return;
        }
        // Pointer events arrive far faster than frames (1000 Hz mice). Render
        // the scene only when some output can take a frame; `needs_redraw`
        // stays set so its frame callback draws the latest state.
        if !self
            .views
            .iter()
            .any(|view| view.needs_redraw && !view.frame_pending)
        {
            return;
        }
        let (Some(base), Some(overlay)) = (self.base.as_ref(), self.overlay.as_mut()) else {
            return;
        };
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

        let now = Instant::now();
        overlay.update(
            base,
            Scene {
                chrome_viewport: (!self.record_mode && self.views.len() > 1)
                    .then(|| {
                        self.views
                            .iter()
                            .find(|v| v.contains(self.cursor))
                            .map(|v| v.viewport)
                    })
                    .flatten(),
                selection: sel,
                editing: matches!(self.mode, Mode::Editing { .. }),
                record: self.record_mode,
                bar: if self.record_mode {
                    self.motion.sample(now)
                } else {
                    Default::default()
                },
                magnifier: (self.alt_held && !self.record_mode).then_some(self.cursor),
            },
            &self.chrome,
        );
        for view in &mut self.views {
            if view.frame_pending || !view.needs_redraw {
                continue;
            }
            let (_, _, w, h) = view.viewport;
            let damage: Vec<_> = overlay
                .damage_since(view.committed_generation)
                .into_iter()
                .filter_map(|r| desktop::local_damage(view.viewport, r))
                .collect();
            if view.committed_generation.is_some() && damage.is_empty() {
                view.committed_generation = Some(overlay.generation());
                view.needs_redraw = false;
                continue;
            }
            // Retire differently sized slots only after release. They count toward
            // the same three-slot cap even during rapid compositor resizes.
            view.buffers
                .retain(|slot| slot.size == (w, h) || slot.buffer.canvas(&mut self.pool).is_none());
            let free = view
                .buffers
                .iter()
                .position(|b| b.size == (w, h) && b.buffer.canvas(&mut self.pool).is_some());
            let index = match free {
                Some(index) => index,
                None if view.buffers.len() < 3 => {
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
                    view.buffers.push(FrameBuffer {
                        buffer,
                        generation: None,
                        size: (w, h),
                    });
                    view.buffers.len() - 1
                }
                None => {
                    view.needs_redraw = true;
                    continue;
                }
            };
            let slot = &mut view.buffers[index];
            let repair = overlay.damage_since(slot.generation);
            let Some(canvas) = slot.buffer.canvas(&mut self.pool) else {
                view.needs_redraw = true;
                continue;
            };
            overlay.copy_viewport(canvas, &repair, view.viewport);
            slot.generation = Some(overlay.generation());
            let surface = view.layer.wl_surface();
            if let Err(error) = slot.buffer.attach_to(surface) {
                self.failure = Some(format!("selector attach: {error}"));
                self.done = true;
                return;
            }
            for (x0, y0, x1, y1) in damage {
                surface.damage_buffer(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32);
            }
            surface.frame(&self.qh, surface.clone());
            view.layer.commit();
            if view.committed_generation.is_none() {
                super::super::timing::mark("selector_first_commit");
            }
            view.committed_generation = Some(overlay.generation());
            view.frame_pending = true;
            view.needs_redraw = false;
        }
        // A spring still running asks for the next frame even when this one
        // showed nothing new, the way the settings window does.
        if self.motion.moving(now) {
            for view in &mut self.views {
                if !view.frame_pending {
                    let surface = view.layer.wl_surface();
                    surface.frame(&self.qh, surface.clone());
                    view.layer.commit();
                    view.frame_pending = true;
                }
                view.needs_redraw = true;
            }
        }
        self.needs_redraw = self.views.iter().any(|v| v.needs_redraw);
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
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        if let Some(view) = self
            .views
            .iter_mut()
            .find(|v| v.layer.wl_surface() == surface)
        {
            view.frame_pending = false;
        }
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
