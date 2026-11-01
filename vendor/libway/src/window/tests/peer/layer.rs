//! Opt-in `zwlr_layer_shell_v1` peer and `wl_region` objects. Requests are
//! recorded literally with their raw wire values.
use super::{Options, Request, State};
use wayland_protocols_wlr::layer_shell::v1::server::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::GlobalId, protocol::wl_region,
};

#[derive(Default)]
pub(in crate::window::tests) struct LayerPeerState {
    global: Option<GlobalId>,
    version: u32,
    /// Regions created so far; the index is the region's ledger identity.
    pub regions: usize,
}

#[derive(Debug)]
pub(in crate::window::tests) enum LayerWireEvent {
    /// Window, serial, width, height.
    Configure(usize, u32, u32, u32),
    Closed(usize),
    Remove,
}

pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut LayerPeerState) {
    state.version = options.layer_shell_version;
    if options.layer_shell {
        state.global = Some(dh.create_global::<State, ZwlrLayerShellV1, _>(state.version, ()));
    }
}

pub(super) fn emit(event: LayerWireEvent, state: &mut State, dh: &DisplayHandle) {
    match event {
        LayerWireEvent::Configure(window, serial, width, height) => state.windows[window]
            .layer
            .as_ref()
            .unwrap()
            .configure(serial, width, height),
        LayerWireEvent::Closed(window) => state.windows[window].layer.as_ref().unwrap().closed(),
        LayerWireEvent::Remove => dh.remove_global::<State>(state.layer.global.take().unwrap()),
    }
}

fn raw<T: Into<u32>>(value: WEnum<T>) -> u32 {
    match value {
        WEnum::Value(value) => value.into(),
        WEnum::Unknown(value) => value,
    }
}

impl GlobalDispatch<ZwlrLayerShellV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrLayerShellV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let shell = init.init(resource, ());
        state.ledger.push(Request::BindLayerShell(shell.version()));
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwlrLayerShellV1,
        request: zwlr_layer_shell_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_layer_shell_v1::Request::GetLayerSurface {
                id,
                surface,
                output,
                layer,
                namespace,
            } => {
                let index = *surface.data::<usize>().unwrap();
                state.windows[index].layer = Some(init.init(id, index));
                let output = output.map(|output| *output.data::<usize>().unwrap());
                state
                    .ledger
                    .push(Request::LayerSurface(index, output, raw(layer), namespace));
            }
            zwlr_layer_shell_v1::Request::Destroy => state.ledger.push(Request::DestroyLayerShell),
            request => panic!("unexpected layer shell request: {request:?}"),
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwlrLayerSurfaceV1,
        request: zwlr_layer_surface_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use zwlr_layer_surface_v1::Request as R;
        let index = *index;
        state.ledger.push(match request {
            R::SetSize { width, height } => Request::LayerSize(index, width, height),
            R::SetAnchor { anchor } => Request::LayerAnchor(index, raw(anchor)),
            R::SetExclusiveZone { zone } => Request::LayerZone(index, zone),
            R::SetMargin {
                top,
                right,
                bottom,
                left,
            } => Request::LayerMargin(index, [top, right, bottom, left]),
            R::SetKeyboardInteractivity {
                keyboard_interactivity,
            } => Request::LayerKeyboard(index, raw(keyboard_interactivity)),
            R::SetLayer { layer } => Request::LayerSetLayer(index, raw(layer)),
            R::SetExclusiveEdge { edge } => Request::LayerEdge(index, raw(edge)),
            R::AckConfigure { serial } => Request::LayerAck(index, serial),
            R::GetPopup { popup } => Request::LayerGetPopup(index, *popup.data::<usize>().unwrap()),
            R::Destroy => Request::DestroyLayerSurface(index),
            request => panic!("unexpected layer surface request: {request:?}"),
        });
    }
}

impl Dispatch<wl_region::WlRegion, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_region::WlRegion,
        request: wl_region::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        state.ledger.push(match request {
            wl_region::Request::Add {
                x,
                y,
                width,
                height,
            } => Request::RegionAdd(*index, x, y, width, height),
            wl_region::Request::Destroy => Request::DestroyRegion(*index),
            request => panic!("unexpected region request: {request:?}"),
        });
    }
}
