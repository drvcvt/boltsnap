//! A single seat object accumulates exact wire components on the shared queue.
use super::*;
use crate::window::{WindowEvent, state::WindowState};
use crate::{Error, Result};
use wayland_client::{
    Connection, Dispatch, delegate_noop,
    protocol::{wl_registry::WlRegistry, wl_surface::WlSurface},
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
impl TextInput {
    pub(in crate::window) fn bind_manager(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if interface != "zwp_text_input_manager_v3" || version == 0 || self.manager.is_some() {
            return;
        }
        let manager: ZwpTextInputManagerV3 = registry.bind(name, 1, qh, ());
        for seat in &mut self.seats {
            seat.object = Some(manager.get_text_input(&seat.native, qh, seat.id));
            seat.count = 0;
            seat.pending = Pending::default();
            seat.waiting = None;
            seat.synchronized = false;
        }
        self.manager = Some((name, manager));
    }
}
delegate_noop!(WindowState: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);
impl Dispatch<ZwpTextInputV3, InputSeatId> for WindowState {
    fn event(
        state: &mut Self,
        object: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        id: &InputSeatId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        let Some(index) = state
            .text_input
            .seats
            .iter()
            .position(|seat| seat.id == *id && seat.object.as_ref() == Some(object))
        else {
            return;
        };
        if let Err(error) = incoming(state, index, event) {
            state.remember(&error);
        }
    }
}
pub(super) fn incoming(
    state: &mut WindowState,
    index: usize,
    event: zwp_text_input_v3::Event,
) -> Result<()> {
    match event {
        zwp_text_input_v3::Event::Enter { surface } => enter(state, index, &surface),
        zwp_text_input_v3::Event::Leave { surface } => leave(state, index, &surface),
        zwp_text_input_v3::Event::Done { serial } => done(state, index, serial),
        event => {
            let seat = &mut state.text_input.seats[index];
            if seat.activation.is_none() {
                return Ok(());
            }
            match event {
                zwp_text_input_v3::Event::CommitString { text } => {
                    seat.pending.commit(text, &state.text_input.budget);
                    Ok(())
                }
                zwp_text_input_v3::Event::PreeditString {
                    text,
                    cursor_begin,
                    cursor_end,
                } => {
                    seat.pending
                        .preedit(text, cursor_begin, cursor_end, &state.text_input.budget);
                    Ok(())
                }
                zwp_text_input_v3::Event::DeleteSurroundingText {
                    before_length,
                    after_length,
                } => {
                    seat.pending.delete(before_length, after_length);
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }
}
fn window(state: &WindowState, surface: &WlSurface) -> Option<WindowId> {
    state
        .windows
        .iter()
        .find_map(|(id, record)| record.native.matches_surface(surface).then_some(*id))
}
fn enter(state: &mut WindowState, index: usize, surface: &WlSurface) -> Result<()> {
    let Some(window) = window(state, surface) else {
        return Ok(());
    };
    if state.text_input.seats[index].focus.is_some() {
        return Err(Error::InvalidInput("IME Enter before Leave"));
    }
    if !state.event_room(2) {
        return Err(Error::LimitExceeded);
    }
    let seat = &mut state.text_input.seats[index];
    seat.focus = Some(window);
    state.events.push_back(WindowEvent::Ime(ImeEvent::Focus {
        window,
        seat: seat.id,
        entered: true,
    }));
    Ok(())
}
fn leave(state: &mut WindowState, index: usize, surface: &WlSurface) -> Result<()> {
    let Some(window) = window(state, surface) else {
        return Ok(());
    };
    let seat = &mut state.text_input.seats[index];
    if seat.focus != Some(window) {
        return Ok(());
    }
    seat.retire(&mut state.events, true)?;
    seat.focus = None;
    state.events.push_back(WindowEvent::Ime(ImeEvent::Focus {
        window,
        seat: seat.id,
        entered: false,
    }));
    Ok(())
}
fn done(state: &mut WindowState, index: usize, serial: u32) -> Result<()> {
    let seat = &state.text_input.seats[index];
    if serial > seat.count {
        return Err(Error::InvalidInput("future IME Done count"));
    }
    let Some(active) = &seat.activation else {
        return Ok(());
    };
    if serial < active.floor {
        state.text_input.seats[index].pending = Pending::default();
        return Ok(());
    }
    if state.text_input.seats[index].pending.poisoned().is_some() {
        // The batch and its credits are gone; Ended uses the activation's reserved slot.
        state.text_input.seats[index].retire(&mut state.events, true)?;
        return state.ime_backend();
    }
    if !state.event_room(1) {
        return Err(Error::LimitExceeded);
    }
    let seat = &mut state.text_input.seats[index];
    let active = seat
        .activation
        .as_ref()
        .expect("invariant: checked activation");
    let session = active.session;
    let synchronized = serial == seat.count;
    if synchronized && seat.waiting.is_some() {
        seat.count.checked_add(1).ok_or(Error::LimitExceeded)?;
    }
    let transaction = seat.pending.take();
    seat.synchronized = synchronized;
    state.events.push_back(WindowEvent::Ime(ImeEvent::Done {
        window: session.window,
        seat: seat.id,
        session,
        serial,
        synchronized,
        transaction,
    }));
    if synchronized && let Some(waiting) = seat.waiting.take() {
        waiting.send(seat);
    }
    state.ime_backend()
}
