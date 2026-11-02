//! The selector's Wayland handlers: outputs, the layer surfaces' configure,
//! shm and the registry. Seat input is in `input.rs`, frames in `present.rs`.

use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::SeatState,
    shell::wlr_layer::{LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    shm::{Shm, ShmHandler},
};
use wayland_client::{Connection, QueueHandle, protocol::wl_output};

use super::Selector;
use crate::selector::{render, scene::SceneCache};

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
        if self.record_mode && self.video.is_none() {
            // No mode from the compositor: the surface's own pixels.
            self.set_video((w, h));
        }
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
            super::super::timing::mark("selector_configured");
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
