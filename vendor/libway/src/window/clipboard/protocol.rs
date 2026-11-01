//! Ordinary selection protocol and the selection paths both kinds share.
//! Drag offers are destroyed without accepting DnD.
use super::*;
use wayland_client::{
    Connection, Dispatch, delegate_noop, event_created_child, protocol::wl_registry,
};

impl Clipboard {
    pub(in crate::window) fn bind_manager(
        &mut self,
        registry: &wl_registry::WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if version == 0 {
            return;
        }
        match interface {
            "wl_data_device_manager" if self.manager.is_none() => {
                let manager: wl_data_device_manager::WlDataDeviceManager =
                    registry.bind(name, version.min(3), qh, ());
                for seat in &mut self.seats {
                    seat.device = Some(manager.get_data_device(&seat.native, qh, seat.id));
                }
                self.manager = Some((name, manager));
                self.manager_live = true;
            }
            "zwp_primary_selection_device_manager_v1" if self.primary_manager.is_none() => {
                let manager: PrimaryManager = registry.bind(name, 1, qh, ());
                for seat in &mut self.seats {
                    seat.primary = Some(manager.get_device(&seat.native, qh, seat.id));
                }
                self.primary_manager = Some((name, manager));
                self.primary_live = true;
            }
            _ => {}
        }
    }
    pub(in crate::window) fn remove_manager(&mut self, name: u32) {
        if self
            .manager
            .as_ref()
            .is_some_and(|(bound, _)| *bound == name)
        {
            self.manager_live = false;
        }
        if self
            .primary_manager
            .as_ref()
            .is_some_and(|(bound, _)| *bound == name)
        {
            self.primary_live = false;
        }
    }
    pub(super) fn announce(
        &mut self,
        native: OfferNative,
        seat: InputSeatId,
        host: u64,
    ) -> Result<()> {
        if self.offers.len() == 64 {
            native.destroy();
            return Err(Error::LimitExceeded);
        }
        let admitted = self
            .offers
            .try_reserve(1)
            .map_err(|_| Error::LimitExceeded)
            .and_then(|()| Self::identity(&mut self.next_offer, host));
        let (host, sequence) = match admitted {
            Ok(identity) => identity,
            Err(error) => {
                native.destroy();
                return Err(error);
            }
        };
        self.offers.push(Offer {
            id: ClipboardOfferId(host, sequence),
            native,
            mimes: Vec::new(),
            seat,
        });
        Ok(())
    }
    fn discard(&mut self, native: ObjectId) {
        if let Some(index) = self
            .offers
            .iter()
            .position(|offer| offer.native.id() == native)
        {
            self.offers.remove(index).native.destroy();
        }
    }
    fn selection(
        &mut self,
        seat: InputSeatId,
        kind: Kind,
        native: Option<ObjectId>,
    ) -> Result<ClipboardEvent> {
        let index = self
            .seats
            .iter()
            .position(|entry| entry.id == seat)
            .ok_or(Error::InvalidInput("unknown selection seat"))?;
        let offer = match native {
            Some(native) => {
                let offer = self
                    .offers
                    .iter()
                    .find(|offer| offer.native.id() == native && offer.seat == seat)
                    .ok_or(Error::InvalidInput("unannounced selection offer"))?;
                Some(ClipboardOffer {
                    id: offer.id,
                    mimes: offer.mimes.clone(),
                })
            }
            None => None,
        };
        let slot = self.seats[index].slot_mut(kind);
        let old = slot.offer;
        slot.offer = offer.as_ref().map(|offer| offer.id);
        let current = slot.offer;
        if let Some(old) = old.filter(|old| Some(*old) != current)
            && let Some(index) = self.offers.iter().position(|offer| offer.id == old)
        {
            self.offers.remove(index).native.destroy();
        }
        Ok(match kind {
            Kind::Clipboard => ClipboardEvent::Selection { seat, offer },
            Kind::Primary => ClipboardEvent::PrimarySelection { seat, offer },
        })
    }
}
impl WindowState {
    /// Replace a seat's current offer of `kind` and publish it into a checked slot.
    pub(super) fn selection_event(
        &mut self,
        seat: InputSeatId,
        kind: Kind,
        native: Option<ObjectId>,
    ) -> Result<()> {
        if !self.event_room(1) {
            return Err(Error::LimitExceeded);
        }
        let event = self.clipboard.selection(seat, kind, native)?;
        self.events.push_back(WindowEvent::Clipboard(event));
        Ok(())
    }
    /// Record one advertised MIME name on an announced offer of either kind.
    pub(super) fn offered(&mut self, native: ObjectId, mime_type: String) {
        let Some(offer) = self
            .clipboard
            .offers
            .iter_mut()
            .find(|offer| offer.native.id() == native)
        else {
            return;
        };
        if mime_type.is_empty()
            || mime_type.len() > 256
            || mime_type.contains('\0')
            || offer.mimes.len() == 32
        {
            self.remember(&Error::LimitExceeded);
        } else if !offer.mimes.contains(&mime_type) {
            match offer.mimes.try_reserve(1) {
                Ok(()) => offer.mimes.push(mime_type),
                Err(_) => self.remember(&Error::LimitExceeded),
            }
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, InputSeatId> for WindowState {
    fn event(
        state: &mut Self,
        proxy: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        seat: &InputSeatId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active()
            || !state
                .clipboard
                .seats
                .iter()
                .any(|entry| entry.id == *seat && entry.device.as_ref() == Some(proxy))
        {
            if let wl_data_device::Event::DataOffer { id } = event {
                id.destroy();
            }
            return;
        }
        let result = match event {
            wl_data_device::Event::DataOffer { id } => {
                state
                    .clipboard
                    .announce(OfferNative::Clipboard(id), *seat, state.connection_id)
            }
            wl_data_device::Event::Selection { id } => {
                state.selection_event(*seat, Kind::Clipboard, id.map(|id| id.id()))
            }
            wl_data_device::Event::Enter { id: Some(id), .. } => {
                state.clipboard.discard(id.id());
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            state.remember(&error);
        }
    }
    event_created_child!(WindowState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}
impl Dispatch<wl_data_offer::WlDataOffer, ()> for WindowState {
    fn event(
        state: &mut Self,
        native: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && state.active()
        {
            state.offered(native.id(), mime_type);
        }
    }
}
impl Dispatch<wl_data_source::WlDataSource, ClipboardSourceId> for WindowState {
    fn event(
        state: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        id: &ClipboardSourceId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                if let Err(error) = state.clipboard.send(*id, &mime_type, fd) {
                    state.remember(&error);
                }
            }
            wl_data_source::Event::Cancelled => {
                state.clipboard.retire_source(*id, &mut state.events)
            }
            _ => {}
        }
    }
}
delegate_noop!(WindowState: ignore wl_data_device_manager::WlDataDeviceManager);
