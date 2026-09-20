use std::io;
use std::os::fd::AsRawFd;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::time::Duration;

pub fn address(name: &str) -> io::Result<SocketAddr> {
    // Abstract sockets disappear when their last owner closes them.
    SocketAddr::from_abstract_name(format!("boltsnap-replay-{}-{name}", unsafe {
        libc::geteuid()
    }))
}

pub fn bind(name: &str) -> io::Result<UnixListener> {
    UnixListener::bind_addr(&address(name)?)
}

pub fn configure(stream: &UnixStream) -> io::Result<()> {
    let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SO_PEERCRED supplies the connected process's kernel credentials.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>()
        || unsafe { credentials.assume_init().uid != libc::geteuid() }
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "replay peer has another user ID",
        ));
    }
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))
}
