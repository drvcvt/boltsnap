//! Input and opaque regions on layers and toplevels: literal region objects,
//! one bufferless commit per change on a layer, none on a toplevel (D3 of the
//! R3 plan), nothing sent on invalid input.
use super::*;

fn rect(x: i32, y: i32, width: u32, height: u32) -> LogicalRect {
    LogicalRect {
        x,
        y,
        width,
        height,
    }
}

fn layer(fixture: &mut Fixture) -> WindowId {
    let id = fixture
        .host
        .create_layer(LayerOptions {
            namespace: "test.region".into(),
            output: None,
            state: LayerState {
                anchor: Anchor::TOP,
                width: 100,
                height: 50,
                ..LayerState::default()
            },
        })
        .unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    id
}

#[test]
fn input_regions_create_set_destroy_and_commit_once() {
    let mut fixture = Fixture::with_options(PeerOptions {
        layer_shell: true,
        ..Default::default()
    });
    let id = layer(&mut fixture);
    fixture
        .host
        .set_input_region(id, InputRegion::None)
        .unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::Region(0),
            Request::InputRegion(0, Some(0)),
            Request::DestroyRegion(0),
            Request::Commit(0, false),
        ]
    );
    fixture.peer.take_requests();
    fixture
        .host
        .set_input_region(
            id,
            InputRegion::Rects(vec![rect(0, 0, 10, 20), rect(-5, 30, 1, 1)]),
        )
        .unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::Region(1),
            Request::RegionAdd(1, 0, 0, 10, 20),
            Request::RegionAdd(1, -5, 30, 1, 1),
            Request::InputRegion(0, Some(1)),
            Request::DestroyRegion(1),
            Request::Commit(0, false),
        ]
    );
    fixture.peer.take_requests();
    fixture.host.set_input_region(id, InputRegion::All).unwrap();
    assert_eq!(
        fixture.sync(),
        [Request::InputRegion(0, None), Request::Commit(0, false)]
    );
    fixture.peer.take_requests();
    fixture
        .host
        .set_opaque_region(id, &[rect(0, 0, 100, 50)])
        .unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::Region(2),
            Request::RegionAdd(2, 0, 0, 100, 50),
            Request::OpaqueRegion(0, Some(2)),
            Request::DestroyRegion(2),
            Request::Commit(0, false),
        ]
    );
}

#[test]
fn toplevel_regions_wait_for_the_next_frame_and_empty_clears_the_hint() {
    let mut fixture = Fixture::new();
    let id = fixture.create(100, 100);
    fixture.peer.take_requests();
    fixture
        .host
        .set_opaque_region(id, &[rect(0, 0, 100, 100)])
        .unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::Region(0),
            Request::RegionAdd(0, 0, 0, 100, 100),
            Request::OpaqueRegion(0, Some(0)),
            Request::DestroyRegion(0),
        ]
    );
    fixture.peer.take_requests();
    fixture.host.set_opaque_region(id, &[]).unwrap();
    fixture
        .host
        .set_input_region(id, InputRegion::None)
        .unwrap();
    assert_eq!(
        fixture.sync(),
        [
            Request::OpaqueRegion(0, None),
            Request::Region(1),
            Request::InputRegion(0, Some(1)),
            Request::DestroyRegion(1),
        ]
    );
}

#[test]
fn invalid_regions_and_targets_send_nothing() {
    let mut fixture = Fixture::with_options(PeerOptions {
        layer_shell: true,
        ..Default::default()
    });
    let id = layer(&mut fixture);
    let too_many = vec![rect(0, 0, 1, 1); 65];
    let cases = [
        (
            fixture
                .host
                .set_input_region(id, InputRegion::Rects(Vec::new())),
            "empty",
        ),
        (
            fixture
                .host
                .set_input_region(id, InputRegion::Rects(vec![rect(0, 0, 0, 1)])),
            "zero",
        ),
        (
            fixture.host.set_opaque_region(id, &[rect(0, 0, 1, 0)]),
            "zero opaque",
        ),
        (
            fixture
                .host
                .set_input_region(id, InputRegion::Rects(too_many.clone())),
            "too many",
        ),
        (
            fixture.host.set_opaque_region(id, &too_many),
            "too many opaque",
        ),
        (
            fixture
                .host
                .set_opaque_region(id, &[rect(0, 0, i32::MAX as u32 + 1, 1)]),
            "too wide",
        ),
    ];
    for (result, case) in cases {
        let expected = match case {
            "too many" | "too many opaque" => matches!(result, Err(Error::LimitExceeded)),
            "too wide" => matches!(result, Err(Error::InvalidDimensions)),
            _ => matches!(result, Err(Error::InvalidInput(_))),
        };
        assert!(expected, "{case}: {result:?}");
    }
    fixture.host.close(id).unwrap();
    let retired = fixture.host.set_input_region(id, InputRegion::None);
    assert!(
        matches!(retired, Err(Error::InvalidInput(_))),
        "{retired:?}"
    );
    let requests = fixture.sync();
    assert!(
        !requests.iter().any(|request| matches!(
            request,
            Request::Region(_)
                | Request::InputRegion(..)
                | Request::OpaqueRegion(..)
                | Request::Commit(..)
        )),
        "{requests:?}"
    );
}
