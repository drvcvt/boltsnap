//! Exact-association retirement uses slots reserved at activation and Enter.
use super::*;
use crate::window::{InputObservation, KeyboardObservation, WindowEvent};
use crate::{Error, Result};
use std::collections::VecDeque;
impl Seat {
    pub(super) fn retire(
        &mut self,
        events: &mut VecDeque<WindowEvent>,
        requests: bool,
    ) -> Result<()> {
        if self.activation.is_some() && requests {
            self.count.checked_add(1).ok_or(Error::LimitExceeded)?;
        }
        if let Some(active) = self.activation.take() {
            if requests {
                let object = self
                    .object
                    .as_ref()
                    .expect("invariant: active text-input object");
                object.disable();
                object.commit();
                self.count += 1;
            }
            events.push_back(WindowEvent::Ime(ImeEvent::Ended {
                window: active.session.window,
                seat: self.id,
                session: active.session,
            }));
        }
        self.pending = Pending::default();
        self.waiting = None;
        self.synchronized = false;
        Ok(())
    }
    fn clear_focus(&mut self, events: &mut VecDeque<WindowEvent>) {
        if let Some(window) = self.focus.take() {
            events.push_back(WindowEvent::Ime(ImeEvent::Focus {
                window,
                seat: self.id,
                entered: false,
            }));
        }
    }
}
impl TextInput {
    pub(in crate::window) fn observe(
        &mut self,
        id: InputSeatId,
        observation: &InputObservation,
        events: &mut VecDeque<WindowEvent>,
    ) -> Result<()> {
        let Some(seat) = self.seats.iter_mut().find(|seat| seat.id == id) else {
            return Ok(());
        };
        match observation {
            InputObservation::Keyboard(KeyboardObservation::Enter { window, .. }) => {
                seat.retire(events, true)?;
                seat.keyboard = Some(*window);
            }
            InputObservation::Keyboard(KeyboardObservation::Keymap(_)) => {
                seat.retire(events, true)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::window) fn blur(
        &mut self,
        id: InputSeatId,
        events: &mut VecDeque<WindowEvent>,
    ) -> Result<()> {
        if let Some(seat) = self.seats.iter_mut().find(|seat| seat.id == id) {
            seat.retire(events, true)?;
            seat.keyboard = None;
        }
        Ok(())
    }
    pub(in crate::window) fn close_window(
        &mut self,
        window: WindowId,
        events: &mut VecDeque<WindowEvent>,
    ) -> Result<()> {
        // Validate every counter first: closing one window cannot partly retire seats.
        for seat in &self.seats {
            if seat
                .activation
                .as_ref()
                .is_some_and(|active| active.session.window == window)
            {
                seat.count.checked_add(1).ok_or(Error::LimitExceeded)?;
            }
        }
        for seat in &mut self.seats {
            if seat.keyboard == Some(window) || seat.focus == Some(window) {
                seat.retire(events, true)?;
                if seat.keyboard == Some(window) {
                    seat.keyboard = None;
                }
                if seat.focus == Some(window) {
                    seat.clear_focus(events);
                }
            }
        }
        Ok(())
    }
    pub(in crate::window) fn remove_seat(
        &mut self,
        id: InputSeatId,
        events: &mut VecDeque<WindowEvent>,
    ) -> Result<()> {
        if let Some(index) = self.seats.iter().position(|seat| seat.id == id) {
            let seat = &mut self.seats[index];
            seat.retire(events, true)?;
            let mut seat = self.seats.remove(index);
            seat.clear_focus(events);
            if let Some(object) = seat.object {
                object.destroy();
            }
        }
        Ok(())
    }
    pub(in crate::window) fn remove_manager(
        &mut self,
        name: u32,
        events: &mut VecDeque<WindowEvent>,
    ) {
        if self
            .manager
            .as_ref()
            .is_none_or(|(bound, _)| *bound != name)
        {
            return;
        }
        for seat in &mut self.seats {
            // Destroying this object retires its state without another protocol commit.
            seat.retire(events, false)
                .expect("invariant: request-free retirement");
            seat.clear_focus(events);
            if let Some(object) = seat.object.take() {
                object.destroy();
            }
        }
        self.manager
            .take()
            .expect("invariant: checked manager")
            .1
            .destroy();
    }
    pub(in crate::window) fn clear(&mut self, events: &mut VecDeque<WindowEvent>) {
        for mut seat in self.seats.drain(..) {
            seat.retire(events, false)
                .expect("invariant: request-free retirement");
            seat.clear_focus(events);
            if let Some(object) = seat.object {
                object.destroy();
            }
        }
        if let Some((_, manager)) = self.manager.take() {
            manager.destroy();
        }
    }
}
