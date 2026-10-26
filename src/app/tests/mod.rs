use std::time::{Duration, Instant};

mod focus;
mod gates;
mod live;
mod shape;
mod writes;

use super::pages::{AudioSource, Corner, Cursor, Prefs};
use super::{App, Block, Channel, Target, gallery, pages};
use crate::ui::text::Fonts;
use crate::ui::theme::{
    CTL_H, GAP_XL, GAP_XS, NAV_NARROW_W, NAV_W, PAGE_RISE, QUICK, ROW_H, TextStyle, WHEEL_STEP,
};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Effect, Input, Key, Mods};

const SIZE: (f32, f32) = (760.0, 520.0);
/// Long enough for every spring in the window to have settled.
pub const SETTLED: Duration = Duration::from_secs(1);

pub fn prefs() -> Prefs {
    Prefs {
        card_width: 190,
        visible_cards: 5,
        corner: Corner::BottomLeft,
        save_dir: "~/Pictures/boltsnap".into(),
        both_combined: false,
        fps: 120,
        cursor: Cursor::System,
        show_frame: true,
        audio: true,
        audio_source: AudioSource::SystemAndMic,
        record_dir: "~/Videos/boltsnap".into(),
        disk_to_shelf: true,
        shelf_to_disk_after: Some(60),
        replay_autostart: false,
        replay_running: false,
        replay_stalled: false,
        replay_seconds: 60,
        replay_memory_mib: 512,
        replay_fps: 60,
    }
}

pub fn app_on(page: &str, size: (f32, f32), now: Instant) -> App {
    let mut app = App::new(
        Fonts::fallback(),
        pages::pages(&prefs(), None, now),
        size,
        now,
    );
    assert!(app.show_named(page, now), "{page}");
    app
}

fn app(page: &str, now: Instant) -> App {
    app_on(page, SIZE, now)
}

/// The first enabled row named `label`.
pub fn block(app: &App, label: &str) -> usize {
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
fn tab_runs_through_navigation_and_enabled_controls_in_reading_order() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let order = app.tab_order();
    assert_eq!(order[0], Target::Nav(2));
    let blocks: Vec<usize> = order[1..]
        .iter()
        .map(|target| match target {
            Target::Control(block) => *block,
            Target::Nav(_) => panic!("one navigation stop"),
        })
        .collect();
    assert!(blocks.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(blocks.len(), 9, "every row of the page");
    for target in &order {
        key(&mut app, Key::Tab, now);
        assert_eq!(app.focus(), Some(*target));
    }
    key(&mut app, Key::Tab, now);
    assert_eq!(app.focus(), Some(order[0]), "Tab wraps");
    let shift = Mods {
        ctrl: false,
        shift: true,
    };
    app.handle(
        Input::Key {
            key: Key::Tab,
            mods: shift,
        },
        now,
    );
    assert_eq!(app.focus(), order.last().copied());
}

#[test]
fn a_switch_toggles_by_click_and_key_and_its_knob_slides() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let audio = block(&app, "Audio");
    let rect = control(&app, "Audio", now);
    press(&mut app, (rect.x + 2.0, rect.y + 15.0), now);
    assert_eq!(app.value(audio), Some(Value::Bool(false)));
    assert!(app.animating(now));
    assert!(!app.animating(now + SETTLED));
    key(&mut app, Key::Space, now + SETTLED);
    assert_eq!(app.value(audio), Some(Value::Bool(true)));
}

#[test]
fn a_segment_takes_the_part_under_the_pointer_and_the_arrow_keys() {
    let now = Instant::now();
    let mut app = app("recording", now);
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
fn a_part_under_the_pointer_lights_with_a_spring() {
    let now = Instant::now();
    let mut app = app("recording", now + SETTLED);
    let now = now + SETTLED;
    let rect = control(&app, "Cursor", now);
    app.handle(
        Input::Move {
            x: rect.right() - 4.0,
            y: rect.y + 15.0,
        },
        now,
    );
    assert!(app.animating(now));
    assert!(!app.animating(now + SETTLED));
}

#[test]
fn a_dropdown_opens_picks_and_closes_without_closing_the_window() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let rate = block(&app, "Frame rate");
    focus(&mut app, "Frame rate", now);
    assert_eq!(app.value(rate), Some(Value::Choice(2)), "120 fps");
    assert!(key(&mut app, Key::Enter, now).is_empty());
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Enter, now);
    assert_eq!(app.value(rate), Some(Value::Choice(3)));
    // A press beside the open list closes it and changes nothing.
    key(&mut app, Key::Enter, now);
    press(&mut app, (NAV_W + 4.0, 4.0), now);
    assert_eq!(app.value(rate), Some(Value::Choice(3)));
    let later = now + SETTLED;
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
    let mut app = app("recording", now);
    let rate = block(&app, "Frame rate");
    let rect = control(&app, "Frame rate", now);
    press(&mut app, (rect.x + 4.0, rect.y + 4.0), now);
    let (_, list) = app.open_overlay(now).expect("open");
    assert!(list.y >= 0.0 && list.bottom() <= SIZE.1, "{list:?}");
    // Entries are one control tall below a 4 px inset; pick the first.
    press(
        &mut app,
        (list.x + 10.0, list.y + GAP_XS + CTL_H / 2.0),
        now,
    );
    assert_eq!(app.value(rate), Some(Value::Choice(0)));
}

#[test]
fn a_slider_follows_the_drag_and_steps_by_key() {
    let now = Instant::now();
    let mut app = app("shelf", now);
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
    let mut app = app("shelf", now);
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
    let mut app = app("screenshots", now);
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
    let mut app = app("screenshots", now);
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
fn hover_press_and_focus_fade_and_leaving_the_window_clears_hover() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let rect = control(&app, "Frame rate", now);
    let target = Target::Control(block(&app, "Frame rate"));
    let at = (rect.x + 4.0, rect.y + 4.0);
    app.handle(Input::Move { x: at.0, y: at.1 }, now);
    let quick = Duration::from_secs_f32(QUICK.duration);
    assert_eq!(app.fade(target, Channel::Hover, now), 0.0);
    assert!(app.fade(target, Channel::Hover, now + quick) > 0.98);
    assert_eq!(app.fade(target, Channel::Hover, now + SETTLED), 1.0);
    app.handle(
        Input::Press {
            x: at.0,
            y: at.1,
            clicks: 1,
        },
        now,
    );
    assert!(app.fade(target, Channel::Press, now + quick) > 0.98);
    assert!(app.fade(target, Channel::Focus, now + quick) > 0.98);
    app.handle(Input::Release, now + quick);
    assert_eq!(app.fade(target, Channel::Press, now + SETTLED), 0.0);
    let later = now + SETTLED;
    app.handle(Input::Leave, later);
    assert_eq!(app.fade(target, Channel::Hover, later + SETTLED), 0.0);
}

#[test]
fn keys_switch_pages_and_close_the_window() {
    let now = Instant::now();
    let mut app = app("shelf", now);
    ctrl(&mut app, Key::Char('2'), now);
    assert_eq!(app.current(), 1);
    key(&mut app, Key::Tab, now);
    assert_eq!(app.focus(), Some(Target::Nav(1)));
    key(&mut app, Key::Up, now);
    assert_eq!(app.current(), 0);
    assert_eq!(app.focus(), Some(Target::Nav(0)));
    key(&mut app, Key::Up, now);
    assert_eq!(app.current(), 0, "the first page stays");
    assert_eq!(ctrl(&mut app, Key::Char('w'), now), vec![Effect::Close]);
    assert_eq!(ctrl(&mut app, Key::Char('q'), now), vec![Effect::Close]);
}

#[test]
fn a_new_page_fades_in_and_rises_unless_motion_is_reduced() {
    let now = Instant::now();
    let mut app = app("shelf", now);
    let settled_title = app.layout(now + SETTLED).title;
    ctrl(&mut app, Key::Char('3'), now);
    assert_eq!(app.page_alpha(now), 0.0);
    assert_eq!(app.layout(now).title.y, settled_title.y + PAGE_RISE);
    assert_eq!(app.page_alpha(now + SETTLED), 1.0);
    assert_eq!(app.layout(now + SETTLED).title.y, settled_title.y);
    app.reduced_motion = true;
    ctrl(&mut app, Key::Char('1'), now);
    assert_eq!(app.page_alpha(now), 0.0, "the fade stays");
    assert_eq!(app.layout(now).title.y, settled_title.y, "no rise");
}

#[test]
fn the_wheel_glides_the_page_and_stops_at_its_end() {
    let start = Instant::now();
    let mut app = app("recording", start);
    // The page has come in.
    let now = start + SETTLED;
    let top = app.block_rects(now)[0].y;
    app.handle(
        Input::Wheel {
            notches: 1,
            pixels: 0.0,
        },
        now,
    );
    assert_eq!(app.block_rects(now + SETTLED)[0].y, top - WHEEL_STEP);
    app.handle(
        Input::Wheel {
            notches: 100,
            pixels: 0.0,
        },
        now + SETTLED,
    );
    let end = now + SETTLED * 3;
    let layout = app.layout(end);
    let last = layout.panels.last().unwrap();
    assert_eq!(
        last.bottom() + GAP_XL,
        SIZE.1,
        "the last group ends one page edge above the bottom"
    );
}

#[test]
fn a_narrow_window_shows_icon_navigation_and_stacks_rows_that_do_not_fit() {
    let now = Instant::now();
    let wide = app_on("recording", (1000.0, 520.0), now);
    let narrow = app_on("recording", (360.0, 520.0), now);
    assert_eq!(wide.nav_w(now), NAV_W);
    assert_eq!(narrow.nav_w(now), NAV_NARROW_W);
    let source = block(&narrow, "Source");
    assert_eq!(wide.block_rects(now)[source].h, ROW_H);
    let rect = narrow.block_rects(now)[source];
    assert!(rect.h > ROW_H, "stacked");
    let Block::Row(row) = &narrow.page().blocks[source] else {
        unreachable!()
    };
    let parts = narrow.row_parts(rect, row);
    assert!(
        parts.control.y >= parts.label.bottom(),
        "control below name"
    );
    assert_eq!(parts.control.x, parts.label.x, "under the name");
    // A switch still fits beside its name; the hint wraps instead of
    // being cut, so the row grows.
    let audio = block(&narrow, "Audio");
    let rect = narrow.block_rects(now)[audio];
    let Block::Row(row) = &narrow.page().blocks[audio] else {
        unreachable!()
    };
    let parts = narrow.row_parts(rect, row);
    assert!(parts.control.x > parts.text.right(), "beside its name");
    assert!(parts.hints.len() > 1 && rect.h > ROW_H, "hint on two lines");
}

#[test]
fn every_page_and_the_fixture_draw_at_every_width_and_scale() {
    let now = Instant::now();
    for (fixture, names) in [
        (false, vec!["shelf", "screenshots", "recording", "replay"]),
        (true, vec!["controls", gallery::COMPOUND]),
    ] {
        for size in [(360.0, 480.0), (760.0, 520.0), (1400.0, 900.0)] {
            let pages = if fixture {
                gallery::pages(now)
            } else {
                pages::pages(&prefs(), None, now)
            };
            let mut app = App::new(Fonts::fallback(), pages, size, now);
            for name in &names {
                assert!(app.show_named(name, now), "{name}");
                for scale in [1, 2] {
                    let mut canvas = Canvas::new(size.0 as u32, size.1 as u32, scale).unwrap();
                    app.draw(&mut canvas, now + SETTLED);
                    assert!(canvas.pixmap.data().chunks_exact(4).any(|px| px[0] > 0xE0));
                }
            }
        }
    }
}
