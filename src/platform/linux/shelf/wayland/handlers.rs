//! The Wayland plumbing the shelf needs: compositor, outputs, layer surfaces,
//! seats, shm and the registry.

use smithay_client_toolkit::compositor::CompositorHandler;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::ThemeSpec;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_data_device, delegate_keyboard, delegate_layer, delegate_output,
    delegate_pointer, delegate_registry, delegate_seat, delegate_shm, registry_handlers,
};
use wayland_client::protocol::wl_output;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use super::Daemon;

impl CompositorHandler for Daemon {
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
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, surface: &WlSurface, _: u32) {
        let shelf = self
            .layer
            .as_ref()
            .is_some_and(|layer| layer.wl_surface() == surface);
        if shelf {
            self.frame_requested = None;
            if self.animating() {
                self.tick_animations(qh);
            }
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

impl OutputHandler for Daemon {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_tray_snapshot();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_tray_snapshot();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_tray_snapshot();
    }
}

impl LayerShellHandler for Daemon {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.is_popup_surface(layer.wl_surface()) {
            self.remove_popup();
            return;
        }
        if self.is_marker_surface(layer.wl_surface()) {
            self.remove_marker();
            return;
        }
        if self
            .layer
            .as_ref()
            .map(|shelf| shelf.wl_surface() == layer.wl_surface())
            .unwrap_or(false)
        {
            // Its output went away (hotplug, KVM). That must not end the
            // daemon and a recording with it; the next card places a new one.
            self.layer = None;
            self.output_name = None;
            self.shelf_configured = false;
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if self.is_popup_surface(layer.wl_surface()) {
            self.popup_configured = true;
            self.draw_popup();
            return;
        }
        if self.is_marker_surface(layer.wl_surface()) {
            self.marker_configured = true;
            self.draw_marker();
            return;
        }
        let is_shelf = self
            .layer
            .as_ref()
            .map(|shelf| shelf.wl_surface() == layer.wl_surface())
            .unwrap_or(false);
        if !is_shelf {
            return;
        }
        // wlr-layer-shell: a configure dimension of 0 means "client, pick your own
        // size". For our self-sized, bottom-left-anchored shelf, Hyprland replies
        // to set_size() with new_size=(0,0); keeping the stale value here left the
        // surface at the startup 1x1 (mapped but invisible). Fall back to layout.
        self.width = if configure.new_size.0 != 0 {
            configure.new_size.0
        } else {
            self.layout.width.max(1)
        };
        self.height = if configure.new_size.1 != 0 {
            configure.new_size.1
        } else {
            self.layout.height.max(1)
        };
        let should_draw = self.shelf_pending_draw || self.model.newest_first().next().is_some();
        self.mark_shelf_configured();
        if should_draw {
            self.draw(qh);
        }
    }
}

impl SeatHandler for Daemon {
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
        if cap == Capability::Keyboard
            && self.keyboard.is_none()
            && let Ok(k) = self.seat_state.get_keyboard(qh, &seat, None)
        {
            self.keyboard = Some(k);
        }
        if self.data_device.is_none() {
            self.data_device = Some(self.ddm.get_data_device(qh, &seat));
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

impl ShmHandler for Daemon {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Daemon {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Daemon);

delegate_output!(Daemon);

delegate_shm!(Daemon);

delegate_seat!(Daemon);

delegate_keyboard!(Daemon);

delegate_pointer!(Daemon);

delegate_layer!(Daemon);

delegate_data_device!(Daemon);

delegate_registry!(Daemon);
