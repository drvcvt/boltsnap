//! One Boltsnap window per session. The first process holds a lock and
//! listens beside the daemon's socket; a later `boltsnap settings` asks it to
//! come to the front (with a page) and exits.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use super::super::ipc;

fn path(extension: &str) -> PathBuf {
    ipc::socket_path().with_file_name(format!("boltsnap-app.{extension}"))
}

pub(super) enum Instance {
    /// This process owns the window. Keep the lock for its lifetime.
    First { _lock: File, listener: UnixListener },
    /// Another process owns it and was asked to come forward.
    Raised,
}

pub(super) fn claim(page: Option<&str>) -> crate::DynResult<Instance> {
    let Some(lock) = ipc::lock_file(&path("lock"))? else {
        raise(page)?;
        return Ok(Instance::Raised);
    };
    let socket = path("sock");
    // The lock says no other window lives, so a socket left here is stale.
    ipc::remove_stale_socket(&socket)?;
    let listener = UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    Ok(Instance::First {
        _lock: lock,
        listener,
    })
}

fn raise(page: Option<&str>) -> crate::DynResult<()> {
    let mut stream = UnixStream::connect(path("sock"))
        .map_err(|error| format!("Boltsnap is already open but does not answer: {error}"))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    writeln!(stream, "raise {}", page.unwrap_or(""))?;
    Ok(())
}

/// The page a `raise` asks for: `Some("")` for none in particular, `None`
/// for anything that is not a raise.
pub(super) fn read_raise(stream: UnixStream) -> Option<String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    let mut line = String::new();
    // At most one short line.
    BufReader::new(stream.take(256)).read_line(&mut line).ok()?;
    line.trim_end()
        .strip_prefix("raise")
        .map(|page| page.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raise_names_its_page_and_anything_else_is_ignored() {
        for (sent, read) in [
            ("raise text\n", Some("text")),
            ("raise \n", Some("")),
            ("open text\n", None),
        ] {
            let (mut tx, rx) = UnixStream::pair().unwrap();
            tx.write_all(sent.as_bytes()).unwrap();
            drop(tx);
            assert_eq!(read_raise(rx).as_deref(), read, "{sent:?}");
        }
    }
}
