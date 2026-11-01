//! All outbound validation and allocation precede producer replacement or requests.
use super::*;
use crate::{Error, Result};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ChangeCause, ContentHint, ContentPurpose,
};
pub(super) struct State {
    surrounding: Option<(String, i32, i32)>,
    caret: ImeRectangle,
    cause: ImeCause,
}
impl State {
    fn prepare(state: ImeState<'_>) -> Result<Self> {
        let caret = state.caret;
        if caret.width < 0
            || caret.height < 0
            || caret.x.checked_add(caret.width).is_none()
            || caret.y.checked_add(caret.height).is_none()
        {
            return Err(Error::InvalidInput("invalid IME rectangle"));
        }
        let surrounding = state
            .surrounding
            .map(|value| {
                if value.text.len() > 4000 {
                    return Err(Error::LimitExceeded);
                }
                if value.text.contains('\0')
                    || !value.text.is_char_boundary(value.cursor)
                    || !value.text.is_char_boundary(value.anchor)
                {
                    return Err(Error::InvalidInput("invalid IME surrounding offsets"));
                }
                let mut text = String::new();
                text.try_reserve_exact(value.text.len())
                    .map_err(|_| Error::LimitExceeded)?;
                text.push_str(value.text);
                Ok((text, value.cursor as i32, value.anchor as i32))
            })
            .transpose()?;
        Ok(Self {
            surrounding,
            caret,
            cause: state.cause,
        })
    }
    pub(super) fn send(self, seat: &mut Seat) {
        let object = seat
            .object
            .as_ref()
            .expect("invariant: admitted text-input object");
        if let Some((text, cursor, anchor)) = self.surrounding {
            object.set_surrounding_text(text, cursor, anchor);
        }
        object.set_text_change_cause(match self.cause {
            ImeCause::InputMethod => ChangeCause::InputMethod,
            ImeCause::Other => ChangeCause::Other,
        });
        object.set_content_type(ContentHint::None, ContentPurpose::Normal);
        object.set_cursor_rectangle(
            self.caret.x,
            self.caret.y,
            self.caret.width,
            self.caret.height,
        );
        object.commit();
        seat.count += 1; // Complete admission checked the successor before any requests.
    }
}
impl WindowState {
    fn ime_configuration(&self, window: WindowId, state: ImeState<'_>) -> Result<()> {
        self.ensure_active()?;
        let configuration = self.record(window)?.accepted.ok_or(Error::NotReady)?;
        if configuration.revision != state.configuration {
            return Err(Error::InvalidInput("stale IME window configuration"));
        }
        Ok(())
    }
    fn ime_enable_admission(
        &self,
        window: WindowId,
        id: InputSeatId,
        state: ImeState<'_>,
    ) -> Result<u64> {
        self.ime_configuration(window, state)?;
        if self.text_input.manager.is_none() {
            return Err(Error::Unsupported("zwp_text_input_manager_v3"));
        }
        let seat = self
            .text_input
            .seats
            .iter()
            .find(|seat| seat.id == id)
            .ok_or(Error::InvalidInput("unknown IME seat"))?;
        if seat.keyboard != Some(window) || seat.focus != Some(window) {
            return Err(Error::NotReady);
        }
        let next = self
            .text_input
            .next_session
            .checked_add(1)
            .ok_or(Error::LimitExceeded)?;
        seat.count
            .checked_add(1 + u32::from(seat.activation.is_some()))
            .ok_or(Error::LimitExceeded)?;
        if !self.event_room(1) {
            return Err(Error::LimitExceeded);
        }
        Ok(next)
    }
    pub(crate) fn enable_ime(
        &mut self,
        window: WindowId,
        id: InputSeatId,
        state: ImeState<'_>,
    ) -> Result<ImeSession> {
        let next = self.ime_enable_admission(window, id, state)?;
        let prepared = State::prepare(state)?;
        let session = ImeSession {
            window,
            seat: id,
            sequence: next,
        };
        let seat = self
            .text_input
            .seats
            .iter_mut()
            .find(|seat| seat.id == id)
            .expect("invariant: validated seat");
        seat.retire(&mut self.events, true)?;
        seat.object
            .as_ref()
            .expect("invariant: manager bound seat")
            .enable();
        let surrounding = prepared.surrounding.is_some();
        prepared.send(seat);
        seat.activation = Some(Activation {
            session,
            floor: seat.count,
            surrounding,
        });
        seat.synchronized = true;
        self.text_input.next_session = next;
        self.ime_backend()?;
        Ok(session)
    }
    pub(crate) fn update_ime(&mut self, session: ImeSession, state: ImeState<'_>) -> Result<()> {
        self.ime_configuration(session.window, state)?;
        let seat = self
            .text_input
            .seats
            .iter_mut()
            .find(|seat| seat.id == session.seat)
            .ok_or(Error::InvalidInput("unknown IME seat"))?;
        let activation = seat
            .activation
            .as_ref()
            .filter(|active| active.session == session)
            .ok_or(Error::InvalidInput("retired IME association"))?;
        if activation.surrounding != state.surrounding.is_some() {
            return Err(Error::InvalidInput(
                "IME surrounding capability requires new activation",
            ));
        }
        seat.count.checked_add(1).ok_or(Error::LimitExceeded)?;
        let prepared = State::prepare(state)?;
        if seat.synchronized {
            prepared.send(seat);
        } else {
            seat.waiting = Some(prepared);
        }
        self.ime_backend()
    }
    pub(crate) fn disable_ime(&mut self, session: ImeSession) -> Result<bool> {
        self.ensure_active()?;
        if session.seat.connection != self.connection_id
            || session.sequence == 0
            || session.sequence > self.text_input.next_session
        {
            return Err(Error::InvalidInput("unknown IME association"));
        }
        let Some(seat) = self
            .text_input
            .seats
            .iter_mut()
            .find(|seat| seat.id == session.seat)
        else {
            return Ok(false);
        };
        if seat
            .activation
            .as_ref()
            .is_none_or(|active| active.session != session)
        {
            return Ok(false);
        }
        seat.retire(&mut self.events, true)?;
        self.ime_backend()?;
        Ok(true)
    }
    pub(super) fn ime_backend(&mut self) -> Result<()> {
        if let Some(error) = self.connection.backend().last_error() {
            let error = Error::Wayland(error.to_string());
            self.remember(&error);
            return Err(error);
        }
        Ok(())
    }
}
