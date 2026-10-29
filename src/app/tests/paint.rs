//! What a frame has to paint (settings paint speed plan).

use std::time::Instant;

use super::{SETTLED, app, control, press};

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
        crate::ui::Input::Press {
            x: size.x,
            y: size.y + 15.0,
            clicks: 1,
        },
        now,
    );
    let later = now + SETTLED;
    let (_, shows) = app.pointer_moved(size.x, size.y + 15.0, later);
    assert!(shows, "dragging, even where the value stays");
    app.handle(crate::ui::Input::Release, later);

    let mut app = super::app("recording", now);
    let rate = control(&app, "Frame rate", now);
    press(&mut app, (rate.x + 4.0, rate.y + 4.0), now);
    let (_, list) = app.open_overlay(now).expect("open");
    let (_, shows) = app.pointer_moved(list.x + 10.0, list.y + 10.0, later);
    assert!(shows, "the list is open");
}
