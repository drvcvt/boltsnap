//! Window management requests. Each validates its target (and, for the interactive
//! three, a recent pointer press) before any native request is issued.
use super::{Host, InputSeatId, state::WindowState, types::WindowId};
use crate::{Error, Result};
use wayland_protocols::xdg::shell::client::xdg_toplevel;

/// The edge or corner grabbed by [`Host::resize_window`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResizeEdge {
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Top left corner.
    TopLeft,
    /// Top right corner.
    TopRight,
    /// Bottom left corner.
    BottomLeft,
    /// Bottom right corner.
    BottomRight,
}

impl ResizeEdge {
    fn native(self) -> xdg_toplevel::ResizeEdge {
        match self {
            Self::Top => xdg_toplevel::ResizeEdge::Top,
            Self::Bottom => xdg_toplevel::ResizeEdge::Bottom,
            Self::Left => xdg_toplevel::ResizeEdge::Left,
            Self::Right => xdg_toplevel::ResizeEdge::Right,
            Self::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
            Self::TopRight => xdg_toplevel::ResizeEdge::TopRight,
            Self::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
            Self::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
        }
    }
}

impl Host {
    /// Start an interactive move of `window` with the pointer of `seat`.
    ///
    /// Needs the seat's latest pointer button press, which must have targeted
    /// this window, and uses that press's serial. No such press (none yet, another
    /// window, pointer capability lost) returns `NotReady`; a retired or foreign
    /// window or an unknown or removed seat returns `InvalidInput`. Nothing is sent
    /// on error. The compositor may still ignore the request, for example while the
    /// window is maximized or fullscreen, and reports nothing back.
    pub fn move_window(&mut self, window: WindowId, seat: InputSeatId) -> Result<()> {
        self.coordinator
            .mutate(|state| state.move_window(window, seat))
    }

    /// Start an interactive resize of `window` at `edge` with the pointer of
    /// `seat`. The press requirement, errors and compositor latitude are those of
    /// [`Host::move_window`]. The result arrives as ordinary configurations.
    pub fn resize_window(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        edge: ResizeEdge,
    ) -> Result<()> {
        self.coordinator
            .mutate(|state| state.resize_window(window, seat, edge))
    }

    /// Ask the compositor to pop up the window menu at `x`, `y` in surface-local
    /// logical coordinates. The press requirement, errors and compositor latitude
    /// are those of [`Host::move_window`].
    pub fn show_window_menu(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        x: i32,
        y: i32,
    ) -> Result<()> {
        self.coordinator
            .mutate(|state| state.show_window_menu(window, seat, x, y))
    }

    /// Request that `window` becomes maximized (`true`) or unmaximized (`false`).
    /// A hint: the compositor decides, and the outcome arrives as a configuration
    /// with `maximized` set or cleared. A retired or foreign window returns
    /// `InvalidInput`.
    pub fn set_maximized(&mut self, window: WindowId, maximized: bool) -> Result<()> {
        self.coordinator
            .mutate(|state| state.set_maximized(window, maximized))
    }

    /// Request that `window` is minimized. There is no unminimize request and no
    /// confirmation; the compositor owns restoring it. A retired or foreign
    /// window returns `InvalidInput`.
    pub fn set_minimized(&mut self, window: WindowId) -> Result<()> {
        self.coordinator.mutate(|state| state.set_minimized(window))
    }

    /// Request that `window` becomes fullscreen (`true`, without an output
    /// preference, so the compositor picks) or leaves fullscreen (`false`). A hint
    /// confirmed by a configuration with `fullscreen` set or cleared. A retired or
    /// foreign window returns `InvalidInput`.
    pub fn set_fullscreen(&mut self, window: WindowId, fullscreen: bool) -> Result<()> {
        self.coordinator
            .mutate(|state| state.set_fullscreen(window, fullscreen))
    }
}

impl WindowState {
    pub(crate) fn move_window(&mut self, window: WindowId, seat: InputSeatId) -> Result<()> {
        let (seat, serial) = self.press_target(window, seat)?;
        self.record(window)?.native.toplevel()?._move(&seat, serial);
        self.requested()
    }

    pub(crate) fn resize_window(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        edge: ResizeEdge,
    ) -> Result<()> {
        let (seat, serial) = self.press_target(window, seat)?;
        self.record(window)?
            .native
            .toplevel()?
            .resize(&seat, serial, edge.native());
        self.requested()
    }

    pub(crate) fn show_window_menu(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let (seat, serial) = self.press_target(window, seat)?;
        self.record(window)?
            .native
            .toplevel()?
            .show_window_menu(&seat, serial, x, y);
        self.requested()
    }

    pub(crate) fn set_maximized(&mut self, window: WindowId, maximized: bool) -> Result<()> {
        self.ensure_active()?;
        let toplevel = self.record(window)?.native.toplevel()?;
        if maximized {
            toplevel.set_maximized();
        } else {
            toplevel.unset_maximized();
        }
        self.requested()
    }

    pub(crate) fn set_minimized(&mut self, window: WindowId) -> Result<()> {
        self.ensure_active()?;
        self.record(window)?.native.toplevel()?.set_minimized();
        self.requested()
    }

    pub(crate) fn set_fullscreen(&mut self, window: WindowId, fullscreen: bool) -> Result<()> {
        self.ensure_active()?;
        let toplevel = self.record(window)?.native.toplevel()?;
        if fullscreen {
            toplevel.set_fullscreen(None);
        } else {
            toplevel.unset_fullscreen();
        }
        self.requested()
    }

    /// Validate the window and the seat's latest press on it, before any request.
    fn press_target(
        &self,
        window: WindowId,
        seat: InputSeatId,
    ) -> Result<(wayland_client::protocol::wl_seat::WlSeat, u32)> {
        self.ensure_active()?;
        self.record(window)?.native.toplevel()?;
        self.input.press(seat, window)
    }

    pub(super) fn requested(&mut self) -> Result<()> {
        if let Some(error) = self.connection.backend().last_error() {
            let error = Error::Wayland(error.to_string());
            self.remember(&error);
            return Err(error);
        }
        Ok(())
    }
}
