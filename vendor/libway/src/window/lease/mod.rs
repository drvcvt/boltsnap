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
    shell::client::{
        xdg_popup::XdgPopup, xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel,
        xdg_wm_base::XdgWmBase,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

mod pending;
use pending::PendingNative;

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
    /// The parent link keeps the parent's role and surface alive until this
    /// popup's objects are destroyed, so the wire order is always child first.
    Popup {
        popup: XdgPopup,
        xdg_surface: XdgSurface,
        parent: Arc<WindowNative>,
    },
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
    Popup {
        options: &'a super::popup::PopupOptions,
        parent: Arc<WindowNative>,
        grab: Option<(wayland_client::protocol::wl_seat::WlSeat, u32)>,
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
    /// Live popup natives whose parent this is. CPU retire keeps the surface
    /// mapped while it is not zero, so no popup sits on an unmapped parent.
    pub(super) popup_children: AtomicUsize,
    /// CPU retire skipped its unmap because popups were alive; the last popup
    /// native to go performs it. Whoever swaps it to false unmaps.
    detach_deferred: AtomicBool,
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
            RoleRequest::Popup {
                options,
                parent,
                grab,
            } => pending.create_popup(qh, id, options, parent, grab)?,
        }
        // Successful role creation initializes every proxy. No intervening step
        // removes a proxy before ownership transfers out of this cleanup guard.
        let role = match (pending.layer.take(), pending.popup.take()) {
            (Some(layer), _) => Role::Layer(layer),
            (None, Some(popup)) => {
                let parent = pending.parent.take().expect("invariant: popup parent");
                parent.popup_children.fetch_add(1, Ordering::AcqRel);
                Role::Popup {
                    popup,
                    xdg_surface: pending
                        .xdg_surface
                        .take()
                        .expect("invariant: xdg surface created"),
                    parent,
                }
            }
            (None, None) => Role::Toplevel {
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
            popup_children: AtomicUsize::new(0),
            detach_deferred: AtomicBool::new(false),
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

    /// The toplevel role, or InvalidInput for a layer or popup.
    pub(super) fn toplevel(&self) -> Result<&XdgToplevel> {
        match &self.role {
            Role::Toplevel { toplevel, .. } => Ok(toplevel),
            _ => Err(Error::InvalidInput("window is not a toplevel")),
        }
    }

    /// The layer role, or InvalidInput for a toplevel or popup.
    pub(super) fn layer(&self) -> Result<&ZwlrLayerSurfaceV1> {
        match &self.role {
            Role::Layer(layer) => Ok(layer),
            _ => Err(Error::InvalidInput("window is not a layer")),
        }
    }

    /// The popup role, or InvalidInput for a toplevel or layer.
    pub(super) fn popup(&self) -> Result<&XdgPopup> {
        match &self.role {
            Role::Popup { popup, .. } => Ok(popup),
            _ => Err(Error::InvalidInput("window is not a popup")),
        }
    }

    /// The `xdg_surface` a popup can name as its parent; `None` for a layer.
    pub(super) fn xdg_surface(&self) -> Option<&XdgSurface> {
        match &self.role {
            Role::Toplevel { xdg_surface, .. } | Role::Popup { xdg_surface, .. } => {
                Some(xdg_surface)
            }
            Role::Layer(_) => None,
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
            Role::Popup {
                popup,
                xdg_surface,
                parent,
            } => {
                popup.destroy();
                xdg_surface.destroy();
                if parent.popup_children.fetch_sub(1, Ordering::AcqRel) == 1
                    && parent.detach_deferred.swap(false, Ordering::AcqRel)
                {
                    // The parent was retired while this popup lived: unmap it
                    // now that no popup depends on it.
                    parent.surface.attach(None, 0, 0);
                    parent.surface.commit();
                }
            }
        }
        self.scaling.destroy();
        self.surface.destroy();
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
    /// Defer this retired window's unmap to its last popup native. Returns
    /// false when none is alive (any more): the caller unmaps now.
    pub(super) fn defer_detach(&self) -> bool {
        let native = &self.native;
        native.detach_deferred.store(true, Ordering::Release);
        !(native.popup_children.load(Ordering::Acquire) == 0
            && native.detach_deferred.swap(false, Ordering::AcqRel))
    }

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
