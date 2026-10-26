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

#[test]
fn the_replay_bar_marks_round_steps_fills_with_what_it_holds_and_greys_what_cannot_fit() {
    use crate::app::replay_step as step;
    // At most six marks on a wide bar, fewer on a narrow one.
    assert_eq!(
        (step(15.0, 656.0), step(30.0, 656.0), step(60.0, 656.0)),
        (5, 5, 10)
    );
    assert_eq!(step(600.0, 656.0), 120);
    assert_eq!(step(60.0, 266.0), 15);
    let now = Instant::now();
    let mut app = app_on("replay", SIZE, now);
    assert_eq!(app.replay_bar(now), (0.0, 0.0), "stopped and enough memory");
    // A quarter of a minute held: a quarter filled, from the right.
    app.set_replay_held(Some(15.0), now);
    let (filled, _) = app.replay_bar(now + SETTLED);
    assert!((filled - 0.25).abs() < 0.001, "{filled}");
    // Ten minutes in 256 MiB: about 335 s fit, the older rest is greyed.
    let mut p = super::prefs();
    p.replay_seconds = 600;
    p.replay_memory_mib = 256;
    let mut tight = App::new(
        crate::ui::text::Fonts::fallback(),
        crate::app::pages::pages(&p, None, now),
        SIZE,
        now,
    );
    tight.show_named("replay", now);
    let (_, lost) = tight.replay_bar(now);
    assert!((lost - (1.0 - 335.0 / 600.0)).abs() < 0.01, "{lost}");
    // A new length, once the fill stands: the scale glides there.
    let later = now + SETTLED;
    focus(&mut app, "Length", later);
    key(&mut app, Key::Enter, later);
    key(&mut app, Key::Down, later);
    key(&mut app, Key::Enter, later);
    let (half, _) = app.replay_bar(later + Duration::from_millis(150));
    let (end, _) = app.replay_bar(later + SETTLED);
    assert!((end - 0.125).abs() < 0.001, "15 s of 2 min: {end}");
    assert!(half > end && half < 0.25, "gliding: {half}");
}

#[test]
fn the_replay_head_says_what_it_holds_and_the_bar_is_cells() {
    use crate::app::replay_cells as cells;
    assert_eq!(
        (cells(15.0), cells(30.0), cells(60.0), cells(600.0)),
        (15, 30, 60, 60)
    );
    let now = Instant::now();
    let mut app = app_on("replay", SIZE, now);
    // 60 s in 512 MiB: 20 994 kbit/s, as the recorder starts with.
    assert_eq!(
        app.replay_head(now),
        ("Off".into(), "21\u{a0}Mbit/s".into())
    );
    app.set_replay_held(Some(15.0), now);
    assert_eq!(app.replay_head(now + SETTLED).0, "Holds 15\u{a0}s");
}

#[test]
fn monitors_are_named_by_maker_and_model() {
    use crate::app::pages::monitor_name as name;
    assert_eq!(name("BNQ", "ZOWIE XL LCD", "DP-3"), "BenQ ZOWIE XL LCD");
    assert_eq!(name("AOC", "27G4HRE", "DP-1"), "AOC 27G4HRE");
    assert_eq!(
        name("GSM", "LG ULTRAGEAR", "DP-2"),
        "LG ULTRAGEAR",
        "not twice"
    );
    assert_eq!(name("del", "U2720Q", "DP-4"), "Dell U2720Q");
    assert_eq!(name("Unknown", "", "HDMI-A-1"), "HDMI-A-1");
    assert_eq!(name("", "Virtual-1", "WL-1"), "Virtual-1");
}

fn recording_with(prefs: &crate::app::pages::Prefs, now: Instant) -> App {
    let mut app = App::new(
        crate::ui::text::Fonts::fallback(),
        crate::app::pages::pages(prefs, None, now),
        SIZE,
        now,
    );
    app.show_named("recording", now);
    app
}

fn screen(name: &str, x: f32, y: f32) -> crate::app::pages::Screen {
    crate::app::pages::Screen {
        name: name.into(),
        label: format!("Model {name}"),
        x,
        y,
        w: 1920.0,
        h: 1080.0,
    }
}

#[test]
fn the_line_under_the_monitors_says_what_a_recording_makes_and_crossfades() {
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    let captions = |app: &App, at| app.display_captions(at);
    assert_eq!(
        captions(&app, now),
        vec![("The one with focus when you start".into(), 1.0)]
    );
    // The AOC alone: one clip; the old line fades out as the new one comes.
    focus(&mut app, "Display", now);
    for k in [Key::Enter, Key::Down, Key::Down, Key::Enter] {
        key(&mut app, k, now);
    }
    let fading = captions(&app, now + Duration::from_millis(60));
    assert_eq!(fading.len(), 2, "{fading:?}");
    assert!(fading.iter().all(|(_, shown)| *shown > 0.0 && *shown < 1.0));
    assert_eq!(
        captions(&app, now + SETTLED),
        vec![("One clip".into(), 1.0)]
    );
    // Both, separately: two clips; combined: one.
    let later = now + SETTLED;
    for k in [Key::Enter, Key::Down, Key::Enter] {
        key(&mut app, k, later);
    }
    assert_eq!(captions(&app, later + SETTLED)[0].0, "Two clips");
    let later = later + SETTLED;
    focus(&mut app, "Both displays as", later);
    key(&mut app, Key::Right, later);
    assert_eq!(
        captions(&app, later + SETTLED),
        vec![("One clip".into(), 1.0)]
    );
    // A configured monitor that is not plugged in: the focused one records.
    let mut prefs = super::prefs();
    prefs.display = crate::app::pages::Display::Output("HDMI-A-9".into());
    let missing = recording_with(&prefs, now);
    assert_eq!(
        missing.display_captions(now)[0].0,
        "Not connected, the focused one records"
    );
    let (lit, _) = displays(&missing, now);
    assert!(lit.iter().all(|(_, on)| *on == 0.5));
}

#[test]
fn three_monitors_in_a_row_and_two_stacked_keep_their_names_apart() {
    let now = Instant::now();
    let mut prefs = super::prefs();
    prefs.displays = vec![
        screen("DP-1", 0.0, 0.0),
        screen("DP-2", 1920.0, 0.0),
        screen("DP-3", 3840.0, 0.0),
    ];
    prefs.display = crate::app::pages::Display::Both;
    let app = recording_with(&prefs, now);
    assert_eq!(app.display_captions(now)[0].0, "Three clips");
    let index = |app: &App| {
        app.page()
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Preview(Preview::Displays(_))))
            .unwrap()
    };
    let rect = app.layout(now).blocks[index(&app)];
    let names = crate::app::display_name_places(rect, &prefs.displays);
    let bodies = crate::app::display_places(rect, &prefs.displays);
    for (name, body) in names.iter().zip(&bodies) {
        assert_eq!((name.x, name.w), (body.x, body.w), "under its monitor");
        assert!(name.y > body.bottom() + 8.0, "below the stand");
    }
    // One above the other: each name gets half the width, in order.
    prefs.displays = vec![screen("DP-1", 0.0, 0.0), screen("DP-2", 0.0, 1080.0)];
    let app = recording_with(&prefs, now);
    let rect = app.layout(now).blocks[index(&app)];
    let names = crate::app::display_name_places(rect, &prefs.displays);
    assert!(!names[0].overlaps(names[1]) && names[0].w == names[1].w);
}

#[test]
fn a_whole_screen_is_told_from_an_area_and_pictures_fit_and_cover() {
    use crate::app::shots::{cover, fit, same_shape, whole_screen};
    assert!(whole_screen((1920, 1080)) && whole_screen((3840, 1080)));
    assert!(!whole_screen((588, 375)) && !whole_screen((1920, 600)));
    assert_eq!(fit((3840, 1080), (1600, 900)), (1600, 450));
    assert_eq!(fit((1920, 1080), (1600, 900)), (1600, 900));
    assert_eq!(fit((300, 200), (1600, 900)), (300, 200), "never grown");
    let pair = Rect::new(0.0, 0.0, 569.0, 160.0);
    assert!(same_shape((3840, 1080), pair) && !same_shape((1920, 1080), pair));
    // Over both monitors onto a 16:9 screen: the left part, full height.
    let screen = Rect::new(10.0, 20.0, 640.0, 360.0);
    assert_eq!(
        cover((3840, 1080), screen),
        Rect::new(10.0, 20.0, 1280.0, 360.0)
    );
    // 4:3 onto 16:9: full width, the middle.
    assert_eq!(
        cover((1600, 1200), screen),
        Rect::new(10.0, -40.0, 640.0, 480.0)
    );
}
