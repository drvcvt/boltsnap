//! The previews follow the controls (settings plan, 5d); the shelf's is
//! the shelf itself to try out (5g V6).

use std::time::{Duration, Instant};

use super::{SETTLED, app_on, focus, key};
use crate::app::{App, Block, Preview};
use crate::ui::{Input, Key, Rect};

const SIZE: (f32, f32) = (1000.0, 800.0);

fn shelf_rect(app: &App, now: Instant) -> Rect {
    let index = app
        .page()
        .blocks
        .iter()
        .position(|block| matches!(block, Block::Preview(Preview::Shelf)))
        .expect("a shelf preview");
    app.layout(now).blocks[index]
}

/// The cards now, newest first, with their close buttons.
fn cards(app: &App, now: Instant) -> Vec<(u64, Rect, Rect)> {
    app.play_cards(shelf_rect(app, now), now)
}

fn middle(rect: Rect) -> (f32, f32) {
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

#[test]
fn two_cards_rest_in_the_corner_at_their_real_size() {
    let now = Instant::now();
    let app = app_on("shelf", SIZE, now);
    let rect = shelf_rect(&app, now);
    // Two default cards, 10 between them, 24 from the edges.
    assert_eq!(rect.h, 24.0 + 132.0 + 10.0 + 132.0 + 24.0);
    let cards = cards(&app, now);
    assert_eq!(cards.len(), 2, "two cards, though five are shown");
    // Bottom left, the newest on top as on the shelf.
    let (newest, oldest) = (cards[0].1, cards[1].1);
    assert_eq!(
        oldest,
        Rect::new(rect.x + 24.0, rect.bottom() - 24.0 - 132.0, 190.0, 132.0)
    );
    assert_eq!(newest, Rect::new(oldest.x, oldest.y - 142.0, 190.0, 132.0));
    // The X sits in the top right, 7 in, 15 across, as on the shelf.
    assert_eq!(
        cards[0].2,
        Rect::new(newest.right() - 22.0, newest.y + 7.0, 15.0, 15.0)
    );
}

#[test]
fn a_new_corner_glides_the_cards_there() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let rect = shelf_rect(&app, now);
    let start = cards(&app, now)[1].1;
    focus(&mut app, "Corner", now);
    key(&mut app, Key::Right, now);
    let mid = cards(&app, now + Duration::from_millis(120))[1].1;
    let end = cards(&app, now + SETTLED)[1].1;
    assert!(mid.x > start.x && mid.x < end.x, "gliding");
    assert_eq!(end.right(), rect.right() - 24.0, "on the right");
    assert_eq!(end.y, start.y, "still at the bottom");
}

#[test]
fn one_card_shown_folds_the_second_away() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    focus(&mut app, "Cards shown", now);
    for _ in 0..4 {
        key(&mut app, Key::Left, now);
    }
    let folding = cards(&app, now + Duration::from_millis(60));
    assert_eq!(folding.len(), 2);
    assert!(folding[1].1.h < 132.0, "folding");
    let rest = cards(&app, now + SETTLED);
    assert_eq!(rest.len(), 1);
    let rect = shelf_rect(&app, now);
    assert_eq!(
        rest[0].1.bottom(),
        rect.bottom() - 24.0,
        "down in the corner"
    );
}

#[test]
fn the_x_dismisses_a_card_and_a_new_one_arrives_on_top() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let before = cards(&app, now);
    let (gone, kept) = (before[1].0, before[0].0);
    let (x, y) = middle(before[1].2);
    app.handle(Input::Move { x, y }, now);
    app.handle(Input::Press { x, y, clicks: 1 }, now);
    app.handle(Input::Release, now);
    // First the old one leaves, the other closes up; then the new one
    // arrives.
    let leaving = cards(&app, now + Duration::from_millis(100));
    assert_eq!(leaving.len(), 1, "leaving, not a card");
    assert_eq!(leaving[0].0, kept);
    let arriving = cards(&app, now + Duration::from_millis(300));
    assert_eq!(arriving.len(), 2);
    let after = cards(&app, now + SETTLED);
    assert_eq!(after.len(), 2);
    assert_eq!(after[1].0, kept, "the other card moved down a place");
    assert!(after[0].0 != gone && after[0].0 != kept, "a new one on top");
    assert_eq!(after[0].1, before[0].1);
    assert_eq!(after[1].1, before[1].1);
    // A click on the card, not its X, keeps it.
    let later = now + SETTLED;
    let (x, y) = (after[1].1.x + 20.0, after[1].1.y + 60.0);
    app.handle(Input::Press { x, y, clicks: 1 }, later);
    app.handle(Input::Release, later);
    assert_eq!(cards(&app, later + SETTLED), after);
}

#[test]
fn without_animations_a_card_goes_and_comes_at_once() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    focus(&mut app, "Animations", now);
    key(&mut app, Key::Space, now);
    // After the page has scrolled to the row.
    let now = now + SETTLED;
    let before = cards(&app, now);
    let (x, y) = middle(before[0].2);
    app.handle(Input::Press { x, y, clicks: 1 }, now);
    let after = cards(&app, now);
    assert_eq!(after.len(), 2);
    assert_eq!(after[1].0, before[1].0);
    assert!(after[0].0 != before[0].0, "a new one at once");
    assert_eq!(
        cards(&app, now + Duration::from_millis(60)),
        after,
        "nothing moves"
    );
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
fn one_clip_joins_the_monitors_bodies_and_keeps_their_screens_whole() {
    use crate::ui::theme::{PANEL, RAISE};
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    let pixel = |app: &App, at: Instant, (x, y): (f32, f32)| {
        let mut canvas = crate::ui::Canvas::new(SIZE.0 as u32, SIZE.1 as u32, 1).unwrap();
        app.draw(&mut canvas, at);
        let i = ((y as u32 * SIZE.0 as u32 + x as u32) * 4) as usize;
        let d = canvas.pixmap.data();
        (d[i], d[i + 1], d[i + 2])
    };
    // Both displays, as two clips first.
    focus(&mut app, "Display", now);
    key(&mut app, Key::Enter, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Down, now);
    key(&mut app, Key::Enter, now);
    let at = now + SETTLED;
    let (lit, _) = displays(&app, at);
    let (left, right) = (lit[0].0, lit[1].0);
    let middle = left.y + left.h / 2.0;
    let gap = ((left.right() + right.x) / 2.0, middle);
    // The left screen's inner frame, a pixel in from the body's edge.
    let frame = (left.right() - 1.0, middle);
    assert_eq!(pixel(&app, at, gap), PANEL, "two clips: a gap");
    assert_eq!(pixel(&app, at, frame), RAISE);
    focus(&mut app, "Both displays as", at);
    key(&mut app, Key::Right, at);
    let at = at + SETTLED;
    assert_eq!(pixel(&app, at, gap), RAISE, "one clip: one body");
    assert_eq!(pixel(&app, at, frame), RAISE, "the screen keeps its frame");
    // Top and bottom of the join: no notch of the panel between them.
    assert_eq!(pixel(&app, at, (gap.0, left.y + 1.0)), RAISE);
    assert_eq!(pixel(&app, at, (gap.0, left.bottom() - 1.0)), RAISE);
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
fn the_front_card_measures_190_by_132_beside_it_and_follows_the_slider() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    assert_eq!(app.measure_shown(now), 1.0, "room beside the card");
    assert_eq!(
        app.shelf_measure_text().as_deref(),
        Some("190\u{a0}×\u{a0}132\u{a0}px")
    );
    let rect = shelf_rect(&app, now);
    let front = cards(&app, now)[1].1;
    assert_eq!(app.front_card(rect, now), Some(front));
    let m = app.shelf_measure(rect, now).unwrap();
    // Bottom left: the line right of the card in the corner, its number
    // right of it on the card's middle.
    assert_eq!(m.height_line.x, front.right() + 8.0);
    assert_eq!(
        (m.height_line.y, m.height_line.bottom()),
        (front.y, front.bottom())
    );
    assert_eq!(m.chip.x, m.height_line.right() + 4.0);
    assert!((middle(m.chip).1 - middle(m.height_line).1).abs() <= 0.5);
    assert_eq!(m.chip.h, 16.0);
    focus(&mut app, "Card size", now);
    key(&mut app, Key::Right, now);
    assert_eq!(
        app.shelf_measure_text().as_deref(),
        Some("200\u{a0}×\u{a0}139\u{a0}px")
    );
    // The biggest cards leave no room beside them in a narrow window.
    let mut narrow = app_on("shelf", (360.0, 800.0), now);
    focus(&mut narrow, "Card size", now);
    for _ in 0..20 {
        key(&mut narrow, Key::Right, now);
    }
    assert_eq!(narrow.measure_shown(now + SETTLED), 0.0);
}

#[test]
fn dragging_the_cards_edge_sets_card_size_in_steps_and_writes_like_the_slider() {
    use crate::app::{Target, Write};
    use crate::ui::Effect;
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let row = super::block(&app, "Card size");
    let block = app
        .page()
        .blocks
        .iter()
        .position(|b| matches!(b, Block::Preview(Preview::Shelf)))
        .unwrap();
    let edge = app.shelf_measure(shelf_rect(&app, now), now).unwrap().edge;
    let (x, y) = middle(edge);
    app.handle(Input::Move { x, y }, now);
    let target = Target::Preview(block);
    let hover = app.fade(
        target,
        crate::app::Channel::Hover,
        now + Duration::from_millis(60),
    );
    assert!(hover > 0.0, "the edge lights the measure");
    app.handle(Input::Press { x, y, clicks: 1 }, now);
    // The preview is to scale: 28 px wider is 218, the slider makes it 220.
    let at = now + Duration::from_millis(10);
    let effects = app.handle(Input::Move { x: x + 28.0, y }, at);
    assert_eq!(effects, vec![Effect::Changed(row)]);
    assert_eq!(
        app.write(row),
        Some(Write::Key(
            "shelf.card_width",
            crate::settings::Value::Int(220)
        ))
    );
    // Within the throttle nothing more goes out; the daemon's older value
    // does not pull the row back while it is dragged.
    let at = at + Duration::from_millis(20);
    assert!(app.handle(Input::Move { x: x + 40.0, y }, at).is_empty());
    let pages = crate::app::pages::pages(&super::prefs(), None, at);
    app.update(pages, at, |_, _| false);
    assert_eq!(app.value(row), Some(crate::ui::widgets::Value::Int(230)));
    // Letting go sends the last value.
    let effects = app.handle(Input::Release, at + Duration::from_millis(5));
    assert_eq!(effects, vec![Effect::Changed(row)]);
    assert_eq!(
        app.write(row),
        Some(Write::Key(
            "shelf.card_width",
            crate::settings::Value::Int(230)
        ))
    );
}

#[test]
fn the_cards_are_the_shelfs_own_painting_with_the_look_of_the_page() {
    use crate::ui::theme::{PANEL, SELECTED, mix};
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let draw = |app: &App, at: Instant| {
        let mut canvas = crate::ui::Canvas::new(SIZE.0 as u32, SIZE.1 as u32, 1).unwrap();
        app.draw(&mut canvas, at);
        canvas
    };
    let pixel = |canvas: &crate::ui::Canvas, (x, y): (f32, f32)| {
        let i = ((y as u32 * SIZE.0 as u32 + x as u32) * 4) as usize;
        let d = canvas.pixmap.data();
        (d[i], d[i + 1], d[i + 2])
    };
    let close = |a: (u8, u8, u8), b: (u8, u8, u8)| {
        [(a.0, b.0), (a.1, b.1), (a.2, b.2)]
            .iter()
            .all(|(x, y)| x.abs_diff(*y) <= 1)
    };
    let rect = shelf_rect(&app, now);
    let card = cards(&app, now)[1].1;
    // Left of the picture, on the card's middle row.
    let on_card = (card.x + 20.0, card.y + card.h / 2.0);
    // 80 % of a placeholder over the block, as the shelf over a desktop.
    let rest = pixel(&draw(&app, now + SETTLED), on_card);
    assert!(close(rest, mix(PANEL, SELECTED, 0.8)), "{rest:?}");
    // Hovered it turns solid.
    app.handle(
        Input::Move {
            x: on_card.0,
            y: on_card.1,
        },
        now,
    );
    assert_eq!(pixel(&draw(&app, now + SETTLED), on_card), SELECTED);
    // Without "Solid on hover" it stays as it is.
    focus(&mut app, "Solid on hover", now);
    key(&mut app, Key::Space, now);
    app.handle(
        Input::Move {
            x: on_card.0,
            y: on_card.1,
        },
        now,
    );
    assert!(close(pixel(&draw(&app, now + SETTLED), on_card), rest));
    // Away from the cards and the measure, the plain block.
    let empty = (rect.right() - 40.0, rect.y + 40.0);
    assert_eq!(pixel(&draw(&app, now + SETTLED), empty), PANEL);
}

#[test]
fn the_whole_measure_drags_and_shows_the_sideways_pointer() {
    use crate::ui::PointerShape;
    let now = Instant::now();
    let app = app_on("shelf", SIZE, now);
    let rect = shelf_rect(&app, now);
    let m = app.shelf_measure(rect, now).unwrap();
    let front = cards(&app, now)[1].1;
    // Just inside the card, the line, the numbers, just past them.
    for at in [
        (front.right() - 2.0, middle(front).1),
        middle(m.height_line),
        middle(m.chip),
        (m.chip.right() + 2.0, middle(m.chip).1),
    ] {
        assert_eq!(app.pointer_shape(at, now), PointerShape::ResizeX, "{at:?}");
    }
    // The card itself and the free block beside the measure do not.
    for at in [middle(front), (m.chip.right() + 20.0, middle(m.chip).1)] {
        assert_eq!(app.pointer_shape(at, now), PointerShape::Default, "{at:?}");
    }
}

#[test]
fn hovering_a_card_blends_kept_pictures_instead_of_painting_again() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let mut canvas = crate::ui::Canvas::new(SIZE.0 as u32, SIZE.1 as u32, 1).unwrap();
    app.draw(&mut canvas, now + SETTLED);
    let at_rest = app.shelf_paints();
    let (x, y) = middle(cards(&app, now)[0].1);
    let later = now + SETTLED;
    app.handle(Input::Move { x, y }, later);
    for ms in [0, 16, 33, 50, 100, 400] {
        app.draw(&mut canvas, later + Duration::from_millis(ms));
    }
    assert_eq!(
        app.shelf_paints(),
        at_rest + 1,
        "the hovered state once, then kept"
    );
    // Back off the card and onto it again: nothing new to paint.
    app.handle(Input::Move { x: 5.0, y: 5.0 }, later + SETTLED);
    app.handle(Input::Move { x, y }, later + SETTLED);
    app.draw(&mut canvas, later + SETTLED + Duration::from_millis(30));
    assert_eq!(app.shelf_paints(), at_rest + 1);
}

#[test]
fn the_card_left_behind_keeps_its_picture_while_the_top_one_goes() {
    use crate::ui::theme::{PANEL, SELECTED, mix};
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let before = cards(&app, now);
    let kept = before[1].1;
    let (x, y) = middle(before[0].2);
    app.handle(Input::Press { x, y, clicks: 1 }, now);
    app.handle(Input::Release, now);
    let mut canvas = crate::ui::Canvas::new(SIZE.0 as u32, SIZE.1 as u32, 1).unwrap();
    // Low on the card left behind, beside its picture: a placeholder all
    // the way, never its shadow or the block.
    let at = (kept.x + 20.0, kept.bottom() - 12.0);
    for ms in [30, 80, 140, 200] {
        app.draw(&mut canvas, now + Duration::from_millis(ms));
        let i = ((at.1 as u32 * SIZE.0 as u32 + at.0 as u32) * 4) as usize;
        let d = canvas.pixmap.data();
        let want = mix(PANEL, SELECTED, 0.8);
        assert!(
            d[i].abs_diff(want.0) <= 2,
            "at {ms} ms: {:?}, want {want:?}",
            &d[i..i + 3]
        );
    }
}

/// `PREVIEW_OUT=dir cargo test --lib display_shots -- --ignored`: the
/// monitors of the test prefs as two clips and as one, as PNGs.
#[test]
#[ignore]
fn display_shots() {
    let out = std::path::PathBuf::from(std::env::var("PREVIEW_OUT").expect("PREVIEW_OUT"));
    let now = Instant::now();
    let mut app = app_on("recording", SIZE, now);
    focus(&mut app, "Display", now);
    key(&mut app, Key::Enter, now);
    for _ in 0..3 {
        key(&mut app, Key::Down, now);
    }
    key(&mut app, Key::Enter, now);
    let save = |app: &App, at: Instant, name: &str| {
        let mut canvas = crate::ui::Canvas::new(SIZE.0 as u32, SIZE.1 as u32, 2).unwrap();
        app.draw(&mut canvas, at);
        canvas.pixmap.save_png(out.join(name)).unwrap();
    };
    save(&app, now + SETTLED, "displays-two.png");
    focus(&mut app, "Both displays as", now + SETTLED);
    key(&mut app, Key::Right, now + SETTLED);
    save(&app, now + SETTLED + SETTLED, "displays-one.png");
}

#[test]
fn the_replay_load_says_off_in_use_and_before_your_changes() {
    use crate::app::ReplayLoad;
    let now = Instant::now();
    let mut app = app_on("replay", SIZE, now);
    assert!(
        app.page()
            .blocks
            .last()
            .is_some_and(|block| matches!(block, Block::Preview(Preview::Load { .. }))),
        "at the bottom of the page"
    );
    assert_eq!(app.replay_load_text(false).0, "Off, nothing in use");
    // Running, but nothing measured (an older daemon, no process found).
    app.set_replay_held(Some(12.0), now);
    assert_eq!(app.replay_load_text(false).0, "Running, not measured");
    let load = |cpu| ReplayLoad {
        cpu_percent: cpu,
        memory_bytes: 414_000_000,
    };
    app.set_replay_load(Some(load(None)));
    assert_eq!(
        app.replay_load_text(false),
        ("In use now".into(), "414\u{a0}MB".into())
    );
    app.set_replay_load(Some(load(Some(24.6))));
    assert_eq!(
        app.replay_load_text(true),
        (
            "Before your changes".into(),
            "25% of one core, 414\u{a0}MB".into()
        )
    );
}

#[test]
fn a_card_past_the_block_is_not_there_to_hover() {
    use crate::app::{Channel, Target};
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    // A top corner and big cards: the lower card runs past the block.
    focus(&mut app, "Corner", now);
    key(&mut app, Key::Left, now);
    focus(&mut app, "Card size", now);
    for _ in 0..10 {
        key(&mut app, Key::Right, now);
    }
    let at = now + SETTLED;
    let rect = shelf_rect(&app, at);
    let cards = cards(&app, at);
    let low = cards.last().unwrap();
    assert!(low.1.bottom() > rect.bottom() + 20.0, "it runs past");
    let block = app
        .page()
        .blocks
        .iter()
        .position(|b| matches!(b, Block::Preview(Preview::Shelf)))
        .unwrap();
    app.handle(
        Input::Move {
            x: low.1.x + 20.0,
            y: rect.bottom() + 10.0,
        },
        at,
    );
    let hover = app.fade(Target::Card(block, low.0), Channel::Hover, at + SETTLED);
    assert_eq!(hover, 0.0, "the row below keeps the pointer");
}

#[test]
fn closing_a_card_that_is_still_arriving_takes_it_away_and_leaves_two() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    let click = |app: &mut App, at: Instant, index: usize| {
        let close = cards(app, at)[index].2;
        let (x, y) = middle(close);
        app.handle(Input::Move { x, y }, at);
        app.handle(Input::Press { x, y, clicks: 1 }, at);
        app.handle(Input::Release, at);
    };
    click(&mut app, now, 0);
    // The new card arrives 240 ms on; close it while it does.
    let arriving = now + Duration::from_millis(300);
    let new = cards(&app, arriving)[0].0;
    click(&mut app, arriving, 0);
    let soon = cards(&app, arriving + Duration::from_millis(20));
    assert!(soon.iter().all(|card| card.0 != new), "it is leaving");
    let after = cards(&app, arriving + SETTLED);
    assert_eq!(after.len(), 2);
    assert!(after.iter().all(|card| card.0 != new));
}

#[test]
fn cards_past_the_block_are_cut_with_one_mask_however_the_page_scrolls() {
    let now = Instant::now();
    let mut app = app_on("shelf", SIZE, now);
    focus(&mut app, "Corner", now);
    key(&mut app, Key::Left, now);
    focus(&mut app, "Card size", now);
    for _ in 0..10 {
        key(&mut app, Key::Right, now);
    }
    let mut canvas = crate::ui::Canvas::new(SIZE.0 as u32, SIZE.1 as u32, 1).unwrap();
    let mut at = now + SETTLED;
    let top = shelf_rect(&app, at).y;
    let before = app.shelf_masks();
    for _ in 0..3 {
        app.draw(&mut canvas, at);
        app.handle(
            Input::Wheel {
                notches: 1,
                pixels: 0.0,
            },
            at,
        );
        at += SETTLED;
    }
    app.draw(&mut canvas, at);
    assert_eq!(
        app.shelf_masks(),
        before + 1,
        "built once, kept while scrolling"
    );
    assert!(shelf_rect(&app, at).y < top, "the page scrolled");
    // Under the block, where the lower card would be, the page's own ground.
    let rect = shelf_rect(&app, at);
    let low = *cards(&app, at).last().unwrap();
    assert!(low.1.bottom() > rect.bottom() + 20.0);
    let (x, y) = (low.1.x + 30.0, rect.bottom() + 6.0);
    let i = ((y as u32 * SIZE.0 as u32 + x as u32) * 4) as usize;
    let d = canvas.pixmap.data();
    assert_eq!(
        (d[i], d[i + 1], d[i + 2]),
        crate::ui::theme::WINDOW,
        "cut at the block"
    );
}
