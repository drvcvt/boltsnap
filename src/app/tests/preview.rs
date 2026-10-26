//! The shelf preview follows the controls, to the shelf's own scale
//! (settings plan, 5d P1).

use std::time::{Duration, Instant};

use super::{SETTLED, app_on, focus, key};
use crate::app::{App, Block, Preview, shelf_cards};
use crate::ui::{Key, Rect};

const SIZE: (f32, f32) = (1000.0, 800.0);

fn cards(app: &App, now: Instant) -> Vec<(Rect, f32)> {
    let index = app
        .page()
        .blocks
        .iter()
        .position(|block| matches!(block, Block::Preview(Preview::Shelf)))
        .expect("a shelf preview");
    app.preview_cards(app.layout(now).blocks[index], now)
}

#[test]
fn the_cards_sit_where_the_shelf_puts_them() {
    // 24 from the screen's edges, 22 inside the shelf, 10 between cards.
    let bottom_left = shelf_cards(2, 190.0, 2);
    assert_eq!(
        bottom_left[0],
        Rect::new(46.0, 1080.0 - 46.0 - 132.0, 190.0, 132.0)
    );
    assert_eq!(bottom_left[1].y, bottom_left[0].y - 142.0);
    let top_right = shelf_cards(1, 380.0, 1);
    assert_eq!(
        top_right[0],
        Rect::new(1920.0 - 46.0 - 380.0, 46.0, 380.0, 264.0)
    );
}

#[test]
fn a_new_corner_glides_the_cards_there_and_a_new_card_fades_in_in_place() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let start = cards(&app, now);
    assert_eq!(start.len(), 5, "as many as Cards shown");
    // Bottom left to bottom right: halfway they are between the two.
    focus(&mut app, "Corner", now);
    key(&mut app, Key::Right, now);
    let mid = cards(&app, now + Duration::from_millis(120));
    let end = cards(&app, now + SETTLED);
    assert!(
        mid[0].0.x > start[0].0.x && mid[0].0.x < end[0].0.x,
        "gliding"
    );
    assert!(end[0].0.x > start[0].0.x + 100.0, "on the right");
    // One more card: it fades in where it belongs, the others stay put.
    let later = now + SETTLED;
    focus(&mut app, "Cards shown", later);
    key(&mut app, Key::Right, later);
    let fading = cards(&app, later + Duration::from_millis(60));
    assert_eq!(fading.len(), 6);
    assert!(
        fading[5].1 > 0.0 && fading[5].1 < 1.0,
        "the new card fades in"
    );
    for (before, after) in end.iter().zip(&fading) {
        assert_eq!(before.0, after.0, "the others do not move");
    }
}

fn displays(app: &App, now: Instant) -> (Vec<(Rect, f32)>, f32) {
    let index = app
        .page()
        .blocks
        .iter()
        .position(|block| matches!(block, Block::Preview(Preview::Displays(_))))
        .expect("a display preview");
    app.display_lights(app.layout(now).blocks[index], now)
}

#[test]
fn the_monitors_sit_in_their_order_and_light_up_for_where_a_recording_goes() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    let (lit, bridge) = displays(&app, now);
    // BenQ at 0, AOC at 1920: side by side, the same size, left to right.
    assert_eq!(lit.len(), 2);
    assert!(lit[0].0.x < lit[1].0.x && lit[0].0.w == lit[1].0.w);
    assert_eq!(
        (lit[0].1, lit[1].1, bridge),
        (0.5, 0.5, 0.0),
        "focused: any of them"
    );
    // The AOC: it lights, the BenQ goes dark.
    focus(&mut app, "Display", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Enter, now);
    let (lit, _) = displays(&app, now + SETTLED);
    assert_eq!((lit[0].1, lit[1].1), (0.0, 1.0));
    // Both, as two clips: both light, the seam stays; combined, it closes.
    key(&mut app, Key::Enter, now + SETTLED);
    key(&mut app, Key::Down, now + SETTLED);
    key(&mut app, Key::Enter, now + SETTLED);
    let (lit, bridge) = displays(&app, now + SETTLED + SETTLED);
    assert_eq!((lit[0].1, lit[1].1, bridge), (1.0, 1.0, 0.0));
    let later = now + SETTLED + SETTLED;
    focus(&mut app, "Both displays as", later);
    key(&mut app, Key::Right, later);
    let (_, closing) = displays(&app, later + Duration::from_millis(60));
    assert!(closing > 0.0 && closing < 1.0, "the bridge fades in");
    assert_eq!(displays(&app, later + SETTLED).1, 1.0);
}

#[test]
fn without_known_monitors_there_is_no_display_preview() {
    let now = Instant::now();
    let mut prefs = super::prefs();
    prefs.displays.clear();
    let mut app = App::new(
        crate::ui::text::Fonts::fallback(),
        crate::app::pages::pages(&prefs, None, now),
        SIZE,
        now,
    );
    app.show_named("recording", now);
    assert!(
        !app.page()
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Preview(_)))
    );
}
