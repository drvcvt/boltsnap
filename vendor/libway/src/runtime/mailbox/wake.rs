//! Durable nonblocking socket notification and terminal peer EOF ownership.
use std::{
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::net::UnixStream,
    },
};

const INTERRUPTED_ATTEMPTS: usize = 4;

pub(super) struct Wake {
    reader: UnixStream,
    writer: UnixStream,
    #[cfg(test)]
    fail_write: std::sync::atomic::AtomicI32,
    #[cfg(test)]
    fail_write_count: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    fail_read: std::sync::atomic::AtomicI32,
}

impl Wake {
    pub(super) fn new() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        Ok(Self {
            reader,
            writer,
            #[cfg(test)]
            fail_write: std::sync::atomic::AtomicI32::new(0),
            #[cfg(test)]
            fail_write_count: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            fail_read: std::sync::atomic::AtomicI32::new(0),
        })
    }

    pub(super) fn fd(&self) -> BorrowedFd<'_> {
        self.reader.as_fd()
    }

    pub(super) fn notify(&self) -> io::Result<()> {
        let mut attempts = INTERRUPTED_ATTEMPTS;
        loop {
            let error = match self.notify_once() {
                Ok(()) => return Ok(()),
                Err(error) => error,
            };
            if error.kind() == io::ErrorKind::WouldBlock {
                // A full writer buffer already leaves bytes readable at the peer.
                return Ok(());
            }
            attempts -= 1;
            if error.kind() != io::ErrorKind::Interrupted || attempts == 0 {
                // Publish durable EOF before the mailbox latches its failure.
                // The read fd remains owned/live even across the final-check/poll
                // gap. Linux half-close yields EOF readability, not necessarily
                // POLLHUP. Valid connected endpoints make shutdown independent
                // of buffer space; an unexpected shutdown failure is explicit.
                if let Err(shutdown_error) = self.writer.shutdown(Shutdown::Write) {
                    return Err(io::Error::other(format!(
                        "{error}; wake shutdown failed: {shutdown_error}"
                    )));
                }
                return Err(error);
            }
        }
    }

    fn notify_once(&self) -> io::Result<()> {
        #[cfg(test)]
        if let Some(error) = self.injected_write() {
            return Err(error);
        }
        // UnixStream's Linux Write implementation sends with MSG_NOSIGNAL, so
        // peer shutdown cannot signal the process. A byte carries no command data.
        match (&self.writer).write(&[1])? {
            1 => Ok(()),
            _ => Err(io::Error::new(io::ErrorKind::WriteZero, "empty wake write")),
        }
    }

    pub(super) fn drain(&self) -> io::Result<()> {
        let mut attempts = INTERRUPTED_ATTEMPTS;
        loop {
            let error = match self.drain_once() {
                Ok(()) => return Ok(()),
                Err(error) => error,
            };
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            attempts -= 1;
            if error.kind() != io::ErrorKind::Interrupted || attempts == 0 {
                return Err(error);
            }
        }
    }

    fn drain_once(&self) -> io::Result<()> {
        #[cfg(test)]
        if let Some(error) = Self::injected(&self.fail_read) {
            return Err(error);
        }
        // One finite chunk cannot starve under active notification producers.
        // Remaining bytes stay readable and cause a subsequent wake turn.
        match (&self.reader).read(&mut [0; 4096])? {
            0 => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "wake writer closed",
            )),
            _ => Ok(()),
        }
    }

    #[cfg(test)]
    fn injected_write(&self) -> Option<io::Error> {
        use std::sync::atomic::Ordering::Relaxed;
        let remaining = self
            .fail_write_count
            .fetch_update(Relaxed, Relaxed, |count| count.checked_sub(1));
        remaining
            .ok()
            .map(|_| io::Error::from_raw_os_error(self.fail_write.load(Relaxed)))
    }

    #[cfg(test)]
    fn injected(slot: &std::sync::atomic::AtomicI32) -> Option<io::Error> {
        let errno = slot.swap(0, std::sync::atomic::Ordering::Relaxed);
        (errno != 0).then(|| io::Error::from_raw_os_error(errno))
    }

    #[cfg(test)]
    pub(super) fn fail_next_write(&self, errno: i32) {
        self.fail_writes(errno, 1);
    }

    #[cfg(test)]
    pub(super) fn fail_writes(&self, errno: i32, count: usize) {
        self.fail_write
            .store(errno, std::sync::atomic::Ordering::Relaxed);
        self.fail_write_count
            .store(count, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn remaining_write_failures(&self) -> usize {
        self.fail_write_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(super) fn try_notify_once(&self) -> io::Result<()> {
        self.notify_once()
    }

    #[cfg(test)]
    pub(super) fn fail_next_read(&self, errno: i32) {
        self.fail_read
            .store(errno, std::sync::atomic::Ordering::Relaxed);
    }
}
