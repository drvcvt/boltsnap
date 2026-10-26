//! Crossing a layout threshold moves on springs (settings plan, Phase 4c).

use std::time::{Duration, Instant};

use super::{SETTLED, app_on, block};
use crate::app::{Channel, Target};
use crate::ui::text::Fonts;
use crate::ui::theme::{GLIDE, NAV_NARROW_W, NAV_W, ROW_H};
use crate::ui::widgets::{Part, Segment, Stepper, Widget};
use crate::ui::{Key, Mods, Rect};

const WIDE: (f32, f32) = (1000.0, 520.0);
const NARROW: (f32, f32) = (360.0, 520.0);

fn halfway(now: Instant) -> Instant {
    now + Duration::from_secs_f32(GLIDE.duration / 2.0)
}

#[test]
fn closing_the_navigation_and_stacking_a_row_glide_and_end_exactly() {
    let now = Instant::now();
    let mut app = app_on("recording", WIDE, now);
    let source = block(&app, "Source");
    let stacked = app_on("recording", NARROW, now).block_rects(now)[source].h;
    assert!(stacked > ROW_H);
    app.resize(NARROW, now);
    assert_eq!(app.nav_w(now), NAV_W, "starts where it was");
    assert_eq!(app.block_rects(now)[source].h, ROW_H);
    let mid = halfway(now);
    let w = app.nav_w(mid);
    assert!(NAV_NARROW_W < w && w < NAV_W, "navigation halfway: {w}");
    let h = app.block_rects(mid)[source].h;
    assert!(ROW_H < h && h < stacked, "row halfway: {h}");
    assert!(app.animating(mid));
    let end = now + SETTLED;
    assert_eq!(app.nav_w(end), NAV_NARROW_W);
    assert_eq!(app.block_rects(end)[source].h, stacked);
    assert!(!app.animating(end));
    // And back, from wherever it is.
    app.resize(WIDE, mid);
    let back = app.nav_w(halfway(mid));
    assert!(back > w, "opening again: {back}");
    assert_eq!(app.nav_w(mid + SETTLED), NAV_W);
    assert_eq!(app.block_rects(mid + SETTLED)[source].h, ROW_H);
}

#[test]
fn with_reduced_motion_the_shape_changes_at_once() {
    let now = Instant::now();
    let mut app = app_on("recording", WIDE, now);
    app.reduced_motion = true;
    app.resize(NARROW, now);
    assert_eq!(app.nav_w(now), NAV_NARROW_W);
    assert!(app.block_rects(now)[block(&app, "Source")].h > ROW_H);
}

#[test]
fn a_page_shown_in_a_narrow_window_starts_stacked() {
    let now = Instant::now();
    let mut app = app_on("shelf", NARROW, now);
    let settled = app_on("recording", NARROW, now);
    assert!(app.show_named("recording", now + SETTLED));
    let source = block(&app, "Source");
    assert_eq!(
        app.block_rects(now + SETTLED + SETTLED)[source].h,
        settled.block_rects(now + SETTLED)[source].h
    );
    assert!(app.stacked.iter().all(|spring| spring.done(now + SETTLED)));
}

#[test]
fn the_current_entry_brightens_and_the_old_one_dims_on_springs() {
    let now = Instant::now();
    let mut app = app_on("shelf", WIDE, now);
    assert_eq!(app.fade(Target::Nav(0), Channel::Current, now), 1.0);
    assert!(app.show_named("replay", now));
    let mid = now + Duration::from_millis(60);
    let new = app.fade(Target::Nav(3), Channel::Current, mid);
    let old = app.fade(Target::Nav(0), Channel::Current, mid);
    assert!(0.0 < new && new < 1.0 && 0.0 < old && old < 1.0);
    assert_eq!(
        app.fade(Target::Nav(3), Channel::Current, now + SETTLED),
        1.0
    );
    assert_eq!(
        app.fade(Target::Nav(0), Channel::Current, now + SETTLED),
        0.0
    );
}

#[test]
fn the_scroll_thumb_fades_in_when_the_page_gets_longer_than_the_window() {
    let now = Instant::now();
    let mut app = app_on("recording", (760.0, 2000.0), now);
    assert_eq!(app.thumb_alpha(now), 0.0);
    app.resize((760.0, 400.0), now);
    let mid = now + Duration::from_millis(60);
    assert!(app.thumb_alpha(mid) > 0.0 && app.thumb_alpha(mid) < 1.0);
    assert_eq!(app.thumb_alpha(now + SETTLED), 1.0);
}

#[test]
fn segment_parts_follow_the_width_without_a_jump() {
    let now = Instant::now();
    let fonts = Fonts::fallback();
    let segment = Segment::new(
        vec![
            Part::text("System and mic"),
            Part::text("Mic"),
            Part::text("System"),
        ],
        0,
        now,
    );
    let mut last: Option<(f32, Vec<Rect>)> = None;
    for room in 60..700 {
        let room = room as f32;
        let width = segment.width(&fonts, room);
        let parts = segment.part_rects(&fonts, Rect::new(0.0, 0.0, room, 30.0));
        if let Some((last_width, last_parts)) = &last {
            assert!((width - last_width).abs() <= 1.0, "width jumps at {room}");
            for (a, b) in parts.iter().zip(last_parts) {
                assert!((a.w - b.w).abs() <= 1.0, "a part jumps at {room}");
            }
        }
        last = Some((width, parts));
    }
}

#[test]
fn a_stepper_button_fades_when_it_reaches_its_bound() {
    let now = Instant::now();
    let fonts = Fonts::fallback();
    let mut stepper = Stepper::new(7, 1, 8, now);
    assert!(!stepper.animating(now));
    stepper.key(Key::Right, Mods::default(), &fonts, now);
    assert!(stepper.animating(now + Duration::from_millis(60)));
    assert!(!stepper.animating(now + SETTLED));
}

#[test]
fn the_page_moves_in_whole_pixels_while_it_scrolls_rises_and_reshapes() {
    use crate::ui::Input;
    let now = Instant::now();
    let mut app = app_on("recording", WIDE, now);
    app.handle(
        Input::Wheel {
            notches: 1,
            pixels: 0.0,
        },
        now,
    );
    app.resize(NARROW, now + Duration::from_millis(20));
    assert!(app.show_named("replay", now + Duration::from_millis(30)));
    assert!(app.show_named("recording", now + Duration::from_millis(40)));
    let whole = |v: f32| v == v.round();
    for ms in (40..400).step_by(7) {
        let at = now + Duration::from_millis(ms);
        let layout = app.layout(at);
        assert!(
            whole(layout.title.y) && whole(layout.title.x),
            "title at {ms} ms"
        );
        for rect in layout.blocks.iter().chain(&layout.panels) {
            assert!(
                whole(rect.x) && whole(rect.y) && whole(rect.h),
                "{rect:?} at {ms} ms"
            );
        }
        for (index, block) in app.page().blocks.iter().enumerate() {
            if let crate::app::Block::Row(row) = block {
                let parts = app.row_parts(layout.blocks[index], row);
                assert!(
                    whole(parts.label.y) && whole(parts.control.y),
                    "row {index} at {ms} ms"
                );
            }
        }
    }
}

#[test]
fn navigation_icons_keep_their_place_and_size_while_it_closes() {
    let now = Instant::now();
    let mut app = app_on("recording", WIDE, now);
    let open = app.nav_icon(0, now);
    assert_eq!(
        (open.x, open.w),
        (16.0, 14.0),
        "8 px in, as above and below"
    );
    app.resize(NARROW, now);
    for step in 0..=60 {
        let icon = app.nav_icon(0, now + Duration::from_millis(5 * step));
        assert_eq!((icon.x, icon.w), (open.x, open.w), "at {} ms", 5 * step);
    }
    // Closed, the entry is a square with the icon in its middle.
    let slot = app.nav_rect(0, now + SETTLED);
    assert_eq!((slot.w, slot.h), (30.0, 30.0));
    let icon = app.nav_icon(0, now + SETTLED);
    assert_eq!(icon.x - slot.x, slot.right() - icon.right());
    assert_eq!(app.nav_text_x(), 38.0);
}
