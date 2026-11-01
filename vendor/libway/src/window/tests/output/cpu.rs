//! CPU presentation proved against received SHM storage and independent wire events.
use super::peer::{Buffer, State};
use super::*;
use std::{fs::File, sync::Arc};
use wayland_server::protocol::{wl_buffer, wl_callback, wl_shm, wl_shm_pool};
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New};
impl GlobalDispatch<wl_shm::WlShm, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_shm::WlShm>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ()).format(wl_shm::Format::Argb8888);
        state.ledger.push(Request::BindShm);
    }
}
impl Dispatch<wl_shm::WlShm, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        request: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        let wl_shm::Request::CreatePool { id, fd, size } = request else {
            panic!("unexpected SHM request: {request:?}");
        };
        let file = File::from(fd);
        assert!(size > 0);
        assert_eq!(file.metadata().unwrap().len(), size as u64);
        // SAFETY: the received file owns a live fd; F_GET_SEALS has no pointer argument.
        let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
        let fixed_size = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;
        assert!(seals >= 0 && seals & fixed_size == fixed_size);
        // SAFETY: the received fd owns this pool's immutable-length storage.
        // Snapshots read after a flushed sync barrier, without concurrent host writes.
        let map = unsafe { memmap2::MmapOptions::new().len(size as usize).map(&file) }.unwrap();
        init.init(id, Arc::new(map));
        state.ledger.push(Request::Pool(size));
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, Arc<memmap2::Mmap>> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        map: &Arc<memmap2::Mmap>,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_shm_pool::Request::CreateBuffer {
                id,
                offset,
                width,
                height,
                stride,
                format,
            } => {
                assert_eq!(
                    format,
                    wayland_server::WEnum::Value(wl_shm::Format::Argb8888)
                );
                assert!(offset >= 0 && width > 0 && height > 0 && stride >= width * 4);
                assert!((offset as usize) + (stride * height) as usize <= map.len());
                let index = state.buffers.len();
                let proxy = init.init(id, index);
                state.buffers.push(Buffer {
                    proxy,
                    map: map.clone(),
                    offset: offset as usize,
                    width,
                    height,
                    stride,
                });
                state
                    .ledger
                    .push(Request::Buffer(index, width, height, stride));
            }
            wl_shm_pool::Request::Destroy => state.ledger.push(Request::DestroyPool),
            request => panic!("unexpected SHM pool request: {request:?}"),
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_buffer::WlBuffer,
        request: wl_buffer::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        assert!(matches!(request, wl_buffer::Request::Destroy));
        state.ledger.push(Request::DestroyBuffer(*index));
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_callback::WlCallback,
        request: wl_callback::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        panic!("unexpected callback request: {request:?}");
    }
}

pub(super) fn cpu_fixture(width: u32, height: u32) -> (Fixture, WindowId, Configuration) {
    let mut fixture = Fixture::with_options(PeerOptions {
        shm: true,
        ..Default::default()
    });
    let id = fixture.create(width, height);
    fixture.host.enable_cpu(id).unwrap();
    fixture.peer.take_requests();
    let config = fixture.configure(id, 0, width as i32, height as i32);
    (fixture, id, config)
}

pub(super) fn present(
    fixture: &mut Fixture,
    id: WindowId,
    config: Configuration,
    bytes: &[u8],
) -> PresentOutcome {
    let result = fixture
        .host
        .present_cpu(id, config.revision, bytes)
        .unwrap();
    fixture.sync();
    result
}

#[test]
fn cpu_requires_shm_and_maps_literal_argb_after_configure_ack() {
    let mut missing = Fixture::new();
    let id = missing.create(3, 1);
    missing.peer.take_requests();
    assert!(matches!(
        missing.host.enable_cpu(id),
        Err(Error::Unsupported(_))
    ));
    missing.sync();
    assert!(missing.peer.take_requests().is_empty());
    assert!(missing.host.failure().is_none());
    let (mut fixture, id, config) = cpu_fixture(3, 1);
    let rgba = [0x40, 0x20, 0x10, 0x80, 0xff, 0, 0, 0xff, 0, 0, 0, 0];
    assert_eq!(
        present(&mut fixture, id, config, &rgba),
        PresentOutcome::Presented
    );
    let requests = fixture.peer.take_requests();
    assert_eq!(requests[0], Request::Ack(0, 73));
    let buffer = requests
        .iter()
        .position(|r| *r == Request::Buffer(0, 3, 1, 12))
        .unwrap();
    let attach = requests
        .iter()
        .position(|r| *r == Request::AttachedBuffer(0, Some(0)))
        .unwrap();
    assert!(buffer < attach);
    assert!(requests.contains(&Request::Pool(12)));
    assert!(requests.contains(&Request::DestroyPool));
    assert_eq!(requests.last(), Some(&Request::Commit(0, true)));
    let expected = if cfg!(target_endian = "little") {
        [0x10, 0x20, 0x40, 0x80, 0, 0, 0xff, 0xff, 0, 0, 0, 0]
    } else {
        [0x80, 0x40, 0x20, 0x10, 0xff, 0xff, 0, 0, 0, 0, 0, 0]
    };
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 1);
    assert_eq!(
        (buffers[0].width, buffers[0].height, buffers[0].stride),
        (3, 1, 12)
    );
    assert_eq!(buffers[0].bytes, expected);
}

#[test]
fn release_and_frame_are_independent_before_buffer_reuse() {
    let (mut fixture, id, config) = cpu_fixture(1, 1);
    assert_eq!(
        present(&mut fixture, id, config, &[1, 2, 3, 255]),
        PresentOutcome::Presented
    );
    fixture.peer.emit(PeerEvent::Release(0));
    fixture.sync();
    fixture.peer.take_requests();
    let before = fixture.peer.buffers();
    assert_eq!(
        present(&mut fixture, id, config, &[4, 5, 6, 255]),
        PresentOutcome::NeedsFrame
    );
    assert!(fixture.peer.take_requests().is_empty());
    assert_eq!(fixture.peer.buffers(), before);
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    assert_eq!(
        present(&mut fixture, id, config, &[4, 5, 6, 255]),
        PresentOutcome::Presented
    );
    assert_eq!(fixture.peer.buffers().len(), 1);
    fixture.peer.emit(PeerEvent::Frame(0));
    fixture.sync();
    assert_eq!(
        present(&mut fixture, id, config, &[7, 8, 9, 255]),
        PresentOutcome::Presented
    );
    let buffers = fixture.peer.buffers();
    assert_eq!(buffers.len(), 2);
    assert_ne!(buffers[0].bytes, buffers[1].bytes);
}

#[test]
fn three_busy_slots_survive_resize_and_stale_revision_never_writes() {
    let (mut fixture, id, old) = cpu_fixture(1, 1);
    for channel in 1..=3 {
        assert_eq!(
            present(&mut fixture, id, old, &[channel, 0, 0, 255]),
            PresentOutcome::Presented
        );
        fixture.peer.emit(PeerEvent::Frame(0));
        fixture.sync();
    }
    let before = fixture.peer.buffers();
    assert_eq!(before.len(), 3);
    let new = fixture.configure(id, 0, 4, 1);
    fixture.peer.take_requests();
    assert!(
        fixture
            .host
            .present_cpu(id, old.revision, &[99, 0, 0, 255])
            .is_err()
    );
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    assert_eq!(fixture.peer.buffers(), before);
    let rgba = [0, 8, 0, 255].repeat(4);
    assert_eq!(
        present(&mut fixture, id, new, &rgba),
        PresentOutcome::NeedsRelease
    );
    assert!(fixture.peer.take_requests().is_empty());
    assert_eq!(fixture.peer.buffers(), before);
    fixture.peer.emit(PeerEvent::Release(1));
    fixture.sync();
    assert_eq!(
        present(&mut fixture, id, new, &rgba),
        PresentOutcome::Presented
    );
    let requests = fixture.peer.take_requests();
    let destroyed = requests
        .iter()
        .position(|r| *r == Request::DestroyBuffer(1))
        .unwrap();
    let created = requests
        .iter()
        .position(|r| *r == Request::Buffer(3, 4, 1, 16))
        .unwrap();
    assert!(destroyed < created);
    let after = fixture.peer.buffers();
    assert_eq!(after.len(), 4);
    assert_eq!(after[1], before[1]);
    assert_eq!((after[3].width, after[3].stride), (4, 16));
}

#[test]
fn closing_busy_output_retains_roles_and_storage_until_actual_release() {
    let (mut fixture, id, config) = cpu_fixture(1, 1);
    present(&mut fixture, id, config, &[1, 2, 3, 255]);
    let before = fixture.peer.buffers();
    fixture.peer.take_requests();
    fixture.host.close(id).unwrap();
    fixture.sync();
    assert_eq!(fixture.host.events(), [WindowEvent::Closed { window: id }]);
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::Attach(0, false),
            Request::AttachedBuffer(0, None),
            Request::Commit(0, false)
        ]
    );
    assert_eq!(fixture.peer.buffers(), before);
    fixture.peer.emit(PeerEvent::Release(0));
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::DestroyBuffer(0),
            Request::DestroyToplevel(0),
            Request::DestroyRole(0),
            Request::DestroySurface(0)
        ]
    );
    assert_eq!(fixture.peer.buffers(), before);
}

#[test]
fn host_drop_destroys_busy_local_buffer_without_rewriting_lease_retained_native_role() {
    let (mut fixture, id, config) = cpu_fixture(1, 1);
    let lease = fixture.host.lease(id).unwrap();
    present(&mut fixture, id, config, &[1, 2, 3, 255]);
    let before = fixture.peer.buffers();
    fixture.peer.take_requests();
    let Fixture { host, peer } = fixture;
    drop(host);
    assert!(lease.is_retired());
    assert_eq!(
        peer.take_requests(),
        [
            Request::Attach(0, false),
            Request::AttachedBuffer(0, None),
            Request::Commit(0, false),
            Request::DestroyBuffer(0)
        ]
    );
    assert_eq!(peer.buffers(), before);
    drop(lease);
    assert_eq!(
        peer.requests(),
        [
            Request::DestroyToplevel(0),
            Request::DestroyRole(0),
            Request::DestroySurface(0),
            Request::DestroyShell
        ]
    );
    assert_eq!(peer.buffers(), before);
}
