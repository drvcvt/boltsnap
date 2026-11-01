#![cfg(feature = "capture")]
//! Raw desktop tiles against the isolated test server: positions, untouched
//! server pixels, and the `None` fallback when tiles cannot represent the desktop.
mod support;
use libway::*;
use std::{sync::atomic::Ordering, time::Duration};
use support::*;

fn options() -> CaptureOptions {
    CaptureOptions {
        timeout: Duration::from_millis(500),
        ..Default::default()
    }
}

fn bytes(frame: &Frame) -> &[u8] {
    match &frame.storage {
        FrameStorage::Cpu(buffer) => buffer.bytes(),
        #[allow(unreachable_patterns)]
        _ => panic!("tiles are CPU frames"),
    }
}

#[test]
fn tiles_sit_at_their_canvas_positions_with_untouched_server_pixels() {
    let (server, socket) = Server::start(Config {
        outputs: 2,
        mode_sizes: vec![(4, 2), (6, 3)],
        native_logical_size: true,
        hold_ready: 2,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let desktop = c.capture_desktop_tiles(&opts).unwrap().unwrap();
    assert_eq!((desktop.width, desktop.height, desktop.scale), (10, 3, 1.0));
    assert_eq!(
        desktop.logical_bounds,
        Rect {
            x: -4,
            y: 0,
            width: 10,
            height: 3
        }
    );
    let placed: Vec<_> = desktop
        .tiles
        .iter()
        .map(|t| (t.x, t.y, t.frame.width, t.frame.height, t.frame.stride))
        .collect();
    assert_eq!(placed, [(0, 0, 4, 2, 16), (4, 0, 6, 3, 24)]);
    for tile in &desktop.tiles {
        // The server offers XRGB8888 only; the tile keeps it, no RGBA conversion.
        assert_eq!(tile.frame.format, PixelFormat::Xrgb8888);
    }
    // Output 0 paints 0x00102030 + (x << 16) + (y << 8), little endian.
    let first = bytes(&desktop.tiles[0].frame);
    assert_eq!(&first[..4], &[0x30, 0x20, 0x10, 0x00]);
    let at = (16 + 4) as usize;
    assert_eq!(&first[at..at + 4], &[0x30, 0x21, 0x11, 0x00]);
    // Output 1 paints a constant 0x00a0b0c0.
    assert!(
        bytes(&desktop.tiles[1].frame)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0xc0, 0xb0, 0xa0, 0x00])
    );
    assert_eq!(server.metrics.captures.load(Ordering::SeqCst), 2);
}

#[test]
fn mixed_scales_or_a_rotated_output_fall_back_without_capturing() {
    for config in [
        Config {
            outputs: 2,
            mode_sizes: vec![(4, 2), (8, 4)],
            ..Default::default()
        },
        Config {
            transform: wayland_server::protocol::wl_output::Transform::_90,
            ..Default::default()
        },
    ] {
        let (server, socket) = Server::start(config);
        let opts = options();
        let mut c = Connection::from_socket(socket, &opts).unwrap();
        assert!(c.capture_desktop_tiles(&opts).unwrap().is_none());
        c.outputs(&opts).unwrap();
        server.settle();
        assert_eq!(server.metrics.sessions.load(Ordering::SeqCst), 0);
        assert_eq!(server.metrics.captures.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn a_y_inverted_frame_falls_back_and_releases_its_buffers() {
    let (server, socket) = Server::start(Config {
        ext: false,
        inverted: true,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    assert!(c.capture_desktop_tiles(&opts).unwrap().is_none());
    c.outputs(&opts).unwrap();
    server.settle();
    assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
}

#[test]
fn no_outputs_is_an_error_like_the_composed_path() {
    let (_server, socket) = Server::start(Config {
        outputs: 0,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    assert!(matches!(
        c.capture_desktop_tiles(&opts),
        Err(Error::NoOutputs)
    ));
}
