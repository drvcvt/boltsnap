//! Interactive requests need the seat's latest press on that window; the state
//! requests only need a live window. Errors send nothing.
use super::*;
use crate::window::tests::peer::InputWireEvent as Input;

const BUTTON_LEFT: u32 = 272;

/// A fixture with `seats` full seats, one 100x100 window (native 0), no focus.
fn fixture(seats: usize) -> (Fixture, WindowId, Vec<InputSeatId>) {
    fixture_with(PeerOptions {
        seat_count: seats,
        ..PeerOptions::default()
    })
}

/// As [`fixture`], with the seat count taken from `options`.
pub(super) fn fixture_with(options: PeerOptions) -> (Fixture, WindowId, Vec<InputSeatId>) {
    let seats = options.seat_count;
    let mut fixture = Fixture::with_options(options);
    fixture.sync();
    fixture.sync();
    let mut ids = Vec::new();
    for index in 0..seats {
        // Ready collection already drained SeatAdded; Capabilities re-publishes
        // the same public seat identity without constructing one.
        fixture.host.events();
        send(&mut fixture, Input::Capabilities(index, 3));
        ids.push(
            fixture
                .host
                .events()
                .into_iter()
                .find_map(|event| match event {
                    WindowEvent::Input(event) => Some(event.seat),
                    _ => None,
                })
                .unwrap(),
        );
    }
    let window = fixture.create(100, 100);
    fixture.host.events();
    fixture.peer.take_requests();
    (fixture, window, ids)
}

pub(super) fn send(fixture: &mut Fixture, event: Input) {
    fixture.peer.emit(PeerEvent::Input(event));
    fixture.sync();
}

/// Pointer `pointer` enters native window `window`, if not already inside.
pub(super) fn enter(fixture: &mut Fixture, pointer: usize, window: usize) {
    send(fixture, Input::PointerEnter(pointer, window, 1, 1.0, 1.0));
}

fn button(fixture: &mut Fixture, pointer: usize, serial: u32, down: bool) {
    send(
        fixture,
        Input::Button(pointer, serial, 1, BUTTON_LEFT, down),
    );
    send(fixture, Input::Frame(pointer));
}

fn assert_not_ready(fixture: &mut Fixture, window: WindowId, seat: InputSeatId) {
    let results = [
        fixture.host.move_window(window, seat),
        fixture.host.resize_window(window, seat, ResizeEdge::Top),
        fixture.host.show_window_menu(window, seat, 1, 2),
    ];
    for result in results {
        assert!(matches!(result, Err(Error::NotReady)), "{result:?}");
    }
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
}

#[test]
fn interactive_requests_use_the_latest_press_serial() {
    let (mut fixture, window, seats) = fixture(1);
    enter(&mut fixture, 0, 0);
    button(&mut fixture, 0, 40, true);
    button(&mut fixture, 0, 41, false);
    button(&mut fixture, 0, 42, true);
    fixture.host.move_window(window, seats[0]).unwrap();
    fixture
        .host
        .resize_window(window, seats[0], ResizeEdge::BottomRight)
        .unwrap();
    fixture
        .host
        .show_window_menu(window, seats[0], 12, -3)
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::Move(0, 0, 42),
            Request::Resize(0, 0, 42, 10),
            Request::ShowWindowMenu(0, 0, 42, 12, -3),
        ]
    );
    // The serial is not consumed: another press replaces it.
    button(&mut fixture, 0, 50, true);
    fixture.host.move_window(window, seats[0]).unwrap();
    fixture.sync();
    assert_eq!(fixture.peer.take_requests(), [Request::Move(0, 0, 50)]);
}

#[test]
fn every_resize_edge_maps_to_its_native_value() {
    let (mut fixture, window, seats) = fixture(1);
    enter(&mut fixture, 0, 0);
    button(&mut fixture, 0, 7, true);
    let edges = [
        (ResizeEdge::Top, 1),
        (ResizeEdge::Bottom, 2),
        (ResizeEdge::Left, 4),
        (ResizeEdge::Right, 8),
        (ResizeEdge::TopLeft, 5),
        (ResizeEdge::TopRight, 9),
        (ResizeEdge::BottomLeft, 6),
        (ResizeEdge::BottomRight, 10),
    ];
    for (edge, value) in edges {
        fixture.host.resize_window(window, seats[0], edge).unwrap();
        fixture.sync();
        assert_eq!(
            fixture.peer.take_requests(),
            [Request::Resize(0, 0, 7, value)]
        );
    }
}

#[test]
fn each_seat_uses_its_own_serial_and_native_seat() {
    let (mut fixture, window, seats) = fixture(2);
    enter(&mut fixture, 0, 0);
    enter(&mut fixture, 1, 0);
    button(&mut fixture, 0, 50, true);
    button(&mut fixture, 1, 60, true);
    fixture.host.move_window(window, seats[1]).unwrap();
    fixture.host.move_window(window, seats[0]).unwrap();
    fixture
        .host
        .show_window_menu(window, seats[1], 5, 6)
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::Move(0, 1, 60),
            Request::Move(0, 0, 50),
            Request::ShowWindowMenu(0, 1, 60, 5, 6),
        ]
    );
}

#[test]
fn without_a_press_requests_are_not_ready_and_send_nothing() {
    let (mut fixture, window, seats) = fixture(1);
    assert_not_ready(&mut fixture, window, seats[0]);
    // Entering and releasing never created a press.
    enter(&mut fixture, 0, 0);
    button(&mut fixture, 0, 9, false);
    assert_not_ready(&mut fixture, window, seats[0]);
}

#[test]
fn a_press_on_another_window_does_not_authorize_this_one() {
    let (mut fixture, first, seats) = fixture(1);
    let second = fixture.create(100, 100);
    fixture.host.events();
    fixture.peer.take_requests();
    enter(&mut fixture, 0, 0);
    button(&mut fixture, 0, 70, true);
    send(&mut fixture, Input::PointerLeave(0, 0, 71));
    enter(&mut fixture, 0, 1);
    assert_not_ready(&mut fixture, second, seats[0]);
    fixture.host.move_window(first, seats[0]).unwrap();
    fixture.sync();
    assert_eq!(fixture.peer.take_requests(), [Request::Move(0, 0, 70)]);
    button(&mut fixture, 0, 72, true);
    fixture.host.move_window(second, seats[0]).unwrap();
    assert!(matches!(
        fixture.host.move_window(first, seats[0]),
        Err(Error::NotReady)
    ));
    fixture.sync();
    assert_eq!(fixture.peer.take_requests(), [Request::Move(1, 0, 72)]);
}

#[test]
fn pointer_capability_loss_clears_the_press() {
    let (mut fixture, window, seats) = fixture(1);
    enter(&mut fixture, 0, 0);
    button(&mut fixture, 0, 80, true);
    // Capability bits: pointer 1, keyboard 2. Keep only the keyboard.
    send(&mut fixture, Input::Capabilities(0, 2));
    fixture.peer.take_requests();
    assert_not_ready(&mut fixture, window, seats[0]);
    // The regained pointer is a new object and starts without a press.
    send(&mut fixture, Input::Capabilities(0, 3));
    assert_eq!(fixture.peer.take_requests(), [Request::Pointer(0, 1)]);
    assert_not_ready(&mut fixture, window, seats[0]);
    enter(&mut fixture, 1, 0);
    button(&mut fixture, 1, 90, true);
    fixture.host.move_window(window, seats[0]).unwrap();
    fixture.sync();
    assert_eq!(fixture.peer.take_requests(), [Request::Move(0, 0, 90)]);
}

#[test]
fn removed_seat_and_retired_window_are_invalid_input() {
    let (mut fixture, window, seats) = fixture(1);
    enter(&mut fixture, 0, 0);
    button(&mut fixture, 0, 5, true);
    send(&mut fixture, Input::RemoveSeat(0));
    fixture.peer.take_requests();
    assert!(matches!(
        fixture.host.move_window(window, seats[0]),
        Err(Error::InvalidInput(_))
    ));
    fixture.host.close(window).unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    assert!(matches!(
        fixture.host.move_window(window, seats[0]),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        fixture.host.set_maximized(window, true),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        fixture.host.set_minimized(window),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        fixture.host.set_fullscreen(window, true),
        Err(Error::InvalidInput(_))
    ));
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
}

#[test]
fn maximize_minimize_and_fullscreen_requests_are_sent_without_a_press() {
    let (mut fixture, window, _) = fixture(0);
    fixture.host.set_maximized(window, true).unwrap();
    fixture.host.set_maximized(window, false).unwrap();
    fixture.host.set_minimized(window).unwrap();
    fixture.host.set_fullscreen(window, true).unwrap();
    fixture.host.set_fullscreen(window, false).unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::SetMaximized(0),
            Request::UnsetMaximized(0),
            Request::SetMinimized(0),
            Request::SetFullscreen(0),
            Request::UnsetFullscreen(0),
        ]
    );
}
