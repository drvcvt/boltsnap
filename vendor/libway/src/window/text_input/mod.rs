//! Text-input-v3 version one on the existing caller-owned queue.
mod admission;
mod budget;
mod lifecycle;
mod pending;
mod protocol;
mod types;
use crate::window::{InputSeatId, WindowId, state::WindowState};
use budget::Budget;
use pending::Pending;
pub use types::*;
use wayland_client::{QueueHandle, protocol::wl_seat::WlSeat};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3, zwp_text_input_v3::ZwpTextInputV3,
};

struct Seat {
    id: InputSeatId,
    native: WlSeat,
    object: Option<ZwpTextInputV3>,
    keyboard: Option<WindowId>,
    focus: Option<WindowId>,
    activation: Option<Activation>,
    count: u32,
    synchronized: bool,
    waiting: Option<admission::State>,
    pending: Pending,
}
struct Activation {
    session: ImeSession,
    floor: u32,
    surrounding: bool,
}
#[derive(Default)]
pub(in crate::window) struct TextInput {
    manager: Option<(u32, ZwpTextInputManagerV3)>,
    seats: Vec<Seat>,
    budget: Budget,
    next_session: u64,
}
impl TextInput {
    pub(in crate::window) fn add_seat(
        &mut self,
        id: InputSeatId,
        native: WlSeat,
        qh: &QueueHandle<WindowState>,
    ) -> crate::Result<()> {
        self.seats
            .try_reserve(1)
            .map_err(|_| crate::Error::LimitExceeded)?;
        let object = self
            .manager
            .as_ref()
            .map(|(_, manager)| manager.get_text_input(&native, qh, id));
        self.seats.push(Seat {
            id,
            native,
            object,
            keyboard: None,
            focus: None,
            activation: None,
            count: 0,
            synchronized: false,
            waiting: None,
            pending: Pending::default(),
        });
        Ok(())
    }
    pub(in crate::window) fn reserved(&self) -> usize {
        self.seats
            .iter()
            .map(|seat| usize::from(seat.focus.is_some()) + usize::from(seat.activation.is_some()))
            .sum()
    }
}
#[cfg(test)]
impl TextInput {
    pub(in crate::window) fn test_seat(&self) -> InputSeatId {
        self.seats[0].id
    }
    pub(in crate::window) fn test_session(&self) -> Option<ImeSession> {
        self.seats[0]
            .activation
            .as_ref()
            .map(|active| active.session)
    }
    pub(in crate::window) fn test_session_or_none(&self) -> Option<ImeSession> {
        self.seats
            .first()
            .and_then(|seat| seat.activation.as_ref().map(|active| active.session))
    }
    pub(in crate::window) fn test_count(&mut self, count: u32) {
        self.seats[0].count = count;
    }
    pub(in crate::window) fn test_used(&self) -> usize {
        self.budget.used()
    }
}
/// Feeds one decoded event to the production dispatcher without the wire.
#[cfg(test)]
pub(in crate::window) fn test_incoming(
    state: &mut WindowState,
    event: wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::Event,
) -> crate::Result<()> {
    protocol::incoming(state, 0, event)
}
