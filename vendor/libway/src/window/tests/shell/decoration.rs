//! xdg-decoration: request, negotiation, persistence, removal and teardown order.
use super::*;

const CLIENT_SIDE: u32 = 1;
const SERVER_SIDE: u32 = 2;

fn decorated() -> Fixture {
    Fixture::with_options(PeerOptions {
        decoration: true,
        ..Default::default()
    })
}

fn creation(window: usize, decoration: bool) -> Vec<Request> {
    let mut expected = vec![
        Request::Surface(window),
        Request::Role(window),
        Request::Toplevel(window),
        Request::Title(window, "owned".into()),
        Request::AppId(window, "test.libway".into()),
    ];
    if decoration {
        expected.push(Request::Decoration(window));
        expected.push(Request::DecorationMode(window, SERVER_SIDE));
    }
    expected.push(Request::Commit(window, false));
    expected
}

#[test]
fn server_side_is_requested_before_the_first_commit_and_negotiated() {
    let (peer, socket) = Peer::start(PeerOptions {
        decoration: true,
        ..Default::default()
    });
    let mut host = Host::from_socket(socket).unwrap();
    ready(&mut host);
    assert_eq!(
        peer.take_requests(),
        [
            Request::BindCompositor,
            Request::BindShell,
            Request::BindDecoration
        ]
    );
    let mut fixture = Fixture { host, peer };
    let id = fixture.create(640, 480);
    assert_eq!(fixture.peer.take_requests(), creation(0, true));
    // A decoration configure alone completes nothing; the xdg sequence does.
    fixture.peer.emit(PeerEvent::Decoration(0, SERVER_SIDE));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(fixture.host.configuration(id).unwrap(), None);
    let configuration = configure(&mut fixture, id, 0, &[]);
    assert_eq!(configuration.decorations, Decorations::Server);
    assert_eq!(fixture.peer.take_requests(), [Request::Ack(0, 73)]);
}

#[test]
fn without_a_manager_decorations_are_client_and_nothing_is_requested() {
    let mut fixture = Fixture::new();
    let id = fixture.create(640, 480);
    assert_eq!(fixture.peer.take_requests(), creation(0, false));
    let configuration = configure(&mut fixture, id, 0, &[ACTIVATED]);
    assert_eq!(configuration.decorations, Decorations::Client);
}

#[test]
fn compositor_chosen_client_side_or_unknown_mode_is_client() {
    for mode in [CLIENT_SIDE, 99] {
        let mut fixture = decorated();
        let id = fixture.create(640, 480);
        assert_eq!(fixture.peer.take_requests(), creation(0, true));
        // Before any decoration configure the answer is Client, not the request.
        let first = configure(&mut fixture, id, 0, &[]);
        assert_eq!(first.decorations, Decorations::Client);
        fixture.peer.emit(PeerEvent::Decoration(0, SERVER_SIDE));
        let server = configure(&mut fixture, id, 0, &[]);
        assert_eq!(server.decorations, Decorations::Server);
        fixture.peer.emit(PeerEvent::Decoration(0, mode));
        let client = configure(&mut fixture, id, 0, &[]);
        assert_eq!(client.decorations, Decorations::Client);
        assert!(fixture.host.failure().is_none());
    }
}

#[test]
fn mode_changes_publish_new_configurations_and_persist_across_plain_configures() {
    let mut fixture = decorated();
    let id = fixture.create(640, 480);
    fixture.peer.emit(PeerEvent::Decoration(0, SERVER_SIDE));
    let first = configure(&mut fixture, id, 0, &[]);
    let plain = configure(&mut fixture, id, 0, &[MAXIMIZED]);
    assert_eq!(plain.decorations, Decorations::Server);
    assert!(plain.revision > first.revision);
    fixture.peer.emit(PeerEvent::Decoration(0, CLIENT_SIDE));
    let client = configure(&mut fixture, id, 0, &[MAXIMIZED]);
    assert_eq!(client.decorations, Decorations::Client);
    assert!(client.revision > plain.revision);
    let kept = configure(&mut fixture, id, 0, &[]);
    assert_eq!(kept.decorations, Decorations::Client);
}

#[test]
fn removed_manager_leaves_existing_windows_and_skips_new_ones() {
    let mut fixture = decorated();
    let first = fixture.create(640, 480);
    fixture.peer.take_requests();
    fixture.peer.emit(PeerEvent::RemoveDecorationManager);
    fixture.sync();
    let second = fixture.create(640, 480);
    assert_eq!(fixture.peer.take_requests(), creation(1, false));
    assert_eq!(
        configure(&mut fixture, second, 1, &[]).decorations,
        Decorations::Client
    );
    fixture.peer.emit(PeerEvent::Decoration(0, SERVER_SIDE));
    assert_eq!(
        configure(&mut fixture, first, 0, &[]).decorations,
        Decorations::Server
    );
}

#[test]
fn decoration_is_destroyed_before_its_toplevel() {
    let mut fixture = decorated();
    let id = fixture.create(640, 480);
    fixture.peer.take_requests();
    fixture.host.close(id).unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::DestroyDecoration(0),
            Request::DestroyToplevel(0),
            Request::DestroyRole(0),
            Request::DestroySurface(0),
        ]
    );
}
