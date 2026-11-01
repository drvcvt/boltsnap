//! Opt-in cursor-shape-v1 peer. Devices are indexed by the pointer they belong to.
use super::{Options, Request, State};
use wayland_protocols::wp::cursor_shape::v1::server::{
    wp_cursor_shape_device_v1::{self, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::{self, WpCursorShapeManagerV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::GlobalId,
};

#[derive(Default)]
pub(in crate::window::tests) struct CursorState {
    global: Option<GlobalId>,
    devices: Vec<WpCursorShapeDeviceV1>,
}

pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut CursorState) {
    if options.cursor_shape {
        state.global = Some(dh.create_global::<State, WpCursorShapeManagerV1, _>(1, ()));
    }
}

pub(super) fn remove_manager(state: &mut State, dh: &DisplayHandle) {
    dh.remove_global::<State>(state.cursor.global.take().unwrap());
}

impl GlobalDispatch<WpCursorShapeManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WpCursorShapeManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
        state.ledger.push(Request::BindCursor);
    }
}

impl Dispatch<WpCursorShapeManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WpCursorShapeManagerV1,
        request: wp_cursor_shape_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_cursor_shape_manager_v1::Request::GetPointer {
                cursor_shape_device,
                pointer,
            } => {
                let index = *pointer.data::<usize>().unwrap();
                state
                    .cursor
                    .devices
                    .push(init.init(cursor_shape_device, index));
                state.ledger.push(Request::CursorDevice(index));
            }
            wp_cursor_shape_manager_v1::Request::Destroy => {
                state.ledger.push(Request::DestroyCursorManager)
            }
            request => panic!("unexpected cursor shape manager request: {request:?}"),
        }
    }
}

impl Dispatch<WpCursorShapeDeviceV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WpCursorShapeDeviceV1,
        request: wp_cursor_shape_device_v1::Request,
        pointer: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_cursor_shape_device_v1::Request::SetShape { serial, shape } => {
                state
                    .ledger
                    .push(Request::SetShape(*pointer, serial, shape.into()));
            }
            wp_cursor_shape_device_v1::Request::Destroy => {
                state.ledger.push(Request::DestroyCursorDevice(*pointer))
            }
            request => panic!("unexpected cursor shape device request: {request:?}"),
        }
    }
}
