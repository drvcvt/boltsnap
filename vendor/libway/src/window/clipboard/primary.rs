//! Primary selection protocol on the shared offer, source and transfer paths.
use super::*;
use wayland_client::{Connection, Dispatch, delegate_noop, event_created_child};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_v1, zwp_primary_selection_offer_v1,
    zwp_primary_selection_source_v1,
};

impl Dispatch<PrimaryDevice, InputSeatId> for WindowState {
    fn event(
        state: &mut Self,
        proxy: &PrimaryDevice,
        event: zwp_primary_selection_device_v1::Event,
        seat: &InputSeatId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active()
            || !state
                .clipboard
                .seats
                .iter()
                .any(|entry| entry.id == *seat && entry.primary.as_ref() == Some(proxy))
        {
            if let zwp_primary_selection_device_v1::Event::DataOffer { offer } = event {
                offer.destroy();
            }
            return;
        }
        let result = match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => state
                .clipboard
                .announce(OfferNative::Primary(offer), *seat, state.connection_id),
            zwp_primary_selection_device_v1::Event::Selection { id } => {
                state.selection_event(*seat, Kind::Primary, id.map(|id| id.id()))
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            state.remember(&error);
        }
    }
    event_created_child!(WindowState, PrimaryDevice, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (PrimaryOffer, ()),
    ]);
}
impl Dispatch<PrimaryOffer, ()> for WindowState {
    fn event(
        state: &mut Self,
        native: &PrimaryOffer,
        event: zwp_primary_selection_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event
            && state.active()
        {
            state.offered(native.id(), mime_type);
        }
    }
}
impl Dispatch<PrimarySource, ClipboardSourceId> for WindowState {
    fn event(
        state: &mut Self,
        _: &PrimarySource,
        event: zwp_primary_selection_source_v1::Event,
        id: &ClipboardSourceId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        match event {
            zwp_primary_selection_source_v1::Event::Send { mime_type, fd } => {
                if let Err(error) = state.clipboard.send(*id, &mime_type, fd) {
                    state.remember(&error);
                }
            }
            zwp_primary_selection_source_v1::Event::Cancelled => {
                state.clipboard.retire_source(*id, &mut state.events)
            }
            _ => {}
        }
    }
}
delegate_noop!(WindowState: ignore PrimaryManager);
