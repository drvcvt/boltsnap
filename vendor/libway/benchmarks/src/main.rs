#[path = "../../tests/support/mod.rs"]
mod support;
use std::{hint::black_box, time::Instant};
use support::{Config, Server};
fn measure(label: &str, mut capture: impl FnMut()) {
    for _ in 0..5 {
        capture();
    }
    let mut samples = Vec::new();
    for _ in 0..40 {
        let start = Instant::now();
        capture();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{label}: median={:.3}ms p95={:.3}ms",
        samples[20], samples[37]
    );
}
fn main() {
    for ext in [false, true] {
        for size in [(1920, 1080), (3840, 2160)] {
            let config = Config {
                ext,
                mode_sizes: vec![size],
                native_logical_size: true,
                ..Default::default()
            };
            let (_server, socket) = Server::start(config.clone());
            let options = libway::CaptureOptions::default();
            let mut connection = libway::Connection::from_socket(socket, &options).unwrap();
            measure(&format!("libway EXT={ext} {size:?}"), || {
                let frame = connection.capture_desktop(&options).unwrap();
                assert_eq!(frame.image.dimensions(), size);
                assert_eq!(frame.image.get_pixel(0, 0).0, [16, 32, 48, 255]);
                black_box(frame);
            });
            if ext {
                println!(
                    "libwayshot EXT comparison skipped: 0.7.3 omits required initial buffer damage"
                );
                continue;
            }
            let (_server, socket) = Server::start(config);
            let connection = libwayshot::WayshotConnection::from_connection(
                wayland_client::Connection::from_socket(socket).unwrap(),
            )
            .unwrap();
            measure(&format!("libwayshot EXT={ext} {size:?}"), || {
                let frame = connection.screenshot_all(false).unwrap().into_rgba8();
                assert_eq!(frame.dimensions(), size);
                // libwayshot 0.7.3 preserves the unused X byte as alpha.
                assert_eq!(&frame.get_pixel(0, 0).0[..3], &[16, 32, 48]);
                black_box(frame);
            });
        }
    }
}
