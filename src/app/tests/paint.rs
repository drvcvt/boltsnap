//! What a frame has to paint (settings paint speed plan).

use std::time::{Duration, Instant};

use super::{SETTLED, SIZE, app, control, press};
use crate::app::{App, Block, gallery};
use crate::ui::text::Fonts;
use crate::ui::{Canvas, Input};

#[test]
fn a_pointer_move_that_changes_nothing_shows_nothing_new() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let rate = control(&app, "Frame rate", now);
    let (_, shows) = app.pointer_moved(rate.x + 4.0, rate.y + 4.0, now);
    assert!(shows, "the hover comes to the row");
    let later = now + SETTLED;
    let (_, shows) = app.pointer_moved(rate.x + 9.0, rate.y + 6.0, later);
    assert!(!shows, "still on the same control, nothing moves");
    let cursor = control(&app, "Cursor", later);
    let (_, shows) = app.pointer_moved(cursor.x + 4.0, cursor.y + 15.0, later);
    assert!(shows, "the hover goes to another row");
}

#[test]
fn a_move_onto_another_part_of_the_same_control_shows() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let cursor = control(&app, "Cursor", now);
    app.pointer_moved(cursor.x + 4.0, cursor.y + 15.0, now);
    let later = now + SETTLED;
    let (_, shows) = app.pointer_moved(cursor.right() - 4.0, cursor.y + 15.0, later);
    assert!(shows, "the part under the pointer lights");
}

#[test]
fn a_drag_and_an_open_list_show_every_move() {
    let now = Instant::now();
    let mut app = app("shelf", now);
    let size = control(&app, "Card size", now);
    app.handle(
        Input::Press {
            x: size.x,
            y: size.y + 15.0,
            clicks: 1,
        },
        now,
    );
    let later = now + SETTLED;
    let (_, shows) = app.pointer_moved(size.x, size.y + 15.0, later);
    assert!(shows, "dragging, even where the value stays");
    app.handle(Input::Release, later);

    let mut app = super::app("recording", now);
    let rate = control(&app, "Frame rate", now);
    press(&mut app, (rate.x + 4.0, rate.y + 4.0), now);
    let (_, list) = app.open_overlay(now).expect("open");
    let (_, shows) = app.pointer_moved(list.x + 10.0, list.y + 10.0, later);
    assert!(shows, "the list is open");
}

/// Paints frame after frame 1/240 s apart as the window does, a part where
/// `damage` allows one, with each of `inputs` (ms from the start) in turn,
/// and checks every frame against the whole frame painted anew. As in the
/// window, a frame after any input but a pointer move is whole. Returns how
/// many frames were parts.
fn parts_match_wholes(app: &mut App, scale: u32, frames: u32, inputs: &[(u64, Input)]) -> usize {
    let start = Instant::now() + 2 * SETTLED;
    let whole = |app: &App, at: Instant| {
        let mut canvas = Canvas::new(SIZE.0 as u32, SIZE.1 as u32, scale).expect("canvas");
        app.draw(&mut canvas, at);
        canvas
    };
    assert_eq!(app.damage(start, true), None, "the first frame is whole");
    let mut last = whole(app, start);
    let mut parts = 0;
    let mut next = inputs.iter().peekable();
    for frame in 1..=frames {
        let at = start + Duration::from_secs_f64(f64::from(frame) / 240.0);
        let mut full = false;
        while let Some((_, input)) =
            next.next_if(|(ms, _)| start + Duration::from_millis(*ms) <= at)
        {
            match *input {
                Input::Move { x, y } => {
                    app.pointer_moved(x, y, at);
                }
                ref input => {
                    app.handle(input.clone(), at);
                    full = true;
                }
            }
        }
        let mut canvas = Canvas {
            pixmap: last.pixmap.clone(),
            ..last
        };
        if let Some(rects) = app.damage(at, full) {
            parts += 1;
            for rect in rects {
                if let Some(mut part) = canvas.part(rect) {
                    app.draw(&mut part, at);
                    canvas.paste(&part);
                }
            }
        } else {
            app.draw(&mut canvas, at);
        }
        assert!(
            canvas.pixmap.data() == whole(app, at).pixmap.data(),
            "frame {frame} differs from the whole frame"
        );
        last = canvas;
    }
    parts
}

/// The pointer to the middle of every row's control and of every navigation
/// entry, one after the other `every` ms apart.
fn tour(app: &App, every: u64) -> Vec<(u64, Input)> {
    let now = Instant::now();
    let rects = app.block_rects(now);
    let controls = app
        .page()
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let Block::Row(row) = block else { return None };
            Some(app.row_parts(rects[index], row).control)
        });
    let nav = (0..3).map(|index| app.nav_rect(index, now));
    controls
        .chain(nav)
        .filter(|rect| rect.bottom() < SIZE.1)
        .enumerate()
        .map(|(index, rect)| {
            let (x, y) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            (index as u64 * every, Input::Move { x, y })
        })
        .collect()
}

/// `moves` with a click 8 ms after each.
fn clicking(moves: Vec<(u64, Input)>) -> Vec<(u64, Input)> {
    moves
        .into_iter()
        .flat_map(|(ms, input)| {
            let Input::Move { x, y } = input else {
                return vec![(ms, input)];
            };
            let press = Input::Press { x, y, clicks: 1 };
            vec![(ms, input), (ms + 8, press), (ms + 10, Input::Release)]
        })
        .collect()
}

/// Frames until the fades after the last of `moves` have settled.
fn frames(moves: &[(u64, Input)]) -> u32 {
    let last = moves.last().map_or(0, |(ms, _)| *ms);
    ((last + 150) * 240 / 1000) as u32
}

#[test]
fn hover_fades_paint_parts_that_match_whole_frames() {
    let now = Instant::now();
    let mut app = app("recording", now);
    let rate = control(&app, "Frame rate", now);
    let cursor = control(&app, "Cursor", now);
    let nav = app.nav_rect(1, now);
    // Changes in the middle of fades, between parts of one control, into
    // the navigation and back.
    let moves = [
        (
            0,
            Input::Move {
                x: rate.x + 4.0,
                y: rate.y + 4.0,
            },
        ),
        (
            40,
            Input::Move {
                x: cursor.x + 4.0,
                y: cursor.y + 15.0,
            },
        ),
        (
            90,
            Input::Move {
                x: cursor.right() - 4.0,
                y: cursor.y + 15.0,
            },
        ),
        (
            150,
            Input::Move {
                x: nav.x + 10.0,
                y: nav.y + 10.0,
            },
        ),
        (
            200,
            Input::Move {
                x: nav.x + 10.0,
                y: nav.y + nav.h + 10.0,
            },
        ),
        (
            260,
            Input::Move {
                x: rate.x + 4.0,
                y: rate.y + 4.0,
            },
        ),
    ];
    let parts = parts_match_wholes(&mut app, 1, 96, &moves);
    assert!(parts > 48, "{parts} of 96 frames were parts");
}

#[test]
fn every_row_and_entry_hovered_in_turn_paints_parts_that_match() {
    let now = Instant::now();
    for page in ["shelf", "general"] {
        let mut app = super::app(page, now);
        let moves = tour(&app, 20);
        let parts = parts_match_wholes(&mut app, 1, frames(&moves), &moves);
        assert!(parts > 0, "{page}: no frame was a part");
    }
}

#[test]
fn the_gallery_at_twice_the_scale_paints_parts_that_match() {
    let now = Instant::now();
    let mut app = App::new(Fonts::fallback(), gallery::pages(now), SIZE, now);
    assert!(app.show_named("Controls", now));
    let moves = tour(&app, 20);
    let parts = parts_match_wholes(&mut app, 2, frames(&moves), &moves);
    assert!(parts > 0, "the gallery: no frame was a part");
}

#[test]
fn clicked_controls_slide_in_parts_that_match() {
    // Switches and segments slide, fields take the focus, lists open.
    let now = Instant::now();
    let mut app = App::new(Fonts::fallback(), gallery::pages(now), SIZE, now);
    assert!(app.show_named("Controls", now));
    let clicks = clicking(tour(&app, 60));
    let parts = parts_match_wholes(&mut app, 1, frames(&clicks), &clicks);
    assert!(parts > 0, "the gallery, clicked: no frame was a part");
}
