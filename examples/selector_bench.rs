//! Synthetic CPU/render benchmark. Does not connect to or capture a desktop.
//! cargo run --release --example selector_bench -- /tmp/selector-ui.png
#![allow(dead_code)]
#[path = "../src/shelf/font.rs"]
pub mod font;
#[path = "../src/image.rs"]
mod image_model;
mod shelf {
    pub use crate::font;
}
#[path = "../src/selector/desktop.rs"]
pub mod desktop;
#[path = "../src/selector/edit.rs"]
pub mod edit;
#[path = "../src/selector/render.rs"]
pub mod render;
#[path = "../src/selector/scene.rs"]
pub mod scene;
mod selector {
    pub use crate::edit;
}
use scene::{Scene, SceneCache};

fn main() {
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        let mut base = tiny_skia::Pixmap::new(w, h).unwrap();
        base.fill(tiny_skia::Color::from_rgba8(73, 120, 181, 255));
        for scenario in ["drag", "hover", "loupe", "large-change"] {
            for regional in [false, true] {
                let mut cache = SceneCache::new(&base);
                let mut old = render::CachedOverlay::new(&base);
                let mut slots: Vec<_> = (0..3)
                    .map(|_| (None, vec![0u8; (w * h * 4) as usize]))
                    .collect();
                let mut times = Vec::new();
                let mut total_pixels = 0u64;
                for i in 0..105 {
                    let offset = if scenario == "large-change" {
                        (i % 2 * 800) as f32
                    } else if scenario == "drag" {
                        (i % 10) as f32
                    } else {
                        0.
                    };
                    let state = Scene {
                        chrome_viewport: None,
                        selection: Some((40. + offset, 90., w as f32 * 0.7, h as f32 * 0.7)),
                        editing: true,
                        record: scenario == "hover",
                        toggles: (true, true, false),
                        hovered: if scenario == "hover" {
                            Some(i % 4)
                        } else {
                            None
                        },
                        magnifier: (scenario == "loupe").then_some((300. + (i % 30) as f64, 240.)),
                    };
                    let slot = &mut slots[i % 3];
                    let started = std::time::Instant::now();
                    if regional {
                        cache.update(std::hint::black_box(&base), state);
                        let damage = cache.damage_since(slot.0);
                        cache.copy_regions(&mut slot.1, &damage);
                        slot.0 = Some(cache.generation());
                        if i >= 5 {
                            total_pixels += damage
                                .iter()
                                .map(|r| (r.2 - r.0) as u64 * (r.3 - r.1) as u64)
                                .sum::<u64>();
                        }
                    } else {
                        let frame = old.reset(std::hint::black_box(&base), state.selection);
                        scene::paint(frame, &base, &state);
                        render::pixmap_to_argb8888(frame, &mut slot.1);
                        if i >= 5 {
                            total_pixels += w as u64 * h as u64;
                        }
                    }
                    std::hint::black_box(&slot.1);
                    if i >= 5 {
                        times.push(started.elapsed().as_secs_f64() * 1000.);
                    }
                }
                times.sort_by(f64::total_cmp);
                println!(
                    "{w}x{h} {scenario} regional={regional}: median={:.3}ms p95={:.3}ms copied_bytes/frame={}",
                    times[50],
                    times[94],
                    total_pixels * 4 / 100
                );
            }
        }
    }
    if let Some(path) = std::env::args().nth(1) {
        let mut base = tiny_skia::Pixmap::new(960, 540).unwrap();
        base.fill(tiny_skia::Color::from_rgba8(54, 66, 83, 255));
        let mut cache = SceneCache::new(&base);
        cache.update(
            &base,
            Scene {
                chrome_viewport: None,
                selection: Some((100., 180., 740., 280.)),
                editing: true,
                record: true,
                toggles: (true, true, false),
                hovered: Some(0),
                magnifier: None,
            },
        );
        cache.frame().save_png(path).unwrap();
    }
}
