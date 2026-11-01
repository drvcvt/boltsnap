//! Bounded seat ownership. Dispatch stores facts, never invokes application code.
mod cancel;
mod cursor;
mod keyboard;
mod payload;
mod pointer;
mod seat;
mod types;

use crate::window::{WindowId, state::WindowState};
use crate::{Error, Result};
pub use cursor::CursorShape;
use payload::{Budget, Charge};
use std::collections::BTreeMap;
pub use types::*;
use wayland_client::{
    Proxy,
    protocol::{
        wl_keyboard::WlKeyboard, wl_pointer::WlPointer, wl_seat::WlSeat, wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1;

const MAX_SEATS: usize = 8;
const MAX_CHILDREN: usize = 64;
const MAX_FRAME_RECORDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::window) struct SeatToken {
    name: u32,
    id: InputSeatId,
}

struct StagedPointer {
    records: Vec<PointerObservation>,
    charge: Charge,
}
impl StagedPointer {
    fn new(budget: &Budget) -> Self {
        Self {
            records: Vec::new(),
            charge: budget.reserve(0).expect("zero charge"),
        }
    }
}
struct Seat {
    token: SeatToken,
    proxy: WlSeat,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    /// Shape device of `pointer`, destroyed before the pointer is released.
    device: Option<WpCursorShapeDeviceV1>,
    keyboard_focus: Option<WindowId>,
    pointer_focus: Option<WindowId>,
    /// Serial of the enter that gave `pointer_focus`, for cursor requests.
    enter: Option<u32>,
    /// Latest pointer button press with its window, for serial-gated requests.
    press: Option<(WindowId, u32)>,
    staged: StagedPointer,
}
enum RetiredChild {
    Keyboard { _proxy: WlKeyboard },
    Pointer { _proxy: WlPointer },
}

pub(in crate::window) struct InputSeats {
    seats: BTreeMap<u32, Seat>,
    retired_seats: Vec<WlSeat>,
    retired_children: Vec<RetiredChild>,
    next_seat: u64,
    children: usize,
    keymaps: Budget,
    held: Budget,
    names: Budget,
    pointer_records: Budget,
    cursor: cursor::Cursors,
}
impl Default for InputSeats {
    fn default() -> Self {
        Self {
            seats: BTreeMap::new(),
            retired_seats: Vec::new(),
            retired_children: Vec::new(),
            next_seat: 1,
            children: 0,
            keymaps: Budget::new(8 * 1024 * 1024),
            held: Budget::new(8192),
            names: Budget::new(2048),
            pointer_records: Budget::new(4096),
            cursor: Default::default(),
        }
    }
}
impl InputSeats {
    fn keyboard_live(&self, token: SeatToken, proxy: &WlKeyboard) -> bool {
        self.seats
            .get(&token.name)
            .is_some_and(|seat| seat.token == token && seat.keyboard.as_ref() == Some(proxy))
    }
    fn pointer_live(&self, token: SeatToken, proxy: &WlPointer) -> bool {
        self.seats
            .get(&token.name)
            .is_some_and(|seat| seat.token == token && seat.pointer.as_ref() == Some(proxy))
    }
    fn release_keyboard(&mut self, proxy: WlKeyboard) {
        if proxy.version() >= 3 {
            proxy.release();
            self.children -= 1;
        } else {
            self.retired_children
                .push(RetiredChild::Keyboard { _proxy: proxy });
        }
    }
    /// The shape device, if any, is destroyed first.
    fn release_pointer(&mut self, proxy: WlPointer, device: Option<WpCursorShapeDeviceV1>) {
        if let Some(device) = device {
            device.destroy();
        }
        if proxy.version() >= 3 {
            proxy.release();
            self.children -= 1;
        } else {
            self.retired_children
                .push(RetiredChild::Pointer { _proxy: proxy });
        }
    }
    pub(in crate::window) fn clear(&mut self) {
        let seats = std::mem::take(&mut self.seats);
        for (_, seat) in seats {
            if let Some(keyboard) = seat.keyboard {
                self.release_keyboard(keyboard);
            }
            if let Some(pointer) = seat.pointer {
                self.release_pointer(pointer, seat.device);
            }
            if seat.proxy.version() >= 5 {
                seat.proxy.release();
            } else {
                self.retired_seats.push(seat.proxy);
            }
        }
        self.cursor.forget(None);
        self.cursor.destroy();
        // Legacy proxies stay charged until this connection owner's destruction.
    }
}

impl WindowState {
    fn input_active(&self) -> bool {
        if !self.active() {
            return false;
        }
        #[cfg(feature = "owned-native-system")]
        if self.native_attempt.is_some() {
            return false;
        }
        true
    }
    fn input_window(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find_map(|(id, record)| record.native.matches_surface(surface).then_some(*id))
    }
    fn input_event(&mut self, seat: InputSeatId, observation: InputObservation) -> bool {
        if let Err(error) = self
            .text_input
            .observe(seat, &observation, &mut self.events)
        {
            self.remember(&error);
            return false;
        }
        self.clipboard.observe(seat, &observation, &mut self.events);
        self.push(crate::window::WindowEvent::Input(InputEvent {
            seat,
            observation,
        }))
    }
    fn input_room(&mut self, count: usize, extra: usize) -> Result<()> {
        if !self.event_room(count + extra) {
            self.remember(&Error::LimitExceeded);
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
}

pub(in crate::window) use seat::{bind_seat, remove_seat};
