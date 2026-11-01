//! SHM buffer formats: a straight RGBA copy into ABGR8888 when the compositor
//! offers it, the ARGB8888 swizzle otherwise, and a switch once the format
//! arrives late.
use super::cpu::present;
use super::*;

const ABGR: u32 = wayland_client::protocol::wl_shm::Format::Abgr8888 as u32;
const ARGB: u32 = 0;

fn fixture(shm_abgr: bool) -> (Fixture, WindowId, Configuration) {
    let mut fixture = Fixture::with_options(PeerOptions {
        shm: true,
        shm_abgr,
        ..Default::default()
    });
    let id = fixture.create(2, 1);
    fixture.host.enable_cpu(id).unwrap();
    fixture.peer.take_requests();
    let config = fixture.configure(id, 0, 2, 1);
    (fixture, id, config)
}

#[test]
fn advertised_abgr_buffers_carry_the_rgba_bytes_unchanged() {
    let (mut fixture, id, config) = fixture(true);
    let first = [1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(
        present(&mut fixture, id, config, &first),
        PresentOutcome::Presented
    );
    let buffers = fixture.peer.buffers();
    let expected = if cfg!(target_endian = "little") {
        ABGR
    } else {
        ARGB
    };
    assert_eq!(buffers[0].format, expected);
    if expected == ABGR {
        assert_eq!(buffers[0].bytes, first);
    }
    fixture.peer.emit(PeerEvent::Release(0));
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    let second = [1, 2, 3, 4, 9, 9, 9, 9];
    let outcome = fixture
        .host
        .present_cpu_damaged(id, config.revision, &second, &[PixelRect::new(1, 0, 1, 1)])
        .unwrap();
    fixture.sync();
    assert_eq!(outcome, PresentOutcome::Presented);
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 1, "the released slot is reused");
    if expected == ABGR {
        assert_eq!(buffers[0].bytes, second);
    }
}

#[test]
fn without_abgr_buffers_stay_argb_and_swizzled() {
    let (mut fixture, id, config) = fixture(false);
    present(&mut fixture, id, config, &[1, 2, 3, 4, 5, 6, 7, 8]);
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers[0].format, ARGB);
    let expected = if cfg!(target_endian = "little") {
        [3, 2, 1, 4, 7, 6, 5, 8]
    } else {
        [4, 1, 2, 3, 8, 5, 6, 7]
    };
    assert_eq!(buffers[0].bytes, expected);
}

#[test]
fn a_late_abgr_format_replaces_the_idle_argb_slot_on_the_next_present() {
    if cfg!(target_endian = "big") {
        return;
    }
    let (mut fixture, id, config) = fixture(false);
    present(&mut fixture, id, config, &[1, 2, 3, 4, 5, 6, 7, 8]);
    fixture.peer.emit(PeerEvent::Release(0));
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.peer.emit(PeerEvent::ShmFormat(ABGR));
    fixture.sync();
    fixture.peer.take_requests();
    present(&mut fixture, id, config, &[9, 8, 7, 6, 5, 4, 3, 2]);
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 2);
    assert_eq!(buffers[1].format, ABGR);
    assert_eq!(buffers[1].bytes, [9, 8, 7, 6, 5, 4, 3, 2]);
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::DestroyBuffer(0))
    );
}
