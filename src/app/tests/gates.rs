//! The `/ui` gates as tests: overflow, capitals centred, equal padding and
//! shared axes, contrast, copy. Each prints its numbers for the report
//! (`cargo test gates -- --nocapture`).

use std::time::{Duration, Instant};

use super::prefs;
use crate::app::{App, Block, gallery, measure, pages};
use crate::ui::Rect;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, ICON, Rgb, TextStyle};

const WIDTHS: [f32; 4] = [360.0, 480.0, 760.0, 1400.0];
const SETTLED: Duration = Duration::from_secs(2);

/// Every page of the window, also in its error state, and of the stress
/// fixture, at `size`.
fn every_page(size: (f32, f32), now: Instant, mut check: impl FnMut(&App, &str)) {
    let locked = pages::Notice {
        text: format!("config.toml has an error on line 3: {}", gallery::LONG_PATH),
        locked: true,
    };
    for variant in 0..3 {
        let pages = match variant {
            0 => pages::pages(&prefs(), None, now),
            1 => pages::pages(&prefs(), Some(&locked), now),
            _ => gallery::pages(now),
        };
        let names: Vec<String> = pages.iter().map(|page| page.name.clone()).collect();
        let mut app = App::new(Fonts::fallback(), pages, size, now);
        for name in names {
            app.show_named(&name, now);
            check(&app, &name);
        }
    }
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x - 0.5
        && inner.y >= outer.y - 0.5
        && inner.right() <= outer.right() + 0.5
        && inner.bottom() <= outer.bottom() + 0.5
}

#[test]
fn gate_overflow_nothing_leaves_its_group_or_the_window() {
    let now = Instant::now();
    let mut checked = 0;
    for w in WIDTHS {
        let size = (w, 900.0);
        every_page(size, now, |app, name| {
            let layout = app.layout(now + SETTLED);
            let nav_w = app.nav_w(now + SETTLED);
            let window = Rect::new(nav_w, 0.0, w - nav_w, f32::MAX);
            assert!(inside(layout.title, window), "{name} title at {w}");
            for (index, block) in app.page().blocks.iter().enumerate() {
                let rect = layout.blocks[index];
                assert!(inside(rect, window), "{name} block {index} at {w}");
                let Block::Row(row) = block else { continue };
                let panel = *layout
                    .panels
                    .iter()
                    .find(|panel| inside(rect, **panel))
                    .unwrap_or_else(|| panic!("{name} row {index} in no group at {w}"));
                let content = Rect::new(
                    panel.x + GAP_M,
                    panel.y + GAP_M,
                    panel.w - 2.0 * GAP_M,
                    panel.h - 2.0 * GAP_M,
                );
                let parts = app.row_parts(rect, row);
                // The name's box spans its row for centring; its text line
                // is what must stay in.
                let mid = parts.label.y + parts.label.h / 2.0;
                let name_line = Rect::new(parts.label.x, mid - 8.0, parts.label.w, 16.0);
                for (part, what) in [
                    (parts.icon, "icon"),
                    (name_line, "name"),
                    (parts.control, "control"),
                ] {
                    assert!(inside(part, content), "{name} {} {what} at {w}", row.label);
                    checked += 1;
                }
                assert!(
                    !parts.label.overlaps(parts.control),
                    "{name} {} name under its control at {w}",
                    row.label
                );
                assert!(
                    parts.label.w > 0.0,
                    "{name} {} has no room at {w}",
                    row.label
                );
            }
        });
    }
    // Every list, opened, stays in the window.
    for w in WIDTHS {
        let size = (w, 520.0);
        let mut app = App::new(Fonts::fallback(), gallery::pages(now), size, now);
        let rows: Vec<usize> = (0..app.page().blocks.len()).collect();
        for index in rows {
            // Only what can be clicked: controls in the window.
            let Some(rect) = app
                .control_rect(index, now)
                .filter(|rect| inside(*rect, Rect::new(0.0, 0.0, size.0, size.1)))
            else {
                continue;
            };
            app.handle(
                crate::ui::Input::Press {
                    x: rect.x + 2.0,
                    y: rect.y + 2.0,
                    clicks: 1,
                },
                now,
            );
            app.handle(crate::ui::Input::Release, now);
            if let Some((_, list)) = app.open_overlay(now) {
                assert!(
                    inside(list, Rect::new(0.0, 0.0, size.0, size.1)),
                    "list {list:?} for {rect:?} at {w}"
                );
                checked += 1;
            }
            app.handle(
                crate::ui::Input::Key {
                    key: crate::ui::Key::Escape,
                    mods: Default::default(),
                },
                now,
            );
        }
    }
    println!("gate overflow: {checked} parts inside their group at widths {WIDTHS:?}");
}

#[test]
fn gate_capitals_sit_on_the_middle_of_every_control() {
    let offsets = measure::cap_offsets(Fonts::fallback());
    assert!(offsets.len() >= 8, "{offsets:?}");
    let worst = offsets.iter().map(|(_, o)| o.abs()).fold(0.0, f32::max);
    println!("gate capitals (embedded font): {offsets:?}, worst {worst:.2} px");
    for (what, off) in offsets {
        assert!(
            off.abs() <= 0.5,
            "{what} capitals {off:.2} px off the middle"
        );
    }
}

#[test]
fn the_capitals_measure_sees_a_one_pixel_shift() {
    // Counter-check: text drawn 1 px low reads as 2 px off (1 more on top,
    // 1 less below), so the zeros above are measured, not assumed.
    let fonts = Fonts::fallback();
    let mut canvas = crate::ui::Canvas::new(100, 30, 2).unwrap();
    canvas.clear(theme::RAISE);
    let rect = Rect::new(0.0, 0.0, 100.0, 30.0);
    let baseline = fonts.baseline(TextStyle::Label, rect.y, rect.h) + 1.0;
    fonts.draw(
        &mut canvas,
        TextStyle::Label,
        "HHHH",
        (8.0, baseline),
        theme::TEXT,
        1.0,
    );
    let (top, bottom) = measure::ink(&canvas, rect, theme::RAISE).unwrap();
    let off = (top - rect.y) - (rect.bottom() - bottom);
    assert!((off - 2.0).abs() < 0.1, "{off}");
}

#[test]
fn gate_groups_pad_equally_and_siblings_share_an_axis() {
    let now = Instant::now();
    let mut worst_pad: f32 = 0.0;
    let mut worst_axis: f32 = 0.0;
    for w in WIDTHS {
        every_page((w, 900.0), now, |app, name| {
            let layout = app.layout(now + SETTLED);
            for panel in &layout.panels {
                let rows: Vec<(Rect, &crate::app::Row)> = app
                    .page()
                    .blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(i, b)| match b {
                        Block::Row(row) if inside(layout.blocks[i], *panel) => {
                            Some((layout.blocks[i], row))
                        }
                        _ => None,
                    })
                    .collect();
                let (first, last) = (rows[0], rows[rows.len() - 1]);
                let first_parts = app.row_parts(first.0, first.1);
                // A stacked row starts with its name's line, a single one
                // with its control (the name's box spans the row).
                let top = if first.0.h > theme::ROW_H {
                    first_parts.label.y
                } else {
                    first_parts.control.y
                } - panel.y;
                let bottom = panel.bottom() - app.row_parts(last.0, last.1).control.bottom();
                let mut pads = vec![top, bottom];
                for (rect, row) in &rows {
                    let parts = app.row_parts(*rect, row);
                    pads.push(parts.icon.x - panel.x);
                    pads.push(panel.right() - parts.control.right());
                    assert!(parts.icon.h <= TextStyle::Label.px() && parts.icon.w == ICON);
                    if rect.h == theme::ROW_H && row.hint.is_none() {
                        // Name, icon and control share one middle line.
                        let caps =
                            app.fonts
                                .baseline(TextStyle::Label, parts.label.y, parts.label.h)
                                - app.fonts.cap_height(TextStyle::Label) / 2.0;
                        let control = parts.control.y + parts.control.h / 2.0;
                        let icon = parts.icon.y + parts.icon.h / 2.0;
                        worst_axis = worst_axis
                            .max((caps - control).abs())
                            .max((icon - caps).abs());
                    }
                }
                for pad in pads {
                    worst_pad = worst_pad.max((pad - GAP_M).abs());
                    assert!((pad - GAP_M).abs() <= 0.5, "{name} pad {pad} at {w}");
                }
            }
        });
    }
    assert!(worst_axis <= 0.5, "axis off by {worst_axis}");
    println!(
        "gate padding: every group {GAP_M} px on all four sides (worst off {worst_pad:.2}); \
         shared axis worst off {worst_axis:.2} px"
    );
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

fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn gate_contrast_every_text_on_its_surface() {
    use theme::*;
    // Every pair the window draws text in; disabled text (`DIM`) is exempt.
    let pairs = [
        ("text on window", TEXT, WINDOW),
        ("text on sidebar", TEXT, SIDEBAR),
        ("text on group", TEXT, PANEL),
        ("text on control", TEXT, RAISE),
        ("text on hover", TEXT, HOVER),
        ("text on selected", TEXT, SELECTED),
        ("sub on window", SUB, WINDOW),
        ("sub on sidebar", SUB, SIDEBAR),
        ("sub on group", SUB, PANEL),
        ("sub on control", SUB, RAISE),
        ("sub on hover", SUB, HOVER),
        ("accent text", ON_ACCENT, ACCENT),
    ];
    let mut lowest = f64::MAX;
    for (what, fg, bg) in pairs {
        let ratio = contrast(fg, bg);
        lowest = lowest.min(ratio);
        assert!(ratio >= 4.5, "{what}: {ratio:.2}:1");
    }
    println!("gate contrast: {} pairs, lowest {lowest:.2}:1", pairs.len());
}

#[test]
fn gate_copy_reads_like_a_person_wrote_it() {
    let now = Instant::now();
    let pages = pages::pages(&prefs(), None, now);
    let mut strings: Vec<String> = Vec::new();
    for page in &pages {
        strings.push(page.name.clone());
        for block in &page.blocks {
            match block {
                Block::Heading(text) | Block::Banner { text, .. } => strings.push(text.clone()),
                Block::Row(row) => {
                    strings.push(row.label.clone());
                    strings.extend(row.hint.clone());
                    strings.extend(row.control.labels().iter().map(|s| s.to_string()));
                }
            }
        }
    }
    const DENY: [&str; 12] = [
        "seamless",
        "effortless",
        "elevate",
        "unlock",
        "leverage",
        "robust",
        "powerful",
        "intuitive",
        "supercharge",
        "empower",
        "cutting-edge",
        "next level",
    ];
    // Names that keep their capitals mid-sentence.
    const PROPER: [&str; 4] = ["Boltsnap", "GiB", "MiB", "Off"];
    for text in &strings {
        assert!(!text.contains(['!', '\u{2014}', '\u{2013}']), "{text}");
        assert!(!text.ends_with('.'), "a label, not a sentence: {text}");
        let lower = text.to_lowercase();
        for word in DENY {
            assert!(!lower.contains(word), "{text}");
        }
        let first = text.chars().next().expect("no empty strings");
        assert!(
            first.is_uppercase() || first.is_ascii_digit() || first == '~',
            "{text}"
        );
        // Sentence case: no capital after the first word, but for names.
        for word in text.split([' ', '\u{a0}']).skip(1) {
            let capital = word.chars().next().is_some_and(char::is_uppercase);
            assert!(!capital || PROPER.contains(&word), "title case in {text}");
        }
    }
    println!(
        "gate copy: {} strings, sentence case, no denylisted words",
        strings.len()
    );
}
