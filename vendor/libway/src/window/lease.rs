//! Complete native dependency ownership, without proxy or handle escapes.
use super::{
    decoration::DecorationGlobals,
    layer::{LayerState, native_layer, send_state},
    layer_protocol::LayerShellGlobal,
    proxy::HostCommand,
    scale::{ScaleGlobals, SurfaceScaleObjects, WindowScale},
    state::WindowState,
    types::{Configuration, WindowId},
};
use crate::{Error, Result, runtime::mailbox::Notifier};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    protocol::{
        wl_compositor::WlCompositor, wl_output::WlOutput, wl_registry::WlRegistry, wl_shm::WlShm,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::xdg::{
    decoration::zv1::client::zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
    shell::client::{xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase},
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

pub(super) const MAX_WINDOWS: usize = 16;

pub(super) struct Globals {
    pub(super) connection: Connection,
    pub(super) _registry: WlRegistry,
    pub(super) compositor: WlCompositor,
    pub(super) shell: XdgWmBase,
    pub(super) shm: Option<WlShm>,
    pub(super) scaling: ScaleGlobals,
    pub(super) decoration: DecorationGlobals,
    pub(super) layer_shell: LayerShellGlobal,
    pub(super) attention: super::attention::AttentionGlobals,
    pub(super) compositor_name: u32,
    pub(super) shell_name: u32,
    pub(super) native_windows: AtomicUsize,
    pub(super) notifier: Notifier<HostCommand>,
}

impl Globals {
    fn flush_and_notify(&self) {
        // Connection flush is nonblocking. It is best effort when the host is gone;
        // a surviving pump is durably woken to observe errors or output backpressure.
        let _ = self.connection.flush();
        let _ = self.notifier.notify();
    }
}

impl Drop for Globals {
    fn drop(&mut self) {
        // Every WindowNative retains this owner through all child destroy requests.
        debug_assert_eq!(self.native_windows.load(Ordering::Acquire), 0);
        self.scaling.destroy();
        self.decoration.destroy();
        self.layer_shell.destroy();
        self.attention.destroy();
        self.shell.destroy();
        // wl_registry has no wire destructor; retain its proxy through shell
        // teardown and release its server object with the owned connection.
        self.flush_and_notify();
    }
}

/// The surface role. Each owns its role objects; the surface outlives them.
enum Role {
    Toplevel {
        toplevel: XdgToplevel,
        decoration: Option<ZxdgToplevelDecorationV1>,
        xdg_surface: XdgSurface,
    },
    Layer(ZwlrLayerSurfaceV1),
}

/// What a new native window becomes.
pub(super) enum RoleRequest<'a> {
    Toplevel {
        title: String,
        app_id: String,
    },
    Layer {
        namespace: String,
        output: Option<&'a WlOutput>,
        state: &'a LayerState,
    },
}

pub(super) struct WindowNative {
    pub(super) id: WindowId,
    role: Role,
    surface: WlSurface,
    scaling: SurfaceScaleObjects,
    pub(super) globals: Arc<Globals>,
    retired: AtomicBool,
    presenter_claimed: AtomicBool,
}

impl WindowNative {
    pub(super) fn create(
        globals: Arc<Globals>,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
        request: RoleRequest<'_>,
    ) -> Result<Arc<Self>> {
        let mut pending = PendingNative::reserve(globals)?;
        pending.create_surface(qh, id)?;
        match request {
            RoleRequest::Toplevel { title, app_id } => {
                pending.create_toplevel(qh, id, title, app_id)?
            }
            RoleRequest::Layer {
                namespace,
                output,
                state,
            } => pending.create_layer(qh, id, namespace, output, state)?,
        }
        // Successful role creation initializes every proxy. No intervening step
        // removes a proxy before ownership transfers out of this cleanup guard.
        let role = match pending.layer.take() {
            Some(layer) => Role::Layer(layer),
            None => Role::Toplevel {
                toplevel: pending
                    .toplevel
                    .take()
                    .expect("invariant: toplevel created"),
                decoration: pending.decoration.take(),
                xdg_surface: pending
                    .xdg_surface
                    .take()
                    .expect("invariant: xdg surface created"),
            },
        };
        let native = Self {
            id,
            role,
            surface: pending.surface.take().expect("invariant: surface created"),
            scaling: std::mem::take(&mut pending.scaling),
            globals: pending.globals.clone(),
            retired: AtomicBool::new(false),
            presenter_claimed: AtomicBool::new(false),
        };
        pending.credit = false;
        Ok(Arc::new(native))
    }

    pub(super) fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }

    pub(super) fn matches_surface(&self, surface: &WlSurface) -> bool {
        self.surface == *surface
    }

    pub(super) fn surface(&self) -> &WlSurface {
        &self.surface
    }

    /// The toplevel role, or InvalidInput for a layer.
    pub(super) fn toplevel(&self) -> Result<&XdgToplevel> {
        match &self.role {
            Role::Toplevel { toplevel, .. } => Ok(toplevel),
            Role::Layer(_) => Err(Error::InvalidInput("window is not a toplevel")),
        }
    }

    /// The layer role, or InvalidInput for a toplevel.
    pub(super) fn layer(&self) -> Result<&ZwlrLayerSurfaceV1> {
        match &self.role {
            Role::Layer(layer) => Ok(layer),
            Role::Toplevel { .. } => Err(Error::InvalidInput("window is not a layer")),
        }
    }

    pub(super) fn scale_state(&self) -> WindowScale {
        WindowScale::new(self.surface.version(), self.scaling.fractional())
    }

    pub(super) fn lease(self: &Arc<Self>) -> WindowLease {
        WindowLease {
            native: self.clone(),
        }
    }

    pub(super) fn claim(self: &Arc<Self>) -> Result<PresenterLease> {
        if self.retired.load(Ordering::Acquire) {
            return Err(Error::InvalidInput("window is retired"));
        }
        self.presenter_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::InvalidInput("window presenter is already claimed"))?;
        Ok(PresenterLease {
            native: self.clone(),
        })
    }
}

impl Drop for WindowNative {
    fn drop(&mut self) {
        // This is the sole destruction site; the entire role chain remains alive
        // until the final lease, including when released from another thread.
        // Role objects go before the surface; the decoration object must not
        // outlive its toplevel.
        match &mut self.role {
            Role::Toplevel {
                toplevel,
                decoration,
                xdg_surface,
            } => {
                if let Some(decoration) = decoration.take() {
                    decoration.destroy();
                }
                toplevel.destroy();
                xdg_surface.destroy();
            }
            Role::Layer(layer) => layer.destroy(),
        }
        self.scaling.destroy();
        self.surface.destroy();
        self.globals.native_windows.fetch_sub(1, Ordering::AcqRel);
        self.globals.flush_and_notify();
    }
}

struct PendingNative {
    globals: Arc<Globals>,
    surface: Option<WlSurface>,
    xdg_surface: Option<XdgSurface>,
    toplevel: Option<XdgToplevel>,
    decoration: Option<ZxdgToplevelDecorationV1>,
    layer: Option<ZwlrLayerSurfaceV1>,
    scaling: SurfaceScaleObjects,
    credit: bool,
}

impl PendingNative {
    fn reserve(globals: Arc<Globals>) -> Result<Self> {
        globals
            .native_windows
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_add(1).filter(|next| *next <= MAX_WINDOWS)
            })
            .map_err(|_| Error::LimitExceeded)?;
        Ok(Self {
            globals,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            decoration: None,
            layer: None,
            scaling: SurfaceScaleObjects::default(),
            credit: true,
        })
    }

    fn create_surface(&mut self, qh: &QueueHandle<WindowState>, id: WindowId) -> Result<()> {
        self.surface = Some(self.globals.compositor.create_surface(qh, id));
        self.check()?;
        self.scaling = self.globals.scaling.create_surface(
            self.surface.as_ref().expect("invariant: surface created"),
            qh,
            id,
        )?;
        self.check()
    }

    /// Role setup and state precede the required bufferless first commit.
    fn create_layer(
        &mut self,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
        namespace: String,
        output: Option<&WlOutput>,
        state: &LayerState,
    ) -> Result<()> {
        let surface = self.surface.clone().expect("invariant: surface created");
        let layer = self.globals.layer_shell.available()?.get_layer_surface(
            &surface,
            output,
            native_layer(state.layer),
            namespace,
            qh,
            id,
        );
        send_state(&layer, None, state);
        self.layer = Some(layer);
        self.check()?;
        surface.commit();
        self.check()
    }

    fn create_toplevel(
        &mut self,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
        title: String,
        app_id: String,
    ) -> Result<()> {
        // Each checked construction stores its proxy before the next role uses
        // it, and this method never removes a stored proxy.
        self.xdg_surface = Some(self.globals.shell.get_xdg_surface(
            self.surface.as_ref().expect("invariant: surface created"),
            qh,
            id,
        ));
        self.check()?;
        self.toplevel = Some(
            self.xdg_surface
                .as_ref()
                .expect("invariant: xdg surface created")
                .get_toplevel(qh, id),
        );
        self.check()?;
        let toplevel = self.toplevel.as_ref().expect("invariant: toplevel created");
        toplevel.set_title(title);
        toplevel.set_app_id(app_id);
        // Server-side mode is requested before the first commit; the compositor's
        // reply, not this request, decides the configured mode.
        self.decoration = self.globals.decoration.create(toplevel, qh, id);
        // Role setup precedes the required first bufferless commit.
        self.surface
            .as_ref()
            .expect("invariant: surface created")
            .commit();
        self.check()
    }

    fn check(&self) -> Result<()> {
        if let Some(error) = self.globals.connection.backend().last_error() {
            return Err(Error::Wayland(error.to_string()));
        }
        if !self.scaling.alive()
            || self.surface.as_ref().is_some_and(|value| !value.is_alive())
            || self
                .xdg_surface
                .as_ref()
                .is_some_and(|value| !value.is_alive())
            || self
                .toplevel
                .as_ref()
                .is_some_and(|value| !value.is_alive())
            || self
                .decoration
                .as_ref()
                .is_some_and(|value| !value.is_alive())
            || self.layer.as_ref().is_some_and(|value| !value.is_alive())
        {
            return Err(Error::Wayland("native window creation failed".into()));
        }
        Ok(())
    }
}

impl Drop for PendingNative {
    fn drop(&mut self) {
        if !self.credit {
            return;
        }
        if let Some(layer) = &self.layer {
            layer.destroy();
        }
        if let Some(decoration) = &self.decoration {
            decoration.destroy();
        }
        if let Some(toplevel) = &self.toplevel {
            toplevel.destroy();
        }
        if let Some(surface) = &self.xdg_surface {
            surface.destroy();
        }
        self.scaling.destroy();
        if let Some(surface) = &self.surface {
            surface.destroy();
        }
        self.globals.native_windows.fetch_sub(1, Ordering::AcqRel);
        self.globals.flush_and_notify();
    }
}

/// Owns the complete native role, surface, global and connection dependency chain.
/// Clone and final Drop can run on another thread. Retirement prevents new work
/// without invalidating existing ownership; this lease grants no rendering access.
#[derive(Clone)]
pub struct WindowLease {
    native: Arc<WindowNative>,
}

impl WindowLease {
    /// The identity retained by this lease, including after logical retirement.
    pub fn id(&self) -> WindowId {
        self.native.id
    }

    /// Whether the host retired this identity. Existing native ownership remains
    /// valid until the final lease is released, but retirement forbids new work.
    pub fn is_retired(&self) -> bool {
        self.native.retired.load(Ordering::Acquire)
    }
}

/// An exclusive non-clonable presenter reservation, used by later output slices.
pub(crate) struct PresenterLease {
    native: Arc<WindowNative>,
}

impl PresenterLease {
    pub(super) fn id(&self) -> WindowId {
        self.native.id
    }

    pub(super) fn surface(&self) -> &WlSurface {
        &self.native.surface
    }

    #[cfg(feature = "owned-native-system")]
    pub(super) fn connection(&self) -> &Connection {
        &self.native.globals.connection
    }

    pub(super) fn is_retired(&self) -> bool {
        self.native.retired.load(Ordering::Acquire)
    }

    pub(super) fn configure_surface(&self, configuration: Configuration) -> Result<()> {
        if self.is_retired() {
            return Err(Error::InvalidInput("window is retired"));
        }
        self.native
            .scaling
            .stage(&self.native.surface, configuration)
    }
}

impl Drop for PresenterLease {
    fn drop(&mut self) {
        self.native
            .presenter_claimed
            .store(false, Ordering::Release);
    }
}
