use std::time::{Duration, Instant};

use super::{App, Block, Target, gallery};
use crate::ui::text::Fonts;
use crate::ui::theme::{
    CTL_H, EDGE, HOVER_FADE, ICON, KNOB_SLIDE, NAV_W, SCROLL_GLIDE, TextStyle, WHEEL_STEP,
};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Effect, Input, Key, Mods};

const SIZE: (f32, f32) = (760.0, 520.0);

fn app(now: Instant) -> App {
    App::new(Fonts::fallback(), gallery::pages(now), SIZE, now)
}

/// The first enabled row named `label`.
fn block(app: &App, label: &str) -> usize {
    app.page()
        .blocks
        .iter()
        .position(|block| matches!(block, Block::Row(row) if row.label == label && row.enabled))
        .unwrap_or_else(|| panic!("no row {label}"))
}

fn control(app: &App, label: &str, now: Instant) -> crate::ui::Rect {
    let index = block(app, label);
    let Block::Row(row) = &app.page().blocks[index] else {
        unreachable!()
    };
    app.row_parts(app.block_rects(now)[index], row).control
}

fn press(app: &mut App, (x, y): (f32, f32), now: Instant) -> Vec<Effect> {
    let mut effects = app.handle(Input::Press { x, y, clicks: 1 }, now);
    effects.extend(app.handle(Input::Release, now));
    effects
}

fn key(app: &mut App, key: Key, now: Instant) -> Vec<Effect> {
    app.handle(
        Input::Key {
            key,
            mods: Mods::default(),
        },
        now,
    )
}

fn ctrl(app: &mut App, key: Key, now: Instant) -> Vec<Effect> {
    let mods = Mods {
        ctrl: true,
        shift: false,
    };
    app.handle(Input::Key { key, mods }, now)
}

fn focus(app: &mut App, label: &str, now: Instant) {
    let target = Target::Control(block(app, label));
    while app.focus() != Some(target) {
        key(app, Key::Tab, now);
    }
}

#[test]
fn rows_share_one_height_one_edge_and_never_overlap() {
    let now = Instant::now();
    let app = app(now);
    let rects = app.block_rects(now);
    assert_eq!(rects[0].y, EDGE, "the page starts one edge from the top");
    let mut controls = Vec::new();
    for (index, block) in app.page().blocks.iter().enumerate() {
        let rect = rects[index];
        assert_eq!(rect.x, NAV_W + EDGE);
        assert_eq!(rect.right(), SIZE.0 - EDGE);
        let Block::Row(row) = block else { continue };
        let parts = app.row_parts(rect, row);
        assert_eq!(parts.control.h, CTL_H, "{}", row.label);
        assert_eq!(parts.control.right(), rect.right(), "{}", row.label);
        // Icon no larger than the text, centred on its capitals' middle.
        assert!(parts.icon.w <= TextStyle::Label.px() && parts.icon.w == ICON);
        let label_mid = app
            .fonts
            .baseline(TextStyle::Label, parts.label.y, parts.label.h)
            - app.fonts.cap_height(TextStyle::Label) / 2.0;
        assert!(
            (parts.icon.y + parts.icon.h / 2.0 - label_mid).abs() < 0.01,
            "{}",
            row.label
        );
        assert!(parts.label.right() <= parts.control.x, "{}", row.label);
        controls.push(parts.control);
    }
    for (i, a) in controls.iter().enumerate() {
        for b in &controls[i + 1..] {
            assert!(!a.overlaps(*b), "{a:?} overlaps {b:?}");
        }
    }
    for index in 0..2 {
        let nav = App::nav_rect(index);
        assert_eq!(nav.h, CTL_H);
        assert!(nav.right() <= NAV_W);
    }
}

#[test]
fn tab_runs_through_navigation_and_enabled_controls_in_reading_order() {
    let now = Instant::now();
    let mut app = app(now);
    let order = app.tab_order();
    assert_eq!(order[0], Target::Nav(0));
    let blocks: Vec<usize> = order[1..]
        .iter()
        .map(|target| match target {
            Target::Control(block) => *block,
            Target::Nav(_) => panic!("one navigation stop"),
        })
        .collect();
    assert!(blocks.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(blocks.len(), 9, "the disabled rows are skipped");
    for target in &order {
        key(&mut app, Key::Tab, now);
        assert_eq!(app.focus(), Some(*target));
    }
    key(&mut app, Key::Tab, now);
    assert_eq!(app.focus(), Some(order[0]), "Tab wraps");
    app.handle(
        Input::Key {
            key: Key::Tab,
            mods: Mods {
                ctrl: false,
                shift: true,
            },
        },
        now,
    );
    assert_eq!(app.focus(), order.last().copied());
}

#[test]
fn a_switch_toggles_by_click_and_key_and_its_knob_slides() {
    let now = Instant::now();
    let mut app = app(now);
    let audio = block(&app, "Audio");
    let rect = control(&app, "Audio", now);
    press(&mut app, (rect.x + 2.0, rect.y + 15.0), now);
    assert_eq!(app.value(audio), Some(Value::Bool(false)));
    assert!(app.animating(now));
    assert!(!app.animating(now + KNOB_SLIDE + HOVER_FADE));
    key(&mut app, Key::Space, now);
    assert_eq!(app.value(audio), Some(Value::Bool(true)));
}

#[test]
fn a_segment_takes_the_part_under_the_pointer_and_the_arrow_keys() {
    let now = Instant::now();
    let mut app = app(now);
    let both = block(&app, "Both displays");
    let rect = control(&app, "Both displays", now);
    press(&mut app, (rect.right() - 4.0, rect.y + 15.0), now);
    assert_eq!(app.value(both), Some(Value::Choice(1)));
    key(&mut app, Key::Right, now);
    assert_eq!(
        app.value(both),
        Some(Value::Choice(1)),
        "stays on the last part"
    );
    key(&mut app, Key::Left, now);
    assert_eq!(app.value(both), Some(Value::Choice(0)));
}

#[test]
fn a_dropdown_opens_picks_and_closes_without_closing_the_window() {
    let now = Instant::now();
    let mut app = app(now);
    let rate = block(&app, "Frame rate");
    focus(&mut app, "Frame rate", now);
    assert!(key(&mut app, Key::Enter, now).is_empty());
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(app.value(rate), Some(Value::Choice(3)));
    // A press beside the open list closes it and changes nothing.
    key(&mut app, Key::Enter, now);
    press(&mut app, (NAV_W + 40.0, 4.0), now);
    assert_eq!(app.value(rate), Some(Value::Choice(3)));
    let later = now + Duration::from_millis(200);
    assert!(app.open_overlay(later).is_none());
    // Escape closes the list first, the window only after.
    focus(&mut app, "Frame rate", later);
    key(&mut app, Key::Enter, later);
    assert!(key(&mut app, Key::Escape, later).is_empty());
    assert_eq!(key(&mut app, Key::Escape, later), vec![Effect::Close]);
}

#[test]
fn a_click_in_an_open_list_picks_that_entry() {
    let now = Instant::now();
    let mut app = app(now);
    let rate = block(&app, "Frame rate");
    let rect = control(&app, "Frame rate", now);
    press(&mut app, (rect.x + 4.0, rect.y + 4.0), now);
    let (_, list) = app.open_overlay(now).expect("open");
    // Entries are one control tall below a 4 px inset; pick the first.
    press(&mut app, (list.x + 10.0, list.y + 4.0 + CTL_H / 2.0), now);
    assert_eq!(app.value(rate), Some(Value::Choice(0)));
}

#[test]
fn a_slider_follows_the_drag_and_steps_by_key() {
    let now = Instant::now();
    let mut app = app(now);
    let size = block(&app, "Card size");
    let rect = control(&app, "Card size", now);
    app.handle(
        Input::Press {
            x: rect.x,
            y: rect.y + 15.0,
            clicks: 1,
        },
        now,
    );
    assert_eq!(app.value(size), Some(Value::Int(140)));
    app.handle(
        Input::Move {
            x: rect.x + 500.0,
            y: rect.y,
        },
        now,
    );
    assert_eq!(
        app.value(size),
        Some(Value::Int(380)),
        "the capture outlives the control"
    );
    app.handle(Input::Release, now);
    app.handle(
        Input::Move {
            x: rect.x,
            y: rect.y,
        },
        now,
    );
    assert_eq!(app.value(size), Some(Value::Int(380)));
    key(&mut app, Key::Left, now);
    assert_eq!(app.value(size), Some(Value::Int(370)));
    key(&mut app, Key::Home, now);
    assert_eq!(app.value(size), Some(Value::Int(140)));
}

#[test]
fn a_stepper_stops_at_its_bounds() {
    let now = Instant::now();
    let mut app = app(now);
    let shown = block(&app, "Cards shown");
    let rect = control(&app, "Cards shown", now);
    for _ in 0..5 {
        press(&mut app, (rect.right() - 5.0, rect.y + 15.0), now);
    }
    assert_eq!(app.value(shown), Some(Value::Int(8)));
    press(&mut app, (rect.x + 5.0, rect.y + 15.0), now);
    assert_eq!(app.value(shown), Some(Value::Int(7)));
    key(&mut app, Key::Right, now);
    key(&mut app, Key::Right, now);
    assert_eq!(app.value(shown), Some(Value::Int(8)));
}

#[test]
fn a_text_field_edits_selects_copies_and_commits() {
    let now = Instant::now();
    let mut app = app(now);
    let folder = block(&app, "Save folder");
    focus(&mut app, "Save folder", now);
    app.handle(Input::Text("/shots".into()), now);
    assert_eq!(
        app.value(folder),
        Some(Value::Text("~/Pictures/boltsnap".into())),
        "edits wait for the commit"
    );
    let ctrl_shift = Mods {
        ctrl: true,
        shift: true,
    };
    app.handle(
        Input::Key {
            key: Key::Left,
            mods: ctrl_shift,
        },
        now,
    );
    assert_eq!(
        ctrl(&mut app, Key::Char('c'), now),
        vec![Effect::Copy("shots".into())]
    );
    ctrl(&mut app, Key::Backspace, now);
    assert_eq!(ctrl(&mut app, Key::Char('v'), now), vec![Effect::Paste]);
    app.handle(Input::Pasted("clips".into()), now);
    key(&mut app, Key::Enter, now);
    assert_eq!(
        app.value(folder),
        Some(Value::Text("~/Pictures/boltsnap/clips".into()))
    );
    // Ctrl+Backspace after the slash takes the word before it.
    ctrl(&mut app, Key::Backspace, now);
    ctrl(&mut app, Key::Backspace, now);
    key(&mut app, Key::Tab, now);
    assert_eq!(
        app.value(folder),
        Some(Value::Text("~/Pictures/".into())),
        "leaving commits"
    );
}

#[test]
fn a_double_click_selects_the_word_under_the_pointer() {
    let now = Instant::now();
    let mut app = app(now);
    let rect = control(&app, "Save folder", now);
    let at = (
        rect.x + 12.0 + app.fonts.width(TextStyle::Label, "~/Pic"),
        rect.y + 15.0,
    );
    app.handle(
        Input::Press {
            x: at.0,
            y: at.1,
            clicks: 2,
        },
        now,
    );
    app.handle(Input::Release, now);
    assert_eq!(
        ctrl(&mut app, Key::Char('c'), now),
        vec![Effect::Copy("Pictures".into())]
    );
}

#[test]
fn hover_fades_in_and_out_and_leaving_the_window_clears_it() {
    let now = Instant::now();
    let mut app = app(now);
    let rect = control(&app, "Frame rate", now);
    let target = Target::Control(block(&app, "Frame rate"));
    app.handle(
        Input::Move {
            x: rect.x + 4.0,
            y: rect.y + 4.0,
        },
        now,
    );
    assert_eq!(app.hover_of(target, now), 0.0);
    assert_eq!(app.hover_of(target, now + HOVER_FADE), 1.0);
    let later = now + HOVER_FADE;
    app.handle(Input::Leave, later);
    assert_eq!(app.hover_of(target, later + HOVER_FADE), 0.0);
}

#[test]
fn keys_switch_pages_and_close_the_window() {
    let now = Instant::now();
    let mut app = app(now);
    ctrl(&mut app, Key::Char('2'), now);
    assert_eq!(app.current(), 1);
    key(&mut app, Key::Tab, now);
    assert_eq!(app.focus(), Some(Target::Nav(1)));
    key(&mut app, Key::Up, now);
    assert_eq!(app.current(), 0);
    assert_eq!(ctrl(&mut app, Key::Char('w'), now), vec![Effect::Close]);
    assert_eq!(ctrl(&mut app, Key::Char('q'), now), vec![Effect::Close]);
}

#[test]
fn the_wheel_glides_the_page_and_stops_at_its_end() {
    let now = Instant::now();
    let mut app = app(now);
    let top = app.block_rects(now)[0].y;
    app.handle(
        Input::Wheel {
            notches: 1,
            pixels: 0.0,
        },
        now,
    );
    assert_eq!(app.block_rects(now + SCROLL_GLIDE)[0].y, top - WHEEL_STEP);
    app.handle(
        Input::Wheel {
            notches: 100,
            pixels: 0.0,
        },
        now + SCROLL_GLIDE,
    );
    let end = now + SCROLL_GLIDE * 3;
    let last = *app.block_rects(end).last().unwrap();
    assert_eq!(
        last.bottom() + EDGE,
        SIZE.1,
        "the last row ends one edge above the bottom"
    );
}

#[test]
fn every_page_draws_at_scale_one_and_two() {
    let now = Instant::now();
    let mut app = app(now);
    for page in ["widgets", "text"] {
        assert!(app.show_named(page, now));
        for scale in [1, 2] {
            let mut canvas = Canvas::new(SIZE.0 as u32, SIZE.1 as u32, scale).unwrap();
            app.draw(&mut canvas, now);
            assert!(canvas.pixmap.data().chunks_exact(4).any(|px| px[0] > 0xE0));
        }
    }
}
