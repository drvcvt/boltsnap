//! Native admission and physical-target validation happen before any wire work.
use super::*;

#[test]
fn initial_configuration_foreign_claim_and_stale_revision_reject_before_wire() {
    let mut fixture = Fixture::new();
    let id = fixture.create(2, 1);
    let claim = fixture.host.claim_native(id).unwrap();
    fixture.peer.take_requests();
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, ConfigurationRevision(1)),
        Err(Error::NotReady)
    ));
    let configuration = fixture.configure(id, 0, 2, 1);
    fixture.peer.take_requests();
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, ConfigurationRevision(0)),
        Err(Error::InvalidInput(_))
    ));
    let mut foreign = Fixture::new();
    let (_, foreign_claim, _) = prepared(&mut foreign);
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&foreign_claim, configuration.revision),
        Err(Error::InvalidInput(_))
    ));
    assert!(fixture.sync().is_empty());
    fixture.host.close(id).unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, configuration.revision),
        Err(Error::InvalidInput(_))
    ));
    assert!(fixture.sync().is_empty());
}

#[test]
fn acquired_extent_mismatch_and_unstaged_scheduling_leave_pacing_reusable() {
    let mut fixture = Fixture::new();
    let (_, claim, configuration) = prepared(&mut fixture);
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    assert_eq!(attempt.configuration(), configuration);
    assert!(matches!(attempt.stage(1, 1), Err(Error::InvalidDimensions)));
    attempt.cancel().unwrap();
    assert!(fixture.sync().is_empty());
    assert!(matches!(
        frame(&mut fixture.host, &claim, configuration).scheduled(),
        Err(Error::InvalidInput(_))
    ));
    assert!(fixture.sync().is_empty());
    frame(&mut fixture.host, &claim, configuration)
        .cancel()
        .unwrap();
    assert!(!claim.is_retired());
}

#[test]
fn suspended_start_never_exposes_a_driver_attempt() {
    let mut fixture = Fixture::new();
    let (id, claim, _) = prepared(&mut fixture);
    fixture
        .peer
        .configure(0, 2, 1, 17, 9_u32.to_ne_bytes().to_vec());
    fixture.sync();
    let suspended = fixture.host.configuration(id).unwrap().unwrap();
    fixture.host.events();
    fixture.peer.take_requests();
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, suspended.revision)
            .unwrap(),
        NativeFrameStart::Suspended
    ));
    assert!(fixture.sync().is_empty());
    let resumed = fixture.configure(id, 0, 2, 1);
    fixture.peer.take_requests();
    frame(&mut fixture.host, &claim, resumed).cancel().unwrap();
    assert!(fixture.sync().is_empty());
}

#[test]
fn sequence_and_retirement_event_credit_exhaustion_reject_before_wire() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    fixture
        .host
        .coordinator
        .state
        .windows
        .get_mut(&id)
        .unwrap()
        .native_pacing
        .next_sequence = u64::MAX;
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, configuration.revision),
        Err(Error::LimitExceeded)
    ));
    assert!(fixture.sync().is_empty());
    fixture
        .host
        .coordinator
        .state
        .windows
        .get_mut(&id)
        .unwrap()
        .native_pacing
        .next_sequence = 1;
    for _ in 0..super::super::super::state::MAX_EVENTS {
        fixture
            .host
            .coordinator
            .state
            .events
            .push_back(WindowEvent::CloseRequested { window: id });
    }
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, configuration.revision),
        Err(Error::LimitExceeded)
    ));
    fixture.host.events();
    assert!(fixture.sync().is_empty());
    frame(&mut fixture.host, &claim, configuration)
        .cancel()
        .unwrap();
    assert!(fixture.host.failure().is_none());
}

#[test]
fn final_sequence_issues_once_and_never_wraps_or_reuses() {
    let mut fixture = Fixture::new();
    let (id, claim, configuration) = prepared(&mut fixture);
    fixture
        .host
        .coordinator
        .state
        .windows
        .get_mut(&id)
        .unwrap()
        .native_pacing
        .next_sequence = u64::MAX - 1;
    let mut attempt = frame(&mut fixture.host, &claim, configuration);
    attempt.stage(2, 1).unwrap();
    attempt.scheduled().unwrap();
    fixture.sync();
    assert_stage(fixture.peer.take_requests());
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    fixture.host.events();
    assert!(matches!(
        fixture
            .host
            .begin_native_frame(&claim, configuration.revision),
        Err(Error::LimitExceeded)
    ));
    assert_eq!(
        fixture.host.coordinator.state.windows[&id]
            .native_pacing
            .next_sequence,
        u64::MAX
    );
    assert!(fixture.sync().is_empty());
}

#[test]
fn a_layer_surface_hands_out_raw_handles_and_native_frames() {
    use crate::window::tests::peer::LayerWireEvent;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
    let mut fixture = Fixture::with_options(PeerOptions {
        layer_shell: true,
        ..Default::default()
    });
    let id = fixture
        .host
        .create_layer(LayerOptions {
            namespace: "test.gpu".into(),
            output: None,
            state: LayerState {
                anchor: Anchor::TOP,
                width: 2,
                height: 1,
                ..LayerState::default()
            },
        })
        .unwrap();
    fixture.sync();
    let claim = fixture.host.claim_native(id).unwrap();
    assert!(claim.display_handle().is_ok());
    assert!(matches!(
        claim.window_handle().unwrap().as_raw(),
        RawWindowHandle::Wayland(_)
    ));
    fixture
        .peer
        .emit(PeerEvent::Layer(LayerWireEvent::Configure(0, 7, 2, 1)));
    fixture.sync();
    let configuration = fixture.host.configuration(id).unwrap().unwrap();
    fixture.peer.take_requests();
    let mut frame = frame(&mut fixture.host, &claim, configuration);
    frame.stage(2, 1).unwrap();
    frame.scheduled().unwrap();
    assert_stage(fixture.sync());
    fixture.host.close(id).unwrap();
    assert!(claim.window_handle().is_err());
}
