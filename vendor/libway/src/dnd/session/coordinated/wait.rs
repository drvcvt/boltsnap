//! Borrowed readiness interests protected by exclusive participant ownership.
use super::CoordinatedDnd;
use crate::Result;
use std::{
    os::fd::{AsRawFd, BorrowedFd},
    time::Instant,
};

/// A live descriptor and its requested readiness directions. Error/hangup
/// readiness must also be handled by the coordinator. The library duplicates no
/// descriptor; a consumer-created duplicate has its own lifetime and EOF effects.
#[derive(Debug, Clone, Copy)]
pub struct WaitInterest<'a> {
    /// Borrowed only while this wait view is retained.
    pub fd: BorrowedFd<'a>,
    /// Monitor input readiness.
    pub readable: bool,
    /// Monitor output readiness.
    pub writable: bool,
}

/// A stable DnD wait snapshot with no session mutex held.
///
/// Connection interest comes first, followed by incoming and outgoing pipes.
/// This is a participant view, not a prepared read or a complete connection wait.
/// The coordinator adds other queues and wake descriptors, resolves its prepared
/// read, releases this view and only then services callbacks or mutations.
///
/// Mutation is forbidden while the view remains in use:
/// ```compile_fail
/// use libway::dnd::CoordinatedDnd;
/// fn mutation(owner: &mut CoordinatedDnd) -> libway::Result<()> {
///     let view = owner.wait_view()?;
///     owner.events();
///     let _ = view.interests().count();
///     Ok(())
/// }
/// ```
/// Dropping the owner is forbidden while its view remains in use:
/// ```compile_fail
/// use libway::dnd::CoordinatedDnd;
/// fn destruction(mut owner: CoordinatedDnd) -> libway::Result<()> {
///     let view = owner.wait_view()?;
///     drop(owner);
///     let _ = view.immediate();
///     Ok(())
/// }
/// ```
/// Copied descriptor borrows also keep the view live:
/// ```compile_fail
/// use libway::dnd::CoordinatedDnd;
/// use std::os::fd::AsRawFd;
/// fn copied(owner: &mut CoordinatedDnd) -> libway::Result<()> {
///     let view = owner.wait_view()?;
///     if let Some(interest) = view.interests().next() {
///         drop(view);
///         let _ = interest.fd.as_raw_fd();
///     }
///     Ok(())
/// }
/// ```
/// Releasing the view allows subsequent service:
/// ```no_run
/// use libway::dnd::CoordinatedDnd;
/// fn next_turn(owner: &mut CoordinatedDnd) -> libway::Result<()> {
///     let view = owner.wait_view()?;
///     let _ = view.interests().count();
///     drop(view);
///     owner.dispatch()?;
///     Ok(())
/// }
/// ```
pub struct WaitView<'a> {
    // Retains the connection and every transfer fd, and blocks all owner mutation.
    _owner: &'a mut CoordinatedDnd,
    fds: Vec<libc::pollfd>,
    deadline: Option<Instant>,
    immediate: bool,
}

impl CoordinatedDnd {
    /// Snapshot live interests without holding a mutex across the wait. Retained
    /// events or yielded transfer work mark the view immediate; a terminal cause
    /// prevents construction. This call neither flushes nor prepares a read.
    pub fn wait_view(&mut self) -> Result<WaitView<'_>> {
        self.ensure_active()?;
        let snapshot = {
            let core = self.dnd.lock();
            let fd = self
                .dnd
                .display
                .connection()
                .backend()
                .poll_fd()
                .as_raw_fd();
            core.engine
                .reactor
                .poll_set(fd, core.write_pending, &core.engine.limits)
                .map(|(fds, deadline)| {
                    let immediate = !core.engine.events.is_empty() || core.engine.reactor.ready();
                    (fds, deadline, immediate)
                })
        };
        let (fds, deadline, immediate) = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.remember(&error);
                return Err(error);
            }
        };
        Ok(WaitView {
            _owner: self,
            fds,
            deadline,
            immediate,
        })
    }
}

impl WaitView<'_> {
    /// Borrow the stable connection-first interest set for this view's lifetime.
    pub fn interests(&self) -> impl ExactSizeIterator<Item = WaitInterest<'_>> + '_ {
        self.fds.iter().map(|entry| WaitInterest {
            // SAFETY: Every snapshot fd is owned by the private session's Reactor
            // or its retained Display connection. The fresh session exposes no
            // aliases, readers or lazy providers; only owner methods can mutate
            // Reactor. `_owner` exclusively borrows that owner, preventing requests,
            // dispatch and destruction until this view is released. This method
            // borrows the fd only for &self, so copied interests also prevent view
            // destruction. Peer closure cannot close these locally owned fds.
            fd: unsafe { BorrowedFd::borrow_raw(entry.fd) },
            readable: entry.events & libc::POLLIN != 0,
            writable: entry.events & libc::POLLOUT != 0,
        })
    }

    /// Earliest transfer deadline, including a yielded slice's immediate deadline.
    /// None means this participant needs no timer.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Whether consumer events or a yielded transfer slice require another turn.
    /// False does not establish that other participants or descriptors are idle.
    pub fn immediate(&self) -> bool {
        self.immediate
    }
}
