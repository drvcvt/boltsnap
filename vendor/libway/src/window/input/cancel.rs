//! Cancellation reservations remain inside the host's 1024-event admission cap.
use super::*;
use crate::window::WindowEvent;
use std::collections::VecDeque;

fn selected(focus: Option<WindowId>, window: Option<WindowId>) -> bool {
    focus.is_some_and(|focus| window.is_none_or(|window| window == focus))
}
fn staged_selected(seat: &Seat, window: Option<WindowId>) -> bool {
    !seat.staged.records.is_empty()
        && window.is_none_or(|window| {
            seat.staged
                .records
                .iter()
                .any(|record| record.window() == window)
        })
}
impl InputSeats {
    /// Worst-case pending frames plus focused devices, protected from ordinary events.
    pub(in crate::window) fn retirement_count(&self, window: Option<WindowId>) -> usize {
        self.seats
            .values()
            .map(|seat| {
                usize::from(selected(seat.keyboard_focus, window))
                    + usize::from(selected(seat.pointer_focus, window))
                    + usize::from(staged_selected(seat, window))
            })
            .sum()
    }
    pub(in crate::window) fn cancel(
        &mut self,
        events: &mut VecDeque<WindowEvent>,
        window: Option<WindowId>,
        reason: InputCancelReason,
    ) {
        self.cursor.forget(window);
        let budget = self.pointer_records.clone();
        for seat in self.seats.values_mut() {
            if staged_selected(seat, window)
                && let Some(event) = take_frame(seat, &budget, false, true)
            {
                events.push_back(WindowEvent::Input(event));
            }
            cancel_focus(seat, events, window, None, reason);
        }
    }
}

pub(super) fn cancel_focus(
    seat: &mut Seat,
    events: &mut VecDeque<WindowEvent>,
    window: Option<WindowId>,
    device: Option<InputDevice>,
    reason: InputCancelReason,
) {
    for (kind, focus) in [
        (InputDevice::Keyboard, &mut seat.keyboard_focus),
        (InputDevice::Pointer, &mut seat.pointer_focus),
    ] {
        if device.is_none_or(|device| device == kind) && selected(*focus, window) {
            let window = focus.take().expect("selected focus exists");
            if kind == InputDevice::Pointer {
                seat.enter = None;
            }
            events.push_back(WindowEvent::Input(InputEvent {
                seat: seat.token.id,
                observation: InputObservation::Cancelled {
                    window,
                    device: kind,
                    reason,
                },
            }));
        }
    }
}

pub(super) fn take_frame(
    seat: &mut Seat,
    budget: &Budget,
    legacy: bool,
    interrupted: bool,
) -> Option<InputEvent> {
    if seat.staged.records.is_empty() {
        return None;
    }
    let staged = std::mem::replace(&mut seat.staged, StagedPointer::new(budget));
    Some(InputEvent {
        seat: seat.token.id,
        observation: InputObservation::PointerFrame(PointerFrame {
            records: payload::Owned::new(staged.records, staged.charge),
            legacy,
            interrupted,
        }),
    })
}
