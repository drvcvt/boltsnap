//! Revalidate the seat, serial and origin after eager preparation, before replacing a drag.
use super::{Engine, Request, SeatId};
use crate::{Error, Result};
use wayland_client::{
    Proxy,
    protocol::{wl_data_device, wl_surface},
};

pub(super) struct Origin {
    pub name: u32,
    pub seat: SeatId,
    pub serial: u32,
    pub device: wl_data_device::WlDataDevice,
    pub surface: wl_surface::WlSurface,
    pub held: Vec<u32>,
}
pub(super) fn resolve(engine: &Engine, request: &Request) -> Result<Origin> {
    let name = match request.seat {
        Some(SeatId(name)) => name,
        None => *engine
            .seats
            .iter()
            .filter(|(_, seat)| request.serial.is_some() || seat.grab_serial.is_some())
            .map(|(name, _)| name)
            .next_back()
            .ok_or(Error::InvalidSerial)?,
    };
    let seat = engine.seats.get(&name).ok_or(Error::InvalidSerial)?;
    let serial = request
        .serial
        .or(seat.grab_serial)
        .ok_or(Error::InvalidSerial)?;
    let device = seat.device.clone().ok_or(Error::NotReady)?;
    let surface = wl_surface::WlSurface::from_id(&engine.conn, request.origin.id().clone())
        .map_err(|_| Error::Wayland("origin surface is gone".into()))?;
    let held = if request.serial.is_none() {
        seat.buttons.clone()
    } else {
        Vec::new()
    };
    Ok(Origin {
        name,
        seat: seat.id,
        serial,
        device,
        surface,
        held,
    })
}
