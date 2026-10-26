//! Values that change elsewhere reach the open window (settings plan, 5c S6).

use std::time::{Duration, Instant};

use super::{SETTLED, app_on, block, focus, prefs};
use crate::app::Block;
use crate::app::pages::{self, Cursor, Notice, Prefs};
use crate::ui::widgets::Value;
use crate::ui::{Effect, Input};

const SIZE: (f32, f32) = (1000.0, 700.0);

fn with(change: impl FnOnce(&mut Prefs), now: Instant) -> Vec<crate::app::Page> {
    let mut prefs = prefs();
    change(&mut prefs);
    pages::pages(&prefs, None, now)
}

fn idle(_: usize, _: usize) -> bool {
    false
}

fn changed(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::Changed(_)))
}

#[test]
fn a_value_from_the_tray_glides_in_and_is_not_written_back() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    let (show_frame, cursor) = (block(&app, "Show frame"), block(&app, "Cursor"));
    app.update(
        with(
            |p| {
                p.show_frame = false;
                p.cursor = Cursor::Quick;
            },
            now,
        ),
        now,
        idle,
    );
    assert_eq!(app.value(show_frame), Some(Value::Bool(false)));
    assert_eq!(app.value(cursor), Some(Value::Choice(2)));
    assert!(
        app.animating(now + Duration::from_millis(50)),
        "the knob and the segment glide"
    );
    let later = now + SETTLED;
    assert!(
        !changed(&app.handle(Input::Move { x: 1.0, y: 1.0 }, later)),
        "nothing goes back"
    );
    assert_eq!(app.page().name, "Recording");
}

#[test]
fn the_focused_row_and_a_row_with_a_write_on_its_way_keep_their_value() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    focus(&mut app, "Video folder", now);
    app.handle(Input::Text("x".into()), now);
    let (folder, audio) = (block(&app, "Video folder"), block(&app, "Audio"));
    let current = app.current();
    app.update(
        with(
            |p| {
                p.record_dir = "~/Elsewhere".into();
                p.audio = false;
            },
            now,
        ),
        now,
        |page, row| page == current && row == audio,
    );
    assert_eq!(
        app.value(audio),
        Some(Value::Bool(true)),
        "its write is still out"
    );
    let Block::Row(row) = &app.page().blocks[folder] else {
        unreachable!()
    };
    assert_eq!(
        row.control.value(),
        Value::Text("~/Videos/boltsnap".into()),
        "being edited"
    );
}

#[test]
fn a_new_notice_rebuilds_the_pages_on_the_same_page() {
    let now = Instant::now();
    let mut app = app_on("replay", SIZE, now);
    let notice = Notice {
        text: "Boltsnap is not running, so changes cannot be saved".into(),
        locked: true,
    };
    app.update(pages::pages(&prefs(), Some(&notice), now), now, idle);
    assert_eq!(app.page().name, "Replay");
    assert!(matches!(app.page().blocks[0], Block::Banner { .. }));
}
