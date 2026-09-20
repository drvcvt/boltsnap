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
    let mut child = spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()), None)
        .map_err(|e| format!("start media capability probe: {e}"))?;
    let read = |pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || -> Result<Vec<u8>, String> {
            let mut bytes = Vec::new();
            pipe.take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > limit {
                return Err("media process output exceeds its limit".into());
            }
            Ok(bytes)
        })
    };
    let stdout = read(Box::new(child.stdout.take().ok_or("missing probe output")?));
    let stderr = read(Box::new(child.stderr.take().ok_or("missing probe errors")?));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20))
            }
            result => {
                terminate(&mut child);
                break Err(match result {
                    Err(e) => e.to_string(),
                    _ => "media capability probe timed out".into(),
                });
            }
        }
    };
    let stdout = stdout.join().map_err(|_| "probe reader panicked")?;
    let stderr = stderr.join().map_err(|_| "probe reader panicked")?;
    Ok(Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    })
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
}
