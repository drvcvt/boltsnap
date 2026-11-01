//! Frame-ordered pointer observations, without cross-boundary motion coalescing.
use super::*;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, protocol::wl_pointer};

impl Dispatch<wl_pointer::WlPointer, SeatToken> for WindowState {
    fn event(
        state: &mut Self,
        proxy: &WlPointer,
        event: wl_pointer::Event,
        token: &SeatToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.input_active() || !state.input.pointer_live(*token, proxy) {
            return;
        }
        if let Err(error) = pointer_event(state, *token, proxy.version(), event) {
            state.remember(&error);
        }
    }
}
fn pointer_event(
    state: &mut WindowState,
    token: SeatToken,
    version: u32,
    event: wl_pointer::Event,
) -> Result<()> {
    match event {
        wl_pointer::Event::Frame => {
            flush(state, token, false);
            return Ok(());
        }
        wl_pointer::Event::Enter {
            serial,
            surface,
            surface_x,
            surface_y,
        } => pointer_enter(state, token, serial, &surface, (surface_x, surface_y))?,
        wl_pointer::Event::Leave { serial, surface } => {
            pointer_leave(state, token, serial, &surface)?
        }
        other => {
            let Some(window) = state.input.seats[&token.name].pointer_focus else {
                return Ok(());
            };
            if let Some(observation) = focused(window, other)? {
                stage(state, token, observation, 0)?;
            }
        }
    }
    if version < 5 {
        flush(state, token, true);
    }
    Ok(())
}
fn stage(
    state: &mut WindowState,
    token: SeatToken,
    record: PointerObservation,
    new_focus: usize,
) -> Result<()> {
    let seat = &state.input.seats[&token.name];
    if seat.staged.records.len() >= MAX_FRAME_RECORDS {
        return Err(Error::LimitExceeded);
    }
    state.input_room(0, new_focus + usize::from(seat.staged.records.is_empty()))?;
    let root = match &record {
        PointerObservation::Button {
            window, down: true, ..
        } => Some(state.tree_root(*window)),
        _ => None,
    };
    let seat = state.input.seats.get_mut(&token.name).expect("live seat");
    seat.staged
        .records
        .try_reserve(1)
        .map_err(|_| Error::LimitExceeded)?;
    seat.staged.charge.grow()?;
    if let PointerObservation::Button {
        window,
        serial,
        down: true,
        ..
    } = &record
    {
        state.clipboard.press(token.id, *window, *serial);
        seat.press = Some((*window, *serial));
        seat.latest_input = root.map(|root| LatestInput {
            root,
            serial: *serial,
            device: InputDevice::Pointer,
        });
    }
    seat.staged.records.push(record);
    Ok(())
}
impl InputSeats {
    /// The seat's latest button press on `window`, with its native seat. A seat
    /// without a pointer, or whose latest press targeted another window, has none.
    pub(in crate::window) fn press(
        &self,
        seat: InputSeatId,
        window: WindowId,
    ) -> Result<(WlSeat, u32)> {
        let entry = self
            .seats
            .values()
            .find(|entry| entry.token.id == seat)
            .ok_or(Error::InvalidInput("unknown input seat"))?;
        match entry.press {
            Some((target, serial)) if target == window => Ok((entry.proxy.clone(), serial)),
            _ => Err(Error::NotReady),
        }
    }

    /// The seat's latest pointer or key press inside the popup tree rooted at
    /// `root`, with its native seat (R6 D3). Unknown seat: `InvalidInput`; no
    /// such press: `NotReady`.
    pub(in crate::window) fn grab_serial(
        &self,
        seat: InputSeatId,
        root: WindowId,
    ) -> Result<(WlSeat, u32)> {
        let entry = self
            .seats
            .values()
            .find(|entry| entry.token.id == seat)
            .ok_or(Error::InvalidInput("unknown input seat"))?;
        match entry.latest_input {
            Some(input) if input.root == root => Ok((entry.proxy.clone(), input.serial)),
            _ => Err(Error::NotReady),
        }
    }
}
fn flush(state: &mut WindowState, token: SeatToken, legacy: bool) {
    let seat = state.input.seats.get_mut(&token.name).expect("live seat");
    if let Some(event) = cancel::take_frame(seat, &state.input.pointer_records, legacy, false) {
        // The first staged record reserved this frame's event slot before mutation.
        state
            .events
            .push_back(crate::window::WindowEvent::Input(event));
    }
}
fn axis(value: WEnum<wl_pointer::Axis>) -> Result<PointerAxis> {
    match value {
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => Ok(PointerAxis::Vertical),
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Ok(PointerAxis::Horizontal),
        _ => Err(Error::InvalidInput("invalid pointer axis")),
    }
}
fn focused(window: WindowId, event: wl_pointer::Event) -> Result<Option<PointerObservation>> {
    let record = match event {
        wl_pointer::Event::Motion {
            time,
            surface_x,
            surface_y,
        } => PointerObservation::Motion {
            window,
            time_ms: time,
            x: surface_x,
            y: surface_y,
        },
        wl_pointer::Event::Button {
            serial,
            time,
            button,
            state,
        } => {
            let down = match state {
                WEnum::Value(wl_pointer::ButtonState::Pressed) => true,
                WEnum::Value(wl_pointer::ButtonState::Released) => false,
                _ => return Err(Error::InvalidInput("invalid pointer button state")),
            };
            PointerObservation::Button {
                window,
                serial,
                time_ms: time,
                button,
                down,
            }
        }
        other => return focused_axis(window, other),
    };
    Ok(Some(record))
}
fn source(value: WEnum<wl_pointer::AxisSource>) -> AxisSource {
    match value {
        WEnum::Value(wl_pointer::AxisSource::Wheel) => AxisSource::Wheel,
        WEnum::Value(wl_pointer::AxisSource::Finger) => AxisSource::Finger,
        WEnum::Value(wl_pointer::AxisSource::Continuous) => AxisSource::Continuous,
        WEnum::Value(wl_pointer::AxisSource::WheelTilt) => AxisSource::WheelTilt,
        other => AxisSource::Unknown(other.into()),
    }
}
fn direction(value: WEnum<wl_pointer::AxisRelativeDirection>) -> AxisDirection {
    match value {
        WEnum::Value(wl_pointer::AxisRelativeDirection::Identical) => AxisDirection::Identical,
        WEnum::Value(wl_pointer::AxisRelativeDirection::Inverted) => AxisDirection::Inverted,
        other => AxisDirection::Unknown(other.into()),
    }
}

fn pointer_enter(
    state: &mut WindowState,
    token: SeatToken,
    serial: u32,
    surface: &WlSurface,
    position: (f64, f64),
) -> Result<()> {
    let Some(window) = state.input_window(surface) else {
        return Ok(());
    };
    if state.input.seats[&token.name].pointer_focus.is_some() {
        return Err(Error::InvalidInput("pointer Enter before Leave"));
    }
    stage(
        state,
        token,
        PointerObservation::Enter {
            window,
            serial,
            x: position.0,
            y: position.1,
        },
        1,
    )?;
    let seat = state.input.seats.get_mut(&token.name).expect("live seat");
    seat.pointer_focus = Some(window);
    seat.enter = Some(serial);
    state.input.apply_cursor(token.name);
    Ok(())
}
fn pointer_leave(
    state: &mut WindowState,
    token: SeatToken,
    serial: u32,
    surface: &WlSurface,
) -> Result<()> {
    let Some(window) = state.input_window(surface) else {
        return Ok(());
    };
    if state.input.seats[&token.name].pointer_focus != Some(window) {
        return Ok(());
    }
    stage(
        state,
        token,
        PointerObservation::Leave { window, serial },
        0,
    )?;
    let seat = state.input.seats.get_mut(&token.name).expect("live seat");
    seat.pointer_focus = None;
    seat.enter = None;
    Ok(())
}

fn focused_axis(window: WindowId, event: wl_pointer::Event) -> Result<Option<PointerObservation>> {
    Ok(Some(match event {
        wl_pointer::Event::Axis {
            time,
            axis: value,
            value: distance,
        } => PointerObservation::Axis {
            window,
            time_ms: time,
            axis: axis(value)?,
            value: distance,
        },
        wl_pointer::Event::AxisStop { time, axis: value } => PointerObservation::AxisStop {
            window,
            time_ms: time,
            axis: axis(value)?,
        },
        other => return axis_units(window, other),
    }))
}
fn axis_units(window: WindowId, event: wl_pointer::Event) -> Result<Option<PointerObservation>> {
    Ok(Some(match event {
        wl_pointer::Event::AxisDiscrete {
            axis: value,
            discrete,
        } => PointerObservation::AxisDiscrete {
            window,
            axis: axis(value)?,
            steps: discrete,
        },
        wl_pointer::Event::AxisValue120 {
            axis: value,
            value120,
        } => PointerObservation::AxisValue120 {
            window,
            axis: axis(value)?,
            value: value120,
        },
        wl_pointer::Event::AxisSource { axis_source } => PointerObservation::AxisSource {
            window,
            source: source(axis_source),
        },
        wl_pointer::Event::AxisRelativeDirection {
            axis: value,
            direction: value_direction,
        } => PointerObservation::AxisRelativeDirection {
            window,
            axis: axis(value)?,
            direction: direction(value_direction),
        },
        _ => return Ok(None),
    }))
}
