//! Caller-armed presentation feedback on CPU commits, against the fake compositor.
use super::cpu::present;
use super::*;
use peer::PresentationWireEvent as Wire;
use state::MAX_EVENTS;

const CLOCK_MONOTONIC: u32 = 1;
const PIXEL: [u8; 4] = [1, 2, 3, 255];

fn options() -> PeerOptions {
    PeerOptions {
        shm: true,
        presentation: true,
        ..Default::default()
    }
}

fn prepared(options: PeerOptions) -> (Fixture, WindowId, Configuration) {
    let mut fixture = Fixture::with_options(options);
    let id = fixture.create(1, 1);
    fixture.host.enable_cpu(id).unwrap();
    let config = fixture.configure(id, 0, 1, 1);
    fixture.peer.take_requests();
    (fixture, id, config)
}

fn emit(fixture: &mut Fixture, event: Wire) {
    fixture.peer.emit(PeerEvent::Presentation(event));
    fixture.sync();
}

/// Present one frame, then let the next one through: frame done, buffer 0 released.
fn cycle(fixture: &mut Fixture, id: WindowId, config: Configuration) {
    assert_eq!(
        present(fixture, id, config, &PIXEL),
        PresentOutcome::Presented
    );
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.peer.emit(PeerEvent::Release(0));
    fixture.sync();
}

fn outcomes(fixture: &mut Fixture) -> Vec<(u64, PresentationOutcome)> {
    fixture
        .host
        .events()
        .into_iter()
        .filter_map(|event| match event {
            WindowEvent::Presentation { tag, outcome, .. } => Some((tag, outcome)),
            _ => None,
        })
        .collect()
}

fn feedbacks(requests: &[Request]) -> Vec<Request> {
    requests
        .iter()
        .filter(|request| matches!(request, Request::Feedback(..)))
        .cloned()
        .collect()
}

#[test]
fn without_the_global_requests_are_unsupported_and_commits_carry_nothing() {
    let (mut fixture, id, config) = prepared(PeerOptions {
        shm: true,
        ..Default::default()
    });
    assert!(matches!(
        fixture.host.request_presentation(id, 1),
        Err(Error::Unsupported("wp_presentation"))
    ));
    cycle(&mut fixture, id, config);
    assert!(feedbacks(&fixture.peer.take_requests()).is_empty());
    assert!(outcomes(&mut fixture).is_empty());
    assert!(fixture.host.failure().is_none());
}

#[test]
fn binds_the_advertised_version_up_to_two() {
    for version in [1, 2] {
        let (peer, socket) = Peer::start(PeerOptions {
            presentation: true,
            presentation_version: version,
            ..Default::default()
        });
        let mut host = Host::from_socket(socket).unwrap();
        ready(&mut host);
        assert!(
            peer.take_requests()
                .contains(&Request::BindPresentation(version))
        );
    }
}

#[test]
fn armed_commit_requests_feedback_last_and_reports_exact_timing() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 7).unwrap();
    cycle(&mut fixture, id, config);
    let requests = fixture.peer.take_requests();
    let frame = requests
        .iter()
        .position(|r| matches!(r, Request::Frame(0, _)))
        .unwrap();
    let feedback = requests
        .iter()
        .position(|r| *r == Request::Feedback(0, 0))
        .unwrap();
    assert!(frame < feedback);
    assert_eq!(requests[feedback + 1], Request::Commit(0, true));
    emit(&mut fixture, Wire::Presented(0, [1, 2, 3, 0, 4, 5, 0x11]));
    let [(7, PresentationOutcome::Presented(timing))] = outcomes(&mut fixture)[..] else {
        panic!("expected one presented outcome for tag 7");
    };
    assert_eq!(timing.clock, CLOCK_MONOTONIC);
    assert_eq!(timing.timestamp, Duration::new((1 << 32) | 2, 3));
    assert_eq!(timing.refresh, None);
    assert_eq!(timing.sequence, (4 << 32) | 5);
    let flags = timing.flags;
    assert!(flags.vsync() && !flags.hw_clock() && !flags.hw_completion() && !flags.zero_copy());
    assert_eq!(flags.bits(), 0x11);

    fixture.host.request_presentation(id, 8).unwrap();
    cycle(&mut fixture, id, config);
    emit(
        &mut fixture,
        Wire::Presented(1, [0, 9, 0, 4_166_666, 0, 0, 0xe]),
    );
    let [(8, PresentationOutcome::Presented(timing))] = outcomes(&mut fixture)[..] else {
        panic!("expected one presented outcome for tag 8");
    };
    assert_eq!(timing.refresh, Some(Duration::from_nanos(4_166_666)));
    let flags = timing.flags;
    assert!(!flags.vsync() && flags.hw_clock() && flags.hw_completion() && flags.zero_copy());
}

#[test]
fn unarmed_commits_carry_no_feedback() {
    let (mut fixture, id, config) = prepared(options());
    cycle(&mut fixture, id, config);
    cycle(&mut fixture, id, config);
    assert!(feedbacks(&fixture.peer.take_requests()).is_empty());
}

#[test]
fn arming_again_before_the_commit_replaces_the_tag() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 1).unwrap();
    fixture.host.request_presentation(id, 2).unwrap();
    cycle(&mut fixture, id, config);
    assert_eq!(
        feedbacks(&fixture.peer.take_requests()),
        [Request::Feedback(0, 0)]
    );
    emit(&mut fixture, Wire::Discarded(0));
    assert_eq!(
        outcomes(&mut fixture),
        [(2, PresentationOutcome::Discarded)]
    );
}

#[test]
fn results_without_a_commit_keep_the_tag_armed() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 1).unwrap();
    assert_eq!(
        present(&mut fixture, id, config, &PIXEL),
        PresentOutcome::Presented
    );
    fixture.host.request_presentation(id, 2).unwrap();
    assert_eq!(
        present(&mut fixture, id, config, &PIXEL),
        PresentOutcome::NeedsFrame
    );
    assert_eq!(
        feedbacks(&fixture.peer.take_requests()),
        [Request::Feedback(0, 0)]
    );
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.peer.emit(PeerEvent::Release(0));
    fixture.sync();
    cycle(&mut fixture, id, config);
    assert_eq!(
        feedbacks(&fixture.peer.take_requests()),
        [Request::Feedback(0, 1)]
    );
    emit(&mut fixture, Wire::Discarded(0));
    emit(&mut fixture, Wire::Presented(1, [0, 1, 0, 0, 0, 0, 0]));
    let outcomes = outcomes(&mut fixture);
    assert_eq!(outcomes[0], (1, PresentationOutcome::Discarded));
    assert!(matches!(
        outcomes[1],
        (2, PresentationOutcome::Presented(_))
    ));
}

#[test]
fn eight_requests_per_window_are_admitted() {
    let (mut fixture, id, config) = prepared(options());
    for tag in 0..8 {
        fixture.host.request_presentation(id, tag).unwrap();
        cycle(&mut fixture, id, config);
    }
    assert!(matches!(
        fixture.host.request_presentation(id, 8),
        Err(Error::LimitExceeded)
    ));
    emit(&mut fixture, Wire::Discarded(0));
    assert_eq!(
        outcomes(&mut fixture),
        [(0, PresentationOutcome::Discarded)]
    );
    fixture.host.request_presentation(id, 8).unwrap();
    // Replacing the armed tag needs no slot of its own.
    fixture.host.request_presentation(id, 9).unwrap();
}

#[test]
fn reservations_hold_event_slots_so_delivery_never_overflows() {
    let (mut fixture, id, config) = prepared(options());
    let other = fixture.create(1, 1);
    assert!(fixture.host.coordinator.state.event_room(MAX_EVENTS));
    fixture.host.request_presentation(id, 1).unwrap();
    assert!(!fixture.host.coordinator.state.event_room(MAX_EVENTS));
    assert!(fixture.host.coordinator.state.event_room(MAX_EVENTS - 1));
    cycle(&mut fixture, id, config);
    fixture.host.request_presentation(id, 2).unwrap();
    fixture.host.events();
    let state = &mut fixture.host.coordinator.state;
    while state.event_room(1) {
        state
            .events
            .push_back(WindowEvent::RedrawRequested { window: id });
    }
    assert_eq!(state.events.len(), MAX_EVENTS - 2);
    assert!(matches!(
        fixture.host.request_presentation(other, 1),
        Err(Error::LimitExceeded)
    ));
    fixture.host.request_presentation(id, 3).unwrap();
    emit(&mut fixture, Wire::Discarded(0));
    let state = &fixture.host.coordinator.state;
    assert_eq!(state.events.len(), MAX_EVENTS - 1);
    assert_eq!(
        state.events.back(),
        Some(&WindowEvent::Presentation {
            window: id,
            tag: 1,
            outcome: PresentationOutcome::Discarded,
        })
    );
    assert!(fixture.host.failure().is_none());
}

#[test]
fn closing_releases_slots_and_late_answers_produce_nothing() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 1).unwrap();
    cycle(&mut fixture, id, config);
    let other = fixture.create(1, 1);
    fixture.host.request_presentation(other, 2).unwrap();
    fixture.host.events();
    fixture.host.close(id).unwrap();
    fixture.host.close(other).unwrap();
    assert_eq!(
        fixture.host.events(),
        [
            WindowEvent::Closed { window: id },
            WindowEvent::Closed { window: other }
        ]
    );
    assert_eq!(fixture.host.coordinator.state.presentation.reserved(), 0);
    emit(&mut fixture, Wire::Presented(0, [0, 1, 0, 0, 0, 0, 0]));
    assert!(fixture.host.events().is_empty());
    assert_eq!(feedbacks(&fixture.peer.take_requests()).len(), 1);
    assert!(fixture.host.failure().is_none());
}

#[test]
fn removal_keeps_outstanding_feedback_and_turns_armed_tags_unavailable() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 1).unwrap();
    cycle(&mut fixture, id, config);
    fixture.host.request_presentation(id, 2).unwrap();
    emit(&mut fixture, Wire::Remove);
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::DestroyPresentation)
    );
    assert!(matches!(
        fixture.host.request_presentation(id, 3),
        Err(Error::Unsupported("wp_presentation"))
    ));
    emit(&mut fixture, Wire::Presented(0, [0, 1, 0, 0, 0, 0, 0]));
    assert!(matches!(
        outcomes(&mut fixture)[..],
        [(1, PresentationOutcome::Presented(_))]
    ));
    cycle(&mut fixture, id, config);
    assert!(feedbacks(&fixture.peer.take_requests()).is_empty());
    assert_eq!(
        outcomes(&mut fixture),
        [(2, PresentationOutcome::Unavailable)]
    );
    assert_eq!(fixture.host.coordinator.state.presentation.reserved(), 0);

    emit(&mut fixture, Wire::Advertise);
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::BindPresentation(2))
    );
    fixture.host.request_presentation(id, 4).unwrap();
    cycle(&mut fixture, id, config);
    assert_eq!(
        feedbacks(&fixture.peer.take_requests()),
        [Request::Feedback(0, 1)]
    );
}

#[test]
fn malformed_timing_is_unavailable_and_the_host_keeps_running() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 1).unwrap();
    cycle(&mut fixture, id, config);
    emit(
        &mut fixture,
        Wire::Presented(0, [0, 1, 1_000_000_000, 0, 0, 0, 0]),
    );
    assert_eq!(
        outcomes(&mut fixture),
        [(1, PresentationOutcome::Unavailable)]
    );
    assert!(fixture.host.failure().is_none());
    fixture.host.request_presentation(id, 2).unwrap();
}

#[test]
fn timing_before_any_clock_is_unavailable() {
    let (mut fixture, id, config) = prepared(PeerOptions {
        presentation_clock: None,
        ..options()
    });
    fixture.host.request_presentation(id, 1).unwrap();
    cycle(&mut fixture, id, config);
    emit(&mut fixture, Wire::Presented(0, [0, 1, 0, 0, 0, 0, 0]));
    assert_eq!(
        outcomes(&mut fixture),
        [(1, PresentationOutcome::Unavailable)]
    );
}

#[test]
fn shutdown_with_outstanding_feedback_destroys_the_manager() {
    let (mut fixture, id, config) = prepared(options());
    fixture.host.request_presentation(id, 1).unwrap();
    cycle(&mut fixture, id, config);
    fixture.peer.take_requests();
    let Fixture { host, peer } = fixture;
    host.shutdown();
    assert!(peer.requests().contains(&Request::DestroyPresentation));
}
