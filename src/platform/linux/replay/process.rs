use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

pub fn spawn(command: &mut Command, priority: Option<i32>) -> io::Result<Child> {
    let parent = std::process::id();
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() as u32 != parent {
                return Err(io::Error::other("replay owner exited"));
            }
            if let Some(priority) = priority {
                libc::nice(priority);
            }
            Ok(())
        });
    }
    command.spawn()
}

pub fn terminate(child: &mut Child) {
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    // An unreaped child keeps its PID reserved.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn output(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    output_bounded(command.stdin(Stdio::null()), timeout, 64 * 1024)
}

pub fn output_bounded(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> Result<Output, String> {
    output_with_interval(command, timeout, limit, Duration::from_millis(20))
}

/// Short startup helpers should not inherit the media probe's 20 ms polling floor.
pub fn output_setup(command: &mut Command) -> Result<Output, String> {
    output_with_interval(
        command.stdin(Stdio::null()),
        Duration::from_millis(500),
        64 * 1024,
        Duration::from_millis(1),
    )
}

/// Runs `command` to completion within `timeout`, reading stdout and stderr
/// as they come. The deadline covers the pipes too: a descendant that keeps
/// them open after the process exited cannot hold the caller past it.
fn output_with_interval(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
    interval: Duration,
) -> Result<Output, String> {
    use std::os::fd::{AsRawFd, OwnedFd};
    let mut child = spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()), None)
        .map_err(|e| format!("start media capability probe: {e}"))?;
    let deadline = Instant::now() + timeout;
    let mut pipes = [
        child.stdout.take().map(OwnedFd::from),
        child.stderr.take().map(OwnedFd::from),
    ]
    .map(|pipe| pipe.map(std::fs::File::from));
    let fail = |child: &mut Child, error: String| {
        terminate(child);
        Err(error)
    };
    for pipe in pipes.iter().flatten() {
        if unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } != 0 {
            return fail(&mut child, io::Error::last_os_error().to_string());
        }
    }
    let mut output = [Vec::new(), Vec::new()];
    let mut status = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(exited) => status = exited,
                Err(e) => return fail(&mut child, e.to_string()),
            }
        }
        if let Some(status) = status
            && pipes.iter().all(Option::is_none)
        {
            let [stdout, stderr] = output;
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        let now = Instant::now();
        if now >= deadline {
            return fail(&mut child, "media capability probe timed out".into());
        }
        // Wakes on output or closed pipes; the interval bounds the exit poll.
        let mut fds: Vec<libc::pollfd> = pipes
            .iter()
            .flatten()
            .map(|pipe| libc::pollfd {
                fd: pipe.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        let wait = interval.min(deadline - now).as_millis().max(1) as libc::c_int;
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait) };
        for (pipe, bytes) in pipes.iter_mut().zip(&mut output) {
            let Some(file) = pipe.as_mut() else {
                continue;
            };
            let mut chunk = [0; 8192];
            loop {
                match file.read(&mut chunk) {
                    Ok(0) => {
                        *pipe = None;
                        break;
                    }
                    Ok(read) if bytes.len() + read > limit => {
                        return fail(&mut child, "media process output exceeds its limit".into());
                    }
                    Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return fail(&mut child, e.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_probe_timeout_reaps_its_child() {
        let started = Instant::now();
        let error = output(Command::new("sleep").arg("5"), Duration::from_millis(40)).unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_descendant_holding_the_pipes_cannot_outlast_the_deadline() {
        // The shell exits at once; its background sleep keeps stdout and/or
        // stderr open for 1.2 s.
        for script in [
            "sleep 1.2 & exit 0",
            "sleep 1.2 >/dev/null & exit 0",
            "sleep 1.2 2>/dev/null & exit 0",
        ] {
            let started = Instant::now();
            let error = output(
                Command::new("sh").args(["-c", script]),
                Duration::from_millis(40),
            )
            .unwrap_err();
            assert!(error.contains("timed out"), "{script}: {error}");
            assert!(
                started.elapsed() < Duration::from_millis(600),
                "{script}: {:?}",
                started.elapsed()
            );
        }
    }

    #[test]
    fn output_is_read_whole_from_both_pipes() {
        let result = output(
            Command::new("sh").args(["-c", "printf 'a\\0b'; printf err >&2; exit 3"]),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(result.stdout, b"a\0b");
        assert_eq!(result.stderr, b"err");
        assert_eq!(result.status.code(), Some(3));

        let empty = output(&mut Command::new("true"), Duration::from_secs(5)).unwrap();
        assert!(empty.status.success() && empty.stdout.is_empty() && empty.stderr.is_empty());

        // More than a pipe buffer on each stream at once.
        let big = output_bounded(
            Command::new("sh").args([
                "-c",
                "head -c 200000 /dev/zero >&2; head -c 200000 /dev/zero",
            ]),
            Duration::from_secs(5),
            300_000,
        )
        .unwrap();
        assert_eq!((big.stdout.len(), big.stderr.len()), (200_000, 200_000));

        let error = output_bounded(
            Command::new("sh").args(["-c", "head -c 5000 /dev/zero; sleep 5"]),
            Duration::from_secs(5),
            4096,
        )
        .unwrap_err();
        assert!(error.contains("exceeds its limit"));
    }
}
