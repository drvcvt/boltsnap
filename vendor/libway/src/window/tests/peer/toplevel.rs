//! Opt-in `xdg_activation_v1` and `zwp_idle_inhibit_manager_v1` peers. Requests
//! are recorded literally; a token's `done` is only sent when a test emits it.
use super::{Options, Request, State};
use wayland_protocols::wp::idle_inhibit::zv1::server::{
    zwp_idle_inhibit_manager_v1::{self, ZwpIdleInhibitManagerV1},
    zwp_idle_inhibitor_v1::{self, ZwpIdleInhibitorV1},
};
use wayland_protocols::xdg::activation::v1::server::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::{self, XdgActivationV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::GlobalId,
};

#[derive(Default)]
pub(in crate::window::tests) struct AttentionPeerState {
    activation: Option<GlobalId>,
    idle_inhibit: Option<GlobalId>,
    /// Tokens in creation order; the index is the token's ledger identity.
    tokens: Vec<XdgActivationTokenV1>,
    /// Inhibitors created so far; the index is the inhibitor's ledger identity.
    inhibitors: usize,
}

#[derive(Debug)]
pub(in crate::window::tests) enum ActivationWireEvent {
    /// Token index and the token string sent with `done`.
    Done(usize, String),
    /// Remove the activation global.
    Remove,
}

#[derive(Debug)]
pub(in crate::window::tests) enum IdleInhibitWireEvent {
    /// Remove the idle inhibit manager global.
    Remove,
}

pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut AttentionPeerState) {
    if options.activation {
        state.activation = Some(dh.create_global::<State, XdgActivationV1, _>(1, ()));
    }
    if options.idle_inhibit {
        state.idle_inhibit = Some(dh.create_global::<State, ZwpIdleInhibitManagerV1, _>(1, ()));
    }
}

pub(super) fn emit_activation(event: ActivationWireEvent, state: &mut State, dh: &DisplayHandle) {
    match event {
        ActivationWireEvent::Done(token, string) => state.attention.tokens[token].done(string),
        ActivationWireEvent::Remove => {
            dh.remove_global::<State>(state.attention.activation.take().unwrap())
        }
    }
}

pub(super) fn emit_idle_inhibit(
    event: IdleInhibitWireEvent,
    state: &mut State,
    dh: &DisplayHandle,
) {
    match event {
        IdleInhibitWireEvent::Remove => {
            dh.remove_global::<State>(state.attention.idle_inhibit.take().unwrap())
        }
    }
}

impl GlobalDispatch<XdgActivationV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<XdgActivationV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
        state.ledger.push(Request::BindActivation);
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &XdgActivationV1,
        request: xdg_activation_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_activation_v1::Request::GetActivationToken { id } => {
                let index = state.attention.tokens.len();
                let token = init.init(id, index);
                state.attention.tokens.push(token);
                state.ledger.push(Request::ActivationToken(index));
            }
            xdg_activation_v1::Request::Activate { token, surface } => {
                let window = *surface.data::<usize>().unwrap();
                state.ledger.push(Request::Activate(token, window));
            }
            xdg_activation_v1::Request::Destroy => state.ledger.push(Request::DestroyActivation),
            request => panic!("unexpected activation request: {request:?}"),
        }
    }
}

impl Dispatch<XdgActivationTokenV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &XdgActivationTokenV1,
        request: xdg_activation_token_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use xdg_activation_token_v1::Request as R;
        let index = *index;
        state.ledger.push(match request {
            R::SetSerial { serial, .. } => Request::TokenSerial(index, serial),
            R::SetAppId { app_id } => Request::TokenAppId(index, app_id),
            R::SetSurface { surface } => {
                Request::TokenSurface(index, *surface.data::<usize>().unwrap())
            }
            R::Commit => Request::TokenCommit(index),
            R::Destroy => Request::DestroyToken(index),
            request => panic!("unexpected activation token request: {request:?}"),
        });
    }
}

impl GlobalDispatch<ZwpIdleInhibitManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwpIdleInhibitManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
        state.ledger.push(Request::BindIdleInhibit);
    }
}

impl Dispatch<ZwpIdleInhibitManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpIdleInhibitManagerV1,
        request: zwp_idle_inhibit_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_idle_inhibit_manager_v1::Request::CreateInhibitor { id, surface } => {
                let index = state.attention.inhibitors;
                state.attention.inhibitors += 1;
                init.init(id, index);
                let window = *surface.data::<usize>().unwrap();
                state.ledger.push(Request::Inhibitor(index, window));
            }
            zwp_idle_inhibit_manager_v1::Request::Destroy => {
                state.ledger.push(Request::DestroyIdleInhibitManager)
            }
            request => panic!("unexpected idle inhibit manager request: {request:?}"),
        }
    }
}

impl Dispatch<ZwpIdleInhibitorV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpIdleInhibitorV1,
        request: zwp_idle_inhibitor_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_idle_inhibitor_v1::Request::Destroy => {
                state.ledger.push(Request::DestroyInhibitor(*index))
            }
            request => panic!("unexpected idle inhibitor request: {request:?}"),
        }
    }
}
