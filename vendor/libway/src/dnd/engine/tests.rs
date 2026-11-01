//! Target priority and wire action regression tests.
use super::*;
use crate::dnd::Accept;
fn target(x: f64, priority: i32, enabled: bool) -> Target {
    let rect = LocalRect {
        x,
        y: 0.,
        width: 10.,
        height: 10.,
    };
    Target {
        surface: ObjectId::null(),
        spec: TargetSpec {
            rect,
            accepts: vec![Accept::Files],
            actions: Actions::default(),
            preferred: Action::Copy,
            priority,
            enabled,
        },
    }
}
#[test]
fn hit_prefers_priority_then_recency_and_respects_enabled() {
    let mut targets = BTreeMap::new();
    targets.insert(TargetId(1, 1), target(0., 0, true));
    targets.insert(TargetId(1, 2), target(5., 0, true));
    targets.insert(TargetId(1, 3), target(5., -1, true));
    targets.insert(TargetId(1, 4), target(5., 9, false));
    let s = ObjectId::null();
    assert_eq!(hit_in(targets.iter(), &s, 1., 1.), Some(TargetId(1, 1)));
    assert_eq!(hit_in(targets.iter(), &s, 7., 1.), Some(TargetId(1, 2)));
    assert_eq!(hit_in(targets.iter(), &s, 12., 1.), Some(TargetId(1, 2)));
    assert_eq!(hit_in(targets.iter(), &s, 30., 1.), None);
}
#[test]
fn actions_round_trip_through_wire_bits() {
    let a = Actions {
        copy: true,
        move_: false,
        ask: true,
    };
    assert_eq!(Actions::from_wire(a.to_wire()), a);
    assert_eq!(Action::from_wire(Action::Move.to_wire()), Action::Move);
    assert_eq!(Action::from_wire(8), Action::None);
}
