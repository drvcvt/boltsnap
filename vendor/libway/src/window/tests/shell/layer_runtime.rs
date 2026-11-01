//! Layer-shell surfaces at runtime: configure/ack, state changes, `closed`,
//! role checks, CPU presentation and global removal.
use super::layer::{configure, corner, create, emit, layer_fixture, options};
use super::*;
use crate::window::tests::peer::{InputWireEvent as Input, LayerWireEvent as Wire};

#[test]
fn configure_is_acknowledged_once_and_zero_uses_the_requested_size() {
    let mut fixture = layer_fixture(PeerOptions::default());
    let id = create(&mut fixture, corner());
    let first = configure(&mut fixture, id, 0, (0, 0));
    assert_eq!(fixture.peer.take_requests(), [Request::LayerAck(0, 7)]);
    assert_eq!((first.width, first.height), (240, 120));
    assert_eq!(
        (first.pixel_width, first.pixel_height, first.scale_120),
        (240, 120, 120)
    );
    assert!(!first.maximized && !first.fullscreen && !first.activated && !first.suspended);
    assert_eq!(first.decorations, Decorations::Client);
    assert_eq!(fixture.host.configuration(id).unwrap(), Some(first));
    fixture
        .peer
        .emit(PeerEvent::Layer(Wire::Configure(0, 8, 250, 0)));
    fixture
        .peer
        .emit(PeerEvent::Layer(Wire::Configure(0, 9, 260, 130)));
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::LayerAck(0, 8), Request::LayerAck(0, 9)]
    );
    let sizes: Vec<_> = fixture
        .host
        .events()
        .into_iter()
        .map(|event| match event {
            WindowEvent::Configured { configuration, .. } => (
                configuration.width,
                configuration.height,
                configuration.revision,
            ),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(sizes.len(), 2);
    assert_eq!((sizes[0].0, sizes[0].1), (250, 120));
    assert_eq!((sizes[1].0, sizes[1].1), (260, 130));
    assert!(first.revision < sizes[0].2 && sizes[0].2 < sizes[1].2);
}

#[test]
fn a_zero_configure_on_a_stretched_axis_fails_closed_without_ack() {
    let mut fixture = layer_fixture(PeerOptions::default());
    create(&mut fixture, LayerState::default());
    fixture
        .peer
        .emit(PeerEvent::Layer(Wire::Configure(0, 7, 0, 1080)));
    read_barrier(&fixture.host);
    let _ = fixture.host.pump(Some(Instant::now()));
    assert!(
        fixture
            .host
            .failure()
            .is_some_and(|failure| failure.contains("layer configure without size")),
        "{:?}",
        fixture.host.failure()
    );
    assert!(!fixture.peer.requests().contains(&Request::LayerAck(0, 7)));
}

#[test]
fn state_changes_send_only_changed_requests_and_one_commit() {
    let mut fixture = layer_fixture(PeerOptions::default());
    let id = create(&mut fixture, corner());
    let first = configure(&mut fixture, id, 0, (240, 120));
    fixture.peer.take_requests();
    fixture.host.set_layer_state(id, corner()).unwrap();
    assert!(fixture.sync().is_empty(), "unchanged state sends nothing");
    let moved = LayerState {
        margin: [48, 24, 0, 0],
        keyboard: KeyboardInteractivity::Exclusive,
        ..corner()
    };
    fixture.host.set_layer_state(id, moved).unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::LayerMargin(0, [48, 24, 0, 0]),
            Request::LayerKeyboard(0, 1),
            Request::Commit(0, false),
        ]
    );
    let resized = LayerState {
        width: 300,
        layer: Layer::Top,
        ..moved
    };
    fixture.host.set_layer_state(id, resized).unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::LayerSize(0, 300, 120),
            Request::LayerSetLayer(0, 2),
            Request::Commit(0, false),
        ]
    );
    assert_eq!(fixture.host.layer_state(id).unwrap(), resized);
    let next = configure(&mut fixture, id, 0, (300, 120));
    assert_eq!(next.width, 300);
    assert!(next.revision > first.revision);
}

#[test]
fn invalid_state_changes_send_nothing_and_keep_the_old_state() {
    let mut fixture = layer_fixture(PeerOptions::default());
    let state = LayerState {
        exclusive_edge: Some(Edge::Top),
        ..corner()
    };
    let id = create(&mut fixture, state);
    let cleared = fixture.host.set_layer_state(
        id,
        LayerState {
            exclusive_edge: None,
            ..state
        },
    );
    assert!(
        matches!(cleared, Err(Error::InvalidInput(_))),
        "{cleared:?}"
    );
    let unanchored = fixture.host.set_layer_state(
        id,
        LayerState {
            anchor: Anchor::BOTTOM | Anchor::RIGHT,
            ..state
        },
    );
    assert!(
        matches!(unanchored, Err(Error::InvalidInput(_))),
        "{unanchored:?}"
    );
    assert!(fixture.sync().is_empty());
    assert_eq!(fixture.host.layer_state(id).unwrap(), state);
}

#[test]
fn closed_retires_the_identity_once_and_destroys_the_role_before_the_surface() {
    let (mut fixture, _toplevel, seats) = super::manage::fixture_with(PeerOptions {
        seat_count: 1,
        layer_shell: true,
        shm: true,
        ..Default::default()
    });
    let id = create(&mut fixture, corner());
    fixture.host.enable_cpu(id).unwrap();
    let configuration = configure(&mut fixture, id, 1, (240, 120));
    for event in [
        Input::KeyboardEnter(0, 1, 5, Vec::new()),
        Input::PointerEnter(0, 1, 6, 2., 2.),
    ] {
        fixture.peer.emit(PeerEvent::Input(event));
    }
    fixture.sync();
    let entered = fixture.host.events();
    assert!(
        entered.iter().any(|event| matches!(
            event,
            WindowEvent::Input(InputEvent {
                observation: InputObservation::Keyboard(KeyboardObservation::Enter { window, .. }),
                seat,
            }) if *window == id && *seat == seats[0]
        )),
        "{entered:?}"
    );
    let lease = fixture.host.lease(id).unwrap();
    fixture.peer.take_requests();
    emit(&mut fixture, Wire::Closed(1));
    let events = fixture.host.events();
    for device in [InputDevice::Keyboard, InputDevice::Pointer] {
        assert!(
            events.iter().any(|event| matches!(
                event,
                WindowEvent::Input(InputEvent {
                    observation: InputObservation::Cancelled {
                        window,
                        device: cancelled,
                        reason: InputCancelReason::WindowClosed,
                    },
                    ..
                }) if *window == id && *cancelled == device
            )),
            "{device:?}: {events:?}"
        );
    }
    let closed = events
        .iter()
        .filter(|event| matches!(event, WindowEvent::Closed { .. }))
        .count();
    assert_eq!(closed, 1, "{events:?}");
    assert_eq!(events.last(), Some(&WindowEvent::Closed { window: id }));
    assert!(lease.is_retired());
    let present = fixture
        .host
        .present_cpu(id, configuration.revision, &[0; 240 * 120 * 4]);
    assert!(
        matches!(present, Err(Error::InvalidInput("window is retired"))),
        "{present:?}"
    );
    fixture.host.close(id).unwrap();
    emit(&mut fixture, Wire::Closed(1));
    assert!(
        fixture.host.events().is_empty(),
        "closed after retirement adds nothing"
    );
    assert!(
        fixture.peer.take_requests().is_empty(),
        "the lease keeps the role alive"
    );
    drop(lease);
    fixture.sync();
    let requests = fixture.peer.take_requests();
    let role = requests
        .iter()
        .position(|r| *r == Request::DestroyLayerSurface(1));
    let surface = requests
        .iter()
        .position(|r| *r == Request::DestroySurface(1));
    assert!(role.is_some() && role < surface, "{requests:?}");
}

#[test]
fn compositor_closed_after_an_own_close_adds_nothing() {
    let mut fixture = layer_fixture(PeerOptions::default());
    let id = create(&mut fixture, corner());
    configure(&mut fixture, id, 0, (240, 120));
    fixture.host.close(id).unwrap();
    fixture.sync();
    fixture.host.events();
    emit(&mut fixture, Wire::Closed(0));
    assert!(fixture.host.events().is_empty());
}

#[test]
fn keys_and_leave_on_a_focused_layer_reach_the_consumer_with_its_identity() {
    let (mut fixture, _toplevel, _seats) = super::manage::fixture_with(PeerOptions {
        seat_count: 1,
        layer_shell: true,
        ..Default::default()
    });
    let id = create(
        &mut fixture,
        LayerState {
            keyboard: KeyboardInteractivity::OnDemand,
            ..corner()
        },
    );
    for event in [
        Input::KeyboardEnter(0, 1, 5, Vec::new()),
        Input::Key(0, 6, 40, 1, true),
        Input::Key(0, 7, 41, 1, false),
        Input::KeyboardLeave(0, 1, 8),
    ] {
        fixture.peer.emit(PeerEvent::Input(event));
    }
    fixture.sync();
    let keyboard: Vec<_> = fixture
        .host
        .events()
        .into_iter()
        .filter_map(|event| match event {
            WindowEvent::Input(InputEvent {
                observation: InputObservation::Keyboard(observation),
                ..
            }) => Some(observation),
            _ => None,
        })
        .collect();
    assert!(
        matches!(
            keyboard.as_slice(),
            [
                KeyboardObservation::Enter { window: a, .. },
                KeyboardObservation::Key { window: b, physical: 1, down: true, serial: 6, .. },
                KeyboardObservation::Key { window: c, physical: 1, down: false, .. },
                KeyboardObservation::Leave { window: d, serial: 8 },
            ] if [a, b, c, d].iter().all(|w| **w == id)
        ),
        "{keyboard:?}"
    );
}

#[test]
fn toplevel_requests_on_a_layer_and_layer_requests_on_a_toplevel_send_nothing() {
    let (mut fixture, toplevel, seats) = super::manage::fixture_with(PeerOptions {
        seat_count: 1,
        layer_shell: true,
        ..Default::default()
    });
    let layer = create(&mut fixture, corner());
    let seat = seats[0];
    let not_toplevel = [
        fixture.host.set_title(layer, "title".into()),
        fixture.host.set_maximized(layer, true),
        fixture.host.set_minimized(layer),
        fixture.host.set_fullscreen(layer, true),
        fixture.host.move_window(layer, seat),
        fixture.host.resize_window(layer, seat, ResizeEdge::Top),
        fixture.host.show_window_menu(layer, seat, 1, 1),
    ];
    for result in not_toplevel {
        assert!(
            matches!(result, Err(Error::InvalidInput("window is not a toplevel"))),
            "{result:?}"
        );
    }
    let not_layer = [
        fixture.host.set_layer_state(toplevel, corner()),
        fixture.host.layer_state(toplevel).map(|_| ()),
    ];
    for result in not_layer {
        assert!(
            matches!(result, Err(Error::InvalidInput("window is not a layer"))),
            "{result:?}"
        );
    }
    assert!(fixture.sync().is_empty());
}

#[test]
fn a_configured_layer_presents_cpu_frames() {
    let mut fixture = layer_fixture(PeerOptions {
        shm: true,
        ..Default::default()
    });
    let id = create(
        &mut fixture,
        LayerState {
            width: 2,
            height: 1,
            ..corner()
        },
    );
    fixture.host.enable_cpu(id).unwrap();
    let configuration = configure(&mut fixture, id, 0, (2, 1));
    let outcome = fixture
        .host
        .present_cpu(id, configuration.revision, &[0; 8])
        .unwrap();
    assert_eq!(outcome, PresentOutcome::Presented);
    let requests = fixture.sync();
    let position = |wanted: Request| requests.iter().position(|r| *r == wanted);
    let ack = position(Request::LayerAck(0, 7));
    let attach = position(Request::Attach(0, true));
    assert!(ack.is_some() && ack < attach, "{requests:?}");
    assert_eq!(requests.last(), Some(&Request::Commit(0, true)));
}

#[test]
fn global_removal_keeps_existing_layers_and_refuses_new_ones() {
    let mut fixture = layer_fixture(PeerOptions {
        shm: true,
        ..Default::default()
    });
    let id = create(&mut fixture, corner());
    fixture.host.enable_cpu(id).unwrap();
    emit(&mut fixture, Wire::Remove);
    let configuration = configure(&mut fixture, id, 0, (240, 120));
    assert_eq!(configuration.width, 240);
    let presented = fixture
        .host
        .present_cpu(id, configuration.revision, &[0; 240 * 120 * 4])
        .unwrap();
    assert_eq!(presented, PresentOutcome::Presented);
    fixture.sync();
    let requests = fixture.peer.take_requests();
    assert!(requests.contains(&Request::LayerAck(0, 7)));
    assert_eq!(requests.last(), Some(&Request::Commit(0, true)));
    let result = fixture.host.create_layer(options(corner()));
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert!(fixture.sync().is_empty());
}
