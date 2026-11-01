//! A window's native objects while it is being created: destroyed again if
//! creation fails before they move into a [`super::WindowNative`].
use super::*;

pub(super) struct PendingNative {
    pub(super) globals: Arc<Globals>,
    pub(super) surface: Option<WlSurface>,
    pub(super) xdg_surface: Option<XdgSurface>,
    pub(super) toplevel: Option<XdgToplevel>,
    pub(super) decoration: Option<ZxdgToplevelDecorationV1>,
    pub(super) layer: Option<ZwlrLayerSurfaceV1>,
    pub(super) popup: Option<XdgPopup>,
    /// Released after this popup's objects in `Drop`.
    pub(super) parent: Option<Arc<WindowNative>>,
    pub(super) scaling: SurfaceScaleObjects,
    pub(super) credit: bool,
}

impl PendingNative {
    pub(super) fn reserve(globals: Arc<Globals>) -> Result<Self> {
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
            popup: None,
            parent: None,
            scaling: SurfaceScaleObjects::default(),
            credit: true,
        })
    }

    pub(super) fn create_surface(
        &mut self,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
    ) -> Result<()> {
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
    pub(super) fn create_layer(
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

    pub(super) fn create_toplevel(
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

    pub(super) fn create_popup(
        &mut self,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
        options: &super::super::popup::PopupOptions,
        parent: Arc<WindowNative>,
        grab: Option<(wayland_client::protocol::wl_seat::WlSeat, u32)>,
    ) -> Result<()> {
        let surface = self.surface.clone().expect("invariant: surface created");
        self.xdg_surface = Some(self.globals.shell.get_xdg_surface(&surface, qh, id));
        self.check()?;
        let positioner = super::super::popup::positioner(&self.globals.shell, qh, options);
        let popup = self
            .xdg_surface
            .as_ref()
            .expect("invariant: xdg surface created")
            .get_popup(parent.xdg_surface(), &positioner, qh, id);
        // The compositor copied the positioner's rules into the popup.
        positioner.destroy();
        self.popup = Some(popup);
        self.check()?;
        let popup = self.popup.as_ref().expect("invariant: popup created");
        if parent.xdg_surface().is_none() {
            parent.layer()?.get_popup(popup);
        }
        if let Some((seat, serial)) = grab {
            popup.grab(&seat, serial);
        }
        self.parent = Some(parent);
        // Role setup precedes the required first bufferless commit.
        surface.commit();
        self.check()
    }

    pub(super) fn check(&self) -> Result<()> {
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
            || self.popup.as_ref().is_some_and(|value| !value.is_alive())
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
        if let Some(popup) = &self.popup {
            popup.destroy();
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
