//! Values that change elsewhere reach the open window (settings plan, 5c S6).

use std::time::{Duration, Instant};

use super::{SETTLED, app_on, block, focus, prefs};
use crate::app::pages::{self, Cursor, Notice, Prefs};
use crate::app::{App, Block};
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

#[test]
fn a_running_recording_shows_one_note_and_no_row_repeats_it() {
    let now = Instant::now();
    let mut app = App::new(
        crate::ui::text::Fonts::fallback(),
        with(|p| p.recording = true, now),
        SIZE,
        now,
    );
    app.show_named("recording", now);
    let Block::Banner { text, .. } = &app.page().blocks[0] else {
        panic!("a note on top while recording");
    };
    assert!(text.contains("next one"));
    let idle = app_on("recording", SIZE, now);
    assert!(
        !matches!(idle.page().blocks[0], Block::Banner { .. }),
        "no note otherwise"
    );
    for page in pages::pages(&prefs(), None, now) {
        for block in &page.blocks {
            if let Block::Row(row) = block {
                assert!(
                    !row.hint
                        .as_deref()
                        .is_some_and(|h| h.starts_with("Applies")),
                    "{} repeats a note",
                    row.label
                );
            }
        }
    }
}

#[test]
fn restart_replay_shows_only_for_a_stale_running_buffer() {
    let now = Instant::now();
    let has_restart = |prefs: &mut dyn FnMut(&mut Prefs)| {
        let mut p = super::prefs();
        prefs(&mut p);
        pages::pages(&p, None, now)
            .iter()
            .flat_map(|page| &page.blocks)
            .any(|block| matches!(block, Block::Row(row) if row.label == "Restart replay"))
    };
    assert!(!has_restart(&mut |_| {}));
    assert!(
        !has_restart(&mut |p| p.replay_running = true),
        "running and current"
    );
    assert!(!has_restart(&mut |p| p.replay_stale = true), "not running");
    assert!(has_restart(&mut |p| {
        p.replay_running = true;
        p.replay_stale = true;
    }));
    let mut app = App::new(
        crate::ui::text::Fonts::fallback(),
        with(
            |p| {
                p.replay_running = true;
                p.replay_stale = true;
            },
            now,
        ),
        SIZE,
        now,
    );
    app.show_named("replay", now);
    let restart = block(&app, "Restart replay");
    let Block::Row(row) = &app.page().blocks[restart] else {
        unreachable!()
    };
    let button = app.row_parts(app.block_rects(now)[restart], row).control;
    let at = (button.x + button.w / 2.0, button.y + button.h / 2.0);
    app.handle(
        Input::Press {
            x: at.0,
            y: at.1,
            clicks: 1,
        },
        now,
    );
    app.handle(Input::Release, now);
    assert_eq!(app.write(restart), Some(crate::app::Write::RestartReplay));
}
