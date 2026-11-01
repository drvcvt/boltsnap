use super::*;
#[test]
fn exact_request_sequence_has_no_extra_surface_commit() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = fixture
        .host
        .enable_ime(window, seat, state(revision))
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::ImeEnable(0),
            Request::ImeSurrounding(0, "aéz".into(), 3, 1),
            Request::ImeCause(0, 1),
            Request::ImeContent(0, 0, 0),
            Request::ImeCaret(0, -2, 3, 5, 9),
            Request::ImeCommit(0)
        ]
    );
    assert!(fixture.host.disable_ime(session).unwrap());
    assert_eq!(
        fixture.host.events(),
        [WindowEvent::Ime(ImeEvent::Ended {
            window,
            seat,
            session
        })]
    );
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::ImeDisable(0), Request::ImeCommit(0)]
    );
    assert!(!fixture.host.disable_ime(session).unwrap());
}
#[test]
fn admission_preserves_activation_before_every_validation_error() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    let mut invalid = state(revision);
    invalid.surrounding.as_mut().unwrap().cursor = 2;
    assert!(matches!(
        fixture.host.enable_ime(window, seat, invalid),
        Err(Error::InvalidInput(_))
    ));
    invalid = state(revision);
    invalid.caret.width = -1;
    assert!(matches!(
        fixture.host.update_ime(session, invalid),
        Err(Error::InvalidInput(_))
    ));
    invalid = state(revision);
    invalid.surrounding = None;
    assert!(matches!(
        fixture.host.update_ime(session, invalid),
        Err(Error::InvalidInput(_))
    ));
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    assert_eq!(
        fixture
            .host
            .coordinator
            .state
            .text_input
            .test_session()
            .unwrap(),
        session
    );
    assert!(fixture.host.events().is_empty());
}
#[test]
fn missing_manager_and_enter_are_explicit() {
    let mut no_manager = Fixture::with_options(PeerOptions {
        seat_count: 1,
        ..PeerOptions::default()
    });
    let window = no_manager.create(20, 20);
    let revision = no_manager.configure(window, 0, 20, 20).revision;
    let seat = no_manager.host.coordinator.state.text_input.test_seat();
    assert!(matches!(
        no_manager.host.enable_ime(window, seat, state(revision)),
        Err(Error::Unsupported(_))
    ));
    let (mut fixture, window, seat, revision) = fixture();
    emit(&fixture, Wire::Leave(0, 0));
    collected(&mut fixture);
    assert!(matches!(
        fixture.host.enable_ime(window, seat, state(revision)),
        Err(Error::NotReady)
    ));
}
#[test]
fn surrounding_limit_and_utf8_boundaries() {
    let (mut fixture, window, seat, revision) = fixture();
    let text = "🦀".repeat(1000);
    let mut value = state(revision);
    value.surrounding = Some(ImeSurrounding {
        text: &text,
        cursor: 4000,
        anchor: 0,
    });
    let session = fixture.host.enable_ime(window, seat, value).unwrap();
    let oversized = format!("{text}x");
    value.surrounding = Some(ImeSurrounding {
        text: &oversized,
        cursor: 4000,
        anchor: 0,
    });
    assert!(matches!(
        fixture.host.update_ime(session, value),
        Err(Error::LimitExceeded)
    ));
    fixture.sync();
    assert_eq!(
        fixture
            .peer
            .take_requests()
            .iter()
            .filter(|event| matches!(event, Request::ImeCommit(_)))
            .count(),
        1
    );
}
#[test]
fn exhaustion_injection_preserves_active_prefix_and_issues_no_requests() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    fixture
        .host
        .coordinator
        .state
        .text_input
        .test_count(u32::MAX);
    assert!(matches!(
        fixture.host.enable_ime(window, seat, state(revision)),
        Err(Error::LimitExceeded)
    ));
    assert!(matches!(
        fixture.host.update_ime(session, state(revision)),
        Err(Error::LimitExceeded)
    ));
    assert!(matches!(
        fixture.host.disable_ime(session),
        Err(Error::LimitExceeded)
    ));
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    assert!(fixture.host.events().is_empty());
}
