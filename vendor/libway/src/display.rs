//! One Wayland connection shared by libway capabilities.
use crate::{Error, Result};
use std::os::unix::net::UnixStream;
use wayland_client::{Connection as WlConnection, Proxy, backend::ObjectId, protocol::wl_surface};

/// Owned, shared or guest connection. A guest never closes the foreign display and must be
/// dropped (together with every object created from it) before the display's owner.
#[derive(Clone)]
pub struct Display {
    conn: WlConnection,
    owned: bool,
    guest: bool,
}
impl Display {
    /// Own connection selected by `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`.
    pub fn connect() -> Result<Self> {
        let conn = WlConnection::connect_to_env().map_err(|e| Error::Wayland(e.to_string()))?;
        Ok(Self {
            conn,
            owned: true,
            guest: false,
        })
    }
    /// Own connection over a caller-supplied socket (tests, sandboxes).
    pub fn from_socket(socket: UnixStream) -> Result<Self> {
        let conn = WlConnection::from_socket(socket).map_err(|e| Error::Wayland(e.to_string()))?;
        Ok(Self {
            conn,
            owned: true,
            guest: false,
        })
    }
    /// Share a cooperating client's wayland-rs connection. libway creates its own event queues
    /// and never dispatches the caller's queues.
    pub fn from_connection(conn: WlConnection) -> Self {
        Self {
            conn,
            owned: false,
            guest: false,
        }
    }
    /// Attach as a guest to a `wl_display*` owned by another library (winit, GTK, SDL).
    ///
    /// # Safety
    /// `display` must be a live `wl_display` created by libwayland-client in this process.
    /// The owner must keep it alive until this `Display` and everything created from it are
    /// dropped. Exactly one libwayland-client must be linked; `foreign-display` enables the
    /// `wayland-client/system` feature so that winit and libway share it.
    #[cfg(feature = "foreign-display")]
    pub unsafe fn from_raw(display: std::ptr::NonNull<std::ffi::c_void>) -> Self {
        let backend = unsafe {
            wayland_client::backend::Backend::from_foreign_display(display.as_ptr().cast())
        };
        Self {
            conn: WlConnection::from_backend(backend),
            owned: false,
            guest: true,
        }
    }
    /// Underlying connection for cooperating event queues. Reading it can enqueue events
    /// for other queues; each owner must still dispatch its own queue.
    pub fn connection(&self) -> &WlConnection {
        &self.conn
    }
    /// Whether libway opened this connection and closes it with the last clone.
    pub fn is_owned(&self) -> bool {
        self.owned
    }
    /// Attached to a `wl_display*` whose owner runs its own read loop.
    #[cfg_attr(not(feature = "dnd"), allow(dead_code))]
    pub(crate) fn is_guest(&self) -> bool {
        self.guest
    }
}

/// Identity of a `wl_surface` on the connection of a [`Display`]; used to route drop targets.
#[derive(Clone, Debug)]
pub struct SurfaceHandle {
    id: ObjectId,
    /// Provenance for safe handles. Raw handles rely on the constructor's safety contract.
    #[cfg(feature = "dnd")]
    backend: Option<wayland_client::backend::WeakBackend>,
}
// Keep ObjectId's equality/hash semantics, including raw/safe equivalence. Weak backend
// liveness must not change a handle's hash or equality after insertion into a collection.
impl PartialEq for SurfaceHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for SurfaceHandle {}
impl std::hash::Hash for SurfaceHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl SurfaceHandle {
    /// Copy a surface's identity and retain weak connection provenance. This does not keep
    /// the surface or connection alive. DnD requests reject foreign or destroyed safe handles
    /// with [`Error::InvalidInput`]; unregister targets before destroying their surface.
    pub fn from_surface(surface: &wl_surface::WlSurface) -> Self {
        Self {
            id: surface.id(),
            #[cfg(feature = "dnd")]
            backend: Some(surface.backend().clone()),
        }
    }
    /// Import a foreign proxy without taking ownership (`foreign-display` feature).
    /// Unlike safe handles, display provenance cannot be checked for this constructor.
    ///
    /// # Safety
    /// `surface` must be a `wl_surface*` on the same display as the [`Display`] it is used
    /// with, and must stay alive for as long as this handle or a clone is used: unregister its
    /// targets and end drags from it before the owner destroys the surface.
    #[cfg(feature = "foreign-display")]
    pub unsafe fn from_raw(surface: std::ptr::NonNull<std::ffi::c_void>) -> Result<Self> {
        let id = unsafe {
            ObjectId::from_ptr(wl_surface::WlSurface::interface(), surface.as_ptr().cast())
        }
        .map_err(|_| Error::Wayland("pointer is not a wl_surface".into()))?;
        Ok(Self { id, backend: None })
    }
    /// Native object identity. Compare/use it only within the owning connection;
    /// this handle's equality and hash follow the backend's ObjectId semantics.
    pub fn id(&self) -> &ObjectId {
        &self.id
    }
    #[cfg(feature = "dnd")]
    pub(crate) fn validate(&self, conn: &WlConnection) -> Result<()> {
        if let Some(weak) = &self.backend {
            let original = weak
                .upgrade()
                .ok_or(Error::InvalidInput("surface connection is gone"))?;
            let same = original == conn.backend();
            #[cfg(feature = "foreign-display")]
            let same = same || original.display_ptr() == conn.backend().display_ptr();
            if !same {
                return Err(Error::InvalidInput("surface belongs to another display"));
            }
        }
        conn.backend()
            .info(self.id.clone())
            .map_err(|_| Error::InvalidInput("surface is no longer alive"))?;
        Ok(())
    }
}
