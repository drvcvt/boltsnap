//! Synthetic Linux transport/encode/thumbnail benchmark, no daemon/desktop.
//! cargo run --release --example image_transfer_bench
#![allow(dead_code)]
#[cfg(target_os = "linux")]
pub use boltsnap::protocol;
#[cfg(target_os = "linux")]
#[path = "../src/platform/linux/image_transfer.rs"]
mod image_transfer;
#[path = "../src/shelf/thumbnail.rs"]
mod thumbnail;
#[cfg(target_os = "linux")]
#[path = "../src/platform/linux/timing.rs"]
mod timing;
#[cfg(target_os = "linux")]
mod ipc {
    pub fn ensure_daemon() -> std::io::Result<std::os::unix::net::UnixStream> {
        Err(std::io::Error::other(
            "benchmark never connects to the daemon",
        ))
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {}
#[cfg(target_os = "linux")]
fn main() {
    use image::ImageEncoder;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    for (w, h) in [(1920, 1080), (3840, 2160)] {
        for noise in [false, true] {
            let mut seed = 1u32;
            let source = image::RgbImage::from_fn(w, h, |x, y| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                if noise {
                    image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
                } else {
                    image::Rgb([(x / 8) as u8, (y / 8) as u8, 80])
                }
            });
            for raw in [false, true] {
                let (mut client, mut server) = UnixStream::pair().unwrap();
                let worker = std::thread::spawn(move || {
                    for _ in 0..105 {
                        let (header, png) = protocol::read_frame(&mut server).unwrap();
                        let thumb;
                        let encoded;
                        if raw {
                            let meta = image_transfer::Metadata::parse(
                                &serde_json::from_slice(&header).unwrap(),
                            )
                            .unwrap();
                            protocol::write_frame(&mut server, b"{}", &[]).unwrap();
                            let fd = image_transfer::receive_fd(&server).unwrap();
                            let pixels = image_transfer::Pixels::map(fd, meta.len).unwrap();
                            let image = image::ImageBuffer::from_raw(w, h, pixels.bytes()).unwrap();
                            thumb = thumbnail::make_rgb_card_thumbnail(&image, 190, 132);
                            let mut png = Vec::new();
                            image::codecs::png::PngEncoder::new(&mut png)
                                .write_image(pixels.bytes(), w, h, image::ExtendedColorType::Rgb8)
                                .unwrap();
                            encoded = png;
                        } else {
                            let image =
                                image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                                    .unwrap()
                                    .into_rgba8();
                            thumb = thumbnail::make_card_thumbnail(&image, 190, 132);
                            encoded = png;
                        }
                        std::hint::black_box(&encoded);
                        protocol::write_frame(&mut server, b"{}", thumb.as_raw()).unwrap();
                    }
                });
                let meta =
                    image_transfer::Metadata::new(w, h, "bench".into(), None, false).unwrap();
                let mut samples = Vec::new();
                for i in 0..105 {
                    let start = std::time::Instant::now();
                    if raw {
                        protocol::write_frame(&mut client, &meta.header(), &[]).unwrap();
                        protocol::read_frame(&mut client).unwrap();
                        let fd = image_transfer::sealed_pixels(source.as_raw()).unwrap();
                        image_transfer::send_fd(&client, &fd).unwrap();
                    } else {
                        let mut png = Vec::new();
                        image::codecs::png::PngEncoder::new(&mut png)
                            .write_image(source.as_raw(), w, h, image::ExtendedColorType::Rgb8)
                            .unwrap();
                        // The existing Request encoder also allocates one contiguous frame.
                        client
                            .write_all(
                                &protocol::Request::Add {
                                    source: "bench".into(),
                                    png,
                                    output: None,
                                    thumb: None,
                                }
                                .encode(),
                            )
                            .unwrap();
                    }
                    let (_, thumb) = protocol::read_frame(&mut client).unwrap();
                    std::hint::black_box(thumb);
                    if i >= 5 {
                        samples.push(start.elapsed().as_secs_f64() * 1000.);
                    }
                }
                worker.join().unwrap();
                samples.sort_by(f64::total_cmp);
                println!(
                    "{w}x{h} noise={noise} raw={raw}: median={:.3}ms p95={:.3}ms",
                    samples[50], samples[94]
                );
            }
        }
    }
}
