//! Ordinary and primary selections on the owned window queue and its existing seat identities.
mod budget;
mod primary;
mod protocol;
mod read;
mod source;
pub(crate) mod transfer;
mod types;

use super::{
    InputObservation, InputSeatId, KeyboardObservation, WindowEvent, WindowId, state::WindowState,
};
use crate::{Error, Result};
pub(crate) use budget::Payload;
use budget::{Budget, Charge};
use std::{collections::VecDeque, os::fd::BorrowedFd, time::Instant};
pub use types::*;
use wayland_client::{
    Proxy, QueueHandle,
    backend::ObjectId,
    protocol::{wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_seat},
};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1 as PrimaryManager,
    zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1 as PrimaryDevice,
    zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1 as PrimaryOffer,
    zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1 as PrimarySource,
};

/// Which selection a seat slot, offer or source belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Clipboard,
    Primary,
}
#[derive(Default)]
struct Slot {
    offer: Option<ClipboardOfferId>,
    source: Option<ClipboardSourceId>,
}
struct Seat {
    id: InputSeatId,
    native: wl_seat::WlSeat,
    device: Option<wl_data_device::WlDataDevice>,
    primary: Option<PrimaryDevice>,
    focus: Option<WindowId>,
    press: Option<u32>,
    slots: [Slot; 2],
}
impl Seat {
    fn slot(&self, kind: Kind) -> &Slot {
        &self.slots[kind as usize]
    }
    fn slot_mut(&mut self, kind: Kind) -> &mut Slot {
        &mut self.slots[kind as usize]
    }
    fn device(&self, kind: Kind) -> Option<Device> {
        match kind {
            Kind::Clipboard => self.device.clone().map(Device::Clipboard),
            Kind::Primary => self.primary.clone().map(Device::Primary),
        }
    }
}
enum Device {
    Clipboard(wl_data_device::WlDataDevice),
    Primary(PrimaryDevice),
}
#[derive(Clone)]
enum OfferNative {
    Clipboard(wl_data_offer::WlDataOffer),
    Primary(PrimaryOffer),
}
impl OfferNative {
    fn kind(&self) -> Kind {
        match self {
            Self::Clipboard(_) => Kind::Clipboard,
            Self::Primary(_) => Kind::Primary,
        }
    }
    fn id(&self) -> ObjectId {
        match self {
            Self::Clipboard(native) => native.id(),
            Self::Primary(native) => native.id(),
        }
    }
    fn receive(&self, mime: String, fd: BorrowedFd<'_>) {
        match self {
            Self::Clipboard(native) => native.receive(mime, fd),
            Self::Primary(native) => native.receive(mime, fd),
        }
    }
    fn destroy(&self) {
        match self {
            Self::Clipboard(native) => native.destroy(),
            Self::Primary(native) => native.destroy(),
        }
    }
}
enum SourceNative {
    Clipboard(wl_data_source::WlDataSource),
    Primary(PrimarySource),
}
impl SourceNative {
    fn destroy(&self) {
        match self {
            Self::Clipboard(native) => native.destroy(),
            Self::Primary(native) => native.destroy(),
        }
    }
}
struct Offer {
    id: ClipboardOfferId,
    native: OfferNative,
    mimes: Vec<String>,
    seat: InputSeatId,
}
struct Source {
    id: ClipboardSourceId,
    native: SourceNative,
    entries: Vec<(String, Payload)>,
}
struct Read {
    id: ClipboardTransferId,
    window: WindowId,
    seat: InputSeatId,
    _charge: Charge,
}
#[derive(Default)]
pub(crate) struct Clipboard {
    manager: Option<(u32, wl_data_device_manager::WlDataDeviceManager)>,
    primary_manager: Option<(u32, PrimaryManager)>,
    seats: Vec<Seat>,
    offers: Vec<Offer>,
    sources: Vec<Source>,
    reads: Vec<Read>,
    queued: Vec<Charge>,
    budget: Budget,
    pub(crate) transfers: transfer::Transfers,
    next_offer: u64,
    next_source: u64,
    next_transfer: u64,
    manager_live: bool,
    primary_live: bool,
}
impl Clipboard {
    fn identity(counter: &mut u64, host: u64) -> Result<(u64, u64)> {
        let next = counter.checked_add(1).ok_or(Error::LimitExceeded)?;
        *counter = next;
        Ok((host, next))
    }
    pub(super) fn reserved(&self) -> usize {
        self.reads.len() + self.sources.len()
    }
    fn target(&self, window: WindowId, seat: InputSeatId, kind: Kind) -> Result<&Seat> {
        match kind {
            Kind::Clipboard if !self.manager_live => {
                return Err(Error::Unsupported("wl_data_device_manager"));
            }
            Kind::Primary if !self.primary_live => {
                return Err(Error::Unsupported(
                    "zwp_primary_selection_device_manager_v1",
                ));
            }
            _ => {}
        }
        let seat = self
            .seats
            .iter()
            .find(|entry| entry.id == seat)
            .ok_or(Error::InvalidInput("unknown clipboard seat"))?;
        if seat.focus != Some(window) {
            return Err(Error::InvalidInput("clipboard requires keyboard focus"));
        }
        Ok(seat)
    }
    pub(super) fn add_seat(
        &mut self,
        id: InputSeatId,
        native: wl_seat::WlSeat,
        qh: &QueueHandle<WindowState>,
    ) -> Result<()> {
        self.seats
            .try_reserve(1)
            .map_err(|_| Error::LimitExceeded)?;
        let device = self
            .manager
            .as_ref()
            .filter(|_| self.manager_live)
            .map(|(_, manager)| manager.get_data_device(&native, qh, id));
        let primary = self
            .primary_manager
            .as_ref()
            .filter(|_| self.primary_live)
            .map(|(_, manager)| manager.get_device(&native, qh, id));
        self.seats.push(Seat {
            id,
            native,
            device,
            primary,
            focus: None,
            press: None,
            slots: Default::default(),
        });
        Ok(())
    }
    pub(super) fn observe(
        &mut self,
        seat: InputSeatId,
        observation: &InputObservation,
        events: &mut VecDeque<WindowEvent>,
    ) {
        let Some(index) = self.seats.iter().position(|entry| entry.id == seat) else {
            return;
        };
        match observation {
            InputObservation::Keyboard(KeyboardObservation::Enter { window, .. }) => {
                self.blur(seat, events);
                self.seats[index].focus = Some(*window);
                self.seats[index].press = None;
            }
            InputObservation::Keyboard(KeyboardObservation::Leave { .. }) => {
                self.blur(seat, events)
            }
            InputObservation::Keyboard(KeyboardObservation::Key {
                window,
                serial,
                down: true,
                ..
            }) => self.press(seat, *window, *serial),
            InputObservation::Keyboard(KeyboardObservation::Keymap(_)) => {
                self.seats[index].press = None
            }
            _ => {}
        }
    }
    pub(super) fn press(&mut self, seat: InputSeatId, window: WindowId, serial: u32) {
        if let Some(entry) = self
            .seats
            .iter_mut()
            .find(|entry| entry.id == seat && entry.focus == Some(window))
        {
            entry.press = Some(serial);
        }
    }
    pub(super) fn retire_press(&mut self, seat: InputSeatId) {
        if let Some(entry) = self.seats.iter_mut().find(|entry| entry.id == seat) {
            entry.press = None;
        }
    }
    pub(super) fn blur(&mut self, seat: InputSeatId, events: &mut VecDeque<WindowEvent>) {
        if let Some(entry) = self.seats.iter_mut().find(|entry| entry.id == seat) {
            entry.focus = None;
            entry.press = None;
        }
        while let Some(id) = self
            .reads
            .iter()
            .find(|read| read.seat == seat)
            .map(|read| read.id)
        {
            self.cancel(id, events);
        }
    }
    pub(super) fn close_window(&mut self, window: WindowId, events: &mut VecDeque<WindowEvent>) {
        while let Some(seat) = self
            .seats
            .iter()
            .find(|seat| seat.focus == Some(window))
            .map(|seat| seat.id)
        {
            self.blur(seat, events);
        }
    }
    pub(super) fn remove_seat(&mut self, id: InputSeatId, events: &mut VecDeque<WindowEvent>) {
        self.blur(id, events);
        self.transfers.cancel_sends(id);
        if let Some(index) = self.seats.iter().position(|seat| seat.id == id) {
            let seat = self.seats.remove(index);
            for source in seat.slots.iter().filter_map(|slot| slot.source) {
                self.retire_source(source, events);
            }
            if let Some(device) = seat.device.filter(|device| device.version() >= 2) {
                device.release();
            }
            if let Some(primary) = seat.primary {
                primary.destroy();
            }
        }
        self.offers.retain(|offer| {
            if offer.seat == id {
                offer.native.destroy();
                false
            } else {
                true
            }
        });
    }
    pub(super) fn clear(&mut self, events: &mut VecDeque<WindowEvent>) {
        while let Some(id) = self.seats.first().map(|seat| seat.id) {
            self.remove_seat(id, events);
        }
        self.transfers.clear();
        self.manager = None;
        if let Some((_, manager)) = self.primary_manager.take() {
            manager.destroy();
        }
    }
    pub(super) fn drained(&mut self) {
        self.queued.clear();
    }
}
impl WindowState {
    pub(crate) fn clipboard_offer(
        &self,
        window: WindowId,
        seat: InputSeatId,
    ) -> Result<Option<ClipboardOffer>> {
        self.selection_offer(Kind::Clipboard, window, seat)
    }
    pub(crate) fn primary_selection_offer(
        &self,
        window: WindowId,
        seat: InputSeatId,
    ) -> Result<Option<ClipboardOffer>> {
        self.selection_offer(Kind::Primary, window, seat)
    }
    fn selection_offer(
        &self,
        kind: Kind,
        window: WindowId,
        seat: InputSeatId,
    ) -> Result<Option<ClipboardOffer>> {
        self.ensure_active()?;
        self.record(window)?;
        let seat = self.clipboard.target(window, seat, kind)?;
        Ok(seat
            .slot(kind)
            .offer
            .and_then(|id| self.clipboard.offers.iter().find(|offer| offer.id == id))
            .map(|offer| ClipboardOffer {
                id: offer.id,
                mimes: offer.mimes.clone(),
            }))
    }
}
