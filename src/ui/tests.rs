use std::time::{Duration, Instant};

use tiny_skia::PathBuilder;

use super::icons::{self, Icon};
use super::motion::{Scroll, Spring};
use super::text::Fonts;
use super::theme::{self, TextStyle};
use super::{Canvas, Rect};

fn last_point(pb: PathBuilder) -> (f32, f32) {
    let path = pb.finish().expect("path");
    let point = *path.points().last().expect("points");
    (point.x, point.y)
}

#[test]
fn every_icon_parses_inside_the_lucide_grid() {
    for icon in Icon::ALL {
        let path = icon
            .path()
            .unwrap_or_else(|| panic!("{} missing", icon.id()));
        let bounds = path.bounds();
        assert!(
            bounds.left() >= -0.01
                && bounds.top() >= -0.01
                && bounds.right() <= 24.01
                && bounds.bottom() <= 24.01,
            "{} outside the grid: {bounds:?}",
            icon.id()
        );
    }
}

#[test]
fn arcs_end_on_their_target_and_keep_the_radius() {
    for (sweep, large) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut pb = PathBuilder::new();
        pb.move_to(3.0, 4.0);
        icons::arc(
            &mut pb,
            (3.0, 4.0),
            (5.0, 3.0),
            30.0,
            large,
            sweep,
            (9.5, 7.25),
        );
        let (x, y) = last_point(pb);
        assert!((x - 9.5).abs() < 0.01 && (y - 7.25).abs() < 0.01, "{x} {y}");
    }
    // A quarter circle of radius 10 around the origin, as one cubic.
    let mut pb = PathBuilder::new();
    pb.move_to(10.0, 0.0);
    icons::arc(
        &mut pb,
        (10.0, 0.0),
        (10.0, 10.0),
        0.0,
        false,
        true,
        (0.0, 10.0),
    );
    let path = pb.finish().unwrap();
    let points = path.points();
    assert_eq!(points.len(), 4);
    let mid = |a: f32, b: f32, c: f32, d: f32| (a + 3.0 * b + 3.0 * c + d) / 8.0;
    let (mx, my) = (
        mid(points[0].x, points[1].x, points[2].x, points[3].x),
        mid(points[0].y, points[1].y, points[2].y, points[3].y),
    );
    assert!(((mx * mx + my * my).sqrt() - 10.0).abs() < 0.01);
}

#[test]
fn path_data_reads_compact_numbers_and_relative_commands() {
    let path = icons::parse(r#"<path d="M1 1h4v4H1zm8 0l.5.5a.5.5 0 0 1 1-1" />"#).unwrap();
    let last = path.points().last().unwrap();
    // m8 0 from the closed subpath's start (1,1), l.5.5, then the arc by (1,-1).
    assert!((last.x - 10.5).abs() < 0.01 && (last.y - 0.5).abs() < 0.01);
    assert!(icons::parse(r#"<path d="M1 1 x" />"#).is_none());
}

#[test]
fn an_icon_draws_only_inside_its_box() {
    let mut canvas = Canvas::new(40, 40, 2).unwrap();
    canvas.clear(theme::WINDOW);
    Icon::Plus.draw(
        &mut canvas,
        Rect::new(10.0, 10.0, 16.0, 16.0),
        theme::TEXT,
        1.0,
    );
    let width = canvas.pixmap.width() as usize;
    let lit: Vec<(usize, usize)> = canvas
        .pixmap
        .data()
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, px)| px[0] > 0x12)
        .map(|(i, _)| (i % width, i / width))
        .collect();
    assert!(!lit.is_empty());
    assert!(
        lit.iter()
            .all(|&(x, y)| (20..52).contains(&x) && (20..52).contains(&y))
    );
}

#[test]
fn text_width_adds_up_and_carets_follow_it() {
    let fonts = Fonts::fallback();
    let one = fonts.width(TextStyle::Label, "Hx");
    assert!((fonts.width(TextStyle::Label, "HxHx") - 2.0 * one).abs() < 0.001);
    let carets = fonts.carets(TextStyle::Label, "Hx");
    assert_eq!(carets.len(), 3);
    assert_eq!(carets[0], (0, 0.0));
    assert!((carets[2].1 - one).abs() < 0.001);
    assert!(fonts.width(TextStyle::Title, "Hx") > fonts.width(TextStyle::Key, "Hx"));
}

#[test]
fn the_baseline_centres_the_capitals() {
    let fonts = Fonts::fallback();
    for style in [
        TextStyle::Title,
        TextStyle::Label,
        TextStyle::Hint,
        TextStyle::Key,
    ] {
        let cap = fonts.cap_height(style);
        assert!(
            cap > style.px() * 0.6 && cap < style.px() * 0.8,
            "{style:?} {cap}"
        );
        let baseline = fonts.baseline(style, 10.0, theme::CTL_H);
        assert!((baseline - cap / 2.0 - (10.0 + theme::CTL_H / 2.0)).abs() < 0.001);
    }
}

#[test]
fn ellipsize_fits_and_marks_the_cut() {
    let fonts = Fonts::fallback();
    let style = TextStyle::Label;
    let text = "Recordings folder on the second disk";
    assert_eq!(fonts.ellipsize(style, "Short", 200.0), "Short");
    let cut = fonts.ellipsize(style, text, 90.0);
    assert!(fonts.width(style, &cut) <= 90.0);
    // The embedded subset has no "…", so it falls back to three dots.
    assert!(cut.ends_with("..."));
    assert!(text.starts_with(cut.trim_end_matches('.')));
}

#[test]
fn ellipsize_middle_keeps_both_ends_of_a_path() {
    let fonts = Fonts::fallback();
    let style = TextStyle::Label;
    let path = "/home/someone/Videos/boltsnap/recordings/2026/final";
    let cut = fonts.ellipsize_middle(style, path, 160.0);
    assert!(fonts.width(style, &cut) <= 160.0);
    let (head, tail) = cut.split_once("...").expect("cut in the middle");
    assert!(path.starts_with(head) && path.ends_with(tail));
    assert!(!head.is_empty() && tail.ends_with("final"));
}

const T: Duration = Duration::from_millis(200);

fn token() -> super::motion::Token {
    super::motion::Token { duration: 0.2 }
}

#[test]
fn a_spring_starts_at_rest_covers_its_perceptual_duration_and_ends_exactly() {
    let now = Instant::now();
    let mut spring = Spring::settled(0.0, now);
    spring.retarget(100.0, token(), now);
    assert_eq!(spring.value(now), 0.0);
    assert_eq!(spring.velocity(now), 0.0);
    // At its duration a critically damped spring is about 98.6 % there.
    let at = spring.value(now + T);
    assert!((98.0..100.0).contains(&at), "{at}");
    // Never past the target.
    for ms in 0..1000 {
        assert!(spring.value(now + Duration::from_millis(ms)) <= 100.0);
    }
    let end = now + Duration::from_secs(1);
    assert!(spring.done(end));
    assert_eq!(spring.value(end), 100.0);
    assert_eq!(spring.velocity(end), 0.0);
}

#[test]
fn a_retarget_keeps_position_and_speed() {
    let now = Instant::now();
    let mut spring = Spring::settled(0.0, now);
    spring.retarget(100.0, token(), now);
    let at = now + Duration::from_millis(60);
    let (position, speed) = (spring.value(at), spring.velocity(at));
    assert!(speed > 0.0);
    spring.retarget(-40.0, token(), at);
    assert!((spring.value(at) - position).abs() < 1e-3);
    assert!((spring.velocity(at) - speed).abs() < 1e-3 * speed.abs());
    assert_eq!(spring.value(at + Duration::from_secs(1)), -40.0);
}

#[test]
fn a_spring_lies_on_one_path_whatever_the_frame_rate() {
    let now = Instant::now();
    let retarget_at = now + Duration::from_millis(50);
    let mut runs = Vec::new();
    for hz in [60.0, 120.0, 144.0] {
        let mut spring = Spring::settled(0.0, now);
        spring.retarget(1.0, token(), now);
        let mut frame = 0.0;
        // Sample like a display at `hz`, retarget at the same instant.
        loop {
            let t = now + Duration::from_secs_f64(frame / hz);
            if t >= retarget_at {
                break;
            }
            let _ = spring.value(t);
            frame += 1.0;
        }
        spring.retarget(0.25, token(), retarget_at);
        runs.push([80, 120, 200, 400].map(|ms| spring.value(now + Duration::from_millis(ms))));
    }
    assert!(runs.windows(2).all(|pair| pair[0] == pair[1]), "{runs:?}");
}

#[test]
fn wheel_notches_glide_and_a_touchpad_follows_exactly() {
    let now = Instant::now();
    let settled = Duration::from_secs(1);
    let mut scroll = Scroll::new(now);
    scroll.set_max(100.0, now);
    scroll.wheel(1, 0.0, now);
    assert_eq!(scroll.offset(now), 0.0);
    assert!(scroll.moving(now));
    let later = now + settled;
    assert_eq!(scroll.offset(later), theme::WHEEL_STEP);
    // A second notch mid-glide aims one notch further, clamped to the end.
    let mid = now + Duration::from_millis(60);
    scroll.wheel(2, 0.0, mid);
    assert_eq!(scroll.offset(mid + settled), 100.0);
    scroll.wheel(0, -30.0, later + settled);
    assert_eq!(scroll.offset(later + settled), 70.0);
    scroll.reveal(10.0, 40.0, 50.0, later + settled);
    assert_eq!(scroll.offset(later + settled * 2), 10.0);
    // A resize that shortens the page clamps at once.
    scroll.set_max(5.0, later + settled * 2);
    assert_eq!(scroll.offset(later + settled * 2), 5.0);
    assert!(!scroll.moving(later + settled * 2));
}

#[test]
fn fade_mixes_towards_the_colour_and_leaves_that_colour_alone() {
    let mut canvas = Canvas::new(4, 1, 1).unwrap();
    canvas.clear(theme::WINDOW);
    canvas.fill(Rect::new(2.0, 0.0, 2.0, 1.0), 0.0, theme::TEXT, 1.0);
    canvas.fade(Rect::new(0.0, 0.0, 4.0, 1.0), theme::WINDOW, 1.0);
    let before = canvas.pixmap.data().to_vec();
    canvas.fade(Rect::new(0.0, 0.0, 4.0, 1.0), theme::WINDOW, 0.5);
    let data = canvas.pixmap.data();
    assert_eq!(data[..8], before[..8], "the window colour stays exact");
    let mid = |a: u8, b: u8| ((a as u32 * 128 + b as u32 * 128) >> 8) as u8;
    assert_eq!(data[8], mid(theme::TEXT.0, theme::WINDOW.0));
    assert_eq!(data[11], 255);
}

#[test]
fn cached_glyphs_draw_the_same_pixels_as_the_first_time() {
    let fonts = Fonts::fallback();
    let draw = || {
        let mut canvas = Canvas::new(200, 30, 1).unwrap();
        canvas.clear(theme::WINDOW);
        fonts.draw(
            &mut canvas,
            TextStyle::Label,
            "Cards shown 12",
            (3.3, 20.0),
            theme::TEXT,
            1.0,
        );
        canvas.pixmap.data().to_vec()
    };
    let first = draw();
    let second = draw();
    assert!(first.chunks_exact(4).any(|px| px[0] > 0xC0), "text drew");
    assert_eq!(first, second);
}

#[test]
fn wrap_breaks_words_paths_and_compounds_and_marks_what_is_left() {
    let fonts = Fonts::fallback();
    let style = TextStyle::Label;
    let width = fonts.width(style, "Show saved videos");
    let lines = fonts.wrap(style, "Show saved videos on the shelf", width, 3);
    assert_eq!(lines, vec!["Show saved videos", "on the shelf"]);
    assert!(lines.iter().all(|line| fonts.width(style, line) <= width));
    // A path breaks after a slash, a compound between letters.
    let path = fonts.wrap(
        style,
        "/home/someone/Videos/boltsnap",
        fonts.width(style, "/home/someone/"),
        4,
    );
    assert_eq!(path[0], "/home/someone/");
    let compound = "Bildschirmaufnahmeeinstellungsänderung";
    let parts = fonts.wrap(style, compound, 60.0, 9);
    assert!(parts.len() > 1 && parts.concat() == compound);
    assert!(parts.iter().all(|line| fonts.width(style, line) <= 60.0));
    // Too much for the lines it may take: the last one says so.
    let cut = fonts.wrap(
        style,
        "one two three four five six",
        fonts.width(style, "one two"),
        2,
    );
    assert_eq!(cut.len(), 2);
    assert!(cut[1].ends_with('…') || cut[1].ends_with("..."));
    assert_eq!(fonts.wrap(style, "", 100.0, 3), vec![""]);
}
