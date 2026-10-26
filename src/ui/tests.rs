use std::time::{Duration, Instant};

use tiny_skia::PathBuilder;

use super::icons::{self, Icon};
use super::motion::{Scroll, Tween, min_jerk};
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
fn minimum_jerk_runs_from_rest_to_rest() {
    assert_eq!(min_jerk(0.0), 0.0);
    assert!((min_jerk(0.5) - 0.5).abs() < 1e-6);
    assert_eq!(min_jerk(1.0), 1.0);
    let now = Instant::now();
    let mut tween = Tween::settled(0.0, now);
    tween.retarget(100.0, Duration::from_millis(200), now);
    assert_eq!(tween.value(now), 0.0);
    assert_eq!(tween.velocity(now), 0.0);
    assert!((tween.value(now + Duration::from_millis(100)) - 50.0).abs() < 0.01);
    assert_eq!(tween.value(now + Duration::from_millis(250)), 100.0);
    assert!(tween.done(now + Duration::from_millis(200)));
}

#[test]
fn a_retarget_keeps_position_and_speed() {
    let now = Instant::now();
    let mut tween = Tween::settled(0.0, now);
    tween.retarget(100.0, Duration::from_millis(200), now);
    let at = now + Duration::from_millis(80);
    let (position, speed) = (tween.value(at), tween.velocity(at));
    tween.retarget(-40.0, Duration::from_millis(200), at);
    assert!((tween.value(at) - position).abs() < 0.001);
    assert!((tween.velocity(at) - speed).abs() < 0.01 * speed.abs());
    assert_eq!(tween.value(at + Duration::from_millis(200)), -40.0);
}

#[test]
fn wheel_notches_glide_and_a_touchpad_follows_exactly() {
    let now = Instant::now();
    let mut scroll = Scroll::new(now);
    scroll.set_max(100.0, now);
    scroll.wheel(1, 0.0, now);
    assert_eq!(scroll.offset(now), 0.0);
    assert!(scroll.moving(now));
    let later = now + theme::SCROLL_GLIDE;
    assert_eq!(scroll.offset(later), theme::WHEEL_STEP);
    // A second notch mid-glide aims one notch further, clamped to the end.
    let mid = now + Duration::from_millis(60);
    scroll.wheel(2, 0.0, mid);
    assert_eq!(scroll.offset(mid + theme::SCROLL_GLIDE), 100.0);
    scroll.wheel(0, -30.0, later + theme::SCROLL_GLIDE);
    assert_eq!(scroll.offset(later + theme::SCROLL_GLIDE), 70.0);
    scroll.reveal(10.0, 40.0, 50.0, later + theme::SCROLL_GLIDE);
    assert_eq!(scroll.offset(later + theme::SCROLL_GLIDE * 3), 10.0);
}
