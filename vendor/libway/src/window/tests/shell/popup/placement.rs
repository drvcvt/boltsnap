//! Reposition (v3) and layer parents.
use super::*;

#[test]
fn reposition_sends_a_fresh_positioner_and_the_completing_configure_carries_the_token() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let popup = shown(&mut fixture, menu(toplevel), 1, PLACED);
    let lower = PopupOptions {
        offset: (0, 40),
        ..menu(toplevel)
    };
    assert_eq!(fixture.host.reposition_popup(popup, &lower).unwrap(), 1);
    let mut expected = rules(1, (0, 40));
    expected.extend([Request::Reposition(1, 1, 1), Request::DestroyPositioner(1)]);
    assert_eq!(fixture.sync(), expected);
    fixture.peer.take_requests();
    // XML order: repositioned, popup configure, surface configure.
    emit(
        &mut fixture,
        [
            Wire::Repositioned(1, 1),
            Wire::Configure(1, 40, 70, 200, 160),
            Wire::SurfaceConfigure(1, 21),
        ],
    );
    let moved = single_configured(&mut fixture, popup).placement.unwrap();
    assert_eq!(
        moved,
        PopupPlacement {
            x: 40,
            y: 70,
            width: 200,
            height: 160,
            repositioned: Some(1),
        }
    );
    assert_eq!(fixture.host.reposition_popup(popup, &lower).unwrap(), 2);
    fixture.sync();
    fixture.peer.take_requests();
    // Hyprland order: popup configure, repositioned, surface configure.
    emit(
        &mut fixture,
        [
            Wire::Configure(1, 40, 70, 200, 160),
            Wire::Repositioned(1, 2),
            Wire::SurfaceConfigure(1, 22),
        ],
    );
    assert_eq!(fixture.peer.take_requests(), [Request::Ack(1, 22)]);
    let again = single_configured(&mut fixture, popup).placement.unwrap();
    assert_eq!(
        again,
        PopupPlacement {
            repositioned: Some(2),
            ..moved
        }
    );
    // A later bare sequence completes no reposition.
    emit(&mut fixture, [Wire::SurfaceConfigure(1, 23)]);
    let bare = single_configured(&mut fixture, popup).placement.unwrap();
    assert_eq!(
        bare,
        PopupPlacement {
            repositioned: None,
            ..moved
        }
    );
}

#[test]
fn a_scale_only_reconfigure_keeps_the_placement_without_a_token() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions {
        fractional: true,
        viewport: true,
        ..Default::default()
    });
    let popup = shown(&mut fixture, menu(toplevel), 1, PLACED);
    fixture
        .host
        .reposition_popup(popup, &menu(toplevel))
        .unwrap();
    fixture.sync();
    emit(
        &mut fixture,
        [
            Wire::Repositioned(1, 1),
            Wire::Configure(1, 40, 30, 200, 160),
            Wire::SurfaceConfigure(1, 21),
        ],
    );
    let moved = single_configured(&mut fixture, popup);
    assert_eq!(moved.placement.unwrap().repositioned, Some(1));
    fixture.peer.take_requests();
    fixture.peer.emit(PeerEvent::Fractional(1, 180));
    fixture.sync();
    let scaled = single_configured(&mut fixture, popup);
    assert_eq!(scaled.scale_120, 180);
    assert!(scaled.revision > moved.revision);
    assert_eq!(
        scaled.placement,
        Some(PopupPlacement {
            repositioned: None,
            ..moved.placement.unwrap()
        })
    );
    let requests = fixture.peer.take_requests();
    assert!(
        !requests.iter().any(|r| matches!(r, Request::Ack(..))),
        "{requests:?}"
    );
}

#[test]
fn reposition_needs_v3_a_shown_popup_and_its_own_parent() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions {
        shell_version: 2,
        ..Default::default()
    });
    let popup = shown(&mut fixture, menu(toplevel), 1, PLACED);
    let result = fixture.host.reposition_popup(popup, &menu(toplevel));
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert!(fixture.sync().is_empty());
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let popup = open(&mut fixture, menu(toplevel));
    fixture.host.enable_cpu(popup).unwrap();
    configure_popup(&mut fixture, popup, 1, PLACED, 11);
    fixture.peer.take_requests();
    let early = fixture.host.reposition_popup(popup, &menu(toplevel));
    assert!(matches!(early, Err(Error::NotReady)), "{early:?}");
    show(&mut fixture, popup);
    let cases = [
        fixture.host.reposition_popup(toplevel, &menu(toplevel)),
        fixture.host.reposition_popup(popup, &menu(popup)),
    ];
    for result in cases {
        assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
    }
    let empty = PopupOptions {
        size: (0, 160),
        ..menu(toplevel)
    };
    let result = fixture.host.reposition_popup(popup, &empty);
    assert!(
        matches!(result, Err(Error::InvalidDimensions)),
        "{result:?}"
    );
    assert!(fixture.sync().is_empty());
    close(&mut fixture, popup);
    let retired = fixture.host.reposition_popup(popup, &menu(toplevel));
    assert!(
        matches!(retired, Err(Error::InvalidInput(_))),
        "{retired:?}"
    );
    assert!(fixture.sync().is_empty());
}

/// A seated fixture with a shown 240x120 layer (native 1) next to the toplevel.
fn layered(keyboard: KeyboardInteractivity) -> (Fixture, WindowId, InputSeatId) {
    let (mut fixture, _, seats) = popup_fixture(PeerOptions {
        seat_count: 1,
        layer_shell: true,
        ..Default::default()
    });
    let layer = super::layer::create(
        &mut fixture,
        LayerState {
            keyboard,
            ..super::layer::corner()
        },
    );
    fixture.host.enable_cpu(layer).unwrap();
    super::layer::configure(&mut fixture, layer, 1, (240, 120));
    show(&mut fixture, layer);
    (fixture, layer, seats[0])
}

#[test]
fn a_layer_parent_gets_a_null_parent_then_the_layer_get_popup() {
    for keyboard in [KeyboardInteractivity::None, KeyboardInteractivity::OnDemand] {
        let (mut fixture, layer, seat) = layered(keyboard);
        let mut expected = vec![Request::Surface(2), Request::Role(2)];
        expected.extend(rules(0, (0, 0)));
        expected.extend([
            Request::Popup(2, None, 0),
            Request::DestroyPositioner(0),
            Request::LayerGetPopup(1, 2),
            Request::Commit(2, false),
        ]);
        assert_eq!(create(&mut fixture, menu(layer)).1, expected);
        // Hyprland accepts a grab on a layer parent; libway sends it (D6).
        click(&mut fixture, 1, 41);
        let (grabbing, requests) = create(&mut fixture, grab(layer, seat));
        let mut expected = vec![Request::Surface(3), Request::Role(3)];
        expected.extend(rules(1, (0, 0)));
        expected.extend([
            Request::Popup(3, None, 1),
            Request::DestroyPositioner(1),
            Request::LayerGetPopup(1, 3),
            Request::Grab(3, 0, 41),
            Request::Commit(3, false),
        ]);
        assert_eq!(requests, expected);
        emit(&mut fixture, [Wire::Done(3)]);
        assert_eq!(
            lifecycle(fixture.host.events()),
            [
                WindowEvent::PopupGrabRefused { window: grabbing },
                WindowEvent::Closed { window: grabbing },
            ]
        );
    }
}

#[test]
fn closing_a_layer_or_its_compositor_closed_cascades_to_its_popups() {
    for compositor in [false, true] {
        let (mut fixture, layer, _) = layered(KeyboardInteractivity::None);
        let popup = open(&mut fixture, menu(layer));
        if compositor {
            super::layer::emit(&mut fixture, LayerWire::Closed(1));
        } else {
            fixture.host.close(layer).unwrap();
            fixture.sync();
        }
        assert_eq!(lifecycle(fixture.host.events()), closed(&[popup, layer]));
        assert_eq!(
            destroys(fixture.peer.take_requests()),
            [
                &popup_gone(2)[..],
                &[Request::DestroyLayerSurface(1), Request::DestroySurface(1)],
            ]
            .concat()
        );
    }
}
