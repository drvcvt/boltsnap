//! CPU damage regions on the wire and in reused SHM slots, against the fake compositor.
mod slots;

use super::cpu::cpu_fixture;
use super::*;

/// A 4x2 image whose pixels are `[value, 0, 0, 255]`, row-major.
#[derive(Clone, Copy)]
pub(super) struct Image(pub [u8; 8]);

impl Image {
    pub(super) fn rgba(&self) -> Vec<u8> {
        self.0.iter().flat_map(|v| [*v, 0, 0, 255]).collect()
    }

    /// The bytes a compositor sees: native-endian ARGB8888.
    pub(super) fn native(&self) -> Vec<u8> {
        let pixel = |v: &u8| u32::from_be_bytes([255, *v, 0, 0]).to_ne_bytes();
        self.0.iter().flat_map(pixel).collect()
    }
}

pub(super) fn show(
    fixture: &mut Fixture,
    id: WindowId,
    config: Configuration,
    rgba: &[u8],
    damage: &[PixelRect],
) -> PresentOutcome {
    let outcome = fixture
        .host
        .present_cpu_damaged(id, config.revision, rgba, damage)
        .unwrap();
    fixture.sync();
    outcome
}

pub(super) fn frame_done(fixture: &mut Fixture) {
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
}

pub(super) fn release(fixture: &mut Fixture, buffer: usize) {
    fixture.peer.emit(PeerEvent::Release(buffer));
    fixture.sync();
}

fn damages(requests: &[Request]) -> Vec<(i32, i32, i32, i32)> {
    requests
        .iter()
        .filter_map(|request| match request {
            Request::Damage(0, x, y, w, h) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect()
}

fn rect(x: u32, y: u32, w: u32, h: u32) -> PixelRect {
    PixelRect::new(x, y, w, h)
}

#[test]
fn first_frame_damages_everything_then_clamped_rects_reach_the_wire() {
    let (mut fixture, id, config) = cpu_fixture(4, 2);
    let image = Image([10; 8]);
    fixture.peer.take_requests();
    // A fresh surface has no previous content, so a small damage list still
    // damages the whole buffer.
    let first = show(&mut fixture, id, config, &image.rgba(), &[rect(1, 1, 1, 1)]);
    assert_eq!(first, PresentOutcome::Presented);
    assert_eq!(damages(&fixture.peer.take_requests()), [(0, 0, 4, 2)]);
    frame_done(&mut fixture);
    fixture.peer.take_requests();
    let list = [
        rect(1, 0, 2, 1),
        rect(3, 1, 50, 50),
        rect(2, 1, 0, 1),
        rect(4, 0, 3, 3),
        rect(u32::MAX, u32::MAX, 5, 5),
    ];
    let second = show(&mut fixture, id, config, &image.rgba(), &list);
    assert_eq!(second, PresentOutcome::Presented);
    assert_eq!(
        damages(&fixture.peer.take_requests()),
        [(1, 0, 2, 1), (3, 1, 1, 1)]
    );
}

#[test]
fn undamaged_present_cpu_still_damages_the_whole_buffer() {
    let (mut fixture, id, config) = cpu_fixture(4, 2);
    let image = Image([10; 8]);
    fixture.peer.take_requests();
    show(&mut fixture, id, config, &image.rgba(), &[rect(0, 0, 1, 1)]);
    frame_done(&mut fixture);
    fixture.peer.take_requests();
    let outcome = fixture
        .host
        .present_cpu(id, config.revision, &Image([11; 8]).rgba())
        .unwrap();
    fixture.sync();
    assert_eq!(outcome, PresentOutcome::Presented);
    assert_eq!(damages(&fixture.peer.take_requests()), [(0, 0, 4, 2)]);
    assert_eq!(fixture.peer.buffers()[1].bytes, Image([11; 8]).native());
}

#[test]
fn empty_or_fully_empty_lists_are_rejected_before_any_native_request() {
    let (mut fixture, id, config) = cpu_fixture(4, 2);
    fixture.peer.take_requests();
    let rgba = Image([10; 8]).rgba();
    let nothing = [rect(0, 0, 0, 2), rect(0, 1, 3, 0), rect(4, 0, 1, 1)];
    for list in [&[][..], &nothing[..]] {
        let result = fixture
            .host
            .present_cpu_damaged(id, config.revision, &rgba, list);
        assert!(matches!(
            result,
            Err(Error::InvalidInput("empty CPU damage"))
        ));
        fixture.sync();
        assert!(fixture.peer.take_requests().is_empty());
        assert!(fixture.peer.buffers().is_empty());
    }
    assert!(fixture.host.failure().is_none());
    // The rejected attempts consumed nothing: the next present is the first frame.
    let outcome = show(&mut fixture, id, config, &rgba, &[rect(0, 0, 1, 1)]);
    assert_eq!(outcome, PresentOutcome::Presented);
    assert_eq!(damages(&fixture.peer.take_requests()), [(0, 0, 4, 2)]);
}

#[test]
fn sixteen_rects_stay_separate_and_more_collapse_to_their_bounding_box() {
    let (mut fixture, id, config) = cpu_fixture(4, 2);
    let rgba = Image([10; 8]).rgba();
    show(&mut fixture, id, config, &rgba, &[rect(0, 0, 4, 2)]);
    frame_done(&mut fixture);
    fixture.peer.take_requests();
    let sixteen: Vec<_> = (0..16).map(|i| rect(i % 4, i / 8, 1, 1)).collect();
    show(&mut fixture, id, config, &rgba, &sixteen);
    let wire = damages(&fixture.peer.take_requests());
    assert_eq!(wire.len(), 16);
    frame_done(&mut fixture);
    fixture.peer.take_requests();
    let twenty: Vec<_> = (0..20).map(|i| rect(1 + i % 2, i % 2, 1, 1)).collect();
    show(&mut fixture, id, config, &rgba, &twenty);
    // The rects alternate between (1,0) and (2,1): bounding box x 1..3, y 0..2.
    assert_eq!(damages(&fixture.peer.take_requests()), [(1, 0, 2, 2)]);
}

#[test]
fn old_surface_versions_keep_whole_surface_damage() {
    let mut fixture = Fixture::with_options(PeerOptions {
        shm: true,
        surface_version: 3,
        ..Default::default()
    });
    let id = fixture.create(4, 2);
    fixture.host.enable_cpu(id).unwrap();
    let config = fixture.configure(id, 0, 4, 2);
    let rgba = Image([10; 8]).rgba();
    fixture.peer.take_requests();
    show(&mut fixture, id, config, &rgba, &[rect(0, 0, 1, 1)]);
    assert_eq!(damages(&fixture.peer.take_requests()), [(0, 0, 4, 2)]);
    frame_done(&mut fixture);
    show(&mut fixture, id, config, &rgba, &[rect(3, 1, 1, 1)]);
    assert_eq!(damages(&fixture.peer.take_requests()), [(0, 0, 4, 2)]);
}

#[test]
fn resize_damages_the_whole_new_buffer() {
    let (mut fixture, id, config) = cpu_fixture(4, 2);
    show(
        &mut fixture,
        id,
        config,
        &Image([10; 8]).rgba(),
        &[rect(0, 0, 4, 2)],
    );
    frame_done(&mut fixture);
    let resized = fixture.configure(id, 0, 2, 2);
    fixture.peer.take_requests();
    let rgba = [7, 0, 0, 255].repeat(4);
    show(&mut fixture, id, resized, &rgba, &[rect(0, 0, 1, 1)]);
    assert_eq!(damages(&fixture.peer.take_requests()), [(0, 0, 2, 2)]);
}
