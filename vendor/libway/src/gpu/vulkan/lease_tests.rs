//! Hardware tests of leased host memory: tiles that read the readback's
//! mapping directly. Own buffers on an explicit render node, never the
//! desktop; run like `tests.rs`.
use super::{
    lease::{MAX_IDLE, MAX_LEASED},
    tests::{allocate, fill, open, open_fds, pattern, testable},
};
use crate::CpuBuffer;
use std::time::{Duration, Instant};

fn end() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn leased_bytes_cross_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<CpuBuffer>();
    send_sync::<crate::DesktopTiles>();
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates and fills its own buffers"]
fn reads_lease_distinct_host_memory() {
    let (mut readback, allocator) = open();
    let size = (640, 360);
    let buffer = allocate(&allocator, size, testable(&readback)[0]).unwrap();
    let first = pattern(size);
    fill(&readback, &buffer, size, &first);
    let a = readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(a.is_leased(), "cached host memory is leased, not copied");
    let second: Vec<u8> = first.iter().map(|b| b ^ 0x5a).collect();
    fill(&readback, &buffer, size, &second);
    let b = readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(b.is_leased());
    assert_ne!(
        a.bytes().as_ptr(),
        b.bytes().as_ptr(),
        "two shots share a host buffer"
    );
    // A third read while both are held must not touch either.
    readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(
        a.bytes() == first.as_slice(),
        "the first lease was overwritten"
    );
    assert!(
        b.bytes() == second.as_slice(),
        "the second lease was overwritten"
    );
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates and fills its own buffers"]
fn leases_outlive_the_readback() {
    let fds = open_fds();
    let (mut readback, allocator) = open();
    let size = (256, 128);
    let buffer = allocate(&allocator, size, testable(&readback)[0]).unwrap();
    let pixels = pattern(size);
    fill(&readback, &buffer, size, &pixels);
    let tile = readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(tile.is_leased());
    drop(readback);
    drop(buffer);
    drop(allocator);
    // The device is still alive through the lease, and on another thread.
    let check = std::thread::spawn(move || {
        assert!(
            tile.bytes() == pixels.as_slice(),
            "bytes changed after the readback was dropped"
        );
        drop(tile);
    });
    check.join().unwrap();
    assert_eq!(open_fds(), fds, "descriptors leaked");
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates and fills its own buffers"]
fn the_lease_cap_falls_back_to_heap_copies() {
    let (mut readback, allocator) = open();
    let size = (128, 64);
    let buffer = allocate(&allocator, size, testable(&readback)[0]).unwrap();
    let pixels = pattern(size);
    fill(&readback, &buffer, size, &pixels);
    let held: Vec<CpuBuffer> = (0..MAX_LEASED)
        .map(|_| readback.read(&buffer, size.0, size.1, end()).unwrap())
        .collect();
    assert!(held.iter().all(CpuBuffer::is_leased));
    assert_eq!(readback.host.counts(), (0, MAX_LEASED));
    let extra = readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(!extra.is_leased(), "past the cap reads are copied");
    assert!(extra.bytes() == pixels.as_slice());
    // The copy's buffer went back idle; returned leases fill up to MAX_IDLE.
    drop(held);
    assert_eq!(readback.host.counts(), (MAX_IDLE, 0));
    let again = readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(again.is_leased() && again.bytes() == pixels.as_slice());
    assert_eq!(readback.host.counts(), (MAX_IDLE - 1, 1));
}
