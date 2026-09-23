//! Reader thread for displays whose socket is also read by another loop (winit, GTK).
//! Mirrors winit's own `pump_events` thread: prepare_read → poll → read/cancel → dispatch.
use super::{Core, DndEvent, read_events};
use crate::{Error, Result};
use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};
use wayland_client::Connection as WlConnection;

/// Stops and joins the thread when dropped. Drop it before the `Dnd` and the display owner.
pub struct Reader {
    control: Arc<Control>,
    thread: Option<thread::JoinHandle<()>>,
    core: Arc<Mutex<Core>>,
}
/// Wakes the reader out of `poll`: to stop, or to poll a changed fd set. An eventfd never
/// fills up and has no reader end to lose, so a wake can neither block nor raise SIGPIPE.
pub(crate) struct Control {
    fd: OwnedFd,
    stop: AtomicBool,
}
impl Control {
    pub(crate) fn wake(&self) {
        unsafe { libc::eventfd_write(self.fd.as_raw_fd(), 1) };
    }
}
impl Reader {
    /// False once the session failed; the waker has run and `events()` holds the reason.
    pub fn is_alive(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Release);
        self.control.wake();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let mut c = self.core.lock().unwrap_or_else(|p| p.into_inner());
        (c.kick, c.waker) = (None, None);
    }
}
pub(crate) fn spawn(
    conn: WlConnection,
    core: Arc<Mutex<Core>>,
    waker: impl Fn() + Send + 'static,
) -> Result<Reader> {
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let control = Arc::new(Control {
        fd: unsafe { OwnedFd::from_raw_fd(fd) },
        stop: AtomicBool::new(false),
    });
    {
        // Installed before the thread starts, so its first batch (often `Ready`) wakes too.
        let mut c = core.lock().unwrap_or_else(|p| p.into_inner());
        if c.kick.is_some() {
            return Err(Error::Unsupported("a second reader for one DnD session"));
        }
        c.kick = Some(control.clone());
        c.waker = Some(Box::new(waker));
    }
    let (shared, own) = (core.clone(), control.clone());
    let spawned = thread::Builder::new()
        .name("libway-dnd".into())
        .spawn(move || {
            if let Err(e) = run(&conn, &shared, &own) {
                let mut c = shared.lock().unwrap_or_else(|p| p.into_inner());
                c.engine.events.push_back(DndEvent::Failed(e.to_string()));
                c.engine.wake_armed = false;
                c.wake();
            }
        });
    match spawned {
        Ok(thread) => Ok(Reader {
            control,
            thread: Some(thread),
            core,
        }),
        Err(error) => {
            let mut c = core.lock().unwrap_or_else(|p| p.into_inner());
            (c.kick, c.waker) = (None, None);
            Err(Error::Io(error))
        }
    }
}
/// Dispatch and service transfers; wake the consumer once per `events()` drain.
fn service(core: &Mutex<Core>) -> Result<usize> {
    let mut c = core.lock().unwrap_or_else(|p| p.into_inner());
    let dispatched = c.service(Instant::now());
    c.wake();
    dispatched
}
fn run(conn: &WlConnection, core: &Mutex<Core>, control: &Control) -> Result<()> {
    let display = conn.backend().poll_fd().as_raw_fd();
    loop {
        service(core)?;
        // `None`: another reader already queued events for us (libwayland).
        let Some(guard) = conn.prepare_read() else {
            continue;
        };
        // The Rust backend does not check our queue in prepare_read; drain what another
        // reader queued before we announced ours, or we would sleep on it.
        if service(core)? > 0 {
            continue;
        }
        let (mut fds, deadline) = {
            let mut c = core.lock().unwrap_or_else(|p| p.into_inner());
            c.flush()?;
            c.engine
                .reactor
                .poll_set(display, c.write_pending, &c.engine.limits)?
        };
        fds.push(libc::pollfd {
            fd: control.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        let timeout = deadline.map_or(-1, super::poll_timeout);
        let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        let err = std::io::Error::last_os_error();
        // Resolve the prepared read first so the owner's read never waits on us.
        if fds.iter().any(|fd| fd.revents & libc::POLLNVAL != 0) {
            return Err(std::io::Error::from_raw_os_error(libc::EBADF).into());
        }
        if ret > 0 && fds[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            read_events(guard)?;
        } else {
            drop(guard);
        }
        if ret < 0 && err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err.into());
        }
        if fds.last().is_some_and(|f| f.revents != 0) {
            let mut count = 0;
            unsafe { libc::eventfd_read(control.fd.as_raw_fd(), &mut count) };
        }
        if control.stop.load(Ordering::Acquire) {
            return Ok(());
        }
    }
}
