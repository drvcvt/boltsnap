//! Payload budgets follow final shared ownership; rejected fds close immediately.
use super::*;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::fd::FromRawFd,
};
fn file(bytes: &[u8]) -> File {
    // SAFETY: the name is constant/NUL-terminated and success creates a unique fd.
    let raw = unsafe { libc::memfd_create(c"libway-keymap-payload".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(raw >= 0);
    // SAFETY: no other owner has been created for the successful descriptor.
    let mut file = unsafe { File::from_raw_fd(raw) };
    file.write_all(bytes).unwrap();
    file
}
#[test]
fn final_map_clone_releases_credit_and_positional_read_does_not_seek() {
    let budget = Budget::new(4);
    let mut original = file(b"abc\0");
    original.seek(SeekFrom::Start(2)).unwrap();
    let map = keymap(original.try_clone().unwrap().into(), 4, &budget).unwrap();
    assert_eq!(map.as_str(), "abc");
    assert_eq!(original.stream_position().unwrap(), 2);
    let retained = map.clone();
    drop(map);
    assert!(matches!(
        keymap(file(b"abc\0").into(), 4, &budget),
        Err(Error::LimitExceeded)
    ));
    drop(retained);
    assert_eq!(
        keymap(file(b"abc\0").into(), 4, &budget).unwrap().as_str(),
        "abc"
    );
}
#[test]
fn non_regular_owned_fd_is_closed_on_rejection() {
    let mut descriptors = [-1; 2];
    // SAFETY: two initialized entries are writable, and success creates two unique fds.
    assert_eq!(
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    // SAFETY: each successful descriptor is moved into exactly one owner.
    let read = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
    // SAFETY: the distinct write descriptor likewise has no prior owner.
    let mut write = unsafe { File::from_raw_fd(descriptors[1]) };
    assert!(matches!(
        keymap(read, 1, &Budget::new(4)),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        write.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}
#[test]
fn shortened_file_returns_error_and_releases_reserved_payload_credit() {
    let budget = Budget::new(4);
    let mut shortened = file(b"abc\0");
    shortened.set_len(2).unwrap();
    assert!(keymap(shortened.try_clone().unwrap().into(), 4, &budget).is_err());
    shortened.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    shortened.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"ab");
    assert!(keymap(file(b"abc\0").into(), 4, &budget).is_ok());
}
#[test]
fn held_key_and_name_clones_retain_their_aggregate_charges() {
    let budget = Budget::new(2);
    let bytes: Vec<_> = [1u32, 2].into_iter().flat_map(u32::to_ne_bytes).collect();
    let held = held_keys(&bytes, &budget).unwrap();
    let clone = held.clone();
    drop(held);
    assert!(matches!(
        held_keys(&bytes, &budget),
        Err(Error::LimitExceeded)
    ));
    drop(clone);
    assert!(held_keys(&bytes, &budget).is_ok());
    let budget = Budget::new(3);
    let name = seat_name("abc".into(), &budget).unwrap();
    let clone = name.clone();
    drop(name);
    assert!(matches!(
        seat_name("abc".into(), &budget),
        Err(Error::LimitExceeded)
    ));
    drop(clone);
    assert_eq!(seat_name("abc".into(), &budget).unwrap().as_str(), "abc");
    assert!(seat_name("x".repeat(257), &Budget::new(1024)).is_err());
}

#[test]
fn held_array_exact_limit_and_one_beyond_are_checked_before_allocation() {
    // A 1024-key wl_keyboard Enter exceeds the Rust peer's 4096-byte wire
    // message buffer once its header is included; exercise this payload boundary
    // directly without turning a transport-size failure into a dispatch oracle.
    let bytes: Vec<_> = (0..1024u32).flat_map(u32::to_ne_bytes).collect();
    let budget = Budget::new(8192);
    let held = held_keys(&bytes, &budget).unwrap();
    assert_eq!(held.as_slice().len(), 1024);
    let oversized: Vec<_> = (0..1025u32).flat_map(u32::to_ne_bytes).collect();
    assert!(matches!(
        held_keys(&oversized, &budget),
        Err(Error::InvalidInput(_))
    ));
}
