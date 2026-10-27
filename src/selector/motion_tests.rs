use std::time::{Duration, Instant};

use boltsnap::ui::Rect;
use boltsnap::ui::text::Fonts;

use super::*;
use crate::selector::render::chrome::Placement;
use crate::selector::render::toolbar::{self, Metrics};

fn ms(at: Instant, ms: u64) -> Instant {
    at + Duration::from_millis(ms)
}

fn layout(sel: (f32, f32, f32, f32)) -> Layout {
    let metrics = Metrics::new(&Fonts::fallback());
    toolbar::layout(sel, Rect::new(0.0, 0.0, 1920.0, 1080.0), &metrics).unwrap()
}

fn targets() -> Targets {
    Targets {
        hovered: None,
        pressed: None,
        on: [false, false, true, true],
        enabled: [true, false, true, true],
        reason: Some("Replay is loading"),
    }
}

fn alpha(bar: &Bar) -> f32 {
    bar.current.as_ref().map_or(0.0, |d| d.alpha)
}

#[test]
fn the_toolbar_fades_in_with_quick_once_the_selection_is_let_go() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    assert!(motion.sample(t0).current.is_none());
    motion.update(Some(layout((600.0, 300.0, 700.0, 400.0))), &targets(), t0);
    assert_eq!(alpha(&motion.sample(t0)), 0.0);
    let mid = alpha(&motion.sample(ms(t0, 50)));
    assert!(mid > 0.2 && mid < 0.95, "{mid}");
    // Perceived at QUICK (150 ms), exactly there once settled.
    assert!(alpha(&motion.sample(ms(t0, 150))) > 0.98);
    assert_eq!(alpha(&motion.sample(ms(t0, 400))), 1.0);
    assert!(motion.moving(ms(t0, 50)));
    assert!(!motion.moving(ms(t0, 400)));
    // The toggles open as they are, without fading to them.
    let look = motion.sample(t0).current.map(|d| d.look);
    assert!(look.is_none() || look.unwrap().controls[2].on == 1.0);
    assert_eq!(
        motion.sample(ms(t0, 400)).current.unwrap().look.controls[2].on,
        1.0
    );
}

#[test]
fn dragging_the_selection_moves_the_toolbar_with_it_without_a_fade() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    motion.update(Some(layout((600.0, 300.0, 700.0, 400.0))), &targets(), t0);
    let t1 = ms(t0, 500);
    let moved = layout((640.0, 330.0, 700.0, 400.0));
    motion.update(Some(moved), &targets(), t1);
    let bar = motion.sample(t1);
    assert!(bar.ghosts.is_empty());
    assert_eq!(bar.current.unwrap().layout, moved);
    assert_eq!(alpha(&motion.sample(t1)), 1.0);
    assert!(!motion.moving(t1));
}

#[test]
fn another_arrangement_crossfades_where_each_one_is() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    let above = layout((600.0, 300.0, 700.0, 400.0));
    motion.update(Some(above), &targets(), t0);
    let t1 = ms(t0, 500);
    let below = layout((600.0, 4.0, 700.0, 400.0));
    assert_eq!(
        (above.placement, below.placement),
        (Placement::Above, Placement::Below)
    );
    motion.update(Some(below), &targets(), t1);
    // Only the new one takes input from the moment it is there.
    assert_eq!(motion.layout(), Some(&below));
    let mid = motion.sample(ms(t1, 50));
    assert_eq!(mid.ghosts.len(), 1);
    assert_eq!(
        mid.ghosts[0].layout, above,
        "the old one stays where it was"
    );
    let (old, new) = (mid.ghosts[0].alpha, alpha(&mid));
    assert!(
        old > 0.05 && old < 0.8 && new > 0.2 && new < 0.95,
        "{old} {new}"
    );
    let done = motion.sample(ms(t1, 400));
    assert!(done.ghosts.is_empty());
    assert_eq!(alpha(&done), 1.0);
    // Settled ghosts are dropped at the next update.
    motion.update(Some(below), &targets(), ms(t1, 400));
    assert!(motion.ghosts.is_empty());
}

#[test]
fn a_new_drag_fades_the_toolbar_out_and_it_stops_taking_input() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    let first = layout((600.0, 300.0, 700.0, 400.0));
    motion.update(Some(first), &targets(), t0);
    let t1 = ms(t0, 500);
    motion.update(None, &targets(), t1);
    assert_eq!(motion.layout(), None);
    let mid = motion.sample(ms(t1, 50));
    assert_eq!(mid.current.as_ref().unwrap().layout, first);
    assert!(alpha(&mid) < 0.8);
    assert!(motion.sample(ms(t1, 400)).current.is_none());
    // Let go elsewhere: the new one fades in; the old place is not reused.
    let t2 = ms(t1, 60);
    let second = layout((100.0, 600.0, 300.0, 200.0));
    motion.update(Some(second), &targets(), t2);
    let bar = motion.sample(ms(t2, 30));
    assert_eq!(bar.ghosts.len(), 1);
    assert_eq!(bar.ghosts[0].layout, first);
    assert_eq!(bar.current.unwrap().layout, second);
}

#[test]
fn hover_retargets_from_where_it_is_with_its_speed() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    let l = layout((600.0, 300.0, 700.0, 400.0));
    motion.update(Some(l), &targets(), t0);
    let t1 = ms(t0, 500);
    let hovered = Targets {
        hovered: Some(0),
        ..targets()
    };
    motion.update(Some(l), &hovered, t1);
    let t2 = ms(t1, 40);
    let hover = |m: &BarMotion, t| m.sample(t).current.unwrap().look.controls[0].hover;
    let before = hover(&motion, t2);
    let slope_before =
        (hover(&motion, t2) - hover(&motion, ms(t2, 0) - Duration::from_millis(1))) * 1000.0;
    motion.update(Some(l), &targets(), t2);
    let after = hover(&motion, t2);
    assert!((before - after).abs() < 1e-4, "no jump: {before} {after}");
    assert!(before > 0.1 && before < 0.95);
    // Still rising for a moment: the speed carries over, it does not stop dead.
    let slope_after = (hover(&motion, ms(t2, 1)) - after) * 1000.0;
    assert!(
        slope_before > 0.0 && slope_after > 0.0,
        "{slope_before} {slope_after}"
    );
    assert!((slope_after - slope_before).abs() < slope_before * 0.5);
    assert_eq!(hover(&motion, ms(t2, 500)), 0.0);
}

#[test]
fn clip_getting_its_buffer_brightens_with_quick() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    let l = layout((600.0, 300.0, 700.0, 400.0));
    motion.update(Some(l), &targets(), t0);
    let t1 = ms(t0, 500);
    let mut ready = targets();
    ready.enabled[toolbar::CLIP] = true;
    motion.update(Some(l), &ready, t1);
    let enabled = |t| motion.sample(t).current.unwrap().look.controls[1].enabled;
    assert_eq!(enabled(t1), 0.0);
    assert!(enabled(ms(t1, 50)) > 0.2 && enabled(ms(t1, 50)) < 0.95);
    assert_eq!(enabled(ms(t1, 500)), 1.0);
}

#[test]
fn the_reason_fades_in_on_an_unavailable_clip_and_out_with_its_words() {
    let t0 = Instant::now();
    let mut motion = BarMotion::new(&targets(), t0);
    let l = layout((600.0, 300.0, 700.0, 400.0));
    motion.update(Some(l), &targets(), t0);
    let t1 = ms(t0, 500);
    let on_clip = Targets {
        hovered: Some(toolbar::CLIP),
        ..targets()
    };
    motion.update(Some(l), &on_clip, t1);
    assert!(motion.sample(t1).hint.is_none());
    let (text, alpha) = motion.sample(ms(t1, 50)).hint.unwrap();
    assert_eq!(text, "Replay is loading");
    assert!(alpha > 0.2 && alpha < 0.95);
    assert_eq!(motion.sample(ms(t1, 500)).hint.unwrap().1, 1.0);
    // The buffer arrives under the pointer: Clip brightens, the reason goes
    // with the words it had.
    let t2 = ms(t1, 500);
    let mut ready = on_clip;
    ready.enabled[toolbar::CLIP] = true;
    ready.reason = None;
    motion.update(Some(l), &ready, t2);
    assert_eq!(
        motion.sample(ms(t2, 50)).hint.unwrap().0,
        "Replay is loading"
    );
    assert!(motion.sample(ms(t2, 500)).hint.is_none());
    // On an available Clip, or another control, there is no reason.
    motion.update(
        Some(l),
        &Targets {
            hovered: Some(0),
            ..targets()
        },
        ms(t2, 600),
    );
    assert!(motion.sample(ms(t2, 1200)).hint.is_none());
}
