//! Lease retention and native credits are checked through independent wire destruction.
use super::*;

#[test]
fn host_teardown_flushes_output_releases_before_the_surviving_native_lease_drops() {
    for explicit_shutdown in [false, true] {
        let mut fixture = Fixture::with_options(PeerOptions {
            outputs: true,
            ..Default::default()
        });
        let id = fixture.create(640, 480);
        let lease = fixture.host.lease(id).unwrap();
        fixture.peer.take_requests();
        let Fixture { host, peer } = fixture;
        if explicit_shutdown {
            assert_eq!(host.shutdown().outstanding_windows, 1);
        } else {
            drop(host);
        }
        assert!(lease.is_retired());
        // Taking this acknowledged snapshot cannot flush the retained client's
        // connection. The shutdown owner must already have sent both releases.
        assert_eq!(
            peer.take_requests(),
            [Request::ReleaseOutput(0), Request::ReleaseOutput(1)]
        );
        drop(lease);
        assert_eq!(
            peer.take_requests(),
            [
                Request::DestroyToplevel(0),
                Request::DestroyRole(0),
                Request::DestroySurface(0),
                Request::DestroyShell,
            ]
        );
    }
}

#[test]
fn leases_outlive_host_and_final_drop_on_another_thread_destroys_children_before_shell() {
    for reverse in [false, true] {
        let mut fixture = Fixture::new();
        let id = fixture.create(640, 480);
        let first = fixture.host.lease(id).unwrap();
        let second = first.clone();
        fixture.peer.take_requests();
        let Fixture { host, peer } = fixture;
        if reverse {
            assert_eq!(host.shutdown().outstanding_windows, 1);
        } else {
            drop(host);
        }
        assert!(first.is_retired() && second.is_retired());
        assert_eq!(first.id(), id);
        assert!(peer.requests().is_empty());
        let final_lease = if reverse {
            drop(second);
            first
        } else {
            drop(first);
            second
        };
        assert!(peer.requests().is_empty());
        std::thread::spawn(move || drop(final_lease))
            .join()
            .unwrap();
        assert_eq!(
            peer.requests(),
            [
                Request::DestroyToplevel(0),
                Request::DestroyRole(0),
                Request::DestroySurface(0),
                Request::DestroyShell,
            ]
        );
    }
}

#[test]
fn sixteen_retired_native_windows_keep_credits_until_the_final_lease_releases() {
    let mut fixture = Fixture::new();
    let mut ids = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..16 {
        let id = fixture
            .host
            .create_window(WindowOptions::default())
            .unwrap();
        leases.push(fixture.host.lease(id).unwrap());
        ids.push(id);
        fixture.host.close(id).unwrap();
    }
    fixture.sync();
    let requests = fixture.peer.take_requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| matches!(request, Request::Surface(_)))
            .count(),
        16
    );
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, Request::DestroySurface(_)))
    );
    assert!(leases.iter().all(WindowLease::is_retired));
    assert!(matches!(
        fixture.host.create_window(WindowOptions::default()),
        Err(Error::LimitExceeded)
    ));
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    drop(leases.pop().unwrap());
    let replacement = fixture
        .host
        .create_window(WindowOptions::default())
        .unwrap();
    assert!(!ids.contains(&replacement));
    fixture.sync();
    let requests = fixture.peer.take_requests();
    assert_eq!(
        &requests[..3],
        &[
            Request::DestroyToplevel(15),
            Request::DestroyRole(15),
            Request::DestroySurface(15)
        ]
    );
    assert!(requests.contains(&Request::Surface(16)));
    drop(leases);
    fixture.host.close(replacement).unwrap();
    fixture.sync();
    assert_eq!(fixture.host.shutdown().outstanding_windows, 0);
}

#[test]
fn presenter_claim_is_unique_reusable_and_local_to_its_window() {
    let mut fixture = Fixture::new();
    let first = fixture.create(640, 480);
    let second = fixture.create(320, 240);
    fixture.peer.take_requests();
    let claim = fixture
        .host
        .coordinator
        .state
        .claim_presenter(first)
        .unwrap();
    assert!(
        fixture
            .host
            .coordinator
            .state
            .claim_presenter(first)
            .is_err()
    );
    drop(claim);
    let first_claim = fixture
        .host
        .coordinator
        .state
        .claim_presenter(first)
        .unwrap();
    let second_claim = fixture
        .host
        .coordinator
        .state
        .claim_presenter(second)
        .unwrap();
    fixture.host.close(first).unwrap();
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    assert!(
        fixture
            .host
            .coordinator
            .state
            .claim_presenter(first)
            .is_err()
    );
    assert!(!fixture.host.lease(second).unwrap().is_retired());
    drop(first_claim);
    fixture.sync();
    assert_children_destroyed(&fixture.peer, 0);
    drop(second_claim);
    fixture
        .host
        .set_title(second, "presenter remains independent".into())
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::Title(1, "presenter remains independent".into())]
    );
}

#[cfg(feature = "owned-native-system")]
fn native_handles(
    presenter: &NativePresenter,
) -> (
    std::ptr::NonNull<std::ffi::c_void>,
    std::ptr::NonNull<std::ffi::c_void>,
) {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let RawDisplayHandle::Wayland(display) = presenter.display_handle().unwrap().as_raw() else {
        panic!("owned native display must be Wayland");
    };
    let RawWindowHandle::Wayland(window) = presenter.window_handle().unwrap().as_raw() else {
        panic!("owned native window must be Wayland");
    };
    (display.display, window.surface)
}

#[cfg(feature = "owned-native-system")]
#[test]
fn public_native_claim_exposes_stable_distinct_handles_and_excludes_cpu_claims() {
    let mut fixture = Fixture::with_options(PeerOptions {
        shm: true,
        ..Default::default()
    });
    let first = fixture.create(2, 1);
    let second = fixture.create(1, 1);
    fixture.peer.take_requests();
    let first_claim = fixture.host.claim_native(first).unwrap();
    let second_claim = fixture.host.claim_native(second).unwrap();
    assert_eq!(first_claim.id(), first);
    assert!(!first_claim.is_retired());
    let first_handles = native_handles(&first_claim);
    let second_handles = native_handles(&second_claim);
    assert_eq!(native_handles(&first_claim), first_handles);
    assert_eq!(first_handles.0, second_handles.0);
    assert_ne!(first_handles.1, second_handles.1);
    assert!(matches!(
        fixture.host.claim_native(first),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        fixture.host.enable_cpu(first),
        Err(Error::InvalidInput(_))
    ));
    drop(first_claim);
    fixture.host.enable_cpu(first).unwrap();
    assert!(matches!(
        fixture.host.claim_native(first),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(native_handles(&second_claim), second_handles);
    assert!(fixture.host.failure().is_none());
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
}

#[cfg(feature = "owned-native-system")]
#[test]
fn retired_native_claim_refuses_handles_and_role_destruction_waits_for_every_lease() {
    use raw_window_handle::{HandleError, HasDisplayHandle, HasWindowHandle};
    for presenter_first in [true, false] {
        let mut fixture = Fixture::new();
        let id = fixture.create(2, 1);
        let ordinary = fixture.host.lease(id).unwrap();
        let presenter = fixture.host.claim_native(id).unwrap();
        native_handles(&presenter);
        fixture.peer.take_requests();
        let Fixture { host, peer } = fixture;
        drop(host);
        assert!(ordinary.is_retired() && presenter.is_retired());
        assert!(matches!(
            presenter.display_handle(),
            Err(HandleError::Unavailable)
        ));
        assert!(matches!(
            presenter.window_handle(),
            Err(HandleError::Unavailable)
        ));
        assert!(peer.requests().is_empty());
        if presenter_first {
            drop(presenter);
            assert!(peer.requests().is_empty());
            drop(ordinary);
        } else {
            drop(ordinary);
            assert!(peer.requests().is_empty());
            drop(presenter);
        }
        assert_eq!(
            peer.requests(),
            [
                Request::DestroyToplevel(0),
                Request::DestroyRole(0),
                Request::DestroySurface(0),
                Request::DestroyShell
            ]
        );
    }
}
