//! Optional server-side decoration negotiation (xdg-decoration-unstable-v1).
//!
//! The manager is bound with the other discovery-time globals, so one that appears
//! after discovery completed is never bound, and windows created before a removal
//! keep their decoration object. Windows are never retrofitted.
use super::{
    state::WindowState,
    types::{Decorations, WindowId},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop, protocol::wl_registry::WlRegistry,
};
use wayland_protocols::xdg::{
    decoration::zv1::client::{
        zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        zxdg_toplevel_decoration_v1::{self, Mode, ZxdgToplevelDecorationV1},
    },
    shell::client::xdg_toplevel::XdgToplevel,
};

struct Bound {
    name: u32,
    proxy: ZxdgDecorationManagerV1,
    available: std::sync::atomic::AtomicBool,
}

/// The bound manager remains owned after global removal, through all native leases.
#[derive(Default)]
pub(super) struct DecorationGlobals {
    bound: Option<Bound>,
}

impl DecorationGlobals {
    pub(super) fn bind(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if version == 0 || interface != "zxdg_decoration_manager_v1" || self.bound.is_some() {
            return;
        }
        self.bound = Some(Bound {
            name,
            proxy: registry.bind(name, 1, qh, ()),
            available: true.into(),
        });
    }

    pub(super) fn remove(&self, name: u32) {
        if let Some(bound) = self.bound.as_ref().filter(|bound| bound.name == name) {
            bound
                .available
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    /// Request server-side mode before the first commit, when the manager is live.
    pub(super) fn create(
        &self,
        toplevel: &XdgToplevel,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
    ) -> Option<ZxdgToplevelDecorationV1> {
        let bound = self
            .bound
            .as_ref()
            .filter(|bound| bound.available.load(std::sync::atomic::Ordering::Acquire))?;
        let decoration = bound.proxy.get_toplevel_decoration(toplevel, qh, id);
        decoration.set_mode(Mode::ServerSide);
        Some(decoration)
    }

    /// Parent global teardown calls this before its final nonblocking flush.
    pub(super) fn destroy(&mut self) {
        if let Some(bound) = self.bound.take() {
            bound.proxy.destroy();
        }
    }
}

impl Drop for DecorationGlobals {
    fn drop(&mut self) {
        self.destroy();
    }
}

delegate_noop!(WindowState: ignore ZxdgDecorationManagerV1);

/// The latest compositor mode applies at the next acknowledged xdg configure and
/// persists across configures that carry no decoration event. An unknown mode
/// value is treated as client-side.
impl Dispatch<ZxdgToplevelDecorationV1, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        _: &ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event else {
            return;
        };
        if let Some(record) = state.windows.get_mut(id) {
            record.staged.states.decorations = match mode {
                WEnum::Value(Mode::ServerSide) => Decorations::Server,
                _ => Decorations::Client,
            };
        }
    }
}
