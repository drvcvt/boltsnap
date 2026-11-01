//! Opt-in xdg-decoration peer. Requests are recorded; modes are sent on demand.
use super::{Options, Request, State};
use wayland_protocols::xdg::decoration::zv1::server::{
    zxdg_decoration_manager_v1::{self, ZxdgDecorationManagerV1},
    zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::GlobalId,
};

#[derive(Default)]
pub(in crate::window::tests) struct DecorationState {
    global: Option<GlobalId>,
}

pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut DecorationState) {
    if options.decoration {
        state.global = Some(dh.create_global::<State, ZxdgDecorationManagerV1, _>(1, ()));
    }
}

pub(super) fn remove_manager(state: &mut State, dh: &DisplayHandle) {
    dh.remove_global::<State>(state.decoration.global.take().unwrap());
}

/// Sends the raw mode value; the following configure completes the sequence.
pub(super) fn emit(state: &State, window: usize, mode: u32) {
    state.windows[window]
        .decoration
        .as_ref()
        .unwrap()
        .send_event(zxdg_toplevel_decoration_v1::Event::Configure {
            mode: WEnum::Unknown(mode),
        })
        .unwrap();
}

impl GlobalDispatch<ZxdgDecorationManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZxdgDecorationManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
        state.ledger.push(Request::BindDecoration);
    }
}

impl Dispatch<ZxdgDecorationManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZxdgDecorationManagerV1,
        request: zxdg_decoration_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, toplevel } => {
                let index = *toplevel.data::<usize>().unwrap();
                state.windows[index].decoration = Some(init.init(id, index));
                state.ledger.push(Request::Decoration(index));
            }
            zxdg_decoration_manager_v1::Request::Destroy => {}
            request => panic!("unexpected decoration manager request: {request:?}"),
        }
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZxdgToplevelDecorationV1,
        request: zxdg_toplevel_decoration_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            zxdg_toplevel_decoration_v1::Request::SetMode { mode } => {
                state
                    .ledger
                    .push(Request::DecorationMode(*index, mode.into()));
            }
            zxdg_toplevel_decoration_v1::Request::Destroy => {
                state.ledger.push(Request::DestroyDecoration(*index))
            }
            request => panic!("unexpected toplevel decoration request: {request:?}"),
        }
    }
}
