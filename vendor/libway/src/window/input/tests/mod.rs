//! Actual wire observations with an acknowledged isolated peer and no sleeps.
mod keyboard;
mod lifecycle;
mod pointer;
use super::peer::InputWireEvent as Wire;
use super::*;
use std::{
    fs::File,
    io::Write,
    os::fd::{FromRawFd, OwnedFd},
};

fn fixture(version: u32) -> (Fixture, InputSeatId) {
    let mut fixture = Fixture::with_options(PeerOptions {
        seat_count: 1,
        seat_version: version,
        ..PeerOptions::default()
    });
    fixture.sync();
    fixture.sync();
    fixture.host.events();
    // Obtain provenance from a real retained observation, not forged public IDs.
    fixture
        .peer
        .emit(PeerEvent::Input(Wire::Capabilities(0, 3)));
    fixture.sync();
    let seat = observations(&mut fixture).pop().unwrap().seat;
    fixture.peer.take_requests();
    (fixture, seat)
}
fn observations(fixture: &mut Fixture) -> Vec<InputEvent> {
    fixture
        .host
        .events()
        .into_iter()
        .filter_map(|event| match event {
            WindowEvent::Input(event) => Some(event),
            _ => None,
        })
        .collect()
}
fn emit(fixture: &mut Fixture, event: Wire) {
    fixture.peer.emit(PeerEvent::Input(event));
    fixture.sync();
}
fn keymap_fd(bytes: &[u8]) -> OwnedFd {
    // SAFETY: constant name is NUL-terminated; success creates one unique owned fd.
    let raw = unsafe { libc::memfd_create(c"libway-input-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(raw >= 0);
    // SAFETY: successful memfd_create transfers the sole descriptor into this File.
    let mut file = unsafe { File::from_raw_fd(raw) };
    file.write_all(bytes).unwrap();
    file.into()
}
fn focus(fixture: &mut Fixture) -> WindowId {
    let id = fixture.create(101, 101);
    fixture.host.events();
    emit(fixture, Wire::KeyboardEnter(0, 0, 10, vec![]));
    emit(fixture, Wire::PointerEnter(0, 0, 11, 3.5, 4.25));
    emit(fixture, Wire::Frame(0));
    fixture.host.events();
    fixture.peer.take_requests();
    id
}
fn cancelled(
    seat: InputSeatId,
    window: WindowId,
    device: InputDevice,
    reason: InputCancelReason,
) -> InputEvent {
    InputEvent {
        seat,
        observation: InputObservation::Cancelled {
            window,
            device,
            reason,
        },
    }
}
