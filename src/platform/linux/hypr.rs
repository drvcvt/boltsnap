//! Hyprland queries over its request socket: about 0.15 ms per request instead
//! of about 4 ms for spawning `hyprctl`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// JSON reply of `hyprctl -j <command>`, or `None` off Hyprland. Falls back to
/// `hyprctl` when the socket is unavailable (older socket locations).
pub fn json(command: &str) -> Option<Vec<u8>> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    request(&format!("j/{command}"))
        .filter(|reply| !reply.is_empty())
        .or_else(|| {
            let output =
                super::replay::process::output_setup(Command::new("hyprctl").args(["-j", command]))
                    .ok()?;
            output.status.success().then_some(output.stdout)
        })
}

fn request(command: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?)
        .join("hypr")
        .join(std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?)
        .join(".socket.sock");
    let mut stream = UnixStream::connect(path).ok()?;
    let timeout = Some(Duration::from_millis(500));
    stream.set_read_timeout(timeout).ok()?;
    stream.set_write_timeout(timeout).ok()?;
    stream.write_all(command.as_bytes()).ok()?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).ok()?;
    Some(reply)
}
