//! `popup_done`, the close cascade (D5), the refused-grab inference (D8) and
//! leases.
use super::*;

#[test]
fn popup_done_on_the_root_closes_the_chain_topmost_first() {
    let (mut fixture, toplevel, seat) = seated();
    click(&mut fixture, 0, 41);
    let root = shown(&mut fixture, grab(toplevel, seat), 1, PLACED);
    let (child, _) = create(&mut fixture, grab(root, seat));
    emit(&mut fixture, [Wire::Done(1)]);
    assert_eq!(lifecycle(fixture.host.events()), closed(&[child, root]));
    assert_eq!(
        destroys(fixture.peer.take_requests()),
        [popup_gone(2), popup_gone(1)].concat()
    );
    // Hyprland sends popup_done to every grabbed popup; the child is retired.
    emit(&mut fixture, [Wire::Done(2)]);
    assert!(fixture.host.events().is_empty());
    assert!(fixture.peer.take_requests().is_empty());
    assert!(fixture.host.configuration(toplevel).unwrap().is_some());
}

#[test]
fn popup_done_before_the_first_ack_reports_a_refused_grab_only_for_grabs() {
    let (mut fixture, toplevel, seat) = seated();
    click(&mut fixture, 0, 41);
    let refused_grab = open(&mut fixture, grab(toplevel, seat));
    emit(&mut fixture, [Wire::Done(1)]);
    assert_eq!(
        lifecycle(fixture.host.events()),
        [
            WindowEvent::PopupGrabRefused {
                window: refused_grab
            },
            WindowEvent::Closed {
                window: refused_grab
            },
        ]
    );
    let passive = open(&mut fixture, menu(toplevel));
    emit(&mut fixture, [Wire::Done(2)]);
    assert_eq!(lifecycle(fixture.host.events()), closed(&[passive]));
    let dismissed = open(&mut fixture, grab(toplevel, seat));
    configure_popup(&mut fixture, dismissed, 3, PLACED, 11);
    emit(&mut fixture, [Wire::Done(3)]);
    assert_eq!(lifecycle(fixture.host.events()), closed(&[dismissed]));
}

#[test]
fn closing_a_window_closes_every_popup_above_it_topmost_first() {
    let (mut fixture, toplevel, seat) = seated();
    click(&mut fixture, 0, 41);
    let root = shown(&mut fixture, grab(toplevel, seat), 1, PLACED);
    let (child, _) = create(&mut fixture, grab(root, seat));
    let tooltip = open(&mut fixture, menu(toplevel));
    fixture.host.close(root).unwrap();
    fixture.sync();
    assert_eq!(lifecycle(fixture.host.events()), closed(&[child, root]));
    assert_eq!(
        destroys(fixture.peer.take_requests()),
        [popup_gone(2), popup_gone(1)].concat()
    );
    assert!(fixture.host.configuration(tooltip).is_ok(), "sibling kept");
    let root = shown(&mut fixture, grab(toplevel, seat), 4, PLACED);
    let (child, _) = create(&mut fixture, grab(root, seat));
    fixture.host.close(toplevel).unwrap();
    fixture.sync();
    // Deeper first, then newer siblings first; the toplevel last.
    assert_eq!(
        lifecycle(fixture.host.events()),
        closed(&[child, root, tooltip, toplevel])
    );
    assert_eq!(
        destroys(fixture.peer.take_requests()),
        [
            &popup_gone(5)[..],
            &popup_gone(4),
            &popup_gone(3),
            &toplevel_gone(),
        ]
        .concat()
    );
}

#[test]
fn the_proxy_close_command_takes_the_same_cascade() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let parent = shown(&mut fixture, menu(toplevel), 1, PLACED);
    let child = open(&mut fixture, menu(parent));
    let proxy = fixture.host.proxy();
    proxy.try_send(WindowCommand::Close(toplevel)).unwrap();
    fixture.sync();
    assert_eq!(
        lifecycle(fixture.host.events()),
        closed(&[child, parent, toplevel])
    );
    assert_eq!(
        destroys(fixture.peer.take_requests()),
        [&popup_gone(2)[..], &popup_gone(1), &toplevel_gone()].concat()
    );
}

#[test]
fn a_full_event_queue_refuses_the_whole_cascade() {
    let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
    let parent = shown(&mut fixture, menu(toplevel), 1, PLACED);
    let child = open(&mut fixture, menu(parent));
    let leases = [toplevel, parent, child].map(|id| fixture.host.lease(id).unwrap());
    let filled = crate::window::state::MAX_EVENTS - 2;
    fixture
        .host
        .coordinator
        .state
        .events
        .extend(std::iter::repeat_n(WindowEvent::Ready, filled));
    let result = fixture.host.close(toplevel);
    assert!(matches!(result, Err(Error::LimitExceeded)), "{result:?}");
    // As for a single close, a full queue fails the host. The cascade must not
    // have started: no window was closed before the refusal.
    let state = &fixture.host.coordinator.state;
    assert!(
        !state
            .events
            .iter()
            .any(|event| matches!(event, WindowEvent::Closed { .. })),
        "no partial cascade"
    );
    assert!(fixture.host.failure().is_some());
    drop(leases);
}

#[test]
fn a_held_child_lease_keeps_every_ancestor_until_it_drops() {
    for shutdown in [false, true] {
        let (mut fixture, toplevel, _) = popup_fixture(PeerOptions::default());
        let popup = open(&mut fixture, menu(toplevel));
        let lease = fixture.host.lease(popup).unwrap();
        let Fixture { mut host, peer } = fixture;
        if shutdown {
            assert_eq!(host.shutdown().outstanding_windows, 2);
        } else {
            host.close(toplevel).unwrap();
            assert_eq!(lifecycle(host.events()), closed(&[popup, toplevel]));
            read_barrier(&host);
            drop(host);
        }
        assert!(lease.is_retired());
        assert!(destroys(peer.take_requests()).is_empty());
        drop(lease);
        let mut expected = [&popup_gone(1)[..], &toplevel_gone()].concat();
        expected.push(Request::DestroyShell);
        assert_eq!(destroys(peer.take_requests()), expected);
    }
}

#[test]
fn a_busy_parent_stays_mapped_while_a_child_lease_lives() {
    let (mut fixture, toplevel, _) = super::manage::fixture_with(PeerOptions {
        shm: true,
        ..Default::default()
    });
    fixture.host.enable_cpu(toplevel).unwrap();
    fixture.configure(toplevel, 0, 100, 100);
    let buffer = present(&mut fixture, toplevel);
    fixture.host.events();
    fixture.peer.take_requests();
    let popup = open(&mut fixture, menu(toplevel));
    let lease = fixture.host.lease(popup).unwrap();
    fixture.host.close(toplevel).unwrap();
    let requests = fixture.sync();
    assert!(
        !requests
            .iter()
            .any(|r| matches!(r, Request::Attach(0, _) | Request::Commit(0, _))),
        "{requests:?}"
    );
    assert!(destroys(fixture.peer.take_requests()).is_empty());
    drop(lease);
    fixture.sync();
    let requests = fixture.peer.take_requests();
    // The last popup native performs the parent's deferred unmap, right
    // after its own popup role is gone.
    let role = requests.iter().position(|r| *r == Request::DestroyPopup(1));
    let unmap = requests
        .iter()
        .position(|r| *r == Request::Attach(0, false));
    assert!(role.is_some() && role < unmap, "{requests:?}");
    assert!(
        requests.contains(&Request::Commit(0, false)),
        "{requests:?}"
    );
    assert_eq!(destroys(requests), popup_gone(1));
    fixture.peer.emit(PeerEvent::Release(buffer));
    fixture.sync();
    assert_eq!(destroys(fixture.peer.take_requests()), toplevel_gone());
}

#[test]
fn a_cascade_with_a_busy_popup_unmaps_the_parent_once_the_popup_is_released() {
    let (mut fixture, toplevel, _) = super::manage::fixture_with(PeerOptions {
        shm: true,
        ..Default::default()
    });
    fixture.host.enable_cpu(toplevel).unwrap();
    fixture.configure(toplevel, 0, 100, 100);
    let parent_buffer = present(&mut fixture, toplevel);
    let popup = open(&mut fixture, menu(toplevel));
    fixture.host.enable_cpu(popup).unwrap();
    configure_popup(&mut fixture, popup, 1, (40, 30, 20, 10), 11);
    let popup_buffer = present(&mut fixture, popup);
    fixture.peer.take_requests();
    // No external lease: the popup's busy buffer alone keeps it alive.
    fixture.host.close(toplevel).unwrap();
    let requests = fixture.sync();
    assert!(
        !requests.contains(&Request::Attach(0, false)),
        "the parent stays mapped under its live popup: {requests:?}"
    );
    fixture.peer.take_requests();
    fixture.peer.emit(PeerEvent::Release(popup_buffer));
    fixture.sync();
    let requests = fixture.peer.take_requests();
    assert!(
        requests.contains(&Request::Attach(0, false)),
        "{requests:?}"
    );
    assert_eq!(destroys(requests), popup_gone(1));
    fixture.peer.emit(PeerEvent::Release(parent_buffer));
    fixture.sync();
    assert_eq!(destroys(fixture.peer.take_requests()), toplevel_gone());
}
