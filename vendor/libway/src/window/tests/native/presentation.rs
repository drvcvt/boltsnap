//! Presentation requests staged with native frames, before the driver's commit.
use super::*;
use peer::PresentationWireEvent as Wire;

fn presentation() -> Fixture {
    Fixture::with_options(PeerOptions {
        presentation: true,
        ..Default::default()
    })
}

#[test]
fn staging_requests_feedback_after_the_frame_callback_without_a_commit() {
    let mut fixture = presentation();
    let (id, claim, configuration) = prepared(&mut fixture);
    fixture.host.request_presentation(id, 5).unwrap();
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    attempt.scheduled().unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::BufferScale(0, 1),
            Request::Frame(0, 0),
            Request::Feedback(0, 0)
        ]
    );
    fixture.peer.emit(PeerEvent::Presentation(Wire::Presented(
        0,
        [0, 1, 0, 0, 0, 0, 1],
    )));
    fixture.sync();
    let events = fixture.host.events();
    assert!(matches!(
        events[..],
        [WindowEvent::Presentation {
            window,
            tag: 5,
            outcome: PresentationOutcome::Presented(_),
        }] if window == id
    ));
}

#[test]
fn cancel_before_staging_keeps_the_tag_armed() {
    let mut fixture = presentation();
    let (id, claim, configuration) = prepared(&mut fixture);
    fixture.host.request_presentation(id, 5).unwrap();
    frame(&mut fixture.host, &claim, configuration)
        .cancel()
        .unwrap();
    assert!(fixture.sync().is_empty());
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    attempt.scheduled().unwrap();
    fixture.sync();
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::Feedback(0, 0))
    );
}

#[test]
fn cancel_after_staging_closes_and_no_feedback_event_follows() {
    let mut fixture = presentation();
    let (id, claim, configuration) = prepared(&mut fixture);
    fixture.host.request_presentation(id, 5).unwrap();
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    attempt.cancel().unwrap();
    assert_eq!(fixture.host.events(), [WindowEvent::Closed { window: id }]);
    fixture.sync();
    fixture
        .peer
        .emit(PeerEvent::Presentation(Wire::Discarded(0)));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(fixture.host.coordinator.state.presentation.reserved(), 0);
    assert!(fixture.host.failure().is_none());
}
