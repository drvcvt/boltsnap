//! Offscreen renders of the selector chrome with the user's UI font, for
//! looking at and for byte comparisons between changes:
//! `SEL_OUT=dir [SEL_BASE=backdrop.png] cargo test --bin boltsnap selector_shots -- --ignored`.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::selector::render;
use crate::selector::scene::{Scene, paint};

/// The backdrop: `SEL_BASE`, else a calm gradient, cut to `w`×`h`.
fn backdrop(w: u32, h: u32) -> RgbaImage {
    let source = std::env::var_os("SEL_BASE")
        .and_then(|path| image::open(path).ok())
        .map(|image| image.into_rgba8());
    match source {
        Some(image) if image.width() >= w && image.height() >= h => {
            image::imageops::crop_imm(&image, 0, 0, w, h).to_image()
        }
        Some(image) => image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle),
        None => RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x * 200 / w) as u8 + 30, (y * 160 / h) as u8 + 40, 120, 255])
        }),
    }
}

fn save(frame: &Pixmap, path: &Path) {
    let image = RgbaImage::from_raw(frame.width(), frame.height(), frame.data().to_vec())
        .expect("frame size");
    image.save(path).expect("write shot");
}

fn scene(selection: (f32, f32, f32, f32), record: bool) -> Scene {
    Scene {
        chrome_viewport: None,
        selection: Some(selection),
        editing: true,
        record,
        toggles: (true, true, true),
        hovered: None,
        magnifier: None,
    }
}

/// (name, surface size, scene).
fn shots() -> Vec<(&'static str, (u32, u32), Scene)> {
    let sel = (612.0, 300.0, 760.0, 430.0);
    let mut out = vec![("rest", (1920, 1080), scene(sel, true))];
    for (index, name) in ["hover-record", "hover-clip", "hover-frame", "hover-audio"]
        .into_iter()
        .enumerate()
    {
        let mut s = scene(sel, true);
        s.hovered = Some(index);
        out.push((name, (1920, 1080), s));
    }
    let mut off = scene(sel, true);
    off.toggles = (false, false, false);
    out.push(("noclip-off", (1920, 1080), off.clone()));
    off.hovered = Some(1);
    out.push(("noclip-hover", (1920, 1080), off));
    out.push(("grid", (300, 300), scene((40.0, 160.0, 200.0, 100.0), true)));
    out.push((
        "column",
        (160, 240),
        scene((20.0, 160.0, 100.0, 60.0), true),
    ));
    out.push((
        "below",
        (1920, 1080),
        scene((300.0, 4.0, 700.0, 400.0), true),
    ));
    out.push((
        "inside",
        (1920, 1080),
        scene((0.0, 0.0, 1920.0, 1080.0), true),
    ));
    out.push((
        "edge",
        (1920, 1080),
        scene((1890.0, 1050.0, 30.0, 30.0), true),
    ));
    let mut shot = scene((612.4, 300.6, 760.3, 430.2), false);
    out.push(("screenshot", (1920, 1080), shot.clone()));
    shot.editing = false;
    out.push(("screenshot-drawing", (1920, 1080), shot));
    out
}

#[test]
#[ignore = "writes PNGs to SEL_OUT for looking at"]
fn selector_shots() {
    let out = PathBuf::from(std::env::var_os("SEL_OUT").expect("SEL_OUT=dir"));
    std::fs::create_dir_all(&out).expect("SEL_OUT");
    let chrome = render::Chrome::new(super::ui_fonts());
    for (name, (w, h), scene) in shots() {
        let base = render::base_pixmap_from_image(&backdrop(w, h), w, h);
        let mut overlay = render::CachedOverlay::new(&base);
        let frame = overlay.reset(&base, scene.selection);
        paint(frame, &base, &scene, &chrome);
        save(&frame.pixmap, &out.join(format!("{name}.png")));
    }
    // The gates with the user's face.
    for ((w, h), sel) in [
        ((1920, 1080), (612.0, 300.0, 760.0, 430.0)),
        ((300, 300), (40.0, 160.0, 200.0, 100.0)),
        ((160, 240), (20.0, 180.0, 100.0, 40.0)),
    ] {
        for on in [true, false] {
            let report = crate::selector::render::gates::measure(&chrome.fonts, (w, h), sel, on);
            match crate::selector::render::gates::check(&report, on) {
                Ok(line) => println!("gate geometry (user face, {w}x{h}, on {on}): {line}"),
                Err(error) => panic!("{w}x{h} on {on}: {error}"),
            }
        }
    }
}
