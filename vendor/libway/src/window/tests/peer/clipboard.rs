//! Isolated ordinary clipboard peer with literal serial, MIME and byte records.
use super::{Options, Request, State};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::net::UnixStream,
    },
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::GlobalId,
    protocol::{wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source},
};

#[derive(Debug)]
pub(in crate::window::tests) enum ClipboardWireEvent {
    Offer(usize, Vec<String>, Option<Vec<u8>>),
    SelectionNone(usize),
    Complete(usize, Vec<u8>),
    Send(usize, String),
    DrainSend(usize),
    CancelSource(usize),
    RemoveManager,
    AddManager,
}
struct Offer {
    _native: wl_data_offer::WlDataOffer,
    bytes: Option<Vec<u8>>,
}
#[derive(Default)]
pub(in crate::window::tests) struct ClipboardState {
    global: Option<GlobalId>,
    devices: Vec<wl_data_device::WlDataDevice>,
    offers: Vec<Offer>,
    sources: Vec<wl_data_source::WlDataSource>,
    reads: Vec<Option<OwnedFd>>,
    sends: Vec<UnixStream>,
}
pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut ClipboardState) {
    if options.clipboard {
        state.global = Some(
            dh.create_global::<State, wl_data_device_manager::WlDataDeviceManager, _>(
                options.clipboard_version,
                (),
            ),
        );
    }
}
impl GlobalDispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        native: New<wl_data_device_manager::WlDataDeviceManager>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(native, ());
        state.ledger.push(Request::BindClipboard);
    }
}
impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_data_device_manager::WlDataDeviceManager,
        request: wl_data_device_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_device_manager::Request::CreateDataSource { id } => {
                let index = state.clipboard.sources.len();
                state.clipboard.sources.push(init.init(id, index));
                state.ledger.push(Request::ClipboardSource(index));
            }
            wl_data_device_manager::Request::GetDataDevice { id, seat } => {
                let index = *seat.data::<usize>().unwrap();
                state.clipboard.devices.push(init.init(id, index));
                state.ledger.push(Request::ClipboardDevice(index));
            }
            other => panic!("unexpected clipboard manager request: {other:?}"),
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_data_device::WlDataDevice,
        request: wl_data_device::Request,
        seat: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_device::Request::SetSelection { source, serial } => {
                state.ledger.push(Request::ClipboardSet(
                    source.map(|source| *source.data::<usize>().unwrap()),
                    serial,
                ));
            }
            wl_data_device::Request::Release => {
                state.ledger.push(Request::ClipboardReleaseDevice(*seat))
            }
            other => panic!("unexpected clipboard device request: {other:?}"),
        }
    }
}
impl Dispatch<wl_data_source::WlDataSource, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_data_source::WlDataSource,
        request: wl_data_source::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_source::Request::Offer { mime_type } => {
                state.ledger.push(Request::ClipboardMime(*index, mime_type))
            }
            wl_data_source::Request::Destroy => {
                state.ledger.push(Request::ClipboardDestroySource(*index))
            }
            other => panic!("unexpected clipboard source request: {other:?}"),
        }
    }
}
impl Dispatch<wl_data_offer::WlDataOffer, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_data_offer::WlDataOffer,
        request: wl_data_offer::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_data_offer::Request::Receive { mime_type, fd } => {
                state
                    .ledger
                    .push(Request::ClipboardReceive(*index, mime_type));
                if let Some(bytes) = &state.clipboard.offers[*index].bytes {
                    std::fs::File::from(fd).write_all(bytes).unwrap();
                } else {
                    state.clipboard.reads.push(Some(fd));
                }
            }
            wl_data_offer::Request::Destroy => {
                state.ledger.push(Request::ClipboardDestroyOffer(*index))
            }
            other => panic!("unexpected clipboard offer request: {other:?}"),
        }
    }
}
pub(super) fn emit(event: ClipboardWireEvent, state: &mut State, dh: &DisplayHandle) {
    match event {
        ClipboardWireEvent::Offer(seat, mimes, bytes) => offer(seat, mimes, bytes, state, dh),
        ClipboardWireEvent::SelectionNone(seat) => state.clipboard.devices[seat].selection(None),
        ClipboardWireEvent::Complete(index, bytes) => {
            let fd = state.clipboard.reads[index].take().unwrap();
            std::fs::File::from(fd).write_all(&bytes).unwrap();
        }
        ClipboardWireEvent::Send(index, mime) => {
            let (input, output) = UnixStream::pair().unwrap();
            input.set_nonblocking(true).unwrap();
            state.clipboard.sources[index].send(mime, output.as_fd());
            state.clipboard.sends.push(input);
        }
        ClipboardWireEvent::DrainSend(index) => {
            let mut bytes = Vec::new();
            match state.clipboard.sends[index].read_to_end(&mut bytes) {
                Ok(_) => {}
                Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock),
            }
            state.ledger.push(Request::ClipboardBytes(bytes));
        }
        ClipboardWireEvent::CancelSource(index) => state.clipboard.sources[index].cancelled(),
        ClipboardWireEvent::RemoveManager => {
            dh.remove_global::<State>(state.clipboard.global.take().unwrap())
        }
        ClipboardWireEvent::AddManager => {
            state.clipboard.global = Some(
                dh.create_global::<State, wl_data_device_manager::WlDataDeviceManager, _>(3, ()),
            );
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
    let device = &state.clipboard.devices[seat];
    let index = state.clipboard.offers.len();
    let native = device
        .client()
        .unwrap()
        .create_resource::<wl_data_offer::WlDataOffer, usize, State>(dh, device.version(), index)
        .unwrap();
    device.data_offer(&native);
    for mime in mimes {
        native.offer(mime);
    }
    device.selection(Some(&native));
    state.clipboard.offers.push(Offer {
        _native: native,
        bytes,
    });
}
