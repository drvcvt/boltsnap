//! Blocks that come and go open and close their room (settings plan, Phase
//! 5l): "Restart replay" appears under "Replay buffer" once a replay setting
//! needs a restart, and goes again.

use std::time::{Duration, Instant};

use super::super::{AT_REST, App, Target, pages};
use super::{SIZE, block, prefs};
use crate::ui::text::Fonts;

fn replay_pages(stale: bool, now: Instant) -> Vec<super::super::Page> {
    let mut set = prefs();
    set.replay_running = true;
    set.replay_stale = stale;
    pages::pages(&set, None, now)
}

fn replay_app(now: Instant) -> App {
    let mut app = App::new(Fonts::fallback(), replay_pages(false, now), SIZE, now);
    assert!(app.show_named("Replay", now));
    app
}

/// The top of the row named `label` at `at`.
fn top(app: &App, label: &str, at: Instant) -> f32 {
    app.block_rects(at)[block(app, label)].y
}

#[test]
fn a_row_that_comes_opens_its_room_and_the_rows_below_glide() {
    let now = Instant::now();
    let mut app = replay_app(now);
    let before = top(&app, "Display", now);
    app.update(replay_pages(true, now), now, |_, _| false);
    let restart = block(&app, "Restart replay");
    // At first it takes no room: nothing below has moved yet.
    assert_eq!(top(&app, "Display", now), before);
    let end = top(&app, "Display", now + AT_REST);
    let row = app.block_rects(now + AT_REST)[restart];
    assert!(end >= before + row.h, "{before} → {end}");
    // On the way: between, and only ever further down.
    let mut last = before;
    for ms in [30, 60, 120, 240] {
        let y = top(&app, "Display", now + Duration::from_millis(ms));
        assert!(y >= last && y <= end, "{ms} ms: {y}");
        last = y;
    }
    assert!(last > before && last < end || last == end);
    // A row still opening takes no click; once there it does.
    let control = app
        .row_parts(app.block_rects(now)[restart], app.row(restart).unwrap())
        .control;
    let centre = (control.x + control.w / 2.0, control.y + control.h / 2.0);
    let early = now + Duration::from_millis(30);
    assert_ne!(app.target_at(centre, early), Some(Target::Control(restart)));
    let settled = now + AT_REST;
    let control = app
        .row_parts(app.block_rects(settled)[restart], app.row(restart).unwrap())
        .control;
    let centre = (control.x + control.w / 2.0, control.y + control.h / 2.0);
    assert_eq!(
        app.target_at(centre, settled),
        Some(Target::Control(restart))
    );
    // It needs frames while it moves, and none once it is there.
    assert!(app.animating(early));
}

#[test]
fn a_row_that_goes_closes_its_room_and_the_rows_below_glide_back() {
    let now = Instant::now();
    let mut app = replay_app(now);
    let before = top(&app, "Display", now);
    app.update(replay_pages(true, now), now, |_, _| false);
    let open = now + AT_REST;
    let opened = top(&app, "Display", open);
    app.update(replay_pages(false, open), open, |_, _| false);
    // Gone from the page at once (no target), still taking its room.
    assert!(
        app.page()
            .blocks
            .iter()
            .all(|b| !matches!(b, super::super::Block::Row(r) if r.label == "Restart replay"))
    );
    assert_eq!(top(&app, "Display", open), opened);
    let middle = top(&app, "Display", open + Duration::from_millis(120));
    assert!(
        middle < opened && middle > before,
        "{before} < {middle} < {opened}"
    );
    assert_eq!(top(&app, "Display", open + AT_REST), before);
}

#[test]
fn with_reduced_motion_a_row_is_there_at_once() {
    let now = Instant::now();
    let mut app = replay_app(now);
    app.reduced_motion = true;
    app.update(replay_pages(true, now), now, |_, _| false);
    assert_eq!(
        top(&app, "Display", now),
        top(&app, "Display", now + AT_REST)
    );
    app.update(replay_pages(false, now), now, |_, _| false);
    assert_eq!(
        top(&app, "Display", now),
        top(&app, "Display", now + AT_REST)
    );
}

/// `PRESENCE_OUT=file cargo test --lib presence_shots -- --ignored`: the
/// Replay page as "Restart replay" comes in, at 0, 60, 120, 200 ms and at
/// rest, side by side.
#[test]
#[ignore = "writes a PNG to PRESENCE_OUT"]
fn presence_shots() {
    let out = std::env::var("PRESENCE_OUT").expect("PRESENCE_OUT");
    let now = Instant::now();
    let mut app = replay_app(now);
    app.update(replay_pages(true, now), now, |_, _| false);
    let (w, h) = (SIZE.0 as u32, SIZE.1 as u32);
    let stages = [0u64, 60, 120, 200, 2000];
    let mut strip = image::RgbaImage::new(w * stages.len() as u32, h);
    for (n, ms) in stages.iter().enumerate() {
        let mut canvas = crate::ui::Canvas::new(w, h, 1).unwrap();
        app.draw(&mut canvas, now + Duration::from_millis(*ms));
        for (i, px) in canvas.pixmap.data().as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (i as u32 % w, i as u32 / w);
            strip.put_pixel(n as u32 * w + x, y, image::Rgba(*px));
        }
    }
    strip.save(out).unwrap();
}
