//! Deterministic slice limits, fairness, readiness and exact completion.
use super::super::{TransferId, open_pipe};
use super::*;
use std::{
    fs::File,
    io::{Read, Write},
};

fn allowance(bytes: usize) -> Allowance {
    Allowance {
        remaining: bytes,
        until: Instant::now() + Duration::from_secs(10),
        progressed: false,
        yielded: false,
    }
}
fn incoming(reactor: &mut Reactor, id: u64, bytes: &[u8]) {
    let (read, write) = open_pipe().unwrap();
    File::from(write).write_all(bytes).unwrap();
    let now = Instant::now();
    reactor.push_incoming(TransferId(id, 1), read, now, now);
}

#[test]
fn exact_bytes_survive_yields_and_rotating_peers() {
    let mut reactor = Reactor::default();
    incoming(&mut reactor, 1, b"abcdef");
    incoming(&mut reactor, 2, b"ghijkl");
    let limits = DndLimits::default();
    let mut results = Vec::new();
    for _ in 0..8 {
        results.extend(reactor.service_with(Instant::now(), &limits, allowance(2)));
    }
    assert_eq!(results.len(), 2);
    for done in results {
        let Done::Incoming { id, result } = done else {
            panic!()
        };
        assert_eq!(
            result.unwrap(),
            if id.0 == 1 { b"abcdef" } else { b"ghijkl" }
        );
    }
    assert!(reactor.incoming.is_empty());
    assert!(!reactor.ready);
}

#[test]
fn both_directions_share_the_quota_without_starvation() {
    let mut reactor = Reactor::default();
    incoming(&mut reactor, 1, b"abcdef");
    let (read, write) = open_pipe().unwrap();
    reactor.push_outgoing(
        write,
        super::super::Bytes::from(b"uvwxyz".to_vec()),
        Instant::now(),
    );
    let limits = DndLimits::default();
    reactor.service_with(Instant::now(), &limits, allowance(2));
    assert_eq!(reactor.incoming[0].buf, b"ab");
    assert_eq!(reactor.outgoing[0].offset, 0);
    reactor.service_with(Instant::now(), &limits, allowance(2));
    assert_eq!(reactor.incoming[0].buf, b"ab");
    assert_eq!(reactor.outgoing[0].offset, 2);
    for _ in 0..8 {
        reactor.service_with(Instant::now(), &limits, allowance(2));
    }
    assert!(reactor.incoming.is_empty());
    assert!(reactor.outgoing.is_empty());
    let mut bytes = Vec::new();
    File::from(read).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"uvwxyz");
}

#[test]
fn readiness_is_cleared_on_would_block_and_cancellation() {
    let mut reactor = Reactor::default();
    let (read, write) = open_pipe().unwrap();
    let mut write = File::from(write);
    write.write_all(b"ab").unwrap();
    let now = Instant::now();
    let id = TransferId(1, 1);
    reactor.push_incoming(id, read, now, now);
    let limits = DndLimits::default();
    reactor.service_with(now, &limits, allowance(2));
    assert!(reactor.ready);
    assert!(reactor.poll_set(123, false, &limits).unwrap().1.unwrap() <= Instant::now());
    reactor.service(now, &limits);
    assert!(!reactor.ready);
    assert_eq!(reactor.incoming[0].buf, b"ab");
    reactor.cancel_incoming(id);
    reactor.service(now, &limits);
    assert!(reactor.poll_set(123, false, &limits).unwrap().1.is_none());
}

#[test]
fn expiration_is_checked_even_when_the_quota_is_empty() {
    let mut reactor = Reactor::default();
    incoming(&mut reactor, 1, b"late");
    let limits = DndLimits::default();
    let done = reactor.service_with(Instant::now() + limits.total, &limits, allowance(0));
    assert!(matches!(
        done.as_slice(),
        [Done::Incoming {
            result: Err(TransferError::Timeout),
            ..
        }]
    ));
    assert!(!reactor.ready);
}
