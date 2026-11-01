//! Block SIGPIPE only on the writing thread, preserving its old mask and pending signal.
use std::{io, mem::MaybeUninit, os::fd::RawFd};

#[cfg(test)]
#[path = "signal_tests.rs"]
mod tests;

struct Mask {
    old: libc::sigset_t,
    blocked: libc::sigset_t,
    pending: bool,
    restored: bool,
}

#[derive(Debug)]
pub(super) enum WriteError {
    Io(io::Error),
    Signal(io::Error),
}

impl Mask {
    fn block() -> io::Result<Self> {
        // Linux may write only its kernel-sized prefix of libc's larger sigset_t.
        // Initialize the entire storage before any syscall to avoid trailing UB.
        let mut set = MaybeUninit::<libc::sigset_t>::zeroed();
        let mut old = MaybeUninit::<libc::sigset_t>::zeroed();
        // SAFETY: libc initializes set before adding the valid SIGPIPE number.
        unsafe {
            if libc::sigemptyset(set.as_mut_ptr()) != 0
                || libc::sigaddset(set.as_mut_ptr(), libc::SIGPIPE) != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        // SAFETY: initialized set is borrowed; old receives this thread's mask.
        let error =
            unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, set.as_ptr(), old.as_mut_ptr()) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        // SAFETY: both sets were initialized by successful libc calls above.
        let mut mask = Self {
            old: unsafe { old.assume_init() },
            blocked: unsafe { set.assume_init() },
            pending: false,
            restored: false,
        };
        let mut pending = MaybeUninit::<libc::sigset_t>::zeroed();
        // SAFETY: sigpending initializes the writable set without changing the mask.
        if unsafe { libc::sigpending(pending.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: pending was initialized and SIGPIPE is a valid signal number.
        let member = unsafe { libc::sigismember(pending.as_ptr(), libc::SIGPIPE) };
        if member < 0 {
            return Err(io::Error::last_os_error());
        }
        mask.pending = member == 1;
        Ok(mask)
    }

    fn consume_new(&self) -> io::Result<()> {
        if self.pending {
            return Ok(());
        }
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        loop {
            // SAFETY: blocked contains only SIGPIPE and timeout requests no waiting.
            let result =
                unsafe { libc::sigtimedwait(&self.blocked, std::ptr::null_mut(), &timeout) };
            if result == libc::SIGPIPE {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        // SAFETY: old is the exact mask captured on this same synchronous thread.
        let error =
            unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut()) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        self.restored = true;
        Ok(())
    }
}

impl Drop for Mask {
    fn drop(&mut self) {
        if !self.restored {
            // Successful restore is returned explicitly; this is unwind/error cleanup.
            let _ = self.restore();
        }
    }
}

pub(super) fn write(fd: RawFd, bytes: &[u8]) -> Result<usize, WriteError> {
    let mut mask = Mask::block().map_err(WriteError::Signal)?;
    // SAFETY: the caller retains its owned fd; bytes stays readable for its length.
    let count = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    let result = if count < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(count as usize)
    };
    if result
        .as_ref()
        .is_err_and(|error| error.raw_os_error() == Some(libc::EPIPE))
    {
        mask.consume_new().map_err(WriteError::Signal)?;
    }
    mask.restore().map_err(WriteError::Signal)?;
    result.map_err(WriteError::Io)
}
