//! Standard native interop on the exclusive claim, without exposing protocol owners.
mod frame;
mod protocol;
pub(super) mod state;
use super::{WindowId, lease::PresenterLease};
pub(super) use frame::begin;
pub use frame::{NativeFrame, NativeFrameStart};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use std::ptr::NonNull;
use wayland_client::Proxy;

/// Retains every native dependency while one cooperating renderer owns presentation.
/// This claim is not clonable. Standard handle traits cannot prevent a consumer
/// from creating multiple foreign surfaces from one borrowed handle: create exactly
/// one and drop it before releasing the claim. Perform driver calls only between
/// host pump calls, after every connection read reservation has been resolved.
/// Use [`super::Host::begin_native_frame`] to retain the owner boundary through
/// configure/acquire/encode/submit and immediately through the driver present call.
pub struct NativePresenter {
    lease: PresenterLease,
}

impl NativePresenter {
    pub(super) fn new(lease: PresenterLease) -> Self {
        Self { lease }
    }

    /// The retained window identity.
    pub fn id(&self) -> WindowId {
        self.lease.id()
    }

    /// Retirement forbids new presentation; existing native ownership stays valid
    /// until the renderer and this claim have both been released.
    pub fn is_retired(&self) -> bool {
        self.lease.is_retired()
    }
}

impl HasDisplayHandle for NativePresenter {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        if self.is_retired() {
            return Err(HandleError::Unavailable);
        }
        let pointer = NonNull::new(self.lease.connection().backend().display_ptr().cast())
            .ok_or(HandleError::Unavailable)?;
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(pointer));
        // SAFETY: the complete owned connection is retained by this presenter for
        // the borrowed handle lifetime, including after host shutdown/retirement.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

impl HasWindowHandle for NativePresenter {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        if self.is_retired() {
            return Err(HandleError::Unavailable);
        }
        let pointer = NonNull::new(self.lease.surface().id().as_ptr().cast())
            .ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer));
        // SAFETY: this lease owns the surface, roles, globals and connection;
        // their destruction follows the last retained presenter/ordinary lease.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}
