//! Pointer frame boundaries and every axis record remain observable in wire order.
use super::*;

#[test]
fn pointer_frame_preserves_motion_buttons_axes_and_unknown_direction() {
    let (mut fixture, seat) = fixture(9);
    let id = fixture.create(101, 101);
    emit(&mut fixture, Wire::PointerEnter(0, 0, u32::MAX, 1.25, 2.5));
    emit(&mut fixture, Wire::Motion(0, u32::MAX, 3.5, 4.75));
    emit(&mut fixture, Wire::Button(0, 0, 0, 0x110, true));
    emit(&mut fixture, Wire::Motion(0, 1, 5.0, 6.0));
    emit(&mut fixture, Wire::Source(0, 99));
    emit(&mut fixture, Wire::Axis(0, 2, 0, -2.25));
    emit(&mut fixture, Wire::Value120(0, 0, -120));
    emit(&mut fixture, Wire::Direction(0, 0, 99));
    emit(&mut fixture, Wire::Stop(0, 3, 0));
    assert!(observations(&mut fixture).is_empty());
    emit(&mut fixture, Wire::Frame(0));
    let events = observations(&mut fixture);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seat, seat);
    let InputObservation::PointerFrame(frame) = &events[0].observation else {
        panic!("missing frame");
    };
    assert!(!frame.legacy());
    assert!(!frame.interrupted());
    assert_eq!(frame.records(), expected_pointer_records(id));
    emit(&mut fixture, Wire::Frame(0));
    assert!(observations(&mut fixture).is_empty());
}
#[test]
fn pointer_frames_retain_each_target_across_leave_enter_and_legacy_is_explicit() {
    let (mut fixture, _) = fixture(7);
    let first = fixture.create(101, 101);
    let second = fixture.create(101, 101);
    emit(&mut fixture, Wire::PointerEnter(0, 0, 1, 0.0, 0.0));
    emit(&mut fixture, Wire::Discrete(0, 1, -2));
    emit(&mut fixture, Wire::PointerLeave(0, 0, 2));
    emit(&mut fixture, Wire::PointerEnter(0, 1, 3, 7.0, 8.0));
    emit(&mut fixture, Wire::Frame(0));
    let InputObservation::PointerFrame(frame) =
        observations(&mut fixture).pop().unwrap().observation
    else {
        panic!("missing frame");
    };
    assert_eq!(
        frame.records(),
        [
            PointerObservation::Enter {
                window: first,
                serial: 1,
                x: 0.0,
                y: 0.0
            },
            PointerObservation::AxisDiscrete {
                window: first,
                axis: PointerAxis::Horizontal,
                steps: -2
            },
            PointerObservation::Leave {
                window: first,
                serial: 2
            },
            PointerObservation::Enter {
                window: second,
                serial: 3,
                x: 7.0,
                y: 8.0
            }
        ]
    );
    let (mut legacy, _) = super::fixture(4);
    let id = legacy.create(101, 101);
    emit(&mut legacy, Wire::PointerEnter(0, 0, 1, 0.0, 0.0));
    let InputObservation::PointerFrame(frame) =
        observations(&mut legacy).pop().unwrap().observation
    else {
        panic!("missing legacy delivery");
    };
    assert!(frame.legacy());
    assert!(!frame.interrupted());
    assert_eq!(
        frame.records(),
        [PointerObservation::Enter {
            window: id,
            serial: 1,
            x: 0.0,
            y: 0.0
        }]
    );
}
#[test]
fn retained_pointer_clones_preserve_aggregate_record_limit() {
    let (mut fixture, _) = fixture(9);
    fixture.create(101, 101);
    emit(&mut fixture, Wire::PointerEnter(0, 0, 1, 0.0, 0.0));
    let mut frames = Vec::new();
    for batch in 0..16 {
        for index in 0..if batch == 0 { 255 } else { 256 } {
            fixture
                .peer
                .emit(PeerEvent::Input(Wire::Motion(0, index, 1.0, 1.0)));
        }
        emit(&mut fixture, Wire::Frame(0));
        let InputObservation::PointerFrame(frame) =
            observations(&mut fixture).pop().unwrap().observation
        else {
            panic!("missing bounded frame");
        };
        assert_eq!(frame.records().len(), 256);
        frames.push(frame);
    }
    let retained = frames[0].clone();
    frames.remove(0);
    fixture
        .peer
        .emit(PeerEvent::Input(Wire::Motion(0, 0, 2.0, 2.0)));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert_eq!(retained.records().len(), 256);
}

fn expected_pointer_records(id: WindowId) -> Vec<PointerObservation> {
    vec![
        PointerObservation::Enter {
            window: id,
            serial: u32::MAX,
            x: 1.25,
            y: 2.5,
        },
        PointerObservation::Motion {
            window: id,
            time_ms: u32::MAX,
            x: 3.5,
            y: 4.75,
        },
        PointerObservation::Button {
            window: id,
            serial: 0,
            time_ms: 0,
            button: 0x110,
            down: true,
        },
        PointerObservation::Motion {
            window: id,
            time_ms: 1,
            x: 5.0,
            y: 6.0,
        },
        PointerObservation::AxisSource {
            window: id,
            source: AxisSource::Unknown(99),
        },
        PointerObservation::Axis {
            window: id,
            time_ms: 2,
            axis: PointerAxis::Vertical,
            value: -2.25,
        },
        PointerObservation::AxisValue120 {
            window: id,
            axis: PointerAxis::Vertical,
            value: -120,
        },
        PointerObservation::AxisRelativeDirection {
            window: id,
            axis: PointerAxis::Vertical,
            direction: AxisDirection::Unknown(99),
        },
        PointerObservation::AxisStop {
            window: id,
            time_ms: 3,
            axis: PointerAxis::Vertical,
        },
    ]
}

#[test]
fn close_preserves_interrupted_pointer_batch_before_cancellation_and_closed() {
    let (mut fixture, seat) = fixture(9);
    let id = fixture.create(101, 101);
    emit(&mut fixture, Wire::PointerEnter(0, 0, 1, 3.5, 4.25));
    emit(&mut fixture, Wire::Button(0, 2, 3, 0x110, true));
    assert!(observations(&mut fixture).is_empty());
    fixture.host.close(id).unwrap();
    let events = fixture.host.events();
    assert_eq!(events.len(), 3);
    let WindowEvent::Input(InputEvent {
        seat: observed_seat,
        observation: InputObservation::PointerFrame(frame),
    }) = &events[0]
    else {
        panic!("missing interrupted batch");
    };
    assert_eq!(*observed_seat, seat);
    assert!(!frame.legacy());
    assert!(frame.interrupted());
    assert_eq!(
        frame.records(),
        [
            PointerObservation::Enter {
                window: id,
                serial: 1,
                x: 3.5,
                y: 4.25
            },
            PointerObservation::Button {
                window: id,
                serial: 2,
                time_ms: 3,
                button: 0x110,
                down: true
            },
        ]
    );
    assert_eq!(
        &events[1..],
        [
            WindowEvent::Input(cancelled(
                seat,
                id,
                InputDevice::Pointer,
                InputCancelReason::WindowClosed
            )),
            WindowEvent::Closed { window: id },
        ]
    );
}
