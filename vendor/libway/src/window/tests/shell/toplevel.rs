//! Toplevel size limits, attention requests and idle inhibition (R3) against
//! literal wire traces.
use super::layer::{corner, create};
use super::*;
use crate::window::tests::peer::{ActivationWireEvent as Activation, IdleInhibitWireEvent};

/// Target, limits and the error they must produce.
type Case = (WindowId, SizeLimits, fn(&Error) -> bool);

fn limits(min: (u32, u32), max: (u32, u32)) -> SizeLimits {
    SizeLimits {
        min_width: min.0,
        min_height: min.1,
        max_width: max.0,
        max_height: max.1,
    }
}

fn fixture(options: PeerOptions) -> (Fixture, WindowId) {
    let mut fixture = Fixture::with_options(options);
    let id = fixture.create(100, 100);
    fixture.host.events();
    fixture.peer.take_requests();
    (fixture, id)
}

fn sent(fixture: &mut Fixture) -> Vec<Request> {
    fixture.sync();
    fixture.peer.take_requests()
}

#[test]
fn size_limits_send_only_changes_without_a_commit_in_a_safe_order() {
    let (mut fixture, id) = fixture(PeerOptions::default());
    assert_eq!(fixture.host.size_limits(id).unwrap(), SizeLimits::default());
    let first = limits((320, 200), (640, 400));
    fixture.host.set_size_limits(id, first).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [Request::MaxSize(0, 640, 400), Request::MinSize(0, 320, 200)]
    );
    fixture.host.set_size_limits(id, first).unwrap();
    assert!(
        sent(&mut fixture).is_empty(),
        "unchanged limits send nothing"
    );
    // A growing maximum goes first, so it never sits below the new minimum.
    let grown = limits((700, 450), (900, 600));
    fixture.host.set_size_limits(id, grown).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [Request::MaxSize(0, 900, 600), Request::MinSize(0, 700, 450)]
    );
    // A shrinking maximum goes last, after the minimum dropped below it.
    let shrunk = limits((100, 100), (200, 150));
    fixture.host.set_size_limits(id, shrunk).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [Request::MinSize(0, 100, 100), Request::MaxSize(0, 200, 150)]
    );
    // Mixed: width grows, height shrinks below the old minimum. Lower the
    // minimum first, then the maximum, then the final minimum.
    let mixed = limits((250, 50), (300, 80));
    fixture.host.set_size_limits(id, mixed).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [
            Request::MinSize(0, 100, 50),
            Request::MaxSize(0, 300, 80),
            Request::MinSize(0, 250, 50),
        ]
    );
    let shrunk = limits((100, 100), (200, 150));
    fixture.host.set_size_limits(id, shrunk).unwrap();
    sent(&mut fixture);
    // Dropping the maximum entirely only sends the maximum.
    let open = limits((100, 100), (0, 0));
    fixture.host.set_size_limits(id, open).unwrap();
    assert_eq!(sent(&mut fixture), [Request::MaxSize(0, 0, 0)]);
    let min_only = limits((120, 100), (0, 0));
    fixture.host.set_size_limits(id, min_only).unwrap();
    assert_eq!(sent(&mut fixture), [Request::MinSize(0, 120, 100)]);
    fixture
        .host
        .set_size_limits(id, limits((100, 100), (200, 150)))
        .unwrap();
    sent(&mut fixture);
    // Dropping the maximum while raising the minimum: maximum first.
    let open = limits((300, 300), (0, 0));
    fixture.host.set_size_limits(id, open).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [Request::MaxSize(0, 0, 0), Request::MinSize(0, 300, 300)]
    );
    assert_eq!(fixture.host.size_limits(id).unwrap(), open);
}

#[test]
fn size_limits_ride_the_next_presented_frame() {
    let (mut fixture, id) = fixture(PeerOptions {
        shm: true,
        ..Default::default()
    });
    fixture.host.enable_cpu(id).unwrap();
    let configuration = fixture.configure(id, 0, 2, 1);
    fixture.peer.take_requests();
    fixture
        .host
        .set_size_limits(id, limits((1, 1), (0, 0)))
        .unwrap();
    fixture
        .host
        .present_cpu(id, configuration.revision, &[0; 8])
        .unwrap();
    let requests = sent(&mut fixture);
    let min = requests
        .iter()
        .position(|r| *r == Request::MinSize(0, 1, 1));
    let commit = requests.iter().position(|r| *r == Request::Commit(0, true));
    assert!(min.is_some() && min < commit, "{requests:?}");
    assert_eq!(
        requests
            .iter()
            .filter(|r| matches!(r, Request::Commit(..)))
            .count(),
        1
    );
}

#[test]
fn invalid_size_limits_and_targets_send_nothing() {
    let mut fixture = Fixture::with_options(PeerOptions {
        layer_shell: true,
        ..Default::default()
    });
    let toplevel = fixture.create(100, 100);
    let layer = create(&mut fixture, corner());
    fixture.host.close(toplevel).unwrap();
    fixture.sync();
    let retired = toplevel;
    let live = fixture.create(100, 100);
    fixture.peer.take_requests();
    let foreign = WindowId {
        connection: live.connection + 1000,
        sequence: live.sequence,
    };
    let cases: [Case; 6] = [
        (live, limits((300, 0), (200, 0)), |e| {
            matches!(e, Error::InvalidInput(_))
        }),
        (live, limits((0, 300), (0, 200)), |e| {
            matches!(e, Error::InvalidInput(_))
        }),
        (live, limits((i32::MAX as u32 + 1, 0), (0, 0)), |e| {
            matches!(e, Error::InvalidDimensions)
        }),
        (layer, limits((1, 1), (0, 0)), |e| {
            matches!(e, Error::InvalidInput("window is not a toplevel"))
        }),
        (retired, limits((1, 1), (0, 0)), |e| {
            matches!(e, Error::InvalidInput(_))
        }),
        (foreign, limits((1, 1), (0, 0)), |e| {
            matches!(e, Error::InvalidInput(_))
        }),
    ];
    for (window, case, expected) in cases {
        let result = fixture.host.set_size_limits(window, case);
        assert!(result.as_ref().is_err_and(expected), "{case:?}: {result:?}");
    }
    assert!(fixture.host.size_limits(layer).is_err());
    assert!(sent(&mut fixture).is_empty());
    assert_eq!(
        fixture.host.size_limits(live).unwrap(),
        SizeLimits::default()
    );
}

#[test]
fn attention_sends_a_serialless_token_then_activates_on_done() {
    let (mut fixture, id) = fixture(PeerOptions {
        activation: true,
        ..Default::default()
    });
    fixture.host.request_attention(id).unwrap();
    fixture.host.request_attention(id).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [
            Request::ActivationToken(0),
            Request::TokenSurface(0, 0),
            Request::TokenCommit(0),
        ],
        "a second request while one is pending sends nothing"
    );
    fixture
        .peer
        .emit(PeerEvent::Activation(Activation::Done(0, "t0".into())));
    assert_eq!(
        sent(&mut fixture),
        [Request::Activate("t0".into(), 0), Request::DestroyToken(0)]
    );
    assert!(fixture.host.events().is_empty(), "no event for attention");
    fixture.host.request_attention(id).unwrap();
    assert_eq!(sent(&mut fixture)[0], Request::ActivationToken(1));
}

#[test]
fn attention_works_on_layers_and_a_closed_window_only_destroys_its_token() {
    let mut fixture = Fixture::with_options(PeerOptions {
        activation: true,
        layer_shell: true,
        ..Default::default()
    });
    let layer = create(&mut fixture, corner());
    fixture.host.request_attention(layer).unwrap();
    assert_eq!(
        sent(&mut fixture),
        [
            Request::ActivationToken(0),
            Request::TokenSurface(0, 0),
            Request::TokenCommit(0),
        ]
    );
    fixture.host.close(layer).unwrap();
    fixture.host.events();
    assert!(
        !sent(&mut fixture).contains(&Request::DestroyToken(0)),
        "closing keeps the pending token until done"
    );
    fixture
        .peer
        .emit(PeerEvent::Activation(Activation::Done(0, "t0".into())));
    assert_eq!(sent(&mut fixture), [Request::DestroyToken(0)]);
}

#[test]
fn attention_without_the_global_is_unsupported_and_removal_keeps_pending_tokens() {
    let (mut fixture, id) = fixture(PeerOptions::default());
    let result = fixture.host.request_attention(id);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert!(sent(&mut fixture).is_empty());

    let (mut fixture, id) = self::fixture(PeerOptions {
        activation: true,
        ..Default::default()
    });
    fixture.host.request_attention(id).unwrap();
    fixture.peer.emit(PeerEvent::Activation(Activation::Remove));
    sent(&mut fixture);
    fixture
        .peer
        .emit(PeerEvent::Activation(Activation::Done(0, "late".into())));
    assert_eq!(
        sent(&mut fixture),
        [
            Request::Activate("late".into(), 0),
            Request::DestroyToken(0)
        ]
    );
    let result = fixture.host.request_attention(id);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert!(sent(&mut fixture).is_empty());
}

#[test]
fn shutdown_destroys_pending_tokens_and_the_manager() {
    let (mut fixture, id) = fixture(PeerOptions {
        activation: true,
        ..Default::default()
    });
    fixture.host.request_attention(id).unwrap();
    sent(&mut fixture);
    let Fixture { host, peer } = fixture;
    host.shutdown();
    let requests = peer.requests();
    assert!(requests.contains(&Request::DestroyToken(0)), "{requests:?}");
    assert!(
        requests.contains(&Request::DestroyActivation),
        "{requests:?}"
    );
    assert!(!requests.iter().any(|r| matches!(r, Request::Activate(..))));
}

#[test]
fn idle_inhibit_is_idempotent_and_dies_before_the_surface() {
    let (mut fixture, id) = fixture(PeerOptions {
        idle_inhibit: true,
        ..Default::default()
    });
    fixture.host.set_idle_inhibit(id, false).unwrap();
    fixture.host.set_idle_inhibit(id, true).unwrap();
    fixture.host.set_idle_inhibit(id, true).unwrap();
    assert_eq!(sent(&mut fixture), [Request::Inhibitor(0, 0)]);
    fixture.host.set_idle_inhibit(id, false).unwrap();
    fixture.host.set_idle_inhibit(id, false).unwrap();
    assert_eq!(sent(&mut fixture), [Request::DestroyInhibitor(0)]);
    fixture.host.set_idle_inhibit(id, true).unwrap();
    sent(&mut fixture);
    fixture.host.close(id).unwrap();
    let requests = sent(&mut fixture);
    let inhibitor = requests
        .iter()
        .position(|r| *r == Request::DestroyInhibitor(1));
    let surface = requests
        .iter()
        .position(|r| *r == Request::DestroySurface(0));
    assert!(inhibitor.is_some() && inhibitor < surface, "{requests:?}");
}

#[test]
fn idle_inhibit_works_on_layers_and_without_the_global_only_disabling_works() {
    let mut fixture = Fixture::with_options(PeerOptions {
        idle_inhibit: true,
        layer_shell: true,
        ..Default::default()
    });
    let layer = create(&mut fixture, corner());
    fixture.host.set_idle_inhibit(layer, true).unwrap();
    assert_eq!(sent(&mut fixture), [Request::Inhibitor(0, 0)]);
    fixture
        .peer
        .emit(PeerEvent::IdleInhibit(IdleInhibitWireEvent::Remove));
    sent(&mut fixture);
    fixture.host.set_idle_inhibit(layer, false).unwrap();
    assert_eq!(sent(&mut fixture), [Request::DestroyInhibitor(0)]);
    let result = fixture.host.set_idle_inhibit(layer, true);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert!(sent(&mut fixture).is_empty());
}

#[test]
fn idle_inhibit_ends_at_close_even_while_a_lease_keeps_the_surface() {
    let (mut fixture, id) = fixture(PeerOptions {
        idle_inhibit: true,
        ..Default::default()
    });
    fixture.host.set_idle_inhibit(id, true).unwrap();
    let lease = fixture.host.lease(id).unwrap();
    sent(&mut fixture);
    fixture.host.close(id).unwrap();
    let requests = sent(&mut fixture);
    assert!(
        requests.contains(&Request::DestroyInhibitor(0)),
        "{requests:?}"
    );
    assert!(
        !requests.contains(&Request::DestroySurface(0)),
        "{requests:?}"
    );
    drop(lease);
    assert!(sent(&mut fixture).contains(&Request::DestroySurface(0)));
}

#[test]
fn shutdown_destroys_the_inhibitor_before_the_surface() {
    let (mut fixture, id) = fixture(PeerOptions {
        idle_inhibit: true,
        ..Default::default()
    });
    fixture.host.set_idle_inhibit(id, true).unwrap();
    sent(&mut fixture);
    let Fixture { host, peer } = fixture;
    host.shutdown();
    let requests = peer.requests();
    let inhibitor = requests
        .iter()
        .position(|r| *r == Request::DestroyInhibitor(0));
    let surface = requests
        .iter()
        .position(|r| *r == Request::DestroySurface(0));
    assert!(inhibitor.is_some() && inhibitor < surface, "{requests:?}");
}

#[test]
fn without_the_inhibit_global_enabling_is_unsupported_and_disabling_is_ok() {
    let (mut fixture, id) = fixture(PeerOptions::default());
    let result = fixture.host.set_idle_inhibit(id, true);
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    fixture.host.set_idle_inhibit(id, false).unwrap();
    assert!(sent(&mut fixture).is_empty());
}
