//! The Linux chrome: border, grips, badge, toolbar layout and its gates.

use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, CTL_H, GAP_S, GAP_XS, Rgb, TextStyle};
use boltsnap::ui::{Canvas, Rect};
use tiny_skia::Pixmap;

use super::chrome::{Placement, SIDE, draw_frame, draw_grips, size_badge_rect};
use super::gates;
use super::toolbar::{self, LABELS, Look, Metrics};
use crate::selector::clip::ClipState;

fn black(w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w, h).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    pm
}

fn at(pm: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * pm.width() + x) * 4) as usize;
    pm.data()[i..i + 4].try_into().unwrap()
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const CONTOUR: [u8; 4] = [theme::WINDOW.0, theme::WINDOW.1, theme::WINDOW.2, 255];

#[test]
fn the_border_rings_the_crop_on_whole_pixels_and_redraws_to_the_same_pixels() {
    // Rounds to the crop 10..61 × 21..51.
    let sel = (10.4, 20.6, 50.2, 30.3);
    let mut pm = black(80, 70);
    draw_frame(&mut pm, sel);
    assert_eq!(at(&pm, 9, 30), WHITE);
    assert_eq!(at(&pm, 8, 30), CONTOUR);
    assert_eq!(at(&pm, 10, 30), [0, 0, 0, 255], "the crop stays clear");
    assert_eq!(at(&pm, 61, 30), WHITE);
    assert_eq!(at(&pm, 62, 30), CONTOUR);
    assert_eq!(at(&pm, 30, 20), WHITE);
    assert_eq!(at(&pm, 30, 19), CONTOUR);
    assert_eq!(at(&pm, 30, 51), WHITE);
    assert_eq!(at(&pm, 30, 21), [0, 0, 0, 255]);
    // Nothing in between: every pixel is ring, contour or untouched.
    for pixel in pm.data().chunks_exact(4) {
        assert!(pixel == WHITE || pixel == CONTOUR || pixel == [0, 0, 0, 255]);
    }
    let once = pm.clone();
    draw_frame(&mut pm, sel);
    draw_grips(&mut pm, sel);
    let twice_grips = pm.clone();
    draw_grips(&mut pm, sel);
    assert_eq!(pm.data(), twice_grips.data(), "grips redraw idempotently");
    let mut again = once.clone();
    draw_frame(&mut again, sel);
    assert_eq!(again.data(), once.data(), "the border redraws idempotently");
}

#[test]
fn grips_are_eight_pixels_centred_on_the_crop_edges() {
    let sel = (40.0, 40.0, 101.0, 60.0); // crop 40..141 × 40..100
    let mut pm = black(200, 160);
    draw_grips(&mut pm, sel);
    // Top-left: 36..44, contour 35 and 44.
    for x in 36..44 {
        assert_eq!(at(&pm, x, 40), WHITE, "x {x}");
    }
    assert_eq!(at(&pm, 35, 40), CONTOUR);
    assert_eq!(at(&pm, 44, 40), CONTOUR);
    assert_eq!(at(&pm, 40, 36), WHITE);
    assert_eq!(at(&pm, 40, 35), CONTOUR);
    // The top middle sits on round(90.5) = 91 (ties away from zero).
    assert_eq!(at(&pm, 87, 40), WHITE);
    assert_eq!(at(&pm, 94, 40), WHITE);
    assert_eq!(at(&pm, 95, 40), CONTOUR);
    // Right edge middle at y 70.
    assert_eq!(at(&pm, 141, 66), WHITE);
    assert_eq!(at(&pm, 141, 73), WHITE);
    assert_eq!(at(&pm, 141, 74), CONTOUR);
}

#[test]
fn the_size_badge_keeps_its_width_while_the_numbers_change_and_stays_on_its_monitor() {
    let fonts = Fonts::fallback();
    let area = Rect::new(320.0, 80.0, 320.0, 280.0);
    let a = size_badge_rect(&fonts, (400.0, 200.0, 111.0, 111.0), area).unwrap();
    let b = size_badge_rect(&fonts, (400.0, 200.0, 999.0, 888.0), area).unwrap();
    assert_eq!(a.w, b.w, "tabular digits");
    assert_eq!(a.h, CTL_H);
    for sel in [
        (200.0, 30.0, 250.0, 180.0),
        (630.0, 350.0, 10.0, 10.0),
        (320.0, 80.0, 320.0, 280.0),
    ] {
        let r = size_badge_rect(&fonts, sel, area).unwrap();
        assert!(
            r.x >= area.x + GAP_S
                && r.y >= area.y + GAP_S
                && r.right() <= area.right() - GAP_S
                && r.bottom() <= area.bottom() - GAP_S,
            "{sel:?} -> {r:?}"
        );
        assert_eq!((r.x.fract(), r.y.fract()), (0.0, 0.0));
    }
}

#[test]
fn the_toolbar_takes_a_strip_a_grid_or_a_column_by_the_room_there_is() {
    let metrics = Metrics::new(&Fonts::fallback());
    let layout = |w: f32, h: f32| {
        toolbar::layout(
            (20.0, h - 70.0, 60.0, 40.0),
            Rect::new(0.0, 0.0, w, h),
            &metrics,
        )
    };
    assert_eq!(layout(1920.0, 1080.0).unwrap().columns, 4);
    assert_eq!(layout(300.0, 300.0).unwrap().columns, 2);
    assert_eq!(layout(160.0, 240.0).unwrap().columns, 1);
    assert!(layout(60.0, 60.0).is_none());
}

/// Gate 1: wherever the selection is, on every screen size, the toolbar
/// stays `GAP_S` inside the screen, on whole pixels, its controls inside it
/// and apart, `GAP_XS` around and between them.
#[test]
fn gate_overflow_the_toolbar_stays_on_screen_everywhere() {
    let metrics = Metrics::new(&Fonts::fallback());
    let mut cases = 0;
    for (w, h) in [
        (1920u32, 1080u32),
        (400, 300),
        (300, 300),
        (240, 200),
        (160, 240),
    ] {
        let (wf, hf) = (w as f32, h as f32);
        for sel in [
            (0.0, 0.0, wf, hf),
            (wf - 30.0, hf - 30.0, 20.0, 20.0),
            (0.0, 0.0, 4.0, 4.0),
            (20.25, 70.5, 80.0, 40.0),
            (wf - 5.0, 0.0, 5.0, hf),
            (0.0, hf / 2.0, wf, 6.0),
            (wf / 3.0, 2.0, 10.0, hf - 4.0),
        ] {
            let area = Rect::new(0.0, 0.0, wf, hf);
            let t = toolbar::layout(sel, area, &metrics).unwrap();
            let b = t.bounds;
            assert!(
                b.x >= GAP_S && b.y >= GAP_S && b.right() <= wf - GAP_S && b.bottom() <= hf - GAP_S,
                "{w}x{h} {sel:?}: {b:?}"
            );
            assert_eq!((b.x.fract(), b.y.fract()), (0.0, 0.0));
            for (i, c) in t.controls.iter().enumerate() {
                assert!(c.x >= b.x + GAP_XS && c.y >= b.y + GAP_XS);
                assert!(c.right() <= b.right() - GAP_XS && c.bottom() <= b.bottom() - GAP_XS);
                assert_eq!(c.h, CTL_H);
                for d in &t.controls[i + 1..] {
                    // At least `GAP_XS` apart.
                    assert!(!c.inset(-(GAP_XS - 0.01)).overlaps(*d));
                }
            }
            // The reason beside it, the longest one, inside the screen too,
            // and not over a toolbar.
            for reason in ClipState::ALL.iter().filter_map(|c| c.reason()) {
                let fonts = Fonts::fallback();
                let (hint, text) = toolbar::hint_rect(&fonts, &t, reason, area);
                assert!(
                    hint.x >= GAP_S
                        && hint.y >= GAP_S
                        && hint.right() <= wf - GAP_S
                        && hint.bottom() <= hf - GAP_S,
                    "{w}x{h} {sel:?}: hint {hint:?}"
                );
                assert!(fonts.width(TextStyle::Label, &text) + 2.0 * SIDE <= hint.w + 0.5);
                assert!(
                    !hint.overlaps(t.bounds),
                    "{w}x{h} {sel:?}: hint over the toolbar"
                );
                assert_eq!((hint.x.fract(), hint.y.fract()), (0.0, 0.0));
            }
            cases += 1;
        }
    }
    println!(
        "gate overflow: {cases} placements on 5 screen sizes, toolbar and each of the six \
         reasons inside the screen"
    );
    let t = toolbar::layout(
        (90.0, 90.0, 400.0, 200.0),
        Rect::new(0.0, 0.0, 800.0, 500.0),
        &metrics,
    )
    .unwrap();
    assert_eq!((t.bounds.x, t.placement), (90.0, Placement::Above));
    assert!(t.bounds.bottom() + theme::GAP_M <= 90.0);
}

#[test]
fn highlights_stay_inside_the_toolbars_contour() {
    let fonts = Fonts::fallback();
    let metrics = Metrics::new(&fonts);
    for (w, h) in [(800u32, 500u32), (300, 300), (160, 240)] {
        for offset in [0.0, 0.25, 0.5, 0.75] {
            let area = Rect::new(0.0, 0.0, w as f32, h as f32);
            let layout =
                toolbar::layout((20.0 + offset, 150.0 + offset, 80.0, 40.0), area, &metrics)
                    .unwrap();
            let mut outline = Canvas::new(w, h, 1).unwrap();
            outline.fill(layout.bounds, theme::RADIUS, theme::PANEL, 1.0);
            for hovered in 0..4 {
                let mut drawn = Canvas::new(w, h, 1).unwrap();
                let look = Look::at_rest(true, true, true, Some(hovered));
                toolbar::draw(&mut drawn, &fonts, &layout, &look);
                for (a, b) in outline.pixmap.pixels().iter().zip(drawn.pixmap.pixels()) {
                    assert_eq!(a.alpha(), b.alpha(), "a highlight changes the contour");
                }
            }
        }
    }
}

#[test]
fn gate_capitals_padding_and_icons_measure_on_the_pixels() {
    let fonts = Fonts::fallback();
    for ((w, h), sel) in [
        ((1920, 1080), (612.0, 300.0, 760.0, 430.0)),
        ((300, 300), (40.0, 160.0, 200.0, 100.0)),
        ((160, 240), (20.0, 180.0, 100.0, 40.0)),
    ] {
        for on in [true, false] {
            let report = gates::measure(&fonts, (w, h), sel, on);
            match gates::check(&report, on) {
                Ok(line) => println!("gate geometry (embedded face, on {on}): {line}"),
                Err(error) => panic!("{w}x{h} on {on}: {error}"),
            }
        }
    }
}

#[test]
fn the_capitals_and_icon_measure_see_a_one_pixel_shift() {
    let fonts = Fonts::fallback();
    let mut report = gates::measure(&fonts, (1920, 1080), (612.0, 300.0, 760.0, 430.0), true);
    report.controls[1].icon_off += 1.0;
    assert!(gates::check(&report, true).is_err());
    let mut report = gates::measure(&fonts, (1920, 1080), (612.0, 300.0, 760.0, 430.0), true);
    report.controls[2].caps_off += 1.0;
    assert!(gates::check(&report, true).is_err());
    let mut report = gates::measure(&fonts, (1920, 1080), (612.0, 300.0, 760.0, 430.0), true);
    report.padding[3] += 1.0;
    assert!(gates::check(&report, true).is_err());
}

fn luminance((r, g, b): Rgb) -> f64 {
    let channel = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Gate 4: every text the selector draws on every surface it can be on.
/// Unavailable Clip (`DIM`) is exempt as disabled.
#[test]
fn gate_contrast_every_label_on_its_surface() {
    use theme::*;
    let mut lowest = f64::MAX;
    let mut pairs = 0;
    for step in [0.0, 1.0, 2.0, 3.0] {
        let surface = toolbar::ladder(step);
        // `SUB` on no surface: an off toggle and the reason beside Clip.
        for (what, fg) in [("text", TEXT), ("sub", SUB)] {
            // An off toggle (SUB) only ever sits on no surface or the hover step.
            if fg == SUB && step > 1.0 {
                continue;
            }
            let ratio = contrast(fg, surface);
            assert!(ratio >= 4.5, "{what} on step {step}: {ratio:.2}:1");
            lowest = lowest.min(ratio);
            pairs += 1;
        }
    }
    // The record mark is a graphic: 3:1 against its resting and hover
    // surface (pressed, the selector is already closing).
    for surface in [RAISE, HOVER] {
        assert!(contrast(ERROR, surface) >= 3.0);
    }
    println!(
        "gate contrast: {pairs} text pairs, lowest {lowest:.2}:1; record mark {:.2}:1 resting, \
         {:.2}:1 under the pointer",
        contrast(ERROR, RAISE),
        contrast(ERROR, HOVER)
    );
}

/// Gate 6 over every word the selector shows.
pub fn copy_check(strings: &[&str]) {
    const DENY: [&str; 8] = [
        "seamless",
        "effortless",
        "elevate",
        "unlock",
        "powerful",
        "intuitive",
        "simply",
        "please",
    ];
    for text in strings {
        assert!(!text.contains(['!', '\u{2014}', '\u{2013}']), "{text}");
        assert!(!text.ends_with('.'), "a label, not a sentence: {text}");
        let lower = text.to_lowercase();
        for word in DENY {
            assert!(!lower.contains(word), "{text}");
        }
        let first = text.chars().next().expect("no empty strings");
        assert!(first.is_uppercase() || first.is_ascii_digit(), "{text}");
        for word in text.split([' ', '\u{a0}']).skip(1) {
            assert!(
                !word.chars().next().is_some_and(char::is_uppercase),
                "title case in {text}"
            );
        }
        assert!(text.split(' ').count() <= 4, "short: {text}");
    }
}

#[test]
fn gate_copy_the_toolbar_reads_like_a_person_wrote_it() {
    let mut strings: Vec<&str> = LABELS.to_vec();
    strings.extend(ClipState::ALL.iter().filter_map(|c| c.reason()));
    copy_check(&strings);
    println!("gate copy: {} strings: {strings:?}", strings.len());
}

#[test]
fn the_reason_sits_away_from_the_selection() {
    let fonts = Fonts::fallback();
    let metrics = Metrics::new(&fonts);
    let area = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    for sel in [(612.0, 300.0, 760.0, 430.0), (300.0, 4.0, 700.0, 400.0)] {
        let t = toolbar::layout(sel, area, &metrics).unwrap();
        let (hint, _) = toolbar::hint_rect(&fonts, &t, "Replay is loading", area);
        let selection = Rect::new(sel.0, sel.1, sel.2, sel.3);
        assert!(!hint.overlaps(selection), "{sel:?}: {hint:?}");
        assert_eq!(hint.x, t.controls[toolbar::CLIP].x, "with the Clip control");
        let gap = if hint.y < t.bounds.y {
            t.bounds.y - hint.bottom()
        } else {
            hint.y - t.bounds.bottom()
        };
        assert_eq!(gap, GAP_XS);
    }
}
