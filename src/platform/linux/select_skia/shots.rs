//! Offscreen renders of the selector chrome with the user's UI font, for
//! looking at and for byte comparisons between changes, and the `/ui`
//! geometry gates measured in that font:
//! `SEL_OUT=dir [SEL_BASE=backdrop.png] cargo test --bin boltsnap selector_shots -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use boltsnap::ui::Rect;
use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::selector::clip::ClipState;
use crate::selector::motion::{Bar, BarMotion, Targets};
use crate::selector::render::{self, Chrome, toolbar};
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

fn scene(selection: (f32, f32, f32, f32), record: bool, bar: Bar) -> Scene {
    Scene {
        chrome_viewport: None,
        selection: Some(selection),
        editing: true,
        record,
        bar,
        magnifier: None,
    }
}

fn targets(hovered: Option<usize>, clip: bool, on: bool) -> Targets {
    Targets {
        hovered,
        pressed: None,
        on: [false, false, on, on],
        enabled: [true, clip, true, true],
        reason: (!clip).then(|| ClipState::Loading.reason()).flatten(),
    }
}

/// One step of a toolbar's life: the selection and what it shows, `ms`
/// after the previous step.
struct Step {
    sel: (f32, f32, f32, f32),
    targets: Targets,
    ms: u64,
}

fn step(sel: (f32, f32, f32, f32), targets: Targets, ms: u64) -> Step {
    Step { sel, targets, ms }
}

/// The toolbar after `steps` in a `w`×`h` area, sampled `sample` ms after
/// the last step.
fn animated(chrome: &Chrome, (w, h): (u32, u32), steps: &[Step], sample: u64) -> Bar {
    let area = Rect::new(0.0, 0.0, w as f32, h as f32);
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&steps[0].targets, t0);
    let mut at = t0;
    for step in steps {
        at += Duration::from_millis(step.ms);
        let layout = toolbar::layout(step.sel, area, &chrome.toolbar);
        motion.update(layout, &step.targets, at);
    }
    motion.sample(at + Duration::from_millis(sample))
}

/// (name, surface size, scene).
fn shots(chrome: &Chrome) -> Vec<(String, (u32, u32), Scene)> {
    let full = (1920, 1080);
    let sel = (612.0, 300.0, 760.0, 430.0);
    let rest =
        |sel, size, t: Targets| scene(sel, true, animated(chrome, size, &[step(sel, t, 0)], 1000));
    let mut out = vec![(
        "rest".to_string(),
        full,
        rest(sel, full, targets(None, true, true)),
    )];
    for (index, name) in ["record", "clip", "frame", "audio"].into_iter().enumerate() {
        let t = targets(Some(index), true, true);
        out.push((format!("hover-{name}"), full, rest(sel, full, t)));
    }
    out.push((
        "off".into(),
        full,
        rest(sel, full, targets(None, true, false)),
    ));
    let t = targets(Some(3), true, false);
    out.push(("off-hover-audio".into(), full, rest(sel, full, t)));
    out.push((
        "noclip".into(),
        full,
        rest(sel, full, targets(None, false, true)),
    ));
    out.push((
        "noclip-hover".into(),
        full,
        rest(sel, full, targets(Some(1), false, true)),
    ));
    let below = (300.0, 4.0, 700.0, 400.0);
    let t = targets(Some(1), false, true);
    out.push(("noclip-hover-below".into(), full, rest(below, full, t)));
    let column = (20.0, 180.0, 100.0, 40.0);
    out.push((
        "noclip-hover-column".into(),
        (160, 240),
        rest(column, (160, 240), t),
    ));
    for ms in [30, 60, 100] {
        let steps = [step(sel, targets(None, false, true), 0), step(sel, t, 500)];
        let bar = animated(chrome, full, &steps, ms);
        out.push((format!("hint-in-{ms}ms"), full, scene(sel, true, bar)));
    }
    let grid = (40.0, 160.0, 200.0, 100.0);
    out.push((
        "grid".into(),
        (300, 300),
        rest(grid, (300, 300), targets(Some(2), true, true)),
    ));
    let t = targets(Some(0), true, false);
    out.push(("column".into(), (160, 240), rest(column, (160, 240), t)));
    let below = (300.0, 4.0, 700.0, 400.0);
    out.push((
        "below".into(),
        full,
        rest(below, full, targets(None, true, true)),
    ));
    let inside = (0.0, 0.0, 1920.0, 1080.0);
    out.push((
        "inside".into(),
        full,
        rest(inside, full, targets(None, true, true)),
    ));
    let edge = (1890.0, 1050.0, 30.0, 30.0);
    out.push((
        "edge".into(),
        full,
        rest(edge, full, targets(None, true, true)),
    ));
    // Mid-animation: appearing, hover coming in, a toggle switching off, a
    // switch from above to below the selection crossfading.
    let t = targets(None, true, true);
    let moved = (612.0, 40.0, 760.0, 430.0);
    for ms in [30, 60, 100] {
        let shot = |steps: &[Step], sel| scene(sel, true, animated(chrome, full, steps, ms));
        out.push((
            format!("appear-{ms}ms"),
            full,
            shot(&[step(sel, t, 0)], sel),
        ));
        let steps = [
            step(sel, t, 0),
            step(sel, targets(Some(0), true, true), 500),
        ];
        out.push((format!("hover-in-{ms}ms"), full, shot(&steps, sel)));
        let steps = [
            step(sel, targets(Some(3), true, true), 0),
            step(sel, targets(Some(3), true, false), 500),
        ];
        out.push((format!("audio-off-{ms}ms"), full, shot(&steps, sel)));
        let steps = [step(sel, t, 0), step(moved, t, 500)];
        out.push((format!("switch-{ms}ms"), full, shot(&steps, moved)));
    }
    let mut shot = scene((612.4, 300.6, 760.3, 430.2), false, Bar::default());
    out.push(("screenshot".into(), full, shot.clone()));
    shot.editing = false;
    out.push(("screenshot-drawing".into(), full, shot));
    out
}

#[test]
#[ignore = "writes PNGs to SEL_OUT for looking at"]
fn selector_shots() {
    let out = PathBuf::from(std::env::var_os("SEL_OUT").expect("SEL_OUT=dir"));
    std::fs::create_dir_all(&out).expect("SEL_OUT");
    let chrome = Chrome::new(super::ui_fonts());
    for (name, (w, h), scene) in shots(&chrome) {
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
            let report = render::gates::measure(&chrome.fonts, (w, h), sel, on);
            match render::gates::check(&report, on) {
                Ok(line) => println!("gate geometry (user face, {w}x{h}, on {on}): {line}"),
                Err(error) => panic!("{w}x{h} on {on}: {error}"),
            }
        }
    }
}
