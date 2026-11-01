//! EPIPE consumes only the write's new signal and restores the exact thread mask.
use super::*;
use crate::window::clipboard::transfer::open_pipe;
use std::os::fd::AsRawFd;

fn current_mask() -> libc::sigset_t {
    let mut mask = MaybeUninit::<libc::sigset_t>::zeroed();
    // SAFETY: a null set only queries the current thread mask into live storage.
    assert_eq!(
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), mask.as_mut_ptr()) },
        0
    );
    // SAFETY: the successful query initialized mask.
    unsafe { mask.assume_init() }
}

fn contains(mask: &libc::sigset_t, signal: i32) -> i32 {
    // SAFETY: mask is initialized; queried signal numbers are in Linux's domain.
    unsafe { libc::sigismember(mask, signal) }
}

fn pending_pipe() -> bool {
    let mut set = MaybeUninit::<libc::sigset_t>::zeroed();
    // SAFETY: sigpending initializes the live writable set.
    assert_eq!(unsafe { libc::sigpending(set.as_mut_ptr()) }, 0);
    // SAFETY: set was initialized and SIGPIPE is valid.
    (unsafe { libc::sigismember(set.as_ptr(), libc::SIGPIPE) }) == 1
}

fn assert_same_mask(before: &libc::sigset_t) {
    let after = current_mask();
    for signal in 1..=64 {
        assert_eq!(contains(before, signal), contains(&after, signal));
    }
}

#[test]
fn closed_reader_does_not_leave_a_new_pending_pipe_or_change_the_mask() {
    let before = current_mask();
    let prior_pending = pending_pipe();
    let (read, fd) = open_pipe().unwrap();
    drop(read);
    let WriteError::Io(error) = write(fd.as_raw_fd(), b"closed").unwrap_err() else {
        panic!("unexpected mask failure");
    };
    assert_eq!(error.raw_os_error(), Some(libc::EPIPE));
    assert_eq!(pending_pipe(), prior_pending);
    assert_same_mask(&before);
}

struct Pending {
    mask: Mask,
}
impl Drop for Pending {
    fn drop(&mut self) {
        // The fixture generated this pending signal; consume it before restoring.
        self.mask.pending = false;
        self.mask.consume_new().unwrap();
        self.mask.restore().unwrap();
    }
}

#[test]
fn an_already_pending_pipe_survives_the_guard_and_unrelated_mask_bits_survive() {
    let before = current_mask();
    assert!(!pending_pipe(), "fixture requires no prior pending SIGPIPE");
    let mut mask = Mask::block().unwrap();
    let mut configured = current_mask();
    // SAFETY: configured is initialized and SIGUSR1 is a valid signal number.
    assert_eq!(
        unsafe { libc::sigaddset(&mut configured, libc::SIGUSR1) },
        0
    );
    // SAFETY: change only this fixture thread's mask, retained for restoration.
    assert_eq!(
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &configured, std::ptr::null_mut()) },
        0
    );
    // SAFETY: pthread_self names this blocked fixture thread; no handler is changed.
    assert_eq!(
        unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGPIPE) },
        0
    );
    mask.pending = true;
    let pending = Pending { mask };
    assert!(pending_pipe());
    let configured = current_mask();
    let (read, fd) = open_pipe().unwrap();
    drop(read);
    let WriteError::Io(error) = write(fd.as_raw_fd(), b"closed").unwrap_err() else {
        panic!("unexpected mask failure");
    };
    assert_eq!(error.raw_os_error(), Some(libc::EPIPE));
    assert!(pending_pipe(), "preexisting SIGPIPE must not be consumed");
    assert_same_mask(&configured);
    drop(pending);
    assert_same_mask(&before);
    assert!(!pending_pipe());
}

#[test]
fn default_sigpipe_child_survives_epipe_and_preserves_pending_mask() {
    const CHILD: &str = "LIBWAY_CLIPBOARD_SIGPIPE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        // SAFETY: this is an isolated test subprocess, never the parent/host. Set
        // the default disposition so an unguarded EPIPE would kill this child.
        assert_ne!(
            unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) },
            libc::SIG_ERR
        );
        closed_reader_does_not_leave_a_new_pending_pipe_or_change_the_mask();
        an_already_pending_pipe_survives_the_guard_and_unrelated_mask_bits_survive();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact",
            "window::clipboard::transfer::signal::tests::default_sigpipe_child_survives_epipe_and_preserves_pending_mask",
            "--test-threads=1"])
        .env(CHILD, "1")
        .output().unwrap();
    assert!(
        output.status.success(),
        "isolated SIG_DFL child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "the child must run the exact survival test"
    );
}
