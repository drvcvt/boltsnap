//! Attention requests (`xdg_activation_v1`) and idle inhibition
//! (`zwp_idle_inhibit_manager_v1`), for toplevels and layers.
//!
//! Both globals are bound with the other discovery-time globals at version 1. A
//! global that appears after discovery is never bound. After removal the bound
//! proxy stays owned, so pending tokens and existing inhibitors keep working,
//! but nothing new is created through it.
use super::{Host, state::WindowState, types::WindowId};
use crate::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, protocol::wl_registry::WlRegistry,
};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};

struct Bound<P> {
    name: u32,
    proxy: P,
    available: AtomicBool,
}

/// One optional version-1 global.
struct Optional<P> {
    bound: Option<Bound<P>>,
}

impl<P> Default for Optional<P> {
    fn default() -> Self {
        Self { bound: None }
    }
}

impl<P: Proxy + 'static> Optional<P> {
    fn bind(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) where
        WindowState: Dispatch<P, ()>,
    {
        let (name, interface, version) = global;
        if version == 0 || interface != P::interface().name || self.bound.is_some() {
            return;
        }
        self.bound = Some(Bound {
            name,
            proxy: registry.bind(name, 1, qh, ()),
            available: AtomicBool::new(true),
        });
    }

    fn remove(&self, name: u32) {
        if let Some(bound) = self.bound.as_ref().filter(|bound| bound.name == name) {
            bound.available.store(false, Ordering::Release);
        }
    }

    fn available(&self, missing: &'static str) -> Result<&P> {
        self.bound
            .as_ref()
            .filter(|bound| bound.available.load(Ordering::Acquire))
            .map(|bound| &bound.proxy)
            .ok_or(Error::Unsupported(missing))
    }
}

/// The activation and idle-inhibit managers.
#[derive(Default)]
pub(super) struct AttentionGlobals {
    activation: Optional<XdgActivationV1>,
    idle: Optional<ZwpIdleInhibitManagerV1>,
}

impl AttentionGlobals {
    pub(super) fn bind(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        self.activation.bind(registry, global, qh);
        self.idle.bind(registry, global, qh);
    }

    pub(super) fn remove(&self, name: u32) {
        self.activation.remove(name);
        self.idle.remove(name);
    }

    /// Parent global teardown calls this before its final nonblocking flush.
    pub(super) fn destroy(&mut self) {
        if let Some(bound) = self.activation.bound.take() {
            bound.proxy.destroy();
        }
        if let Some(bound) = self.idle.bound.take() {
            bound.proxy.destroy();
        }
    }
}

impl Drop for AttentionGlobals {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// A live idle inhibitor; dropping it ends the inhibition. A window record
/// declares it before its native owner, so it is destroyed before the surface.
pub(super) struct Inhibitor(ZwpIdleInhibitorV1);

impl Drop for Inhibitor {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

/// Activation tokens that wait for `done`, at most one per window.
#[derive(Default)]
pub(super) struct PendingTokens(Vec<(WindowId, XdgActivationTokenV1)>);

impl PendingTokens {
    /// Destroy every pending token without activating; used at retirement.
    pub(super) fn clear(&mut self) {
        for (_, token) in self.0.drain(..) {
            token.destroy();
        }
    }
}

impl Drop for PendingTokens {
    fn drop(&mut self) {
        self.clear();
    }
}

impl Host {
    /// Ask the compositor to draw the user's attention to `window`, for example
    /// by marking it urgent. The token carries no input serial, so this never
    /// moves keyboard focus on its own; Hyprland only focuses with
    /// `misc:focus_on_activate`. The protocol reports nothing back. While a
    /// request for this window still waits for its token, another call returns
    /// Ok and sends nothing. Works on toplevels and layers.
    ///
    /// Errors: `Unsupported` without `xdg_activation_v1`; `InvalidInput` for a
    /// retired or foreign window. Nothing is sent on error.
    pub fn request_attention(&mut self, window: WindowId) -> Result<()> {
        self.coordinator
            .mutate(|state| state.request_attention(window))
    }

    /// Keep the screen from blanking, dimming or locking while `window` is
    /// visible (`true`), or stop (`false`). Idempotent; the effect also ends
    /// when the window closes. Works on toplevels and layers.
    ///
    /// Errors: `Unsupported` when enabling without
    /// `zwp_idle_inhibit_manager_v1` (disabling always works); `InvalidInput`
    /// for a retired or foreign window. Nothing is sent on error.
    pub fn set_idle_inhibit(&mut self, window: WindowId, inhibit: bool) -> Result<()> {
        self.coordinator
            .mutate(|state| state.set_idle_inhibit(window, inhibit))
    }
}

impl WindowState {
    pub(crate) fn request_attention(&mut self, window: WindowId) -> Result<()> {
        self.ensure_active()?;
        let record = self.record(window)?;
        if self.tokens.0.iter().any(|(id, _)| *id == window) {
            return Ok(());
        }
        let globals = self.globals.as_ref().ok_or(Error::NotReady)?;
        let activation = globals
            .attention
            .activation
            .available("xdg_activation_v1 is not available")?;
        let token = activation.get_activation_token(&self.qh, window);
        token.set_surface(record.native.surface());
        token.commit();
        self.tokens.0.push((window, token));
        self.requested()
    }

    pub(crate) fn set_idle_inhibit(&mut self, window: WindowId, inhibit: bool) -> Result<()> {
        self.ensure_active()?;
        let record = self.record(window)?;
        if record.inhibitor.is_some() == inhibit {
            return Ok(());
        }
        let inhibitor = if inhibit {
            let globals = self.globals.as_ref().ok_or(Error::NotReady)?;
            let manager = globals
                .attention
                .idle
                .available("zwp_idle_inhibit_manager_v1 is not available")?;
            Some(Inhibitor(manager.create_inhibitor(
                record.native.surface(),
                &self.qh,
                (),
            )))
        } else {
            None
        };
        // Replacing the slot drops (destroys) an old inhibitor.
        self.windows
            .get_mut(&window)
            .expect("validated record")
            .inhibitor = inhibitor;
        self.requested()
    }
}

impl Dispatch<XdgActivationTokenV1, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        proxy: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        window: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_activation_token_v1::Event::Done { token } = event else {
            return;
        };
        let Some(index) = state.tokens.0.iter().position(|(_, t)| t == proxy) else {
            return;
        };
        let (_, pending) = state.tokens.0.swap_remove(index);
        // A closed window's token is only destroyed. The activation proxy is
        // used even after global removal: the token came from it.
        if let (Some(record), Some(globals)) = (state.windows.get(window), state.globals.as_ref())
            && let Some(bound) = &globals.attention.activation.bound
        {
            bound.proxy.activate(token, record.native.surface());
        }
        pending.destroy();
    }
}

delegate_noop!(WindowState: ignore XdgActivationV1);
delegate_noop!(WindowState: ignore ZwpIdleInhibitManagerV1);
delegate_noop!(WindowState: ignore ZwpIdleInhibitorV1);
