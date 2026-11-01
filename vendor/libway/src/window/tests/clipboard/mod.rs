//! Authored ordinary clipboard wire traces, never the owner's clipboard.
mod admission;
mod lifecycle;
mod primary;
mod read;
mod source;
use super::peer::{
    ClipboardWireEvent as Wire, InputWireEvent as Input, PrimaryWireEvent as Primary,
};
use super::*;

fn fixture() -> (Fixture, WindowId, InputSeatId) {
    fixture_with(PeerOptions {
        clipboard: true,
        seat_count: 1,
        ..PeerOptions::default()
    })
}
fn fixture_with(options: PeerOptions) -> (Fixture, WindowId, InputSeatId) {
    let mut fixture = Fixture::with_options(options);
    fixture.sync();
    fixture.sync();
    let seat = fixture
        .host
        .events()
        .into_iter()
        .find_map(|event| match event {
            WindowEvent::Input(InputEvent {
                seat,
                observation: InputObservation::SeatAdded { .. },
            }) => Some(seat),
            _ => None,
        });
    // Ready collection may already have drained SeatAdded; Capabilities retains
    // the same public provenance without constructing an ID for the application.
    fixture
        .peer
        .emit(PeerEvent::Input(Input::Capabilities(0, 3)));
    fixture.sync();
    let seat = seat.unwrap_or_else(|| {
        fixture
            .host
            .events()
            .into_iter()
            .find_map(|event| match event {
                WindowEvent::Input(event) => Some(event.seat),
                _ => None,
            })
            .unwrap()
    });
    let window = fixture.create(100, 100);
    native(&mut fixture, Input::KeyboardEnter(0, 0, 10, vec![]));
    fixture.host.events();
    fixture.peer.take_requests();
    (fixture, window, seat)
}
fn native(fixture: &mut Fixture, event: Input) {
    fixture.peer.emit(PeerEvent::Input(event));
    fixture.sync();
}
fn emit(fixture: &mut Fixture, event: Wire) {
    fixture.peer.emit(PeerEvent::Clipboard(event));
    fixture.sync();
}
fn events(fixture: &mut Fixture) -> Vec<ClipboardEvent> {
    fixture
        .host
        .events()
        .into_iter()
        .filter_map(|event| match event {
            WindowEvent::Clipboard(event) => Some(event),
            _ => None,
        })
        .collect()
}
fn offer(
    fixture: &mut Fixture,
    window: WindowId,
    seat: InputSeatId,
    bytes: Option<Vec<u8>>,
) -> ClipboardOffer {
    emit(fixture, Wire::Offer(0, vec!["text/plain".into()], bytes));
    fixture.host.clipboard_offer(window, seat).unwrap().unwrap()
}
fn request(
    window: WindowId,
    seat: InputSeatId,
    offer: ClipboardOfferId,
    limit: usize,
) -> ClipboardRead {
    ClipboardRead {
        window,
        seat,
        offer,
        mime: "text/plain".into(),
        max_bytes: limit,
        deadline: Instant::now() + WATCHDOG,
    }
}
fn received(
    id: ClipboardTransferId,
    window: WindowId,
    seat: InputSeatId,
    outcome: ClipboardReceiveOutcome,
) -> ClipboardEvent {
    ClipboardEvent::Received {
        transfer: id,
        window,
        seat,
        outcome,
    }
}
fn representation(bytes: &[u8]) -> Vec<ClipboardRepresentation> {
    let bytes: Arc<[u8]> = bytes.into();
    vec![
        ClipboardRepresentation {
            mime: "text/plain;charset=utf-8".into(),
            bytes: bytes.clone(),
        },
        ClipboardRepresentation {
            mime: "text/plain".into(),
            bytes,
        },
    ]
}
