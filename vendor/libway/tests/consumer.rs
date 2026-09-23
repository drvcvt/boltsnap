//! Opt-in integration of the actual Boltsnap executable with a synthetic desktop.
#![cfg(feature = "image")]
mod support;
use std::{
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use support::*;

#[test]
#[ignore = "requires LIBWAY_TEST_BOLTSNAP pointing to a separately built binary"]
fn boltsnap_stdout_png_uses_only_the_supplied_test_socket() {
    let binary = std::env::var_os("LIBWAY_TEST_BOLTSNAP").expect("set the test binary path");
    for (index, (ext, fault)) in [
        (true, Fault::None),
        (false, Fault::None),
        (true, Fault::NoShm),
    ]
    .into_iter()
    .enumerate()
    {
        let (_server, socket) = Server::start(Config {
            ext,
            fault,
            outputs: 2,
            mode_sizes: vec![(4, 2), (6, 3)],
            ..Default::default()
        });
        let fd = socket.as_raw_fd();
        let mut command = Command::new(&binary);
        command
            .args(["full", "--backend", "wayland", "--no-copy", "-o", "-"])
            .env_clear()
            .env("WAYLAND_SOCKET", fd.to_string())
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent-libway-test-bus",
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: only async-signal-safe fcntl calls run after fork. The FD belongs
        // to this test; change CLOEXEC only in the child, not the parallel test host.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(socket);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut timed_out = false;
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                timed_out = true;
                child.kill().unwrap();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let result = child.wait_with_output().unwrap();
        assert!(
            !timed_out,
            "test binary stalled: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let png = image::load_from_memory_with_format(&result.stdout, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(png.dimensions(), (12, 3));
        assert_eq!(png.get_pixel(6, 0).0, [160, 176, 192, 255]);
        assert!(png.get_pixel(0, 0)[0] < 32);
        if index == 0
            && let Some(path) = std::env::var_os("LIBWAY_TEST_PNG")
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap();
            file.write_all(&result.stdout).unwrap();
        }
    }
}
