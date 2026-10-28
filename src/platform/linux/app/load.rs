//! What the running replay buffer costs, measured (settings plan, 5i): the
//! CPU of GPU Screen Recorder with its helpers and of the worker, and their
//! private memory, read from `/proc`. No estimate: one measured point here is
//! not enough to say what other settings would cost.

use std::path::Path;

use boltsnap::app::ReplayLoad;

/// The replay processes of the daemon with `daemon` as pid: its replay
/// worker and the GPU Screen Recorder that feeds it, and their children. A
/// recording's GPU Screen Recorder is the daemon's child too; the replay's is
/// the one writing to its stdout (`linux/replay` start, `-o /dev/stdout`).
fn replay_pids(proc: &Path, daemon: u32) -> Vec<u32> {
    let read = |pid: u32, file: &str| std::fs::read(proc.join(pid.to_string()).join(file));
    let feeds_worker = |pid: u32| {
        read(pid, "cmdline").is_ok_and(|args| args.windows(15).any(|w| w == b"-o\0/dev/stdout\0"))
    };
    let mut pids: Vec<u32> = children(proc, daemon)
        .into_iter()
        .filter(|&pid| {
            let Ok(stat) = read(pid, "stat") else {
                return false;
            };
            let stat = String::from_utf8_lossy(&stat);
            split_stat(&stat).is_some_and(|(comm, _)| {
                comm.starts_with("boltsnap-replay")
                    || comm.starts_with("gpu-screen-reco") && feeds_worker(pid)
            })
        })
        .collect();
    let mut index = 0;
    while index < pids.len() {
        pids.extend(children(proc, pids[index]));
        index += 1;
    }
    pids
}

/// The children of `pid`, from each of its threads' `children` list (a
/// thread lists what it started; when it ends they go to another thread of
/// the process). Not a scan of every process in `/proc`: that cost the
/// settings window 15 ms a second (replay memory plan, part 4).
fn children(proc: &Path, pid: u32) -> Vec<u32> {
    let Ok(tasks) = std::fs::read_dir(proc.join(pid.to_string()).join("task")) else {
        return Vec::new();
    };
    tasks
        .flatten()
        .filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok())
        .flat_map(|list| {
            list.split_whitespace()
                .filter_map(|child| child.parse().ok())
                .collect::<Vec<u32>>()
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

/// Private memory of `pid` in bytes: what goes when it ends (replay memory
/// plan, part 1). Shared libraries other processes map too do not count. A
/// process whose `smaps_rollup` is closed to us (`gsr-kms-server` has
/// capabilities) counts its anonymous pages from `statm`.
fn private(proc: &Path, pid: u32, page: u64) -> Option<u64> {
    let dir = proc.join(pid.to_string());
    if let Ok(rollup) = std::fs::read_to_string(dir.join("smaps_rollup")) {
        return private_of_rollup(&rollup);
    }
    let statm = std::fs::read_to_string(dir.join("statm")).ok()?;
    let mut fields = statm.split_whitespace().skip(1);
    let resident: u64 = fields.next()?.parse().ok()?;
    let shared: u64 = fields.next()?.parse().ok()?;
    Some(resident.saturating_sub(shared) * page)
}

/// `Private_Clean + Private_Dirty` of a `smaps_rollup`, in bytes.
fn private_of_rollup(rollup: &str) -> Option<u64> {
    let kib = |name: &str| -> Option<u64> {
        let line = rollup.lines().find(|line| line.starts_with(name))?;
        line[name.len()..].split_whitespace().next()?.parse().ok()
    };
    Some((kib("Private_Clean:")? + kib("Private_Dirty:")?) * 1024)
}

/// Measures the replay processes between two calls.
pub(super) struct Meter {
    /// The session, when and how many ticks at the last reading.
    last: Option<(String, std::time::Instant, u64)>,
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
            .filter_map(|pid| private(proc, *pid, self.page))
            .sum();
        // Only against a reading of the same session: a restarted buffer
        // starts its count anew.
        let cpu = self
            .last
            .as_ref()
            .filter(|(last, ..)| last == session)
            .and_then(|(_, at, before)| {
                let seconds = now.duration_since(*at).as_secs_f64();
                (seconds > 0.0 && ticks >= *before)
                    .then(|| ((ticks - before) as f64 / self.tick / seconds * 100.0) as f32)
            });
        self.last = Some((session.to_owned(), now, ticks));
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
    fn private_memory_is_the_clean_and_dirty_pages_no_one_else_maps() {
        let rollup = "55d0-7ffd ---p 00000000 00:00 0 [rollup]\n\
            Rss:              250180 kB\n\
            Pss:              169462 kB\n\
            Shared_Clean:      91148 kB\n\
            Private_Clean:     85844 kB\n\
            Private_Dirty:     73188 kB\n\
            Anonymous:         18916 kB\n";
        assert_eq!(private_of_rollup(rollup), Some((85844 + 73188) * 1024));
        assert_eq!(private_of_rollup("Rss: 1 kB\n"), None);
    }

    #[test]
    fn the_replay_processes_are_the_daemons_recorder_and_worker_and_their_children() {
        let proc = crate::paths::temp_file("load-proc", "d");
        let mut made = Vec::new();
        let mut kids = |pid: u32, task: u32, list: &str| {
            let dir = proc
                .join(pid.to_string())
                .join("task")
                .join(task.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("children"), list).unwrap();
            made.push(dir);
        };
        // The daemon started its children from two threads; 105 is not its.
        kids(100, 100, "104 106 ");
        kids(100, 107, "101 103 ");
        kids(101, 101, "102 ");
        kids(105, 105, "");
        let stat = |pid: u32, comm: &str, parent: u32| {
            let dir = proc.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            let args: &[u8] = if pid == 101 {
                b"gpu-screen-recorder\0-w\0DP-3\0-o\0/dev/stdout\0"
            } else {
                b"gpu-screen-recorder\0-o\0/home/me/clip.mkv\0"
            };
            std::fs::write(dir.join("cmdline"), args).unwrap();
            std::fs::write(
                dir.join("stat"),
                format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 7 3 0 0 0"),
            )
            .unwrap();
            std::fs::write(dir.join("statm"), "100 25 10 0 0 0 0").unwrap();
        };
        stat(100, "boltsnap", 1);
        stat(101, "gpu-screen-reco", 100);
        stat(102, "gsr-kms-server", 101);
        stat(103, "boltsnap-replay", 100);
        stat(104, "eddy", 100);
        stat(105, "gpu-screen-reco", 999);
        // A recording: the daemon's child too, writing to a file.
        stat(106, "gpu-screen-reco", 100);
        let mut pids = replay_pids(&proc, 100);
        pids.sort();
        assert_eq!(pids, [101, 102, 103]);
        assert_eq!(ticks(&proc, 101), Some(10));
        // Without a readable rollup: 25 resident pages, 10 of them shared.
        assert_eq!(private(&proc, 101, 4096), Some(15 * 4096));
        let rollup = proc.join("103").join("smaps_rollup");
        std::fs::write(&rollup, "Private_Clean: 2 kB\nPrivate_Dirty: 3 kB\n").unwrap();
        assert_eq!(private(&proc, 103, 4096), Some(5 * 1024));
        std::fs::remove_file(rollup).unwrap();
        for task in made {
            std::fs::remove_file(task.join("children")).unwrap();
            std::fs::remove_dir(&task).unwrap();
            let tasks = task.parent().unwrap();
            if std::fs::read_dir(tasks).unwrap().next().is_none() {
                std::fs::remove_dir(tasks).unwrap();
            }
        }
        for pid in 100..=106 {
            let dir = proc.join(pid.to_string());
            for file in ["stat", "statm", "cmdline"] {
                std::fs::remove_file(dir.join(file)).unwrap();
            }
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
