//! cursor-shape-v1: devices, enter serials, stored shapes, teardown order.
use super::manage::{enter, fixture_with, send};
use super::*;
use crate::window::tests::peer::InputWireEvent as Input;

const POINTER: u32 = 4;
const TEXT: u32 = 9;
const WAIT: u32 = 6;

fn shaped(seats: usize) -> (Fixture, WindowId, Vec<InputSeatId>) {
    fixture_with(PeerOptions {
        seat_count: seats,
        cursor_shape: true,
        ..PeerOptions::default()
    })
}

fn enter_with(fixture: &mut Fixture, pointer: usize, window: usize, serial: u32) {
    send(
        fixture,
        Input::PointerEnter(pointer, window, serial, 1.0, 1.0),
    );
}

fn requests(fixture: &mut Fixture) -> Vec<Request> {
    fixture.sync();
    fixture.peer.take_requests()
}

#[test]
fn shape_is_bound_with_the_manager_and_a_device_follows_each_pointer() {
    let (peer, socket) = Peer::start(PeerOptions {
        seat_count: 1,
        cursor_shape: true,
        ..Default::default()
    });
    let mut host = Host::from_socket(socket).unwrap();
    ready(&mut host);
    let mut fixture = Fixture { host, peer };
    fixture.sync();
    fixture.sync();
    let requests = fixture.peer.take_requests();
    let position = |wanted: Request| requests.iter().position(|request| *request == wanted);
    assert!(position(Request::BindCursor).is_some(), "{requests:?}");
    assert_eq!(
        position(Request::CursorDevice(0)),
        position(Request::Pointer(0, 0)).map(|index| index + 1),
        "{requests:?}"
    );
}

#[test]
fn stored_shape_is_applied_on_enter_with_the_enter_serial_and_again_on_re_enter() {
    let (mut fixture, window, _) = shaped(1);
    // Set while the pointer is outside: stored, nothing sent.
    fixture
        .host
        .set_cursor_shape(window, CursorShape::Pointer)
        .unwrap();
    assert!(requests(&mut fixture).is_empty());
    enter_with(&mut fixture, 0, 0, 17);
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 17, POINTER)]);
    // Leaving sends nothing; the next enter carries the new serial.
    send(&mut fixture, Input::PointerLeave(0, 0, 18));
    assert!(requests(&mut fixture).is_empty());
    enter_with(&mut fixture, 0, 0, 25);
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 25, POINTER)]);
}

#[test]
fn setting_while_inside_applies_immediately_with_the_current_serial() {
    let (mut fixture, window, _) = shaped(1);
    enter_with(&mut fixture, 0, 0, 30);
    // No shape set yet: entering requested nothing.
    assert!(requests(&mut fixture).is_empty());
    fixture
        .host
        .set_cursor_shape(window, CursorShape::Text)
        .unwrap();
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 30, TEXT)]);
    fixture
        .host
        .set_cursor_shape(window, CursorShape::Wait)
        .unwrap();
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 30, WAIT)]);
    send(&mut fixture, Input::PointerLeave(0, 0, 31));
    enter_with(&mut fixture, 0, 0, 32);
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 32, WAIT)]);
    // Once outside again, a new shape waits for the next enter.
    send(&mut fixture, Input::PointerLeave(0, 0, 33));
    fixture
        .host
        .set_cursor_shape(window, CursorShape::Text)
        .unwrap();
    assert!(requests(&mut fixture).is_empty());
    enter_with(&mut fixture, 0, 0, 34);
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 34, TEXT)]);
}

#[test]
fn only_seats_inside_the_window_are_updated_each_with_its_own_serial() {
    let (mut fixture, first, _) = shaped(2);
    let second = fixture.create(100, 100);
    fixture.host.events();
    fixture.peer.take_requests();
    enter_with(&mut fixture, 0, 0, 40);
    enter_with(&mut fixture, 1, 1, 41);
    fixture
        .host
        .set_cursor_shape(first, CursorShape::Text)
        .unwrap();
    assert_eq!(requests(&mut fixture), [Request::SetShape(0, 40, TEXT)]);
    // Shapes are per window: the other seat enters the shaped window later.
    send(&mut fixture, Input::PointerLeave(1, 1, 42));
    enter_with(&mut fixture, 1, 0, 43);
    assert_eq!(requests(&mut fixture), [Request::SetShape(1, 43, TEXT)]);
    fixture
        .host
        .set_cursor_shape(second, CursorShape::Wait)
        .unwrap();
    assert!(requests(&mut fixture).is_empty());
    // Both seats inside one window are both updated.
    send(&mut fixture, Input::PointerLeave(1, 0, 44));
    enter_with(&mut fixture, 1, 1, 45);
    assert_eq!(requests(&mut fixture), [Request::SetShape(1, 45, WAIT)]);
    fixture
        .host
        .set_cursor_shape(second, CursorShape::Text)
        .unwrap();
    assert_eq!(requests(&mut fixture), [Request::SetShape(1, 45, TEXT)]);
}

#[test]
fn every_shape_maps_to_its_protocol_value() {
    use CursorShape::*;
    let (mut fixture, window, _) = shaped(1);
    enter_with(&mut fixture, 0, 0, 7);
    let shapes = [
        Default,
        ContextMenu,
        Help,
        Pointer,
        Progress,
        Wait,
        Cell,
        Crosshair,
        Text,
        VerticalText,
        Alias,
        Copy,
        Move,
        NoDrop,
        NotAllowed,
        Grab,
        Grabbing,
        EResize,
        NResize,
        NeResize,
        NwResize,
        SResize,
        SeResize,
        SwResize,
        WResize,
        EwResize,
        NsResize,
        NeswResize,
        NwseResize,
        ColResize,
        RowResize,
        AllScroll,
        ZoomIn,
        ZoomOut,
    ];
    for (index, shape) in shapes.into_iter().enumerate() {
        fixture.host.set_cursor_shape(window, shape).unwrap();
        assert_eq!(
            requests(&mut fixture),
            [Request::SetShape(0, 7, index as u32 + 1)],
            "{shape:?}"
        );
    }
}

#[test]
fn without_a_manager_the_call_is_unsupported_and_sends_nothing() {
    let (mut fixture, window, _) = fixture_with(PeerOptions {
        seat_count: 1,
        ..PeerOptions::default()
    });
    enter(&mut fixture, 0, 0);
    assert!(matches!(
        fixture.host.set_cursor_shape(window, CursorShape::Pointer),
        Err(Error::Unsupported("wp_cursor_shape_v1"))
    ));
    assert!(requests(&mut fixture).is_empty());
    assert!(fixture.host.failure().is_none());
}

#[test]
fn a_removed_manager_is_destroyed_and_later_calls_are_unsupported() {
    let (mut fixture, window, _) = shaped(1);
    fixture.peer.emit(PeerEvent::RemoveCursorManager);
    assert_eq!(requests(&mut fixture), [Request::DestroyCursorManager]);
    enter(&mut fixture, 0, 0);
    assert!(matches!(
        fixture.host.set_cursor_shape(window, CursorShape::Pointer),
        Err(Error::Unsupported("wp_cursor_shape_v1"))
    ));
    assert!(requests(&mut fixture).is_empty());
}

#[test]
fn retired_windows_are_invalid_input() {
    let (mut fixture, window, _) = shaped(1);
    fixture.host.close(window).unwrap();
    fixture.peer.take_requests();
    assert!(matches!(
        fixture.host.set_cursor_shape(window, CursorShape::Pointer),
        Err(Error::InvalidInput(_))
    ));
    assert!(requests(&mut fixture).is_empty());
}

#[test]
fn device_is_destroyed_before_its_pointer_and_a_new_pointer_gets_a_new_device() {
    let (mut fixture, window, _) = shaped(1);
    fixture
        .host
        .set_cursor_shape(window, CursorShape::Pointer)
        .unwrap();
    enter_with(&mut fixture, 0, 0, 50);
    fixture.peer.take_requests();
    // Capability bits: pointer 1, keyboard 2. Keep only the keyboard.
    send(&mut fixture, Input::Capabilities(0, 2));
    assert_eq!(
        requests(&mut fixture),
        [Request::DestroyCursorDevice(0), Request::ReleasePointer(0)]
    );
    // Without a pointer there is nothing to apply to; the shape stays stored.
    fixture
        .host
        .set_cursor_shape(window, CursorShape::Text)
        .unwrap();
    assert!(requests(&mut fixture).is_empty());
    send(&mut fixture, Input::Capabilities(0, 3));
    assert_eq!(
        requests(&mut fixture),
        [Request::Pointer(0, 1), Request::CursorDevice(1)]
    );
    // The old enter serial died with the old pointer.
    enter_with(&mut fixture, 1, 0, 51);
    assert_eq!(requests(&mut fixture), [Request::SetShape(1, 51, TEXT)]);
}

#[test]
fn seat_removal_and_shutdown_destroy_devices_before_pointers_and_the_manager_last() {
    let (mut fixture, _, _) = shaped(2);
    send(&mut fixture, Input::RemoveSeat(1));
    let removal = fixture.peer.take_requests();
    let device = removal
        .iter()
        .position(|request| *request == Request::DestroyCursorDevice(1))
        .unwrap();
    let pointer = removal
        .iter()
        .position(|request| *request == Request::ReleasePointer(1))
        .unwrap();
    assert!(device < pointer, "{removal:?}");
    assert!(!removal.contains(&Request::DestroyCursorManager));
    let Fixture { host, peer } = fixture;
    drop(host);
    let requests = peer.take_requests();
    let at = |wanted: Request| requests.iter().position(|request| *request == wanted);
    let device = at(Request::DestroyCursorDevice(0)).unwrap();
    let pointer = at(Request::ReleasePointer(0)).unwrap();
    let manager = at(Request::DestroyCursorManager).unwrap();
    assert!(device < pointer && pointer < manager, "{requests:?}");
}
