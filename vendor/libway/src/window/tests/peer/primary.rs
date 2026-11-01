//! Isolated primary selection peer with literal serial, MIME and byte records.
use super::{Options, Request, State};
use std::{
    io::{Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
};
use wayland_protocols::wp::primary_selection::zv1::server::{
    zwp_primary_selection_device_manager_v1::{self, ZwpPrimarySelectionDeviceManagerV1},
    zwp_primary_selection_device_v1::{self, ZwpPrimarySelectionDeviceV1},
    zwp_primary_selection_offer_v1::{self, ZwpPrimarySelectionOfferV1},
    zwp_primary_selection_source_v1::{self, ZwpPrimarySelectionSourceV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::GlobalId,
};

#[derive(Debug)]
pub(in crate::window::tests) enum PrimaryWireEvent {
    /// Seat, MIME names and bytes written at once on receive.
    Offer(usize, Vec<String>, Option<Vec<u8>>),
    SelectionNone(usize),
    Send(usize, String),
    DrainSend(usize),
    CancelSource(usize),
    Remove,
    Add,
}
struct Offer {
    _native: ZwpPrimarySelectionOfferV1,
    bytes: Option<Vec<u8>>,
}
#[derive(Default)]
pub(in crate::window::tests) struct PrimaryState {
    global: Option<GlobalId>,
    devices: Vec<ZwpPrimarySelectionDeviceV1>,
    offers: Vec<Offer>,
    sources: Vec<ZwpPrimarySelectionSourceV1>,
    sends: Vec<UnixStream>,
}
pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut PrimaryState) {
    if options.primary {
        state.global =
            Some(dh.create_global::<State, ZwpPrimarySelectionDeviceManagerV1, _>(1, ()));
    }
}
impl GlobalDispatch<ZwpPrimarySelectionDeviceManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        native: New<ZwpPrimarySelectionDeviceManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(native, ());
        state.ledger.push(Request::BindPrimary);
    }
}
impl Dispatch<ZwpPrimarySelectionDeviceManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpPrimarySelectionDeviceManagerV1,
        request: zwp_primary_selection_device_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_device_manager_v1::Request::CreateSource { id } => {
                let index = state.primary.sources.len();
                state.primary.sources.push(init.init(id, index));
                state.ledger.push(Request::PrimarySource(index));
            }
            zwp_primary_selection_device_manager_v1::Request::GetDevice { id, seat } => {
                let index = *seat.data::<usize>().unwrap();
                state.primary.devices.push(init.init(id, index));
                state.ledger.push(Request::PrimaryDevice(index));
            }
            zwp_primary_selection_device_manager_v1::Request::Destroy => {
                state.ledger.push(Request::PrimaryDestroyManager)
            }
            other => panic!("unexpected primary manager request: {other:?}"),
        }
    }
}
impl Dispatch<ZwpPrimarySelectionDeviceV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpPrimarySelectionDeviceV1,
        request: zwp_primary_selection_device_v1::Request,
        seat: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_device_v1::Request::SetSelection { source, serial } => {
                state.ledger.push(Request::PrimarySet(
                    source.map(|source| *source.data::<usize>().unwrap()),
                    serial,
                ));
            }
            zwp_primary_selection_device_v1::Request::Destroy => {
                state.ledger.push(Request::PrimaryDestroyDevice(*seat))
            }
            other => panic!("unexpected primary device request: {other:?}"),
        }
    }
}
impl Dispatch<ZwpPrimarySelectionSourceV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpPrimarySelectionSourceV1,
        request: zwp_primary_selection_source_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_source_v1::Request::Offer { mime_type } => {
                state.ledger.push(Request::PrimaryMime(*index, mime_type))
            }
            zwp_primary_selection_source_v1::Request::Destroy => {
                state.ledger.push(Request::PrimaryDestroySource(*index))
            }
            other => panic!("unexpected primary source request: {other:?}"),
        }
    }
}
impl Dispatch<ZwpPrimarySelectionOfferV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpPrimarySelectionOfferV1,
        request: zwp_primary_selection_offer_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_primary_selection_offer_v1::Request::Receive { mime_type, fd } => {
                state
                    .ledger
                    .push(Request::PrimaryReceive(*index, mime_type));
                // Without bytes the write end closes here, so the read sees EOF.
                if let Some(bytes) = &state.primary.offers[*index].bytes {
                    std::fs::File::from(fd).write_all(bytes).unwrap();
                }
            }
            zwp_primary_selection_offer_v1::Request::Destroy => {
                state.ledger.push(Request::PrimaryDestroyOffer(*index))
            }
            other => panic!("unexpected primary offer request: {other:?}"),
        }
    }
}
pub(super) fn emit(event: PrimaryWireEvent, state: &mut State, dh: &DisplayHandle) {
    match event {
        PrimaryWireEvent::Offer(seat, mimes, bytes) => offer(seat, mimes, bytes, state, dh),
        PrimaryWireEvent::SelectionNone(seat) => state.primary.devices[seat].selection(None),
        PrimaryWireEvent::Send(index, mime) => {
            let (input, output) = UnixStream::pair().unwrap();
            input.set_nonblocking(true).unwrap();
            state.primary.sources[index].send(mime, output.as_fd());
            state.primary.sends.push(input);
        }
        PrimaryWireEvent::DrainSend(index) => {
            let mut bytes = Vec::new();
            match state.primary.sends[index].read_to_end(&mut bytes) {
                Ok(_) => {}
                Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock),
            }
            state.ledger.push(Request::PrimaryBytes(bytes));
        }
        PrimaryWireEvent::CancelSource(index) => state.primary.sources[index].cancelled(),
        PrimaryWireEvent::Remove => dh.remove_global::<State>(state.primary.global.take().unwrap()),
        PrimaryWireEvent::Add => {
            state.primary.global =
                Some(dh.create_global::<State, ZwpPrimarySelectionDeviceManagerV1, _>(1, ()));
        }
    }
}
fn offer(
    seat: usize,
    mimes: Vec<String>,
    bytes: Option<Vec<u8>>,
    state: &mut State,
    dh: &DisplayHandle,
) {
    let device = &state.primary.devices[seat];
    let index = state.primary.offers.len();
    let native = device
        .client()
        .unwrap()
        .create_resource::<ZwpPrimarySelectionOfferV1, usize, State>(dh, 1, index)
        .unwrap();
    device.data_offer(&native);
    for mime in mimes {
        native.offer(mime);
    }
    device.selection(Some(&native));
    state.primary.offers.push(Offer {
        _native: native,
        bytes,
    });
}
