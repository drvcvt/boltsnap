//! Cancellation distinguishes reusable pre-stage attempts from retired staged ones.
use super::*;
use crate::window::native::state::NativeFrameToken;

#[test]
fn acquire_encode_submit_cancellation_and_drop_issue_no_frame_requests() {
    let mut fixture = Fixture::new();
    let (_, claim, configuration) = prepared(&mut fixture);
    // The three driver failure phases all remain before libway staging.
    for _ in 0..3 {
        frame(&mut fixture.host, &claim, configuration)
            .cancel()
            .unwrap();
    }
    drop(frame(&mut fixture.host, &claim, configuration));
    assert!(fixture.sync().is_empty());
    assert!(!claim.is_retired());
    assert!(fixture.host.failure().is_none());
}

#[test]
fn staged_cancel_retires_without_commit_and_preserves_external_dependencies() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    attempt.cancel().unwrap();
    assert!(claim.is_retired());
    assert_eq!(fixture.host.events(), [WindowEvent::Closed { window: id }]);
    fixture.sync();
    assert_stage(fixture.peer.take_requests());
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    drop(claim);
    fixture.sync();
    assert_children_destroyed(&fixture.peer, 0);
}

#[test]
fn staged_unwind_retires_only_its_window_without_fake_ready_or_second_commit() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    let other = fixture.create(1, 1);
    fixture.peer.take_requests();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut attempt = frame(&mut fixture.host, &claim, configuration);
        attempt.stage(2, 1).unwrap();
        panic!("synthetic driver failure after staging");
    }));
    assert!(panic.is_err());
    assert!(claim.is_retired());
    assert!(!fixture.host.lease(other).unwrap().is_retired());
    assert_eq!(fixture.host.events(), [WindowEvent::Closed { window: id }]);
    fixture.sync();
    assert_stage(fixture.peer.take_requests());
    assert!(fixture.host.failure().is_none());
}

#[test]
fn scheduled_keeps_real_callback_gate_and_does_not_claim_a_wire_commit() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    assert!(matches!(attempt.stage(2, 1), Err(Error::InvalidInput(_))));
    // This fixture checks caller acknowledgment and callback correlation only.
    // It does not simulate a driver commit or claim WSI presentation acceptance.
    attempt.scheduled().unwrap();
    fixture.sync();
    assert_stage(fixture.peer.take_requests());
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, configuration.revision)
            .unwrap(),
        NativeFrameStart::NeedsFrame
    ));
    assert_eq!(
        fixture
            .host
            .coordinator
            .state
            .native_frame_done(NativeFrameToken {
                window: id,
                sequence: 999
            }),
        None
    );
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    assert_eq!(
        fixture.host.events(),
        [WindowEvent::RedrawRequested { window: id }]
    );
    assert_eq!(
        fixture
            .host
            .coordinator
            .state
            .native_frame_done(NativeFrameToken {
                window: id,
                sequence: 1
            }),
        None
    );
    frame(&mut fixture.host, &claim, configuration)
        .cancel()
        .unwrap();
}

#[test]
fn forgotten_staged_guard_blocks_dispatch_and_prepare_until_explicit_close() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    std::mem::forget(attempt);
    let counts = fixture.host.coordinator.read_dispatch_counts();
    assert!(matches!(
        fixture.host.pump(Some(Instant::now())).unwrap_err().error,
        Error::InvalidInput(_)
    ));
    assert_eq!(fixture.host.coordinator.read_dispatch_counts(), counts);
    assert!(matches!(
        fixture.host.set_title(id, "blocked".into()),
        Err(Error::InvalidInput(_))
    ));
    fixture.host.close(id).unwrap();
    assert!(claim.is_retired());
    fixture.sync();
    assert_stage(fixture.peer.take_requests());
}

#[test]
fn forgotten_guard_preserves_reserved_close_credit_against_other_window_close() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    let other = fixture.create(1, 1);
    let other_lease = fixture.host.lease(other).unwrap();
    fixture.peer.take_requests();
    let state = &mut fixture.host.coordinator.state;
    while state.events.len() < super::super::super::state::MAX_EVENTS - 1 {
        state
            .events
            .push_back(WindowEvent::CloseRequested { window: other });
    }
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    std::mem::forget(attempt);
    assert!(matches!(
        fixture.host.close(other),
        Err(Error::InvalidInput(_))
    ));
    assert!(!other_lease.is_retired());
    assert_eq!(
        fixture.host.coordinator.state.events.len(),
        super::super::super::state::MAX_EVENTS - 1
    );
    fixture.host.close(id).unwrap();
    assert!(claim.is_retired());
    assert!(!other_lease.is_retired());
    let events = fixture.host.events();
    assert_eq!(events.len(), super::super::super::state::MAX_EVENTS);
    assert_eq!(events.last(), Some(&WindowEvent::Closed { window: id }));
    assert!(fixture.host.failure().is_none());
    fixture.sync();
    assert_stage(fixture.peer.take_requests());
}

#[test]
fn native_stage_uses_fractional_snapshot_and_keeps_pacing_per_window() {
    let mut fixture = Fixture::with_options(PeerOptions {
        fractional: true,
        viewport: true,
        ..Default::default()
    });
    let (id, claim, _) = prepared(&mut fixture);
    fixture.peer.emit(PeerEvent::Fractional(0, 180));
    fixture.sync();
    let configuration = fixture.host.configuration(id).unwrap().unwrap();
    fixture.host.events();
    let other = fixture.create(1, 1);
    let other_configuration = fixture.configure(other, 1, 1, 1);
    let other_claim = fixture.host.claim_native(other).unwrap();
    fixture.peer.take_requests();
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(3, 2).unwrap();
    attempt.scheduled().unwrap();
    frame(&mut fixture.host, &other_claim, other_configuration)
        .cancel()
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::BufferScale(0, 1),
            Request::ViewportDestination(0, 2, 1),
            Request::Frame(0, 0)
        ]
    );
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    assert_eq!(
        fixture.host.events(),
        [WindowEvent::RedrawRequested { window: id }]
    );
}

#[test]
fn forgotten_guard_shutdown_retires_claim_without_waiting_or_issuing_commit() {
    let mut fixture = Fixture::new();
    let (_, claim, configuration) = prepared(&mut fixture);
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    std::mem::forget(attempt);
    let report = fixture.host.shutdown();
    assert_eq!(report.outstanding_windows, 1);
    assert!(claim.is_retired());
    assert_stage(fixture.peer.take_requests());
    drop(claim);
    // Shutdown already released the host's globals owner. The final claim now
    // releases the role chain and only then the shell retained by WindowNative.
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::DestroyToplevel(0),
            Request::DestroyRole(0),
            Request::DestroySurface(0),
            Request::DestroyShell
        ]
    );
}
