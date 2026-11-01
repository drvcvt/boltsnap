//! Literal pipe traces exercise limits, EOF, fair slices and immutable send lifetimes.
use super::super::budget::Budget;
use super::*;
use std::{
    fs::File,
    io::{Read, Write},
    sync::Arc,
    time::Duration,
};

fn id(sequence: u64) -> ClipboardTransferId {
    ClipboardTransferId(17, sequence)
}
fn seat(sequence: u64) -> InputSeatId {
    InputSeatId {
        connection: 17,
        sequence,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn incoming(transfers: &mut Transfers, sequence: u64, max: usize) -> File {
    let (fd, peer) = open_pipe().unwrap();
    assert!(
        transfers
            .read(ReadAdmission {
                id: id(sequence),
                fd,
                max_bytes: max,
                deadline: deadline(),
                replace: None
            })
            .unwrap()
            .is_none()
    );
    File::from(peer)
}

fn outcomes(service: Service) -> Vec<(ClipboardTransferId, ClipboardReceiveOutcome)> {
    service
        .completions
        .into_iter()
        .flatten()
        .map(|done| (done.id, done.outcome))
        .collect()
}

/// Service until `count` completions arrived. A parallel test that spawns a
/// process (signal_tests) briefly holds copies of every descriptor between fork
/// and exec, so EOF after dropping the last writer can arrive a moment later.
fn completed(
    transfers: &mut Transfers,
    count: usize,
) -> Vec<(ClipboardTransferId, ClipboardReceiveOutcome)> {
    let mut done = Vec::new();
    for _ in 0..500 {
        done.extend(outcomes(transfers.service(Instant::now())));
        if done.len() >= count {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    done
}

/// Read a nonblocking pipe to EOF, tolerating the same fork window.
fn read_to_eof(mut file: File) -> Vec<u8> {
    let mut received = Vec::new();
    for _ in 0..500 {
        match file.read_to_end(&mut received) {
            Ok(_) => return received,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("pipe read failed: {error}"),
        }
    }
    panic!("pipe never reached EOF");
}

fn payload(bytes: &Arc<[u8]>) -> Payload {
    Budget::default().payload(bytes).unwrap()
}

fn fill_pipe(fd: OwnedFd) -> (OwnedFd, usize) {
    let mut file = File::from(fd);
    let mut total = 0;
    for _ in 0..256 {
        match file.write(&[0x55; 4096]) {
            Ok(count) => total += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return (file.into(), total);
            }
            other => panic!("unexpected fill result: {other:?}"),
        }
    }
    panic!("pipe fixture exceeded 1MiB without backpressure");
}

fn drain_pipe(file: &mut File, output: &mut Vec<u8>) {
    let mut bytes = [0u8; 8192];
    loop {
        match file.read(&mut bytes) {
            Ok(0) => return,
            Ok(count) => output.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            other => panic!("unexpected drain result: {other:?}"),
        }
    }
}

#[test]
fn send_backpressure_sleeps_then_resumes_without_losing_or_duplicating_bytes() {
    let (peer, fd) = open_pipe().unwrap();
    let (fd, filled) = fill_pipe(fd);
    let bytes: Arc<[u8]> = Arc::from(vec![0xab; 16_384]);
    let mut transfers = Transfers::default();
    transfers
        .send(fd, payload(&bytes), deadline(), seat(1))
        .unwrap();
    let blocked = transfers.service(Instant::now());
    assert_eq!(blocked.bytes, 0);
    assert!(!blocked.runnable && blocked.failure.is_none());
    let mut peer = File::from(peer);
    let mut filler = vec![0; filled];
    peer.read_exact(&mut filler).unwrap();
    assert!(filler.iter().all(|byte| *byte == 0x55));
    let mut received = Vec::new();
    for _ in 0..64 {
        let turn = transfers.service(Instant::now());
        assert!(turn.bytes <= 65_536 && turn.failure.is_none());
        drain_pipe(&mut peer, &mut received);
        if transfers.jobs.is_empty() {
            break;
        }
    }
    assert!(transfers.jobs.is_empty());
    assert_eq!(received, bytes.as_ref());
}

#[test]
fn exact_limit_waits_for_eof_and_empty_is_a_complete_representation() {
    let mut transfers = Transfers::default();
    let mut peer = incoming(&mut transfers, 1, 3);
    peer.write_all(b"abc").unwrap();
    let first = transfers.service(Instant::now());
    assert_eq!(first.bytes, 3);
    assert!(!first.runnable);
    assert!(
        outcomes(first).is_empty(),
        "limit bytes without EOF are incomplete"
    );
    drop(peer);
    assert_eq!(
        completed(&mut transfers, 1),
        [(id(1), ClipboardReceiveOutcome::Bytes(b"abc".to_vec()))]
    );
    drop(incoming(&mut transfers, 2, 1));
    assert_eq!(
        completed(&mut transfers, 1),
        [(id(2), ClipboardReceiveOutcome::Bytes(Vec::new()))]
    );
    assert!(transfers.deadline().is_none());
}

#[test]
fn overflow_reads_only_one_extra_byte_and_never_delivers_a_prefix() {
    let mut transfers = Transfers::default();
    let mut peer = incoming(&mut transfers, 1, 3);
    peer.write_all(b"abcde").unwrap();
    let first = transfers.service(Instant::now());
    assert_eq!(first.bytes, 4);
    assert_eq!(
        outcomes(first),
        [(id(1), ClipboardReceiveOutcome::TooLarge)]
    );
    assert!(transfers.jobs.is_empty());
}

#[test]
fn partial_reads_block_without_runnable_work_then_preserve_exact_bytes() {
    let mut transfers = Transfers::default();
    let mut peer = incoming(&mut transfers, 1, 8);
    peer.write_all(&[0xff, 0, 10]).unwrap();
    assert!(outcomes(transfers.service(Instant::now())).is_empty());
    let blocked = transfers.service(Instant::now());
    assert_eq!(blocked.bytes, 0);
    assert!(!blocked.runnable);
    peer.write_all(b"z").unwrap();
    drop(peer);
    assert_eq!(
        completed(&mut transfers, 1),
        [(
            id(1),
            ClipboardReceiveOutcome::Bytes(vec![0xff, 0, 10, b'z'])
        )]
    );
}

#[test]
fn admission_failure_preserves_old_read_and_full_capacity_allows_replacement() {
    let mut transfers = Transfers::default();
    let peers: Vec<_> = (1..=8).map(|n| incoming(&mut transfers, n, 8)).collect();
    for (max, replace) in [(0, Some(id(1))), (8, None)] {
        let (fd, _peer) = open_pipe().unwrap();
        assert!(
            transfers
                .read(ReadAdmission {
                    id: id(9),
                    fd,
                    max_bytes: max,
                    deadline: deadline(),
                    replace
                })
                .is_err()
        );
        assert!(transfers.read_index(id(1)).is_some());
        assert_eq!(transfers.jobs.len(), 8);
    }
    let (fd, peer) = open_pipe().unwrap();
    let cancelled = transfers
        .read(ReadAdmission {
            id: id(9),
            fd,
            max_bytes: 8,
            deadline: deadline(),
            replace: Some(id(1)),
        })
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.id, id(1));
    assert_eq!(cancelled.outcome, ClipboardReceiveOutcome::Cancelled);
    assert!(transfers.cancel(id(1)).is_none());
    let mut peer = File::from(peer);
    peer.write_all(b"new").unwrap();
    drop(peer);
    drop(peers);
    let ready = completed(&mut transfers, 8);
    assert_eq!(ready.len(), 8);
    assert!(ready.contains(&(id(9), ClipboardReceiveOutcome::Bytes(b"new".to_vec()))));
}

#[test]
fn io_failure_uses_one_terminal_outcome_and_cannot_replay() {
    let mut transfers = Transfers::default();
    let (_read, fd) = open_pipe().unwrap();
    transfers
        .read(ReadAdmission {
            id: id(1),
            fd,
            max_bytes: 8,
            deadline: deadline(),
            replace: None,
        })
        .unwrap();
    let first = outcomes(transfers.service(Instant::now()));
    let ClipboardReceiveOutcome::Failed(error) = &first[0].1 else {
        panic!("expected read error")
    };
    assert!(
        matches!(error.as_ref(), Error::Io(error) if error.raw_os_error() == Some(libc::EBADF))
    );
    assert!(outcomes(transfers.service(Instant::now())).is_empty());
}

#[test]
fn cancellation_does_not_target_a_new_pipe_reusing_the_old_descriptor() {
    // Parallel tests may take the freed descriptor number first. The identity
    // properties must hold on every attempt; retry until the kernel actually
    // reused the number, so the interesting case is exercised.
    for _ in 0..64 {
        let mut transfers = Transfers::default();
        let (fd, _old_peer) = open_pipe().unwrap();
        let number = fd.as_raw_fd();
        transfers
            .read(ReadAdmission {
                id: id(1),
                fd,
                max_bytes: 8,
                deadline: deadline(),
                replace: None,
            })
            .unwrap();
        transfers.cancel(id(1)).unwrap();
        let (fd, peer) = open_pipe().unwrap();
        let reused = fd.as_raw_fd() == number;
        transfers
            .read(ReadAdmission {
                id: id(2),
                fd,
                max_bytes: 8,
                deadline: deadline(),
                replace: None,
            })
            .unwrap();
        assert!(transfers.cancel(id(1)).is_none());
        let mut peer = File::from(peer);
        peer.write_all(b"new").unwrap();
        drop(peer);
        assert_eq!(
            completed(&mut transfers, 1),
            [(id(2), ClipboardReceiveOutcome::Bytes(b"new".to_vec()))]
        );
        if reused {
            return;
        }
    }
    panic!("the freed descriptor number was never reused in 64 attempts");
}

#[test]
fn deadlines_and_cancellation_remove_only_the_exact_owned_request() {
    let mut transfers = Transfers::default();
    let peer = incoming(&mut transfers, 1, 8);
    let end = transfers.deadline().unwrap();
    assert_eq!(
        outcomes(transfers.service(end)),
        [(id(1), ClipboardReceiveOutcome::TimedOut)]
    );
    drop(peer);
    let peer = incoming(&mut transfers, 2, 8);
    assert!(transfers.cancel(id(1)).is_none());
    assert_eq!(
        transfers.cancel(id(2)).unwrap().outcome,
        ClipboardReceiveOutcome::Cancelled
    );
    assert!(transfers.cancel(id(2)).is_none());
    assert!(transfers.jobs.is_empty());
    drop(peer);
}

#[test]
fn reduced_slice_rotates_the_same_production_scheduler_without_starving_peers() {
    let mut transfers = Transfers::default();
    let mut peers: Vec<_> = (1..=6).map(|n| incoming(&mut transfers, n, 8)).collect();
    for peer in &mut peers {
        peer.write_all(b"12345678").unwrap();
    }
    for expected in [[1, 2], [3, 4], [5, 6]] {
        let report = transfers.service_slice(Instant::now(), 4, 8);
        assert_eq!(report.bytes, 8);
        assert!(report.runnable);
        let progressed: Vec<_> = transfers
            .jobs
            .iter()
            .filter_map(|job| match job {
                Job::Read(read) if read.received() == 4 => Some(read.id.1),
                _ => None,
            })
            .collect();
        for sequence in expected {
            assert!(progressed.contains(&sequence));
        }
    }
    assert_eq!((JOB_BYTES, TURN_BYTES), (65_536, 262_144));
    drop(peers);
}

#[test]
fn send_keeps_original_snapshot_after_source_drop_and_closes_at_completion() {
    let bytes: Arc<[u8]> = Arc::from(&b"old source"[..]);
    let old = payload(&bytes);
    let (peer, fd) = open_pipe().unwrap();
    let mut transfers = Transfers::default();
    transfers
        .send(fd, old.clone(), deadline(), seat(1))
        .unwrap();
    drop(old);
    let replacement = payload(&Arc::from(&b"new source"[..]));
    assert_eq!(Arc::strong_count(&bytes), 2);
    assert_eq!(transfers.service(Instant::now()).bytes, 10);
    let received = read_to_eof(File::from(peer));
    assert_eq!(received, b"old source");
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert!(transfers.jobs.is_empty());
    drop(replacement);
}

#[test]
fn sends_are_bounded_and_seat_retirement_and_deadline_release_payloads() {
    let bytes: Arc<[u8]> = Arc::from(&b"x"[..]);
    let source = payload(&bytes);
    let mut transfers = Transfers::default();
    let mut peers = Vec::new();
    for sequence in 1..=16 {
        let (peer, fd) = open_pipe().unwrap();
        let (fd, _) = fill_pipe(fd);
        transfers
            .send(fd, source.clone(), deadline(), seat(sequence))
            .unwrap();
        peers.push(peer);
    }
    let blocked = transfers.service(Instant::now());
    assert_eq!(blocked.bytes, 0);
    assert!(!blocked.runnable && blocked.failure.is_none());
    assert_eq!(
        transfers.jobs.len(),
        16,
        "all sixteen genuine send pipes must remain active"
    );
    let (_peer, fd) = open_pipe().unwrap();
    assert!(matches!(
        transfers.send(fd, source.clone(), deadline(), seat(17)),
        Err(Error::LimitExceeded)
    ));
    transfers.cancel_sends(seat(1));
    assert_eq!(transfers.jobs.len(), 15);
    drop(source);
    let end = transfers.jobs.iter().map(Job::deadline).max().unwrap();
    assert_eq!(transfers.service(end).bytes, 0);
    assert!(transfers.jobs.is_empty());
    assert_eq!(Arc::strong_count(&bytes), 1);
    drop(peers);
}

#[test]
fn descriptors_are_nonblocking_and_close_on_exec() {
    let (read, write) = open_pipe().unwrap();
    for fd in [&read, &write] {
        // SAFETY: both fcntl queries operate on a live owned fd without pointers.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(flags & libc::O_NONBLOCK, 0);
        // SAFETY: F_GETFD reads flags without transferring descriptor ownership.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }
}
