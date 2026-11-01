//! Popup events: placement, reposition completion and dismissal. The
//! `xdg_surface.configure` that completes a sequence is handled with every
//! other role in `shell/protocol.rs`.
use super::super::{state::WindowState, types::WindowId};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{
    xdg_popup::{self, XdgPopup},
    xdg_positioner::XdgPositioner,
};

impl Dispatch<XdgPopup, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        // Events for a popup closed earlier (for example by a cascade) are stale.
        let Some(popup) = state
            .windows
            .get_mut(id)
            .and_then(|record| record.popup.as_mut())
        else {
            return;
        };
        match event {
            xdg_popup::Event::Configure {
                x,
                y,
                width,
                height,
            } => {
                if width <= 0 || height <= 0 {
                    // A popup cannot be placed without a size: close it
                    // instead of acknowledging an unusable configuration.
                    state.dismiss_popup(*id, false);
                    return;
                }
                popup.placed = Some((x, y, width as u32, height as u32));
            }
            xdg_popup::Event::Repositioned { token } => popup.token = Some(token),
            xdg_popup::Event::PopupDone => {
                let refused = popup.grab
                    && state
                        .windows
                        .get(id)
                        .is_some_and(|record| record.accepted.is_none());
                state.dismiss_popup(*id, refused);
            }
            _ => {}
        }
    }
}

delegate_noop!(WindowState: ignore XdgPositioner);
