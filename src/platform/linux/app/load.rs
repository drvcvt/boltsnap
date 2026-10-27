//! What the running replay buffer costs, measured (settings plan, 5i): the
//! CPU of GPU Screen Recorder with its helpers and of the worker, and their
//! memory, read from `/proc`. No estimate: one measured point here is not
//! enough to say what other settings would cost.

use std::path::Path;

use boltsnap::app::ReplayLoad;

/// The replay processes of the daemon with `daemon` as pid: its children
/// named like GPU Screen Recorder and the worker, and their children.
fn replay_pids(proc: &Path, daemon: u32) -> Vec<u32> {
    let parents = processes(proc);
    let mut pids: Vec<u32> = parents
        .iter()
        .filter(|(_, parent, comm)| {
            *parent == daemon
                && (comm.starts_with("gpu-screen-reco") || comm.starts_with("boltsnap-replay"))
        })
        .map(|(pid, ..)| *pid)
        .collect();
    let mut index = 0;
    while index < pids.len() {
        let parent = pids[index];
        pids.extend(
            parents
                .iter()
                .filter(|(_, p, _)| *p == parent)
                .map(|(pid, ..)| *pid),
        );
        index += 1;
    }
    pids
}

/// Every process as (pid, parent, comm).
fn processes(proc: &Path) -> Vec<(u32, u32, String)> {
    let Ok(entries) = std::fs::read_dir(proc) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let (comm, fields) = split_stat(&stat)?;
            Some((pid, fields.first()?.parse().ok()?, comm.to_owned()))
        })
        .collect()
}

/// `/proc/<pid>/stat` as its name and the fields after it (state first):
/// the name is in parentheses and may hold spaces.
fn split_stat(stat: &str) -> Option<(&str, Vec<&str>)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = &stat[open + 1..close];
    Some((comm, stat[close + 1..].split_whitespace().skip(1).collect()))
}

/// CPU time in clock ticks (`utime + stime`) of `pid`.
fn ticks(proc: &Path, pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(proc.join(pid.to_string()).join("stat")).ok()?;
    let (_, fields) = split_stat(&stat)?;
    // After the state: ppid (stat field 4) is index 0, so utime and stime
    // (fields 14 and 15) are 10 and 11.
    Some(fields.get(10)?.parse::<u64>().ok()? + fields.get(11)?.parse::<u64>().ok()?)
}

/// Resident memory of `pid` in bytes.
fn resident(proc: &Path, pid: u32, page: u64) -> Option<u64> {
    let statm = std::fs::read_to_string(proc.join(pid.to_string()).join("statm")).ok()?;
    Some(statm.split_whitespace().nth(1)?.parse::<u64>().ok()? * page)
}

/// Measures the replay processes between two calls.
pub(super) struct Meter {
    last: Option<(std::time::Instant, u64)>,
    tick: f64,
    page: u64,
}

impl Meter {
    pub(super) fn new() -> Self {
        let tick = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            last: None,
            tick: if tick > 0 { tick as f64 } else { 100.0 },
            page: if page > 0 { page as u64 } else { 4096 },
        }
    }

    /// The load of the session `session` (`worker-<daemon pid>-…`) since the
    /// call before; the first call has no CPU share yet.
    pub(super) fn sample(&mut self, session: &str) -> Option<ReplayLoad> {
        let proc = Path::new("/proc");
        let daemon: u32 = session
            .strip_prefix("worker-")?
            .split('-')
            .next()?
            .parse()
            .ok()?;
        let pids = replay_pids(proc, daemon);
        if pids.is_empty() {
            self.last = None;
            return None;
        }
        let now = std::time::Instant::now();
        let ticks: u64 = pids.iter().filter_map(|pid| ticks(proc, *pid)).sum();
        let memory: u64 = pids
            .iter()
            .filter_map(|pid| resident(proc, *pid, self.page))
            .sum();
        let cpu = self.last.and_then(|(at, before)| {
            let seconds = now.duration_since(at).as_secs_f64();
            (seconds > 0.0 && ticks >= before)
                .then(|| ((ticks - before) as f64 / self.tick / seconds * 100.0) as f32)
        });
        self.last = Some((now, ticks));
        Some(ReplayLoad {
            cpu_percent: cpu,
            memory_bytes: memory,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stat_line_splits_around_a_name_with_spaces_and_parentheses() {
        let line = "42 (gsr (kms) x) S 7 42 42 0 -1 0 0 0 0 0 150 25 0 0 20 0 1 0";
        let (comm, fields) = split_stat(line).unwrap();
        assert_eq!(comm, "gsr (kms) x");
        assert_eq!(fields[0], "7", "the parent");
        assert_eq!((fields[10], fields[11]), ("150", "25"), "utime, stime");
    }

    #[test]
    fn the_replay_processes_are_the_daemons_recorder_and_worker_and_their_children() {
        let proc = crate::paths::temp_file("load-proc", "d");
        let stat = |pid: u32, comm: &str, parent: u32| {
            let dir = proc.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("stat"),
                format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 7 3 0 0 0"),
            )
            .unwrap();
            std::fs::write(dir.join("statm"), "100 25 0 0 0 0 0").unwrap();
        };
        stat(100, "boltsnap", 1);
        stat(101, "gpu-screen-reco", 100);
        stat(102, "gsr-kms-server", 101);
        stat(103, "boltsnap-replay", 100);
        stat(104, "eddy", 100);
        stat(105, "gpu-screen-reco", 999);
        let mut pids = replay_pids(&proc, 100);
        pids.sort();
        assert_eq!(pids, [101, 102, 103]);
        assert_eq!(ticks(&proc, 101), Some(10));
        assert_eq!(resident(&proc, 101, 4096), Some(25 * 4096));
        for pid in 100..=105 {
            let dir = proc.join(pid.to_string());
            std::fs::remove_file(dir.join("stat")).unwrap();
            std::fs::remove_file(dir.join("statm")).unwrap();
            std::fs::remove_dir(dir).unwrap();
        }
        std::fs::remove_dir(&proc).unwrap();
    }

    /// `cargo test --bin boltsnap replay_load_now -- --ignored --nocapture`:
    /// the running buffer's load, read from `/proc` (nothing is started).
    #[test]
    #[ignore]
    fn replay_load_now() {
        let status = crate::platform::linux::replay::status_quiet().expect("a running buffer");
        let session = status["session"].as_str().expect("a session").to_owned();
        let mut meter = Meter::new();
        meter.sample(&session);
        std::thread::sleep(std::time::Duration::from_secs(2));
        println!("{:?}", meter.sample(&session));
    }
}
