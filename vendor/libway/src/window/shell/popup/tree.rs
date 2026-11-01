//! Popup creation, reposition, the grab chain rule and dismissal.
use super::{PopupOptions, PopupRecord, positioner, validate};
use crate::window::{Host, WindowEvent, WindowId, lease::RoleRequest, state::WindowState};
use crate::{Error, Result};
use wayland_client::Proxy;

impl Host {
    /// Create a popup on a toplevel, layer or popup of this host that has
    /// presented a frame. Everything is validated before any request; then the
    /// surface, the positioner, `get_popup` (plus `zwlr_layer_surface_v1.get_popup`
    /// for a layer parent), the optional grab and the bufferless first commit
    /// are sent. Wait for `Configured` (size and `placement`) before presenting.
    ///
    /// With `grab`, the seat's latest pointer or key press must lie in the
    /// parent's popup tree (the parent, its ancestors or popups of that tree
    /// closed since). A grabbing popup's parent is a toplevel, a layer or the
    /// topmost grabbing popup; there is one grab chain per host.
    ///
    /// Errors: `NotReady` before `Ready`, before the parent's first frame, or
    /// without such a press; `Unsupported` for `reactive` below `xdg_wm_base`
    /// v3; `InvalidInput` for an unknown, retired or foreign parent or seat,
    /// an empty anchor rectangle or one outside the parent, a grab on a
    /// non-grabbing or non-topmost popup, or a second grab chain;
    /// `InvalidDimensions` for a size of 0 or above `i32::MAX`;
    /// `LimitExceeded` beyond 16 windows. Nothing is sent on error.
    pub fn create_popup(&mut self, options: PopupOptions) -> Result<WindowId> {
        self.coordinator.mutate(|state| state.create_popup(options))
    }

    /// Ask the compositor to place a shown popup again (`xdg_wm_base` v3).
    /// Reads only the placement fields of `options`; `parent` must be the
    /// popup's parent and `grab` is ignored. Returns the token that the
    /// completing configuration carries in `placement.repositioned`; several
    /// calls may complete as one.
    ///
    /// Errors: `Unsupported` below v3; `InvalidInput` for a window that is not
    /// a popup, a retired window or another parent; `NotReady` before the
    /// popup's first frame; validation as for [`Host::create_popup`].
    pub fn reposition_popup(&mut self, popup: WindowId, options: &PopupOptions) -> Result<u32> {
        self.coordinator
            .mutate(|state| state.reposition_popup(popup, options))
    }
}

impl WindowState {
    pub(crate) fn create_popup(&mut self, options: PopupOptions) -> Result<WindowId> {
        self.ensure_active()?;
        let parent = self.record(options.parent)?;
        if !parent.mapped {
            return Err(Error::NotReady);
        }
        let parent_size = parent
            .accepted
            .map(|accepted| (accepted.width, accepted.height))
            .ok_or(Error::NotReady)?;
        let parent_grabs = parent.popup.map(|popup| popup.grab);
        let native = parent.native.clone();
        let globals = self.creation_globals()?;
        validate(&options, parent_size, globals.shell.version())?;
        let grab = match options.grab {
            None => None,
            Some(seat) => {
                if parent_grabs == Some(false) {
                    return Err(Error::InvalidInput(
                        "a grabbing popup needs a toplevel, layer or grabbing popup parent",
                    ));
                }
                self.check_grab_chain(options.parent)?;
                Some(
                    self.input
                        .grab_serial(seat, self.tree_root(options.parent))?,
                )
            }
        };
        let grabbing = grab.is_some();
        let id = self.insert(
            globals,
            options.size,
            None,
            RoleRequest::Popup {
                options: &options,
                parent: native,
                grab,
            },
        )?;
        let record = self.windows.get_mut(&id).expect("inserted record");
        record.parent = Some(options.parent);
        record.popup = Some(PopupRecord::new(grabbing));
        Ok(id)
    }

    /// One grab chain per host: a new grabbing popup must sit on the topmost
    /// grabbing popup when one exists.
    fn check_grab_chain(&self, parent: WindowId) -> Result<()> {
        let grabbing: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, record)| record.popup.is_some_and(|popup| popup.grab))
            .map(|(id, _)| *id)
            .collect();
        let topmost = grabbing.iter().copied().find(|candidate| {
            !grabbing
                .iter()
                .any(|other| self.windows[other].parent == Some(*candidate))
        });
        match topmost {
            None => Ok(()),
            Some(topmost) if topmost == parent => Ok(()),
            Some(_) => Err(Error::InvalidInput(
                "a grabbing popup must extend the existing grab chain",
            )),
        }
    }

    pub(crate) fn reposition_popup(
        &mut self,
        popup: WindowId,
        options: &PopupOptions,
    ) -> Result<u32> {
        self.ensure_active()?;
        let record = self.record(popup)?;
        let xdg_popup = record.native.popup()?.clone();
        let globals = self.globals.clone().ok_or(Error::NotReady)?;
        if globals.shell.version() < 3 {
            return Err(Error::Unsupported("reposition needs xdg_wm_base v3"));
        }
        if record.parent != Some(options.parent) {
            return Err(Error::InvalidInput(
                "reposition must keep the popup's parent",
            ));
        }
        if !record.mapped {
            return Err(Error::NotReady);
        }
        let token = record.popup.map_or(1, |popup| popup.next_token);
        let parent_size = self
            .record(options.parent)?
            .accepted
            .map(|accepted| (accepted.width, accepted.height))
            .ok_or(Error::NotReady)?;
        validate(options, parent_size, globals.shell.version())?;
        let rules = positioner(&globals.shell, &self.qh, options);
        xdg_popup.reposition(&rules, token);
        rules.destroy();
        if let Some(popup) = self
            .windows
            .get_mut(&popup)
            .and_then(|record| record.popup.as_mut())
        {
            popup.next_token = token.wrapping_add(1).max(1);
        }
        self.requested()?;
        Ok(token)
    }

    /// The compositor dismissed `id` (`popup_done`) or sent an unusable
    /// configuration: report a refused grab when it never got configured,
    /// then close it and every popup above it.
    pub(crate) fn dismiss_popup(&mut self, id: WindowId, refused: bool) {
        // The refusal and every Closed of the cascade fit together, or neither
        // is queued and the host fails as for any full queue.
        if !self.event_room(self.close_order(id).len() + usize::from(refused)) {
            self.remember(&Error::LimitExceeded);
            return;
        }
        if refused {
            self.events
                .push_back(WindowEvent::PopupGrabRefused { window: id });
        }
        if let Err(error) = self.close(id) {
            self.remember(&error);
        }
    }
}
