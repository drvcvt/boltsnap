//! Reused SHM slots hold the full frame, but only damaged regions are rewritten.
//! Frames below lie by changing a pixel outside their declared damage, so a slot
//! that kept its old value there proves that no full copy happened.
use super::*;

const LIE: usize = 7;

fn with(image: Image, pixel: usize, value: u8) -> Image {
    let mut image = image;
    image.0[pixel] = value;
    image
}

fn dot(pixel: usize) -> PixelRect {
    PixelRect::new(pixel as u32 % 4, pixel as u32 / 4, 1, 1)
}

fn setup() -> (Fixture, WindowId, Configuration) {
    let (fixture, id, config) = cpu_fixture(4, 2);
    fixture.peer.take_requests();
    (fixture, id, config)
}

#[test]
fn slot_two_frames_stale_gets_every_region_changed_since_its_frame() {
    let (mut fixture, id, config) = setup();
    let f1 = Image([10; 8]);
    let f2 = with(f1, 0, 20);
    let f3 = with(with(f2, 6, 30), LIE, 99);
    show(&mut fixture, id, config, &f1.rgba(), &[dot(0)]);
    frame_done(&mut fixture);
    show(&mut fixture, id, config, &f2.rgba(), &[dot(0)]);
    frame_done(&mut fixture);
    release(&mut fixture, 0);
    show(&mut fixture, id, config, &f3.rgba(), &[dot(6)]);
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 2);
    // Pixel 0 comes from frame 2's damage, pixel 6 from frame 3's, and the
    // undeclared pixel keeps frame 1's value.
    assert_eq!(buffers[0].bytes, with(f3, LIE, 10).native());
    assert_eq!(buffers[1].bytes, f2.native());
}

#[test]
fn fresh_slot_is_fully_written_whatever_the_damage() {
    let (mut fixture, id, config) = setup();
    let f1 = Image([10; 8]);
    let f2 = with(f1, LIE, 99);
    show(&mut fixture, id, config, &f1.rgba(), &[dot(0)]);
    assert_eq!(fixture.peer.buffers()[0].bytes, f1.native());
    frame_done(&mut fixture);
    // Frame 1 is still busy, so frame 2 gets a fresh slot.
    show(&mut fixture, id, config, &f2.rgba(), &[dot(0)]);
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 2);
    assert_eq!(buffers[1].bytes, f2.native());
}

#[test]
fn slot_after_resize_and_back_is_fully_rewritten() {
    let (mut fixture, id, config) = setup();
    let f1 = Image([10; 8]);
    show(&mut fixture, id, config, &f1.rgba(), &[dot(0)]);
    frame_done(&mut fixture);
    let small = fixture.configure(id, 0, 2, 2);
    show(
        &mut fixture,
        id,
        small,
        &[5, 0, 0, 255].repeat(4),
        &[dot(0)],
    );
    frame_done(&mut fixture);
    let back = fixture.configure(id, 0, 4, 2);
    release(&mut fixture, 0);
    // Slot 0 holds frame 1 in the matching layout, but a frame of another
    // layout lies between, so its recorded damage cannot be combined.
    let f3 = with(Image([40; 8]), LIE, 99);
    show(&mut fixture, id, back, &f3.rgba(), &[dot(0)]);
    assert_eq!(fixture.peer.buffers()[0].bytes, f3.native());
}

#[test]
fn slot_older_than_the_history_ring_is_fully_rewritten() {
    let (mut fixture, id, config) = setup();
    let mut image = Image([10; 8]);
    show(&mut fixture, id, config, &image.rgba(), &[dot(0)]);
    frame_done(&mut fixture);
    // Frames 2 to 7 alternate through slot 1 while slot 0 keeps frame 1.
    for (pixel, value) in (0..6).zip(20..) {
        image = with(image, pixel, value);
        show(&mut fixture, id, config, &image.rgba(), &[dot(pixel)]);
        frame_done(&mut fixture);
        if pixel < 5 {
            release(&mut fixture, 1);
        }
    }
    release(&mut fixture, 0);
    let last = with(with(image, 6, 90), LIE, 99);
    show(&mut fixture, id, config, &last.rgba(), &[dot(6)]);
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 2);
    assert_eq!(buffers[0].bytes, last.native());
}

#[test]
fn recent_slot_keeps_old_content_outside_the_damage() {
    let (mut fixture, id, config) = setup();
    let f1 = Image([10; 8]);
    let f2 = with(f1, LIE, 99);
    show(&mut fixture, id, config, &f1.rgba(), &[dot(0)]);
    frame_done(&mut fixture);
    release(&mut fixture, 0);
    // Slot 0 holds the immediately preceding frame, so only frame 2's damage
    // is rewritten; the undeclared change stays unwritten.
    show(&mut fixture, id, config, &with(f2, 3, 50).rgba(), &[dot(3)]);
    let expected = with(f1, 3, 50);
    assert_eq!(fixture.peer.buffers()[0].bytes, expected.native());
}
