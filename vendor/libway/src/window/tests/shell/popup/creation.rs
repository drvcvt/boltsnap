//! Gates, the creation order, validation, version gates, configure sequences
//! and role checks.
use super::*;

#[test]
fn gates_refuse_before_any_request_until_the_parent_showed_a_frame() {
    let (_peer, socket) = Peer::start(PeerOptions::default());
    let mut early = Host::from_socket(socket).unwrap();
    let forged = WindowId {
        connection: 1,
        sequence: 1,
    };
    // CHECK: the plan names NotReady before Ready; the implementation checks
    // the parent first, and no parent can exist before Ready.
    let result = early.create_popup(PopupOptions::new(forged));
    assert!(
        matches!(result, Err(Error::NotReady | Error::InvalidInput(_))),
        "{result:?}"
    );
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let retired = fixture.create(100, 100);
    fixture.host.close(retired).unwrap();
    let unconfigured = fixture.create(100, 100);
    let unshown = fixture.create(100, 100);
    fixture.host.enable_cpu(unshown).unwrap();
    fixture.configure(unshown, 3, 100, 100);
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
    let invalid = [
        WindowId {
            connection: toplevel.connection,
            sequence: 999,
        },
        WindowId {
            connection: toplevel.connection + 1000,
            sequence: toplevel.sequence,
        },
        retired,
    ];
    for parent in invalid {
        let result = fixture.host.create_popup(menu(parent));
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
    }
    for parent in [unconfigured, unshown] {
        let result = fixture.host.create_popup(menu(parent));
        assert!(matches!(result, Err(Error::NotReady)), "{result:?}");
    }
    assert!(fixture.sync().is_empty());
    show(&mut fixture, unshown);
    fixture.host.create_popup(menu(unshown)).unwrap();
    let requests = fixture.sync();
    assert!(
        requests.contains(&Request::Popup(4, Some(3), 0)),
        "{requests:?}"
    );
}

#[test]
fn creation_sends_surface_role_positioner_popup_grab_then_one_bufferless_commit() {
    let (mut fixture, toplevel, seats) = popup_fixture(PeerOptions {
        seat_count: 1,
        ..Default::default()
    });
    fixture.host.create_popup(menu(toplevel)).unwrap();
    let mut expected = vec![Request::Surface(1), Request::Role(1)];
    expected.extend(rules(0, (0, 0)));
    expected.extend([
        Request::Popup(1, Some(0), 0),
        Request::DestroyPositioner(0),
        Request::Commit(1, false),
    ]);
    assert_eq!(fixture.sync(), expected);
    fixture.peer.take_requests();
    click(&mut fixture, 0, 41);
    fixture
        .host
        .create_popup(PopupOptions {
            grab: Some(seats[0]),
            ..menu(toplevel)
        })
        .unwrap();
    let mut expected = vec![Request::Surface(2), Request::Role(2)];
    expected.extend(rules(1, (0, 0)));
    expected.extend([
        Request::Popup(2, Some(0), 1),
        Request::DestroyPositioner(1),
        Request::Grab(2, 0, 41),
        Request::Commit(2, false),
    ]);
    assert_eq!(fixture.sync(), expected);
    assert!(fixture.host.events().is_empty());
}

#[test]
fn creation_with_the_fractional_pair_adds_both_surface_objects_first() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions {
        fractional: true,
        viewport: true,
        ..Default::default()
    });
    fixture.host.create_popup(menu(toplevel)).unwrap();
    let mut expected = vec![
        Request::Surface(1),
        Request::Viewport(1),
        Request::Fractional(1),
        Request::Role(1),
    ];
    expected.extend(rules(0, (0, 0)));
    expected.extend([
        Request::Popup(1, Some(0), 0),
        Request::DestroyPositioner(0),
        Request::Commit(1, false),
    ]);
    assert_eq!(fixture.sync(), expected);
}

#[test]
fn invalid_options_and_the_window_cap_are_rejected_before_any_request() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let rect = |x, y, width, height| LogicalRect {
        x,
        y,
        width,
        height,
    };
    let sized = |size| PopupOptions {
        size,
        ..menu(toplevel)
    };
    let anchored = |anchor_rect| PopupOptions {
        anchor_rect,
        ..menu(toplevel)
    };
    let too_big = i32::MAX as u32 + 1;
    for case in [
        sized((0, 160)),
        sized((200, 0)),
        sized((too_big, 1)),
        sized((1, too_big)),
    ] {
        let result = fixture.host.create_popup(case);
        assert!(
            matches!(result, Err(Error::InvalidDimensions)),
            "{result:?}"
        );
    }
    for case in [
        anchored(rect(40, 30, 0, 1)),
        anchored(rect(40, 30, 1, 0)),
        anchored(rect(90, 30, 20, 1)),
        anchored(rect(40, 99, 1, 2)),
        anchored(rect(-1, 30, 1, 1)),
    ] {
        let result = fixture.host.create_popup(case);
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
    }
    assert!(fixture.sync().is_empty());
    // The toplevel and fifteen popups fill the sixteen windows.
    for _ in 0..15 {
        open(&mut fixture, PopupOptions::new(toplevel));
    }
    let result = fixture.host.create_popup(PopupOptions::new(toplevel));
    assert!(matches!(result, Err(Error::LimitExceeded)), "{result:?}");
    assert!(fixture.sync().is_empty());
}

#[test]
fn reactive_needs_shell_version_three_and_is_sent_after_the_offset() {
    for version in [2, 3, 6] {
        let (mut fixture, toplevel, _) = popup_fixture(PeerOptions {
            shell_version: version,
            ..Default::default()
        });
        let result = fixture.host.create_popup(PopupOptions {
            reactive: true,
            ..menu(toplevel)
        });
        let requests = fixture.sync();
        if version < 3 {
            assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
            assert!(requests.is_empty(), "v{version}: {requests:?}");
            continue;
        }
        result.unwrap();
        let offset = requests
            .iter()
            .position(|r| *r == Request::PositionerOffset(0, 0, 0))
            .unwrap();
        assert_eq!(requests[offset + 1], Request::PositionerReactive(0));
        assert_eq!(requests[offset + 2], Request::Popup(1, Some(0), 0));
    }
}

#[test]
fn configure_sequences_ack_once_each_and_report_the_placement() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let popup = open(&mut fixture, menu(toplevel));
    let first = configure_popup(&mut fixture, popup, 1, (40, 30, 200, 160), 11);
    assert_eq!(fixture.peer.take_requests(), [Request::Ack(1, 11)]);
    assert_eq!((first.width, first.height), (200, 160));
    assert_eq!(
        (first.pixel_width, first.pixel_height, first.scale_120),
        (200, 160, 120)
    );
    assert!(!first.maximized && !first.fullscreen && !first.activated && !first.suspended);
    assert_eq!(first.tiled, Tiled::default());
    assert_eq!(first.decorations, Decorations::Client);
    let placement = PopupPlacement {
        x: 40,
        y: 30,
        width: 200,
        height: 160,
        repositioned: None,
    };
    assert_eq!(first.placement, Some(placement));
    // Hyprland's initial-commit sequence is a bare surface configure.
    emit(&mut fixture, [Wire::SurfaceConfigure(1, 12)]);
    assert_eq!(fixture.peer.take_requests(), [Request::Ack(1, 12)]);
    let bare = single_configured(&mut fixture, popup);
    assert_eq!(bare.placement, Some(placement));
    assert!(bare.revision > first.revision);
    // Two sequences before any present: two acks and revisions in order.
    emit(
        &mut fixture,
        [
            Wire::Configure(1, 10, 20, 180, 150),
            Wire::SurfaceConfigure(1, 13),
            Wire::Configure(1, -5, 0, 120, 100),
            Wire::SurfaceConfigure(1, 14),
        ],
    );
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::Ack(1, 13), Request::Ack(1, 14)]
    );
    let placed: Vec<_> = fixture
        .host
        .events()
        .into_iter()
        .map(|event| match event {
            WindowEvent::Configured { configuration, .. } => configuration,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(placed.len(), 2);
    assert_eq!((placed[0].width, placed[0].height), (180, 150));
    assert_eq!(placed[1].placement.map(|p| (p.x, p.y)), Some((-5, 0)));
    assert!(bare.revision < placed[0].revision && placed[0].revision < placed[1].revision);
}

#[test]
fn an_empty_size_or_a_missing_placement_closes_the_popup_without_ack() {
    for wire in [
        vec![
            Wire::Configure(1, 0, 0, 0, 160),
            Wire::SurfaceConfigure(1, 9),
        ],
        vec![
            Wire::Configure(1, 0, 0, 200, 0),
            Wire::SurfaceConfigure(1, 9),
        ],
        vec![Wire::SurfaceConfigure(1, 9)],
    ] {
        let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
        let popup = open(&mut fixture, menu(toplevel));
        emit(&mut fixture, wire);
        assert_eq!(lifecycle(fixture.host.events()), closed(&[popup]));
        let requests = fixture.peer.take_requests();
        assert!(!requests.contains(&Request::Ack(1, 9)), "{requests:?}");
        assert_eq!(destroys(requests), popup_gone(1));
        assert!(fixture.host.failure().is_none());
        assert_eq!(
            fixture.host.configuration(toplevel).unwrap().unwrap().width,
            100
        );
    }
}

#[test]
fn a_popup_has_no_toplevel_or_layer_requests_but_presents_and_receives_input() {
    let (mut fixture, toplevel, seats) = popup_fixture(PeerOptions {
        seat_count: 1,
        layer_shell: true,
        ..Default::default()
    });
    let popup = open(&mut fixture, menu(toplevel));
    let seat = seats[0];
    let not_toplevel = [
        fixture.host.set_title(popup, "title".into()),
        fixture.host.set_maximized(popup, true),
        fixture.host.set_minimized(popup),
        fixture.host.set_fullscreen(popup, true),
        fixture.host.move_window(popup, seat),
        fixture.host.resize_window(popup, seat, ResizeEdge::Top),
        fixture.host.show_window_menu(popup, seat, 1, 1),
        fixture.host.set_size_limits(popup, SizeLimits::default()),
        fixture.host.size_limits(popup).map(|_| ()),
        fixture.host.set_layer_state(popup, super::layer::corner()),
        fixture.host.layer_state(popup).map(|_| ()),
    ];
    for result in not_toplevel {
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
    }
    let not_popup = fixture.host.reposition_popup(toplevel, &menu(toplevel));
    assert!(
        matches!(not_popup, Err(Error::InvalidInput(_))),
        "{not_popup:?}"
    );
    assert!(fixture.sync().is_empty());
    fixture
        .host
        .set_input_region(popup, InputRegion::None)
        .unwrap();
    // Regions on a popup wait for its next frame: no bufferless commit.
    assert_eq!(
        fixture.sync(),
        [
            Request::Region(0),
            Request::InputRegion(1, Some(0)),
            Request::DestroyRegion(0),
        ]
    );
    fixture.peer.take_requests();
    fixture.host.enable_cpu(popup).unwrap();
    emit(
        &mut fixture,
        [
            Wire::Configure(1, 40, 30, 2, 1),
            Wire::SurfaceConfigure(1, 11),
        ],
    );
    let configuration = single_configured(&mut fixture, popup);
    let outcome = fixture
        .host
        .present_cpu(popup, configuration.revision, &[0; 8])
        .unwrap();
    assert_eq!(outcome, PresentOutcome::Presented);
    let requests = fixture.sync();
    let position = |wanted: Request| requests.iter().position(|r| *r == wanted);
    let ack = position(Request::Ack(1, 11));
    let attach = position(Request::Attach(1, true));
    assert!(ack.is_some() && ack < attach, "{requests:?}");
    assert_eq!(requests.last(), Some(&Request::Commit(1, true)));
    fixture.peer.take_requests();
    for event in [
        Input::KeyboardEnter(0, 1, 5, Vec::new()),
        Input::PointerEnter(0, 1, 6, 2.0, 3.0),
        Input::Frame(0),
    ] {
        fixture.peer.emit(PeerEvent::Input(event));
    }
    fixture.sync();
    let events = fixture.host.events();
    assert!(
        events.iter().any(|event| matches!(
            event,
            WindowEvent::Input(InputEvent {
                observation: InputObservation::Keyboard(KeyboardObservation::Enter { window, .. }),
                ..
            }) if *window == popup
        )),
        "{events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            WindowEvent::Input(InputEvent {
                observation: InputObservation::PointerFrame(frame),
                ..
            }) if frame.records().iter().any(|record| matches!(
                record,
                PointerObservation::Enter { window, serial: 6, .. } if *window == popup
            ))
        )),
        "{events:?}"
    );
}

#[cfg(feature = "owned-native-system")]
#[test]
fn a_native_claim_on_a_popup_exposes_the_popup_surface() {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use wayland_client::Proxy;
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let popup = open(&mut fixture, menu(toplevel));
    let claim = fixture.host.claim_native(popup).unwrap();
    assert_eq!(claim.id(), popup);
    let RawWindowHandle::Wayland(handle) = claim.window_handle().unwrap().as_raw() else {
        panic!("owned native window must be Wayland");
    };
    let surface = fixture.host.coordinator.state.windows[&popup]
        .native
        .surface()
        .id()
        .as_ptr();
    assert_eq!(handle.surface.as_ptr(), surface.cast::<std::ffi::c_void>());
    assert!(fixture.sync().is_empty());
}
