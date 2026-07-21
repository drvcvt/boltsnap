use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

pub use crate::protocol::{
    RecordingSnapshot, Replacement, Request, Response, read_frame, write_frame,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn socket_path() -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\boltsnap-{}", pipe_user_key()))
}

pub fn daemon_alive() -> bool {
    connect_pipe()
        .and_then(|mut pipe| pipe.write_all(&Request::Ping.encode()))
        .is_ok()
}

pub fn send_to_shelf(request: Request) -> io::Result<()> {
    let mut pipe = ensure_daemon()?;
    pipe.write_all(&request.encode())?;
    pipe.flush()
}

pub fn call_daemon(request: Request) -> io::Result<Response> {
    let mut pipe = ensure_daemon()?;
    pipe.write_all(&request.encode())?;
    pipe.flush()?;
    Response::read(&mut pipe)
}

pub fn watch_recording() -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "live recording event streaming is not connected on Windows yet",
    ))
}

fn connect_pipe() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(socket_path())
}

fn ensure_daemon() -> io::Result<File> {
    if let Ok(pipe) = connect_pipe() {
        return Ok(pipe);
    }

    let executable = std::env::current_exe()?;
    Command::new(executable)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;

    for _ in 0..100 {
        if let Ok(pipe) = connect_pipe() {
            return Ok(pipe);
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "Boltsnap daemon did not open its Windows named pipe",
    ))
}

fn pipe_user_key() -> String {
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    sanitize_pipe_component(&format!("{domain}-{user}"))
}

fn sanitize_pipe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_local_and_user_scoped() {
        let path = socket_path();
        let text = path.to_string_lossy();
        assert!(text.starts_with(r"\\.\pipe\boltsnap-"));
        assert!(text.len() > r"\\.\pipe\boltsnap-".len());
    }

    #[test]
    fn pipe_component_contains_only_safe_ascii() {
        assert_eq!(sanitize_pipe_component("DOMAIN\\A User"), "domain_a_user");
    }
}
