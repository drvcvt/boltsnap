//! Admission and cancellation precede protocol/identity retirement.
use super::*;

#[test]
fn capability_loss_cancels_devices_and_releases_before_reporting_bits() {
    let (mut fixture, seat) = fixture(9);
    let id = focus(&mut fixture);
    emit(&mut fixture, Wire::Capabilities(0, 0));
    assert_eq!(
        observations(&mut fixture),
        [
            cancelled(
                seat,
                id,
                InputDevice::Keyboard,
                InputCancelReason::CapabilityLost
            ),
            cancelled(
                seat,
                id,
                InputDevice::Pointer,
                InputCancelReason::CapabilityLost
            ),
            InputEvent {
                seat,
                observation: InputObservation::Capabilities { bits: 0 }
            },
        ]
    );
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::ReleaseKeyboard(0), Request::ReleasePointer(0)]
    );
    emit(&mut fixture, Wire::Capabilities(0, 3));
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::Keyboard(0, 1), Request::Pointer(0, 1)]
    );
    emit(&mut fixture, Wire::KeyboardEnter(1, 0, 1, vec![]));
    let event = observations(&mut fixture).pop().unwrap();
    assert_eq!(event.seat, seat);
}

#[test]
fn legacy_retired_callbacks_cannot_route_through_replacement_children() {
    let (mut fixture, seat) = fixture(1);
    let id = focus_legacy(&mut fixture);
    emit(&mut fixture, Wire::Capabilities(0, 0));
    observations(&mut fixture);
    assert!(fixture.peer.take_requests().is_empty());
    emit(&mut fixture, Wire::Capabilities(0, 3));
    observations(&mut fixture);
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::Keyboard(0, 1), Request::Pointer(0, 1)]
    );
    emit(&mut fixture, Wire::Key(0, 10, 20, 30, true));
    emit(&mut fixture, Wire::Motion(0, 20, 1.0, 2.0));
    emit(&mut fixture, Wire::KeyboardEnter(0, 0, 10, vec![]));
    assert!(observations(&mut fixture).is_empty());
    emit(&mut fixture, Wire::KeyboardEnter(1, 0, 11, vec![]));
    let events = observations(&mut fixture);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seat, seat);
    assert!(matches!(events[0].observation, InputObservation::Keyboard(
        KeyboardObservation::Enter {window,..}) if window == id));
}
fn focus_legacy(fixture: &mut Fixture) -> WindowId {
    let id = fixture.create(101, 101);
    emit(fixture, Wire::KeyboardEnter(0, 0, 1, vec![]));
    emit(fixture, Wire::PointerEnter(0, 0, 2, 0.0, 0.0));
    fixture.host.events();
    fixture.peer.take_requests();
    id
}
#[test]
fn legacy_capability_churn_reaches_exact_child_credit_bound() {
    let (mut fixture, _) = fixture(1);
    for _ in 1..32 {
        emit(&mut fixture, Wire::Capabilities(0, 0));
        emit(&mut fixture, Wire::Capabilities(0, 3));
        observations(&mut fixture);
    }
    let expected: Vec<_> = (1..32)
        .flat_map(|index| [Request::Keyboard(0, index), Request::Pointer(0, index)])
        .collect();
    assert_eq!(fixture.peer.take_requests(), expected);
    emit(&mut fixture, Wire::Capabilities(0, 0));
    observations(&mut fixture);
    fixture
        .peer
        .emit(PeerEvent::Input(Wire::Capabilities(0, 3)));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert!(observations(&mut fixture).is_empty());
    assert!(fixture.peer.take_requests().is_empty());
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
}
#[test]
fn seat_removal_cancels_before_removal_and_never_accepts_late_legacy_input() {
    let (mut fixture, seat) = fixture(1);
    let id = focus_legacy(&mut fixture);
    emit(&mut fixture, Wire::RemoveSeat(0));
    assert_eq!(
        observations(&mut fixture),
        [
            cancelled(
                seat,
                id,
                InputDevice::Keyboard,
                InputCancelReason::SeatRemoved
            ),
            cancelled(
                seat,
                id,
                InputDevice::Pointer,
                InputCancelReason::SeatRemoved
            ),
            InputEvent {
                seat,
                observation: InputObservation::SeatRemoved
            },
        ]
    );
    emit(&mut fixture, Wire::KeyboardEnter(0, 0, 20, vec![]));
    emit(&mut fixture, Wire::Key(0, 21, 30, 30, true));
    emit(&mut fixture, Wire::Motion(0, 30, 1.0, 1.0));
    assert!(observations(&mut fixture).is_empty());
    assert!(fixture.peer.take_requests().is_empty());
}
#[test]
fn close_consumes_protected_cancellation_and_closed_slots_at_saturation() {
    let (mut fixture, seat) = fixture(9);
    let id = focus(&mut fixture);
    let state = &mut fixture.host.coordinator.state;
    while state.events.len() < super::super::super::state::MAX_EVENTS - 3 {
        assert!(state.push(WindowEvent::Ready));
    }
    assert!(state.event_room(1));
    fixture.host.close(id).unwrap();
    let events = fixture.host.events();
    assert_eq!(events.len(), super::super::super::state::MAX_EVENTS);
    assert_eq!(
        &events[events.len() - 3..],
        [
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Keyboard,
                InputCancelReason::WindowClosed
            )),
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Pointer,
                InputCancelReason::WindowClosed
            )),
            WindowEvent::Closed { window: id },
        ]
    );
    // Late physical transitions after logical close cannot target a new identity.
    emit(&mut fixture, Wire::Key(0, 20, 30, 30, true));
    emit(&mut fixture, Wire::Motion(0, 30, 1.0, 1.0));
    emit(&mut fixture, Wire::Frame(0));
    assert!(observations(&mut fixture).is_empty());
}
#[test]
fn ordinary_overflow_preserves_terminal_cancellation_inside_total_cap() {
    let (mut fixture, seat) = fixture(9);
    let id = focus(&mut fixture);
    let state = &mut fixture.host.coordinator.state;
    while state.events.len() < super::super::super::state::MAX_EVENTS - 2 {
        assert!(state.push(WindowEvent::Ready));
    }
    assert!(!state.push(WindowEvent::Ready));
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    let events = fixture.host.events();
    assert_eq!(events.len(), super::super::super::state::MAX_EVENTS);
    assert_eq!(
        &events[events.len() - 2..],
        [
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Keyboard,
                InputCancelReason::Shutdown
            )),
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Pointer,
                InputCancelReason::Shutdown
            )),
        ]
    );
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert!(fixture.host.events().is_empty());
}

#[cfg(feature = "owned-native-system")]
#[test]
fn native_guard_reserves_input_cancellations_and_closed_before_staging() {
    let (mut fixture, seat) = fixture(9);
    let id = focus(&mut fixture);
    let configuration = fixture.configure(id, 0, 101, 101);
    let presenter = fixture.host.claim_native(id).unwrap();
    let state = &mut fixture.host.coordinator.state;
    while state.events.len() < super::super::super::state::MAX_EVENTS - 2 {
        assert!(state.push(WindowEvent::Ready));
    }
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&presenter, configuration.revision),
        Err(Error::LimitExceeded)
    ));
    fixture.host.events();
    let NativeFrameStart::Ready(mut frame) = fixture
        .host
        .begin_native_frame(&presenter, configuration.revision)
        .unwrap()
    else {
        panic!("unexpected admission");
    };
    frame.stage(101, 101).unwrap();
    drop(frame);
    assert_eq!(
        fixture.host.events(),
        [
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Keyboard,
                InputCancelReason::WindowClosed
            )),
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Pointer,
                InputCancelReason::WindowClosed
            )),
            WindowEvent::Closed { window: id },
        ]
    );
}

#[test]
fn eight_seats_admit_and_ninth_is_terminal_before_additional_native_requests() {
    let mut fixture = Fixture::with_options(PeerOptions {
        seat_count: 8,
        ..PeerOptions::default()
    });
    fixture.sync();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        (0..8)
            .flat_map(|index| [
                Request::Keyboard(index, index),
                Request::Pointer(index, index)
            ])
            .collect::<Vec<_>>()
    );
    let (peer, socket) = Peer::start(PeerOptions {
        seat_count: 9,
        ..PeerOptions::default()
    });
    let mut host = Host::from_socket(socket).unwrap();
    read_barrier(&host);
    assert!(host.pump(Some(Instant::now())).is_err());
    host.coordinator.state.connection.flush().unwrap();
    let mut expected = vec![Request::BindCompositor, Request::BindShell];
    expected.extend((0..8).map(|index| Request::BindSeat(index, 9)));
    expected.extend((0..8).map(Request::ReleaseSeat));
    assert_eq!(peer.take_requests(), expected);
    let events = host.events();
    assert_eq!(events.len(), 8);
    assert!(events.iter().all(|event| matches!(
        event,
        WindowEvent::Input(InputEvent {
            observation: InputObservation::SeatAdded { version: 9 },
            ..
        })
    )));
    assert!(host.pump(Some(Instant::now())).is_err());
    assert!(host.events().is_empty());
    assert!(peer.take_requests().is_empty());
}

#[test]
fn bounded_seat_name_is_owned_and_unknown_capability_bits_do_not_bind_devices() {
    let (mut fixture, seat) = fixture(9);
    emit(&mut fixture, Wire::Name(0, "x".repeat(256)));
    let InputObservation::SeatName(name) = observations(&mut fixture).pop().unwrap().observation
    else {
        panic!("missing seat name");
    };
    assert_eq!(name.as_str().len(), 256);
    emit(&mut fixture, Wire::Capabilities(0, 128));
    assert_eq!(
        observations(&mut fixture),
        [InputEvent {
            seat,
            observation: InputObservation::Capabilities { bits: 128 }
        }]
    );
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::ReleaseKeyboard(0), Request::ReleasePointer(0)]
    );
    fixture
        .peer
        .emit(PeerEvent::Input(Wire::Name(0, "x".repeat(257))));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert!(observations(&mut fixture).is_empty());
    assert_eq!(name.clone().as_str().len(), 256);
}
