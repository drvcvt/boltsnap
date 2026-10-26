//! The keyboard focus outline (settings plan, 5c S2).

use std::time::Instant;

use super::{SETTLED, app_on, block, control, focus, press};
use crate::ui::{Canvas, Rect, theme};

/// The red channel 1 px left of `rect`'s left edge, halfway down.
fn left_of(app: &crate::app::App, rect: Rect, at: Instant) -> u8 {
    let (w, h) = app.size();
    let mut canvas = Canvas::new(w as u32, h as u32, 1).unwrap();
    app.draw(&mut canvas, at);
    let (x, y) = ((rect.x - 1.0) as usize, (rect.y + rect.h / 2.0) as usize);
    canvas.pixmap.data()[(y * w as usize + x) * 4]
}

#[test]
fn tab_shows_the_outline_and_a_click_hides_it() {
    let now = Instant::now();
    let mut app = app_on("recording", (1000.0, 700.0), now);
    let rect = control(&app, "Frame rate", now);
    assert_eq!(
        left_of(&app, rect, now + SETTLED),
        theme::PANEL.0,
        "no focus"
    );
    focus(&mut app, "Frame rate", now);
    let lit = left_of(&app, rect, now + SETTLED);
    assert!(
        lit > theme::PANEL.0 + 0x40,
        "outline from the keyboard: {lit:#x}"
    );
    // Clicking the same control keeps its focus but drops the outline.
    let later = now + SETTLED;
    press(&mut app, (rect.x + 10.0, rect.y + 10.0), later);
    press(&mut app, (rect.x + 10.0, rect.y + 10.0), later);
    assert_eq!(
        app.focus(),
        Some(crate::app::Target::Control(block(&app, "Frame rate")))
    );
    assert_eq!(
        left_of(&app, rect, later + SETTLED),
        theme::PANEL.0,
        "gone after a click"
    );
}
