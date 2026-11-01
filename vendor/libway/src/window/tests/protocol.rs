//! Discovery, complete configuration sequences and close policy use literal wire oracles.
use super::*;

#[test]
fn discovery_is_async_and_every_missing_required_global_fails_without_creation() {
    let (peer, socket) = Peer::start(PeerOptions::default());
    let mut host = Host::from_socket(socket).unwrap();
    assert!(matches!(
        host.create_window(WindowOptions::default()),
        Err(Error::NotReady)
    ));
    ready(&mut host);
    assert_eq!(
        peer.requests(),
        [Request::BindCompositor, Request::BindShell]
    );
    drop(host);
    for (compositor, shell) in [(false, true), (true, false), (false, false)] {
        let (peer, socket) = Peer::start(PeerOptions {
            compositor,
            shell,
            ..Default::default()
        });
        let mut host = Host::from_socket(socket).unwrap();
        let deadline = Instant::now() + WATCHDOG;
        loop {
            assert!(
                Instant::now() < deadline,
                "missing-global discovery timed out"
            );
            if host.pump(Some(deadline)).is_err() {
                break;
            }
        }
        assert!(host.failure().is_some());
        assert!(host.events().is_empty());
        assert!(matches!(
            host.create_window(WindowOptions::default()),
            Err(Error::SessionStopped)
        ));
        assert!(
            !peer
                .requests()
                .iter()
                .any(|request| matches!(request, Request::Surface(_)))
        );
    }
}

#[test]
fn initial_empty_commit_precedes_configure_and_zero_axes_use_receive_order() {
    let mut fixture = Fixture::new();
    let id = fixture.create(640, 480);
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::Surface(0),
            Request::Role(0),
            Request::Toplevel(0),
            Request::Title(0, "owned".into()),
            Request::AppId(0, "test.libway".into()),
            Request::Commit(0, false),
        ]
    );
    assert_eq!(fixture.host.configuration(id).unwrap(), None);
    let cases = [
        (0, 0, u32::MAX - 1, 640, 480),
        (800, 0, u32::MAX, 800, 480),
        (0, 600, 0, 800, 600),
        (0, 0, u32::MAX, 800, 600),
    ];
    for (index, (width, height, serial, expected_width, expected_height)) in
        cases.into_iter().enumerate()
    {
        fixture.peer.configure(0, width, height, serial, vec![]);
        fixture.sync();
        let events = fixture.host.events();
        let [
            WindowEvent::Configured {
                window,
                configuration,
            },
        ] = events.as_slice()
        else {
            panic!("expected exactly one configure: {events:?}");
        };
        assert_eq!(*window, id);
        assert_eq!(
            (configuration.width, configuration.height),
            (expected_width, expected_height)
        );
        assert_eq!(configuration.revision.0, index as u64 + 1);
        assert_eq!(
            fixture.host.configuration(id).unwrap(),
            Some(*configuration)
        );
        assert_eq!(fixture.peer.take_requests(), [Request::Ack(0, serial)]);
    }
}

#[test]
fn invalid_options_and_negative_configures_issue_no_partial_native_work_or_ack() {
    let mut fixture = Fixture::new();
    for (width, height) in [
        (0, 1),
        (1, 0),
        (i32::MAX as u32 + 1, 1),
        (1, i32::MAX as u32 + 1),
    ] {
        assert!(matches!(
            fixture.host.create_window(WindowOptions {
                preferred_width: width,
                preferred_height: height,
                ..Default::default()
            }),
            Err(Error::InvalidDimensions)
        ));
    }
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    drop(fixture);
    for (width, height, states) in [(-1, 10, vec![]), (10, -1, vec![]), (10, 10, vec![1, 2, 3])] {
        let mut fixture = Fixture::new();
        fixture.create(640, 480);
        fixture.peer.take_requests();
        fixture.peer.configure(0, width, height, 55, states);
        read_barrier(&fixture.host);
        assert!(fixture.host.pump(Some(Instant::now())).is_err());
        assert!(fixture.host.failure().is_some());
        assert!(
            !fixture
                .host
                .events()
                .iter()
                .any(|event| matches!(event, WindowEvent::Configured { .. }))
        );
        assert!(
            !fixture
                .peer
                .requests()
                .iter()
                .any(|request| matches!(request, Request::Ack(_, _)))
        );
    }
}

#[test]
fn two_windows_keep_identity_and_compositor_close_waits_for_explicit_policy() {
    let mut fixture = Fixture::new();
    let first = fixture.create(640, 480);
    let second = fixture.create(320, 240);
    assert_ne!(first, second);
    fixture.peer.take_requests();
    fixture.peer.configure(0, 700, 500, 9, vec![]);
    fixture.peer.configure(1, 400, 300, 9, vec![]);
    fixture.sync();
    assert_eq!(fixture.host.events().len(), 2);
    assert_eq!(
        fixture.host.configuration(first).unwrap().unwrap().width,
        700
    );
    assert_eq!(
        fixture.host.configuration(second).unwrap().unwrap().width,
        400
    );
    fixture.peer.take_requests();
    fixture.peer.close_requested(0);
    fixture.sync();
    assert_eq!(
        fixture.host.events(),
        [WindowEvent::CloseRequested { window: first }]
    );
    assert!(fixture.peer.take_requests().is_empty());
    fixture
        .host
        .set_title(first, "policy keeps it open".into())
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::Title(0, "policy keeps it open".into())]
    );
    fixture.host.close(first).unwrap();
    fixture.host.close(first).unwrap();
    fixture.sync();
    assert_eq!(
        fixture.host.events(),
        [WindowEvent::Closed { window: first }]
    );
    assert_children_destroyed(&fixture.peer, 0);
    assert!(fixture.host.configuration(first).is_err());
    fixture
        .host
        .set_title(second, "still alive".into())
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::Title(1, "still alive".into())]
    );
}

#[test]
fn configuration_revision_exhaustion_stops_before_ack_and_never_wraps() {
    let mut fixture = Fixture::new();
    let id = fixture.create(2, 1);
    let lease = fixture.host.lease(id).unwrap();
    fixture
        .host
        .coordinator
        .state
        .windows
        .get_mut(&id)
        .unwrap()
        .next_revision = u64::MAX - 1;
    fixture.peer.take_requests();
    let last = fixture.configure(id, 0, 2, 1);
    assert_eq!(last.revision.0, u64::MAX - 1);
    assert_eq!(fixture.peer.take_requests(), [Request::Ack(0, 73)]);
    fixture.peer.configure(0, 3, 1, u32::MAX, vec![]);
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert!(fixture.host.events().is_empty());
    assert!(fixture.peer.take_requests().is_empty());
    assert!(lease.is_retired());
    let cause = fixture.host.failure().unwrap().to_owned();
    assert!(cause.contains("revision exhausted"));
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert_eq!(fixture.host.failure(), Some(cause.as_str()));
}
