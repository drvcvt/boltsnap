//! `xdg_positioner` and `xdg_popup` server side. Requests are recorded
//! literally with their raw wire values; popup events are only sent when a
//! test emits them, so a test chooses the order inside one sequence.
use super::super::{Request, State};
use wayland_protocols::xdg::shell::server::{
    xdg_popup::{self, XdgPopup},
    xdg_positioner::{self, XdgPositioner},
    xdg_surface::XdgSurface,
};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, New, Resource, WEnum};

#[derive(Default)]
pub(in crate::window::tests) struct PopupPeerState {
    /// Positioners created so far; the index is the positioner's ledger identity.
    positioners: usize,
}

#[derive(Debug)]
pub(in crate::window::tests) enum PopupWireEvent {
    /// Window, x, y, width, height (`xdg_popup.configure`).
    Configure(usize, i32, i32, i32, i32),
    /// Window and token (`xdg_popup.repositioned`, v3).
    Repositioned(usize, u32),
    /// Window and serial: the `xdg_surface.configure` that completes a sequence.
    SurfaceConfigure(usize, u32),
    /// `xdg_popup.popup_done` for this window.
    Done(usize),
}

pub(in crate::window::tests::peer) fn emit(event: PopupWireEvent, state: &State) {
    let popup = |window: usize| state.windows[window].popup.as_ref().unwrap();
    match event {
        PopupWireEvent::Configure(window, x, y, width, height) => {
            popup(window).configure(x, y, width, height)
        }
        PopupWireEvent::Repositioned(window, token) => popup(window).repositioned(token),
        PopupWireEvent::SurfaceConfigure(window, serial) => state.windows[window]
            .role
            .as_ref()
            .unwrap()
            .configure(serial),
        PopupWireEvent::Done(window) => popup(window).popup_done(),
    }
}

/// `xdg_wm_base.create_positioner`.
pub(super) fn create_positioner(
    state: &mut State,
    id: New<XdgPositioner>,
    init: &mut DataInit<'_, State>,
) {
    let index = state.popup.positioners;
    state.popup.positioners += 1;
    init.init(id, index);
    state.ledger.push(Request::Positioner(index));
}

/// `xdg_surface.get_popup` on the xdg_surface of window `index`.
pub(super) fn get_popup(
    state: &mut State,
    index: usize,
    (id, parent, positioner): (New<XdgPopup>, Option<XdgSurface>, XdgPositioner),
    init: &mut DataInit<'_, State>,
) {
    state.windows[index].popup = Some(init.init(id, index));
    let parent = parent.map(|parent| *parent.data::<usize>().unwrap());
    let positioner = *positioner.data::<usize>().unwrap();
    state.ledger.push(Request::Popup(index, parent, positioner));
}

fn raw<T: Into<u32>>(value: WEnum<T>) -> u32 {
    match value {
        WEnum::Value(value) => value.into(),
        WEnum::Unknown(value) => value,
    }
}

impl Dispatch<XdgPositioner, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &XdgPositioner,
        request: xdg_positioner::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use xdg_positioner::Request as R;
        let index = *index;
        state.ledger.push(match request {
            R::SetSize { width, height } => Request::PositionerSize(index, width, height),
            R::SetAnchorRect {
                x,
                y,
                width,
                height,
            } => Request::PositionerAnchorRect(index, x, y, width, height),
            R::SetAnchor { anchor } => Request::PositionerAnchor(index, raw(anchor)),
            R::SetGravity { gravity } => Request::PositionerGravity(index, raw(gravity)),
            R::SetConstraintAdjustment {
                constraint_adjustment,
            } => Request::PositionerConstraint(index, raw(constraint_adjustment)),
            R::SetOffset { x, y } => Request::PositionerOffset(index, x, y),
            R::SetReactive => Request::PositionerReactive(index),
            R::Destroy => Request::DestroyPositioner(index),
            request => panic!("unexpected positioner request: {request:?}"),
        });
    }
}

impl Dispatch<XdgPopup, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &XdgPopup,
        request: xdg_popup::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let index = *index;
        state.ledger.push(match request {
            xdg_popup::Request::Grab { seat, serial } => {
                Request::Grab(index, *seat.data::<usize>().unwrap(), serial)
            }
            xdg_popup::Request::Reposition { positioner, token } => {
                Request::Reposition(index, *positioner.data::<usize>().unwrap(), token)
            }
            xdg_popup::Request::Destroy => Request::DestroyPopup(index),
            request => panic!("unexpected popup request: {request:?}"),
        });
    }
}
