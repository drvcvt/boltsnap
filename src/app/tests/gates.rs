//! The `/ui` gates as tests: overflow, capitals centred, equal padding and
//! shared axes, contrast, copy. Each prints its numbers for the report
//! (`cargo test gates -- --nocapture`).

use std::time::{Duration, Instant};

use super::prefs;

/// The pages in the state with everything showing that only shows
/// sometimes: a recording runs, the replay buffer runs with old settings
/// and more length than its memory holds, every key is set in the file
/// (every row has its reset button).
fn busy() -> pages::Prefs {
    pages::Prefs {
        recording: true,
        replay_running: true,
        replay_stale: true,
        replay_stalled: true,
        // Too long for the memory: the hint says how much fits.
        replay_seconds: 600,
        replay_memory_mib: 256,
        set_keys: crate::settings::schema::KEYS
            .iter()
            .map(|key| key.path.to_owned())
            .collect(),
        ..prefs()
    }
}
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
    for variant in 0..4 {
        let pages = match variant {
            0 => pages::pages(&prefs(), None, now),
            1 => pages::pages(&prefs(), Some(&locked), now),
            2 => pages::pages(&busy(), None, now),
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
                if let Some(reset) = parts.reset {
                    assert!(inside(reset, content), "{name} {} reset at {w}", row.label);
                    assert!(
                        !reset.overlaps(parts.control) && !reset.overlaps(parts.label),
                        "{name} {} reset on top of its row at {w}",
                        row.label
                    );
                    checked += 1;
                }
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
                // A stacked row starts with its name's line, a `CTL_H` slot
                // like a control; beside its control a row starts with
                // whichever is higher, the control or the name's first line.
                let top = if first_parts.control.y >= first_parts.text.bottom() {
                    first_parts.text.y
                } else {
                    first_parts.control.y.min(first_parts.label.y)
                } - panel.y;
                // The lower of the control and the last line of text.
                let last_parts = app.row_parts(last.0, last.1);
                let text_bottom = last_parts.label.y
                    + last_parts.names.len() as f32 * last_parts.name_pitch
                    + last_parts.hints.len() as f32 * last_parts.hint_pitch;
                let stacked = last_parts.control.y >= last_parts.text.bottom();
                let content = if stacked {
                    last_parts.control.bottom()
                } else {
                    last_parts.control.bottom().max(text_bottom)
                };
                let bottom = panel.bottom() - content;
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
    // The shelf preview's screen sits `GAP_M` in on all four sides too.
    for w in WIDTHS {
        every_page((w, 900.0), now, |app, name| {
            let layout = app.layout(now + SETTLED);
            for (block, rect) in app.page().blocks.iter().zip(&layout.blocks) {
                if let Block::Preview(crate::app::Preview::Displays(screens)) = block {
                    // Every monitor inside, the picture centred in the room.
                    let places = crate::app::display_places(*rect, screens);
                    let inner = rect.inset(GAP_M);
                    for place in &places {
                        assert!(inside(*place, inner), "{name} monitor outside at {w}");
                    }
                    let left = places.iter().map(|p| p.x).fold(f32::MAX, f32::min);
                    let right = places.iter().map(|p| p.right()).fold(f32::MIN, f32::max);
                    let off = ((left - inner.x) - (inner.right() - right)).abs();
                    assert!(off <= 1.0, "{name} monitors off centre by {off} at {w}");
                    // Their names under them and the line under all of them
                    // stay inside too, the names apart from each other.
                    let names = crate::app::display_name_places(*rect, screens);
                    for (i, place) in names.iter().enumerate() {
                        assert!(inside(*place, inner), "{name} monitor name outside at {w}");
                        assert!(
                            names[i + 1..].iter().all(|other| !other.overlaps(*place)),
                            "{name} monitor names overlap at {w}"
                        );
                    }
                    let caption = crate::app::display_caption_place(*rect, screens);
                    assert!(inside(caption, inner), "{name} caption outside at {w}");
                    // One or two whole lines of `Label`, nothing left over.
                    assert!(
                        [16.0, 32.0].iter().any(|h| (caption.h - h).abs() <= 0.5),
                        "{name} caption {} high at {w}",
                        caption.h
                    );
                }
                if let Block::Preview(crate::app::Preview::Replay { .. }) = block {
                    let bar = crate::app::replay_bar_rect(*rect);
                    let head = crate::app::replay_head_rect(*rect);
                    let pads = [bar.x - rect.x, rect.right() - bar.right(), head.y - rect.y];
                    for pad in pads {
                        assert!(
                            (pad - GAP_M).abs() <= 0.5,
                            "{name} replay bar pad {pads:?} at {w}"
                        );
                    }
                }
                if let Block::Preview(crate::app::Preview::Shelf) = block {
                    let screen = crate::app::preview_screen(*rect);
                    let pads = [
                        screen.x - rect.x,
                        rect.right() - screen.right(),
                        screen.y - rect.y,
                        rect.bottom() - screen.bottom(),
                    ];
                    for pad in pads {
                        worst_pad = worst_pad.max((pad - GAP_M).abs());
                        assert!(
                            (pad - GAP_M).abs() <= 1.0,
                            "{name} preview pad {pads:?} at {w}"
                        );
                    }
                }
            }
        });
    }
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
    // The focus outline is a state, not text: 3:1 against what it sits on
    // (WCAG 2.4.13), and against the control it surrounds.
    for (what, bg) in [
        ("on group", PANEL),
        ("on sidebar", SIDEBAR),
        ("against control", RAISE),
    ] {
        let ratio = contrast(SUB, bg);
        assert!(ratio >= 3.0, "focus outline {what}: {ratio:.2}:1");
        println!("gate contrast: focus outline {what} {ratio:.2}:1");
    }
}

#[test]
fn gate_copy_reads_like_a_person_wrote_it() {
    let now = Instant::now();
    let mut pages = pages::pages(&prefs(), None, now);
    pages.extend(pages::pages(&busy(), None, now));
    let mut strings: Vec<String> = Vec::new();
    for page in &pages {
        strings.push(page.name.clone());
        for block in &page.blocks {
            match block {
                Block::Heading(text) | Block::Banner { text, .. } => strings.push(text.clone()),
                Block::Preview(crate::app::Preview::Displays(screens)) => {
                    strings.extend(crate::app::display_lines(screens.len()));
                }
                Block::Preview(_) => {}
                Block::Row(row) => {
                    strings.push(row.label.clone());
                    strings.extend(row.hint.clone());
                    // Font families and monitor models are names, not copy.
                    let monitors: Vec<String> =
                        prefs().displays.iter().map(|s| s.entry()).collect();
                    if row.label != "Font" {
                        strings.extend(
                            row.control
                                .labels()
                                .iter()
                                .map(|s| s.to_string())
                                .filter(|s| !monitors.contains(s)),
                        );
                    }
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
    // Names and units that keep their capitals mid-sentence.
    const PROPER: [&str; 7] = ["Boltsnap", "CPU", "GiB", "MiB", "Mbit/s", "Off", "Vulkan"];
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

/// Gate 8: every row's icon ink on the middle of its name's capitals, the
/// icons of a page on one axis, the names on one edge, one `GAP_S` from the
/// widest icon's ink to the names.
#[test]
fn gate_icons_sit_on_the_capitals_in_one_column() {
    let now = Instant::now();
    let rows = measure::icon_rows(
        Fonts::fallback(),
        pages::pages(&prefs(), None, now),
        (1000.0, 900.0),
    );
    report_icon_rows(&rows);
    let worst = measure::icon_gate(&rows).unwrap_or_else(|failure| panic!("{failure}"));
    println!("gate icons: {} rows, {worst}", rows.len());
}

#[test]
fn the_ink_measure_sees_a_one_pixel_shift() {
    use crate::ui::icons::Icon;
    let middle = |y: f32| {
        let mut canvas = crate::ui::Canvas::new(40, 40, 2).unwrap();
        canvas.clear(theme::PANEL);
        Icon::Folder.draw(&mut canvas, Rect::new(10.0, y, 14.0, 14.0), theme::SUB, 1.0);
        measure::ink_box(
            &canvas,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            theme::PANEL,
            theme::SUB,
        )
        .unwrap()
        .middle_y()
    };
    let shift = middle(11.0) - middle(10.0);
    assert!((shift - 1.0).abs() < 0.05, "1 px reads as {shift}");
}

fn report_icon_rows(rows: &[measure::IconRow]) {
    for row in rows {
        println!(
            "{:<12} {:<34} dy {:+.2} ink {:.2}..{:.2} x {:.2}..{:.2} text {:.2}",
            row.page,
            row.label,
            row.dy,
            row.icon.top,
            row.icon.bottom,
            row.icon.left,
            row.icon.right,
            row.text_x
        );
    }
}

/// `/ui`: nothing is cut. Names, hints and banners of the real pages, also
/// in the error state with the real message, wrap instead of ending in an
/// ellipsis at every width.
#[test]
fn gate_text_wraps_instead_of_being_cut() {
    use crate::ui::widgets::banner_lines;
    let now = Instant::now();
    let notice = pages::Notice {
        text: "config.toml has an error on line 3, fix it to change settings here".into(),
        locked: true,
    };
    let cut = |line: &str| line.ends_with('\u{2026}') || line.ends_with("...");
    let mut lines = 0;
    for w in WIDTHS {
        for (state, notice) in [(prefs(), None), (prefs(), Some(&notice)), (busy(), None)] {
            let names: Vec<String> = pages::pages(&state, notice, now)
                .iter()
                .map(|page| page.name.clone())
                .collect();
            let mut app = App::new(
                Fonts::fallback(),
                pages::pages(&state, notice, now),
                (w, 900.0),
                now,
            );
            for name in names {
                app.show_named(&name, now);
                let layout = app.layout(now + SETTLED);
                for (block, rect) in app.page().blocks.iter().zip(&layout.blocks) {
                    let text: Vec<String> = match block {
                        Block::Row(row) => {
                            let parts = app.row_parts(*rect, row);
                            parts.names.into_iter().chain(parts.hints).collect()
                        }
                        Block::Banner { text, .. } => banner_lines(&app.fonts, rect.w, text),
                        Block::Heading(_) | Block::Preview(_) => continue,
                    };
                    for line in text {
                        assert!(!cut(&line), "{name} at {w}: {line:?} is cut");
                        lines += 1;
                    }
                }
            }
        }
    }
    println!("gate wrap: {lines} lines at widths {WIDTHS:?}, none cut");
}

/// Dropdowns beside their names share one left edge when their entries fit
/// `MENU_MIN` (settings plan, 5c S5).
#[test]
fn gate_menus_share_a_left_edge() {
    let now = Instant::now();
    let mut checked = 0;
    for w in [760.0, 1400.0] {
        every_page((w, 900.0), now, |app, name| {
            let layout = app.layout(now + SETTLED);
            let edges: Vec<f32> = app
                .page()
                .blocks
                .iter()
                .zip(&layout.blocks)
                .filter_map(|(block, rect)| match block {
                    // With all the room it wants, only a short dropdown is
                    // exactly `MENU_MIN` wide (sliders 240, fields 360).
                    Block::Row(row)
                        if row.control.width(&app.fonts, f32::MAX) == theme::MENU_MIN =>
                    {
                        let parts = app.row_parts(*rect, row);
                        (parts.control.y < parts.text.bottom()).then_some(parts.control.x)
                    }
                    _ => None,
                })
                .collect();
            if let Some(first) = edges.first() {
                for edge in &edges {
                    assert_eq!(edge, first, "{name} at {w}: dropdown edges {edges:?}");
                }
                checked += edges.len();
            }
        });
    }
    // Frame rate, Keep long shelf saves, Memory and Frame rate on two pages
    // at two widths: an empty pass would prove nothing.
    assert!(checked >= 8, "only {checked} dropdowns found");
    println!("gate menus: {checked} dropdowns on one edge per page");
}
