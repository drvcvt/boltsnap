//! The optional `zwlr_layer_shell_v1` global and layer-surface events.
//!
//! Bound with the other discovery-time globals, at most version 5. A global that
//! appears after discovery is never bound. After removal the bound proxy stays
//! owned (existing layers keep working) but creates nothing new.
use super::{
    configure::SuggestedConfiguration, protocol::configure_surface, state::WindowState,
    types::WindowId,
};
use crate::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, protocol::wl_registry::WlRegistry,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

const MAX_VERSION: u32 = 5;

struct Bound {
    name: u32,
    proxy: ZwlrLayerShellV1,
    available: AtomicBool,
}

#[derive(Default)]
pub(super) struct LayerShellGlobal {
    bound: Option<Bound>,
}

impl LayerShellGlobal {
    pub(super) fn bind(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if version == 0 || interface != "zwlr_layer_shell_v1" || self.bound.is_some() {
            return;
        }
        self.bound = Some(Bound {
            name,
            proxy: registry.bind(name, version.min(MAX_VERSION), qh, ()),
            available: AtomicBool::new(true),
        });
    }

    pub(super) fn remove(&self, name: u32) {
        if let Some(bound) = self.bound.as_ref().filter(|bound| bound.name == name) {
            bound.available.store(false, Ordering::Release);
        }
    }

    /// The live manager, or why there is none.
    pub(super) fn available(&self) -> crate::Result<&ZwlrLayerShellV1> {
        self.bound
            .as_ref()
            .filter(|bound| bound.available.load(Ordering::Acquire))
            .map(|bound| &bound.proxy)
            .ok_or(Error::Unsupported("zwlr_layer_shell_v1 is not available"))
    }

    /// Parent global teardown calls this before its final nonblocking flush.
    /// Versions before 3 have no destructor; the object lives with the connection.
    pub(super) fn destroy(&mut self) {
        if let Some(bound) = self.bound.take()
            && bound.proxy.version() >= 3
        {
            bound.proxy.destroy();
        }
    }
}

impl Drop for LayerShellGlobal {
    fn drop(&mut self) {
        self.destroy();
    }
}

delegate_noop!(WindowState: ignore ZwlrLayerShellV1);

impl Dispatch<ZwlrLayerSurfaceV1, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Events for a retired identity are ignored; its objects may still be
        // alive in a lease.
        if !state.active() || !state.windows.contains_key(id) {
            return;
        }
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                let Some(record) = state.windows.get_mut(id) else {
                    return;
                };
                let Some(requested) = record.layer else {
                    return;
                };
                // 0 means "client decides": use the requested size on that axis.
                let width = if width == 0 { requested.width } else { width };
                let height = if height == 0 {
                    requested.height
                } else {
                    height
                };
                if width == 0 || height == 0 {
                    state.remember(&Error::InvalidInput("layer configure without size"));
                    return;
                }
                record.staged = SuggestedConfiguration::sized(width, height);
                configure_surface(state, *id, serial, |serial| surface.ack_configure(serial));
            }
            zwlr_layer_surface_v1::Event::Closed => {
                // Final: retire like Host::close, which publishes Closed and
                // records its own failures.
                let _ = state.close(*id);
            }
            _ => {}
        }
    }
}
