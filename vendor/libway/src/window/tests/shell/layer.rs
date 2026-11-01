//! Layer-shell creation against literal wire traces: creation order, validation
//! and version gates. Shared helpers for `layer_runtime`.
use super::*;
use crate::window::tests::peer::LayerWireEvent as Wire;

const HUD: &str = "test.hud";

/// Options and the error they must produce.
type Case = (LayerOptions, fn(&Error) -> bool);

pub(super) fn layer_fixture(options: PeerOptions) -> Fixture {
    let mut fixture = Fixture::with_options(PeerOptions {
        layer_shell: true,
        ..options
    });
    fixture.host.events();
    fixture.peer.take_requests();
    fixture
}

/// A 240x120 overlay in the top right corner, 24 logical pixels from both edges.
pub(super) fn corner() -> LayerState {
    LayerState {
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::RIGHT,
        width: 240,
        height: 120,
        margin: [24, 24, 0, 0],
        exclusive_zone: ExclusiveZone::Respect,
        exclusive_edge: None,
        keyboard: KeyboardInteractivity::None,
    }
}

pub(super) fn options(state: LayerState) -> LayerOptions {
    LayerOptions {
        namespace: HUD.into(),
        output: None,
        state,
    }
}

pub(super) fn create(fixture: &mut Fixture, state: LayerState) -> WindowId {
    let id = fixture.host.create_layer(options(state)).unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    id
}

pub(super) fn emit(fixture: &mut Fixture, event: Wire) {
    fixture.peer.emit(PeerEvent::Layer(event));
    fixture.sync();
}

/// Deliver one layer configure and return the single resulting configuration.
pub(super) fn configure(
    fixture: &mut Fixture,
    id: WindowId,
    native: usize,
    size: (u32, u32),
) -> Configuration {
    emit(fixture, Wire::Configure(native, 7, size.0, size.1));
    assert!(
        fixture
            .peer
            .requests()
            .contains(&Request::LayerAck(native, 7)),
        "acked before the consumer observes the configuration"
    );
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
    *configuration
}

#[test]
fn without_the_global_layers_are_unsupported_and_nothing_is_sent() {
    let mut fixture = Fixture::with_options(PeerOptions::default());
    fixture.peer.take_requests();
    let result = fixture.host.create_layer(options(corner()));
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert!(fixture.sync().is_empty());
}

#[test]
fn creating_before_ready_is_not_ready() {
    let (_peer, socket) = Peer::start(PeerOptions {
        layer_shell: true,
        ..Default::default()
    });
    let mut host = Host::from_socket(socket).unwrap();
    let result = host.create_layer(options(corner()));
    assert!(matches!(result, Err(Error::NotReady)), "{result:?}");
}

#[test]
fn creation_sends_role_state_then_one_bufferless_commit() {
    let mut fixture = layer_fixture(PeerOptions::default());
    assert!(fixture.host.events().is_empty());
    fixture.host.create_layer(options(corner())).unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::Surface(0),
            Request::LayerSurface(0, None, 3, HUD.into()),
            Request::LayerSize(0, 240, 120),
            Request::LayerAnchor(0, 1 | 8),
            Request::LayerZone(0, 0),
            Request::LayerMargin(0, [24, 24, 0, 0]),
            Request::LayerKeyboard(0, 0),
            Request::Commit(0, false),
        ]
    );
}

#[test]
fn every_field_reaches_the_wire_with_its_raw_value_and_a_chosen_output() {
    let mut fixture = layer_fixture(PeerOptions {
        outputs: true,
        ..Default::default()
    });
    fixture.sync();
    let outputs = fixture.host.outputs();
    assert_eq!(outputs.len(), 2);
    let state = LayerState {
        layer: Layer::Bottom,
        anchor: Anchor::TOP | Anchor::LEFT,
        width: 300,
        height: 30,
        margin: [1, 2, 3, 4],
        exclusive_zone: ExclusiveZone::Reserve(30),
        exclusive_edge: Some(Edge::Top),
        keyboard: KeyboardInteractivity::OnDemand,
    };
    fixture.peer.take_requests();
    fixture
        .host
        .create_layer(LayerOptions {
            namespace: "panel".into(),
            output: Some(outputs[1].id),
            state,
        })
        .unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::Surface(0),
            Request::LayerSurface(0, Some(1), 1, "panel".into()),
            Request::LayerSize(0, 300, 30),
            Request::LayerAnchor(0, 1 | 4),
            Request::LayerZone(0, 30),
            Request::LayerMargin(0, [1, 2, 3, 4]),
            Request::LayerKeyboard(0, 2),
            Request::LayerEdge(0, 1),
            Request::Commit(0, false),
        ]
    );
    let id = fixture
        .host
        .create_layer(options(LayerState::default()))
        .unwrap();
    let requests = fixture.sync();
    assert!(requests.contains(&Request::LayerAnchor(1, 15)));
    assert!(requests.contains(&Request::LayerZone(1, -1)));
    assert!(requests.contains(&Request::LayerSize(1, 0, 0)));
    assert_eq!(fixture.host.layer_state(id).unwrap(), LayerState::default());
}

#[test]
fn invalid_combinations_are_rejected_before_any_request() {
    let mut fixture = layer_fixture(PeerOptions {
        outputs: true,
        output_count: 1,
        ..Default::default()
    });
    fixture.sync();
    let output = fixture.host.outputs()[0].id;
    fixture.peer.emit(PeerEvent::RemoveOutput(0));
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
    let cases: [Case; 11] = [
        (
            options(LayerState {
                width: 0,
                anchor: Anchor::LEFT | Anchor::TOP,
                ..corner()
            }),
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            options(LayerState {
                height: 0,
                anchor: Anchor::TOP | Anchor::RIGHT,
                ..corner()
            }),
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            options(LayerState {
                exclusive_edge: Some(Edge::Bottom),
                ..corner()
            }),
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            options(LayerState {
                exclusive_zone: ExclusiveZone::Reserve(i32::MAX as u32 + 1),
                ..corner()
            }),
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            options(LayerState {
                width: i32::MAX as u32 + 1,
                ..corner()
            }),
            |e| matches!(e, Error::InvalidDimensions),
        ),
        (
            options(LayerState {
                height: i32::MAX as u32 + 1,
                ..corner()
            }),
            |e| matches!(e, Error::InvalidDimensions),
        ),
        (
            LayerOptions {
                output: Some(OutputId {
                    connection: output.connection,
                    sequence: 999,
                }),
                ..options(corner())
            },
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            LayerOptions {
                namespace: "bad\0name".into(),
                ..options(corner())
            },
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            LayerOptions {
                namespace: "n".repeat(257),
                ..options(corner())
            },
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            LayerOptions {
                output: Some(output),
                ..options(corner())
            },
            |e| matches!(e, Error::InvalidInput(_)),
        ),
        (
            LayerOptions {
                output: Some(OutputId {
                    connection: output.connection + 1000,
                    sequence: 1,
                }),
                ..options(corner())
            },
            |e| matches!(e, Error::InvalidInput(_)),
        ),
    ];
    for (case, expected) in cases {
        let result = fixture.host.create_layer(case.clone());
        assert!(result.as_ref().is_err_and(expected), "{case:?}: {result:?}");
    }
    assert!(fixture.sync().is_empty());
}

#[test]
fn version_gates_reject_newer_fields_and_teardown_follows_the_version() {
    for version in [1, 2, 3, 4, 5] {
        let mut fixture = layer_fixture(PeerOptions {
            layer_shell_version: version,
            ..Default::default()
        });
        // A refused call must leave the ledger empty.
        let quiet = |fixture: &mut Fixture, refused: bool| {
            let sent = fixture.sync();
            assert!(!refused || sent.is_empty(), "v{version}: {sent:?}");
            fixture.peer.take_requests();
        };
        let on_demand = fixture.host.create_layer(options(LayerState {
            keyboard: KeyboardInteractivity::OnDemand,
            ..corner()
        }));
        assert_eq!(on_demand.is_ok(), version >= 4, "v{version}: {on_demand:?}");
        quiet(&mut fixture, on_demand.is_err());
        let edge = fixture.host.create_layer(options(LayerState {
            exclusive_edge: Some(Edge::Top),
            ..corner()
        }));
        assert_eq!(edge.is_ok(), version >= 5, "v{version}: {edge:?}");
        quiet(&mut fixture, edge.is_err());
        let id = create(&mut fixture, corner());
        let moved = fixture.host.set_layer_state(
            id,
            LayerState {
                layer: Layer::Top,
                ..corner()
            },
        );
        assert_eq!(moved.is_ok(), version >= 2, "v{version}: {moved:?}");
        quiet(&mut fixture, moved.is_err());
        for result in [on_demand.err(), edge.err(), moved.err()]
            .into_iter()
            .flatten()
        {
            assert!(matches!(result, Error::Unsupported(_)), "{result:?}");
        }
        fixture.sync();
        fixture.peer.take_requests();
        let Fixture { host, peer } = fixture;
        host.shutdown();
        let requests = peer.requests();
        assert_eq!(requests.contains(&Request::DestroyLayerShell), version >= 3);
        assert!(requests.contains(&Request::DestroyLayerSurface(0)));
    }
}
