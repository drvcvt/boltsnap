use super::*;
#[test]
fn readiness_is_reported_without_activation_in_both_focus_orders() {
    for text_first in [false, true] {
        let mut fixture = Fixture::with_options(PeerOptions {
            ime: true,
            seat_count: 1,
            ..PeerOptions::default()
        });
        let window = fixture.create(30, 20);
        let revision = fixture.configure(window, 0, 30, 20).revision;
        let seat = fixture.host.coordinator.state.text_input.test_seat();
        if text_first {
            emit(&fixture, Wire::Enter(0, 0));
        } else {
            fixture
                .peer
                .emit(PeerEvent::Input(InputWireEvent::KeyboardEnter(
                    0,
                    0,
                    1,
                    vec![],
                )));
        }
        fixture.sync();
        fixture.host.events();
        assert!(matches!(
            fixture.host.enable_ime(window, seat, state(revision)),
            Err(Error::NotReady)
        ));
        if text_first {
            fixture
                .peer
                .emit(PeerEvent::Input(InputWireEvent::KeyboardEnter(
                    0,
                    0,
                    2,
                    vec![],
                )));
        } else {
            emit(&fixture, Wire::Enter(0, 0));
        }
        fixture.sync();
        fixture
            .host
            .enable_ime(window, seat, state(revision))
            .unwrap();
    }
}
#[test]
fn either_leave_order_retires_once_before_leave() {
    for text_first in [false, true] {
        let (mut fixture, window, seat, revision) = fixture();
        let session = activate(&mut fixture, window, seat, revision);
        emit(&fixture, Wire::Preedit(0, Some("pending".into()), 0, 7));
        if text_first {
            emit(&fixture, Wire::Leave(0, 0));
        } else {
            fixture
                .peer
                .emit(PeerEvent::Input(InputWireEvent::KeyboardLeave(0, 0, 3)));
        }
        if text_first {
            fixture
                .peer
                .emit(PeerEvent::Input(InputWireEvent::KeyboardLeave(0, 0, 3)));
        } else {
            emit(&fixture, Wire::Leave(0, 0));
        }
        fixture.sync();
        let events = fixture.host.events();
        assert_eq!(
            events[0],
            WindowEvent::Ime(ImeEvent::Ended {
                window,
                seat,
                session
            })
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WindowEvent::Ime(ImeEvent::Ended { .. })))
                .count(),
            1
        );
        emit(&fixture, Wire::Done(0, 1));
        assert!(collected(&mut fixture).is_empty());
        assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
    }
}
#[test]
fn keymap_capability_seat_window_manager_and_shutdown_retire_exact_session() {
    for cause in 0..6 {
        let (mut fixture, window, seat, revision) = fixture();
        let session = activate(&mut fixture, window, seat, revision);
        retire_cause(&mut fixture, window, cause);
        if cause != 5 {
            fixture.sync();
        }
        let events = fixture.host.events();
        assert_eq!(
            events[0],
            WindowEvent::Ime(ImeEvent::Ended {
                window,
                seat,
                session
            })
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, WindowEvent::Ime(ImeEvent::Ended { .. })))
                .count(),
            1
        );
        assert!(
            fixture
                .host
                .coordinator
                .state
                .text_input
                .test_session_or_none()
                .is_none()
        );
    }
}
#[test]
fn seat_replacement_has_fresh_identity_and_one_object_per_seat() {
    let (peer, socket) = Peer::start(PeerOptions {
        ime: true,
        seat_count: 2,
        ..PeerOptions::default()
    });
    let mut host = Host::from_socket(socket).unwrap();
    ready(&mut host);
    let mut fixture = Fixture { host, peer };
    fixture.sync();
    let requests = fixture.peer.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|event| matches!(event, Request::ImeObject(..)))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|event| matches!(event, Request::BindIme))
            .count(),
        1
    );
}
#[test]
fn manager_rebind_resets_object_count_without_reusing_session_identity() {
    let (mut fixture, window, seat, revision) = fixture();
    let old = activate(&mut fixture, window, seat, revision);
    fixture.host.update_ime(old, state(revision)).unwrap();
    fixture.sync();
    emit(&fixture, Wire::RemoveManager);
    collected(&mut fixture);
    assert!(matches!(
        fixture.host.enable_ime(window, seat, state(revision)),
        Err(Error::Unsupported(_))
    ));
    emit(&fixture, Wire::AddManager);
    fixture.sync();
    fixture.sync();
    emit(&fixture, Wire::Enter(1, 0));
    collected(&mut fixture);
    let current = fixture
        .host
        .enable_ime(window, seat, state(revision))
        .unwrap();
    assert_ne!(old, current);
    fixture.sync();
    fixture.host.events();
    emit(&fixture, Wire::Commit(1, Some("new".into())));
    emit(&fixture, Wire::Done(1, 1));
    assert!(
        matches!(&collected(&mut fixture)[0], ImeEvent::Done { session, serial: 1, synchronized: true, transaction, .. }
        if *session == current && transaction.commit.as_ref().unwrap().as_str() == "new")
    );
}

fn retire_cause(fixture: &mut Fixture, window: WindowId, cause: usize) {
    match cause {
        0 => {
            let file = std::fs::File::open("/dev/null").unwrap();
            fixture.peer.emit(PeerEvent::Input(InputWireEvent::Keymap(
                0,
                0,
                file.into(),
                0,
            )));
        }
        1 => fixture
            .peer
            .emit(PeerEvent::Input(InputWireEvent::Capabilities(0, 1))),
        2 => fixture
            .peer
            .emit(PeerEvent::Input(InputWireEvent::RemoveSeat(0))),
        3 => fixture.host.close(window).unwrap(),
        4 => emit(fixture, Wire::RemoveManager),
        5 => {
            fixture.host.coordinator.state.retire_all();
        }
        _ => unreachable!(),
    }
}
