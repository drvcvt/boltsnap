//! Physical identity, native masks, keymap fds and focus snapshots on the wire.
use super::*;

#[test]
fn keyboard_preserves_held_enter_masks_and_wrapping_times_without_presses() {
    let (mut fixture, seat) = fixture(9);
    let id = fixture.create(101, 101);
    let held = [30u32, 42].into_iter().flat_map(u32::to_ne_bytes).collect();
    emit(&mut fixture, Wire::KeyboardEnter(0, 0, u32::MAX, held));
    emit(&mut fixture, Wire::Modifiers(0, 0, 1, 2, 4, 3));
    emit(&mut fixture, Wire::Repeat(0, 25, 600));
    emit(&mut fixture, Wire::Key(0, u32::MAX, u32::MAX, 30, false));
    emit(&mut fixture, Wire::Key(0, 0, 0, 31, true));
    let events = observations(&mut fixture);
    assert!(events.iter().all(|event| event.seat == seat));
    assert_eq!(events.len(), 5);
    let InputObservation::Keyboard(KeyboardObservation::Enter {
        window,
        serial,
        held,
    }) = &events[0].observation
    else {
        panic!("missing Enter snapshot");
    };
    assert_eq!(
        (*window, *serial, held.as_slice()),
        (id, u32::MAX, &[30, 42][..])
    );
    assert_keyboard_transitions(&events, id);
    emit(&mut fixture, Wire::KeyboardLeave(0, 0, 1));
    emit(&mut fixture, Wire::Key(0, 2, 1, 31, true));
    assert_eq!(
        observations(&mut fixture),
        [InputEvent {
            seat,
            observation: InputObservation::Keyboard(KeyboardObservation::Leave {
                window: id,
                serial: 1
            })
        }]
    );
}

#[test]
fn keymap_is_owned_validated_text_and_no_keymap_clears_it() {
    let (mut fixture, seat) = fixture(9);
    emit(
        &mut fixture,
        Wire::Keymap(0, 1, keymap_fd(b"xkb_keymap {};\0"), 15),
    );
    let event = observations(&mut fixture).pop().unwrap();
    assert_eq!(event.seat, seat);
    let InputObservation::Keyboard(KeyboardObservation::Keymap(Some(map))) = event.observation
    else {
        panic!("missing owned keymap");
    };
    assert_eq!(map.as_str(), "xkb_keymap {};");
    assert_eq!(map.as_bytes(), b"xkb_keymap {};\0");
    emit(
        &mut fixture,
        Wire::Keymap(0, 0, keymap_fd(b"ignored"), u32::MAX),
    );
    assert_eq!(
        observations(&mut fixture),
        [InputEvent {
            seat,
            observation: InputObservation::Keyboard(KeyboardObservation::Keymap(None))
        }]
    );
    drop(fixture.host);
    assert_eq!(map.clone().as_str(), "xkb_keymap {};");
}

#[test]
fn malformed_keymaps_and_repeat_are_terminal_without_observations() {
    for (bytes, size) in [
        (b"bad".as_slice(), 3),
        (b"a\0b\0", 4),
        (&[0xff, 0], 2),
        (b"x\0", 3),
        (b"x\0", 1024 * 1024 + 1),
    ] {
        let (mut fixture, _) = fixture(9);
        fixture
            .peer
            .emit(PeerEvent::Input(Wire::Keymap(0, 1, keymap_fd(bytes), size)));
        read_barrier(&fixture.host);
        assert!(fixture.host.pump(Some(Instant::now())).is_err());
        assert!(observations(&mut fixture).is_empty());
        assert!(fixture.host.pump(Some(Instant::now())).is_err());
    }
    let (mut fixture, _) = fixture(9);
    fixture.peer.emit(PeerEvent::Input(Wire::Repeat(0, -1, 20)));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert!(observations(&mut fixture).is_empty());
}

#[test]
fn retained_keymap_clones_keep_aggregate_charge_until_final_drop() {
    let (mut fixture, _) = fixture(9);
    let mut maps = Vec::new();
    let mut bytes = vec![b'x'; 1024 * 1024];
    *bytes.last_mut().unwrap() = 0;
    for _ in 0..8 {
        emit(
            &mut fixture,
            Wire::Keymap(0, 1, keymap_fd(&bytes), bytes.len() as u32),
        );
        let InputObservation::Keyboard(KeyboardObservation::Keymap(Some(map))) =
            observations(&mut fixture).pop().unwrap().observation
        else {
            panic!("missing map");
        };
        maps.push(map);
    }
    let clone = maps[0].clone();
    drop(maps.remove(0));
    fixture.peer.emit(PeerEvent::Input(Wire::Keymap(
        0,
        1,
        keymap_fd(&bytes),
        bytes.len() as u32,
    )));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert_eq!(clone.as_bytes().len(), 1024 * 1024);
    assert!(observations(&mut fixture).is_empty());
}

#[test]
fn duplicate_and_misaligned_held_arrays_reject_before_focus() {
    for keys in [
        vec![1, 2, 3],
        [30u32, 30].into_iter().flat_map(u32::to_ne_bytes).collect(),
    ] {
        let (mut fixture, _) = fixture(9);
        fixture.create(101, 101);
        fixture
            .peer
            .emit(PeerEvent::Input(Wire::KeyboardEnter(0, 0, 1, keys)));
        read_barrier(&fixture.host);
        assert!(fixture.host.pump(Some(Instant::now())).is_err());
        assert!(observations(&mut fixture).is_empty());
    }
}

fn assert_keyboard_transitions(events: &[InputEvent], id: WindowId) {
    assert_eq!(
        events[1].observation,
        InputObservation::Keyboard(KeyboardObservation::Modifiers {
            window: Some(id),
            serial: 0,
            depressed: 1,
            latched: 2,
            locked: 4,
            group: 3
        })
    );
    assert_eq!(
        events[2].observation,
        InputObservation::Keyboard(KeyboardObservation::RepeatInfo {
            rate: 25,
            delay_ms: 600
        })
    );
    assert_eq!(
        events[3].observation,
        InputObservation::Keyboard(KeyboardObservation::Key {
            window: id,
            serial: u32::MAX,
            time_ms: u32::MAX,
            physical: 30,
            down: false
        })
    );
    assert_eq!(
        events[4].observation,
        InputObservation::Keyboard(KeyboardObservation::Key {
            window: id,
            serial: 0,
            time_ms: 0,
            physical: 31,
            down: true
        })
    );
}
