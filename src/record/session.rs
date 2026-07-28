use super::{Geometry, Monitor, resolve_record_outputs, wf_recorder_args, wf_recorder_output_args};
use crate::config::{RecordBothMode, RecordingPrefs};
pub use crate::protocol::{PublicRecordingState, RecordingAction};
use crate::record::audio::AudioCapture;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Recording,
    Pausing,
    Paused,
    Finalizing,
    Discarding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureScope {
    Area(Geometry),
    Outputs(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StartPlan {
    pub outputs: Vec<Monitor>,
    pub both_mode: RecordBothMode,
    pub notice: Option<String>,
}

pub fn start_plan(prefs: &RecordingPrefs, monitors: &[Monitor]) -> Result<StartPlan, String> {
    let (outputs, notice) = resolve_record_outputs(&prefs.default_target, monitors)?;
    Ok(StartPlan {
        outputs,
        both_mode: prefs.both_mode,
        notice,
    })
}

#[derive(Debug)]
pub struct ActiveRecorder {
    pub output: Option<String>,
    pub path: PathBuf,
    pub child: Child,
}

#[derive(Clone, Debug)]
pub struct RecorderTools {
    pub wf_recorder: PathBuf,
    pub ffmpeg: PathBuf,
    pub segment_dir: PathBuf,
}

impl Default for RecorderTools {
    fn default() -> Self {
        Self {
            wf_recorder: "wf-recorder".into(),
            ffmpeg: "ffmpeg".into(),
            segment_dir: crate::paths::rec_dir(),
        }
    }
}

pub struct RecordingSession {
    pub phase: SessionPhase,
    pub scope: CaptureScope,
    pub monitors: Vec<Monitor>,
    pub codec: String,
    pub both_mode: RecordBothMode,
    pub show_frame: bool,
    pub audio: Option<AudioCapture>,
    pub completed: BTreeMap<Option<String>, Vec<PathBuf>>,
    pub active: Vec<ActiveRecorder>,
    pub active_elapsed: Duration,
    pub active_started: Option<Instant>,
    pub last_error: Option<String>,
}

impl RecordingSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: CaptureScope,
        monitors: Vec<Monitor>,
        codec: String,
        both_mode: RecordBothMode,
        show_frame: bool,
        audio: Option<AudioCapture>,
        active: Vec<ActiveRecorder>,
        now: Instant,
    ) -> Self {
        Self {
            phase: SessionPhase::Recording,
            scope,
            monitors,
            codec,
            both_mode,
            show_frame,
            audio,
            completed: BTreeMap::new(),
            active,
            active_elapsed: Duration::ZERO,
            active_started: Some(now),
            last_error: None,
        }
    }

    #[cfg(test)]
    fn new_for_test(now: Instant) -> Self {
        Self::new(
            CaptureScope::Area(Geometry {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            }),
            Vec::new(),
            "test".into(),
            RecordBothMode::Separate,
            false,
            None,
            Vec::new(),
            now,
        )
    }

    pub fn elapsed_at(&self, now: Instant) -> Duration {
        self.active_elapsed
            + self
                .active_started
                .map(|started| now.saturating_duration_since(started))
                .unwrap_or_default()
    }

    pub fn begin_pause(&mut self, now: Instant) -> Result<(), String> {
        self.require_phase(SessionPhase::Recording, "pause")?;
        self.freeze_elapsed(now);
        self.phase = SessionPhase::Pausing;
        Ok(())
    }

    pub fn finish_pause(&mut self, completed: Vec<StoppedSegment>) -> Result<(), String> {
        self.require_phase(SessionPhase::Pausing, "finish pause")?;
        self.add_completed(completed);
        self.phase = SessionPhase::Paused;
        Ok(())
    }

    pub fn resume(&mut self, active: Vec<ActiveRecorder>, now: Instant) -> Result<(), String> {
        if let Err(error) = self.require_phase(SessionPhase::Paused, "resume") {
            stop_and_reap(active);
            return Err(error);
        }
        self.active = active;
        self.active_started = Some(now);
        self.last_error = None;
        self.phase = SessionPhase::Recording;
        Ok(())
    }

    pub fn begin_finalize(&mut self, now: Instant) -> Result<(), String> {
        if !matches!(self.phase, SessionPhase::Recording | SessionPhase::Paused) {
            return Err(format!("cannot finalize while {:?}", self.phase));
        }
        self.freeze_elapsed(now);
        self.phase = SessionPhase::Finalizing;
        Ok(())
    }

    pub fn finalize_failed(&mut self, error: String) -> Result<(), String> {
        self.require_phase(SessionPhase::Finalizing, "recover finalization")?;
        self.last_error = Some(error);
        self.phase = SessionPhase::Paused;
        Ok(())
    }

    pub fn begin_discard(&mut self, now: Instant) -> Result<(), String> {
        if !matches!(self.phase, SessionPhase::Recording | SessionPhase::Paused) {
            return Err(format!("cannot discard while {:?}", self.phase));
        }
        self.freeze_elapsed(now);
        self.phase = SessionPhase::Discarding;
        Ok(())
    }

    pub fn can_accept(&self, action: RecordingAction) -> bool {
        matches!(
            (self.phase, action),
            (SessionPhase::Recording, RecordingAction::Pause)
                | (
                    SessionPhase::Recording | SessionPhase::Paused,
                    RecordingAction::SaveShelf
                        | RecordingAction::SaveDisk
                        | RecordingAction::Discard
                )
                | (SessionPhase::Paused, RecordingAction::Resume)
        )
    }

    pub fn public_state(&self) -> PublicRecordingState {
        match self.phase {
            SessionPhase::Recording => PublicRecordingState::Recording,
            SessionPhase::Pausing | SessionPhase::Paused => PublicRecordingState::Paused,
            SessionPhase::Finalizing | SessionPhase::Discarding => PublicRecordingState::Finalizing,
        }
    }

    pub fn actions_enabled(&self) -> bool {
        matches!(self.phase, SessionPhase::Recording | SessionPhase::Paused)
    }

    pub fn add_completed(&mut self, completed: Vec<StoppedSegment>) {
        for segment in completed {
            self.completed
                .entry(segment.output)
                .or_default()
                .push(segment.path);
        }
    }

    fn require_phase(&self, expected: SessionPhase, action: &str) -> Result<(), String> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(format!("cannot {action} while {:?}", self.phase))
        }
    }

    fn freeze_elapsed(&mut self, now: Instant) {
        if let Some(started) = self.active_started.take() {
            self.active_elapsed += now.saturating_duration_since(started);
        }
    }
}

static SEGMENT_ID: AtomicU64 = AtomicU64::new(0);

pub fn spawn_segment(
    scope: &CaptureScope,
    codec: &str,
    audio_source: Option<&str>,
    tools: &RecorderTools,
) -> Result<Vec<ActiveRecorder>, String> {
    spawn_segment_with(scope, codec, audio_source, tools, |program, args| {
        let parent = unsafe { libc::getpid() };
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "boltsnap daemon exited while starting recorder",
                    ));
                }
                Ok(())
            });
        }
        command.spawn()
    })
}

fn spawn_segment_with(
    scope: &CaptureScope,
    codec: &str,
    audio_source: Option<&str>,
    tools: &RecorderTools,
    mut spawn: impl FnMut(&Path, &[String]) -> io::Result<Child>,
) -> Result<Vec<ActiveRecorder>, String> {
    fs::create_dir_all(&tools.segment_dir)
        .map_err(|error| format!("create recording cache: {error}"))?;
    let outputs: Vec<Option<&str>> = match scope {
        CaptureScope::Area(_) => vec![None],
        CaptureScope::Outputs(outputs) if outputs.is_empty() => {
            return Err("cannot record an empty output list".into());
        }
        CaptureScope::Outputs(outputs) => {
            outputs.iter().map(|output| Some(output.as_str())).collect()
        }
    };
    let mut active = Vec::with_capacity(outputs.len());

    for output in outputs {
        let path = segment_path(&tools.segment_dir, output);
        let encoder_device = crate::platform::recording_codec::device_for_codec(codec);
        let args = match (scope, output) {
            (CaptureScope::Area(geometry), None) => wf_recorder_args(
                geometry,
                codec,
                encoder_device.as_deref(),
                audio_source,
                &path,
            ),
            (CaptureScope::Outputs(_), Some(output)) => wf_recorder_output_args(
                output,
                codec,
                encoder_device.as_deref(),
                audio_source,
                &path,
            ),
            _ => unreachable!(),
        };
        match spawn(&tools.wf_recorder, &args) {
            Ok(child) => active.push(ActiveRecorder {
                output: output.map(str::to_owned),
                path,
                child,
            }),
            Err(error) => {
                stop_and_reap(active);
                remove_if_empty(&path);
                return Err(format!("start wf-recorder: {error}"));
            }
        }
    }
    Ok(active)
}

fn segment_path(dir: &Path, output: Option<&str>) -> PathBuf {
    let label = output
        .unwrap_or("area")
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    dir.join(format!(
        "boltsnap-segment-{}-{}-{label}.mp4",
        std::process::id(),
        SEGMENT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn stop_and_reap(active: Vec<ActiveRecorder>) {
    for recorder in &active {
        let _ = send_sigint(&recorder.child);
    }
    std::thread::spawn(move || {
        let _ = StopChildrenJob { children: active }.wait();
    });
}

fn send_sigint(child: &Child) -> io::Result<()> {
    send_signal(child, libc::SIGINT)
}

fn send_signal(child: &Child, signal: libc::c_int) -> io::Result<()> {
    let result = unsafe { libc::kill(child.id() as libc::pid_t, signal) };
    if result == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

fn remove_if_empty(path: &Path) {
    if fs::metadata(path)
        .map(|metadata| metadata.len() == 0)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(path);
    }
}

pub struct StopChildrenJob {
    pub children: Vec<ActiveRecorder>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoppedSegment {
    pub output: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StopChildrenResult {
    Ready(Vec<StoppedSegment>),
    Failed {
        kept: Vec<StoppedSegment>,
        error: String,
    },
}

impl StopChildrenJob {
    pub fn interrupt(&self) -> Result<(), String> {
        let errors = self
            .children
            .iter()
            .filter_map(|recorder| send_sigint(&recorder.child).err())
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    pub fn wait(self) -> StopChildrenResult {
        self.wait_with_timeouts(Duration::from_secs(10), Duration::from_secs(2))
    }

    fn wait_with_timeouts(
        mut self,
        graceful_timeout: Duration,
        terminate_timeout: Duration,
    ) -> StopChildrenResult {
        let mut stopped = Vec::with_capacity(self.children.len());
        let mut errors = Vec::new();
        for recorder in &mut self.children {
            match wait_for_exit(&mut recorder.child, graceful_timeout).and_then(|status| {
                if let Some(status) = status {
                    return Ok(status);
                }
                send_signal(&recorder.child, libc::SIGTERM)?;
                if let Some(status) = wait_for_exit(&mut recorder.child, terminate_timeout)? {
                    return Ok(status);
                }
                recorder.child.kill()?;
                recorder.child.wait()
            }) {
                Ok(status) if status.success() && is_nonempty(&recorder.path) => {
                    stopped.push(StoppedSegment {
                        output: recorder.output.clone(),
                        path: recorder.path.clone(),
                    });
                }
                Ok(status) if !status.success() => {
                    errors.push(format!("wf-recorder exited with {status}"));
                }
                Ok(_) => errors.push(format!(
                    "empty recording segment: {}",
                    recorder.path.display()
                )),
                Err(error) => errors.push(format!("wait for wf-recorder: {error}")),
            }
        }
        if errors.is_empty() {
            StopChildrenResult::Ready(stopped)
        } else {
            for recorder in &self.children {
                remove_if_empty(&recorder.path);
            }
            StopChildrenResult::Failed {
                kept: self
                    .children
                    .iter()
                    .filter(|recorder| is_nonempty(&recorder.path))
                    .map(|recorder| StoppedSegment {
                        output: recorder.output.clone(),
                        path: recorder.path.clone(),
                    })
                    .collect(),
                error: errors.join("; "),
            }
        }
    }
}

fn wait_for_exit(
    child: &mut Child,
    timeout: Duration,
) -> io::Result<Option<std::process::ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn is_nonempty(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boltsnap-session-test-{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_recorder(dir: &Path) -> PathBuf {
        let script = dir.join("fake-recorder");
        fs::write(
            &script,
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-f" ]; then
        out="$2"
        shift 2
    else
        shift
    fi
done
trap 'printf segment >> "$out"; exit 0' INT
: > "$out"
while :; do sleep 1; done
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        script
    }

    fn tools(dir: &Path) -> RecorderTools {
        RecorderTools {
            wf_recorder: fake_recorder(dir),
            ffmpeg: PathBuf::from("ffmpeg"),
            segment_dir: dir.to_path_buf(),
        }
    }

    fn stopped(active: Vec<ActiveRecorder>) -> Vec<StoppedSegment> {
        assert!(wait_for(Duration::from_secs(1), || active
            .iter()
            .all(|recorder| recorder.path.exists())));
        let job = StopChildrenJob { children: active };
        job.interrupt().unwrap();
        match job.wait() {
            StopChildrenResult::Ready(segments) => segments,
            StopChildrenResult::Failed { error, .. } => panic!("stop failed: {error}"),
        }
    }

    fn wait_for(timeout: Duration, condition: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        condition()
    }

    fn process_exists(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    fn segment_id(path: &Path) -> u64 {
        path.file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .rsplit('-')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn elapsed_excludes_paused_wall_time_across_two_pauses() {
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);

        session.begin_pause(t0 + Duration::from_secs(10)).unwrap();
        session.finish_pause(Vec::new()).unwrap();
        assert_eq!(
            session.elapsed_at(t0 + Duration::from_secs(50)),
            Duration::from_secs(10)
        );

        session
            .resume(Vec::new(), t0 + Duration::from_secs(50))
            .unwrap();
        session.begin_pause(t0 + Duration::from_secs(55)).unwrap();
        session.finish_pause(Vec::new()).unwrap();
        assert_eq!(
            session.elapsed_at(t0 + Duration::from_secs(500)),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn paused_elapsed_is_frozen() {
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);
        session.begin_pause(t0 + Duration::from_secs(8)).unwrap();
        session.finish_pause(Vec::new()).unwrap();

        assert_eq!(
            session.elapsed_at(t0 + Duration::from_secs(9)),
            Duration::from_secs(8)
        );
        assert_eq!(
            session.elapsed_at(t0 + Duration::from_secs(90)),
            Duration::from_secs(8)
        );
    }

    #[test]
    fn audio_source_survives_pause() {
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);
        session.audio = Some(crate::record::audio::AudioCapture::for_test(
            "boltsnap_mix_test.monitor",
        ));
        session.begin_pause(t0 + Duration::from_secs(1)).unwrap();
        session.finish_pause(Vec::new()).unwrap();
        assert_eq!(
            session.audio.as_ref().map(|audio| audio.source()),
            Some("boltsnap_mix_test.monitor")
        );
    }

    #[test]
    fn action_acceptance_covers_every_phase_and_action() {
        use RecordingAction::*;
        use SessionPhase::*;

        let cases = [
            (Recording, vec![Pause, SaveShelf, SaveDisk, Discard]),
            (Pausing, vec![]),
            (Paused, vec![Resume, SaveShelf, SaveDisk, Discard]),
            (Finalizing, vec![]),
            (Discarding, vec![]),
        ];
        let actions = [Pause, Resume, SaveShelf, SaveDisk, Discard];

        for (phase, accepted) in cases {
            let mut session = RecordingSession::new_for_test(Instant::now());
            session.phase = phase;
            for action in actions {
                assert_eq!(
                    session.can_accept(action),
                    accepted.contains(&action),
                    "{phase:?} / {action:?}"
                );
            }
        }
    }

    #[test]
    fn legal_transitions_reach_expected_phases() {
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);
        session.begin_pause(t0 + Duration::from_secs(1)).unwrap();
        assert_eq!(session.phase, SessionPhase::Pausing);
        session.finish_pause(Vec::new()).unwrap();
        assert_eq!(session.phase, SessionPhase::Paused);
        session
            .resume(Vec::new(), t0 + Duration::from_secs(2))
            .unwrap();
        assert_eq!(session.phase, SessionPhase::Recording);
        session.begin_finalize(t0 + Duration::from_secs(3)).unwrap();
        assert_eq!(session.phase, SessionPhase::Finalizing);
        session.finalize_failed("disk full".into()).unwrap();
        assert_eq!(session.phase, SessionPhase::Paused);
        assert_eq!(session.last_error.as_deref(), Some("disk full"));
        session.begin_discard(t0 + Duration::from_secs(4)).unwrap();
        assert_eq!(session.phase, SessionPhase::Discarding);
    }

    #[test]
    fn finalize_and_discard_are_legal_from_recording_or_paused() {
        let t0 = Instant::now();
        for paused in [false, true] {
            let mut finalize = RecordingSession::new_for_test(t0);
            let mut discard = RecordingSession::new_for_test(t0);
            if paused {
                finalize.begin_pause(t0).unwrap();
                finalize.finish_pause(Vec::new()).unwrap();
                discard.begin_pause(t0).unwrap();
                discard.finish_pause(Vec::new()).unwrap();
            }
            finalize.begin_finalize(t0).unwrap();
            discard.begin_discard(t0).unwrap();
            assert_eq!(finalize.phase, SessionPhase::Finalizing);
            assert_eq!(discard.phase, SessionPhase::Discarding);
        }
    }

    #[test]
    fn invalid_transitions_do_not_mutate_the_session() {
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);
        assert!(session.resume(Vec::new(), t0).is_err());
        assert_eq!(session.phase, SessionPhase::Recording);
        assert_eq!(session.active_elapsed, Duration::ZERO);

        session.phase = SessionPhase::Paused;
        assert!(session.begin_pause(t0).is_err());
        assert_eq!(session.phase, SessionPhase::Paused);

        session.phase = SessionPhase::Recording;
        assert!(session.finish_pause(Vec::new()).is_err());
        assert_eq!(session.phase, SessionPhase::Recording);

        session.phase = SessionPhase::Paused;
        assert!(session.finalize_failed("no".into()).is_err());
        assert_eq!(session.phase, SessionPhase::Paused);

        for phase in [
            SessionPhase::Pausing,
            SessionPhase::Finalizing,
            SessionPhase::Discarding,
        ] {
            session.phase = phase;
            assert!(session.begin_finalize(t0).is_err());
            assert_eq!(session.phase, phase);
            assert!(session.begin_discard(t0).is_err());
            assert_eq!(session.phase, phase);
        }
    }

    #[test]
    fn internal_phases_have_stable_public_snapshots() {
        let mut session = RecordingSession::new_for_test(Instant::now());
        for (phase, state, enabled) in [
            (
                SessionPhase::Recording,
                PublicRecordingState::Recording,
                true,
            ),
            (SessionPhase::Pausing, PublicRecordingState::Paused, false),
            (SessionPhase::Paused, PublicRecordingState::Paused, true),
            (
                SessionPhase::Finalizing,
                PublicRecordingState::Finalizing,
                false,
            ),
            (
                SessionPhase::Discarding,
                PublicRecordingState::Finalizing,
                false,
            ),
        ] {
            session.phase = phase;
            assert_eq!(session.public_state(), state);
            assert_eq!(session.actions_enabled(), enabled);
        }
    }

    #[test]
    fn two_resumes_keep_three_ordered_segments_per_output() {
        let dir = test_dir();
        let tools = tools(&dir);
        let scope = CaptureScope::Outputs(vec!["DP-3".into(), "DP-1".into()]);
        let t0 = Instant::now();
        let active = spawn_segment(&scope, "h264_nvenc", None, &tools).unwrap();
        assert_eq!(active.len(), 2);
        let mut session = RecordingSession::new(
            scope,
            Vec::new(),
            "h264_nvenc".into(),
            RecordBothMode::Separate,
            false,
            None,
            active,
            t0,
        );

        for cycle in 0..3 {
            session
                .begin_pause(t0 + Duration::from_secs(cycle * 2 + 1))
                .unwrap();
            let completed = stopped(std::mem::take(&mut session.active));
            session.finish_pause(completed).unwrap();
            if cycle < 2 {
                let active = spawn_segment(&session.scope, &session.codec, None, &tools).unwrap();
                session
                    .resume(active, t0 + Duration::from_secs(cycle * 2 + 2))
                    .unwrap();
            }
        }

        for output in ["DP-1", "DP-3"] {
            let paths = &session.completed[&Some(output.to_string())];
            assert_eq!(paths.len(), 3);
            assert!(
                paths
                    .iter()
                    .all(|path| fs::read(path).unwrap() == b"segment")
            );
            let ids = paths
                .iter()
                .map(|path| segment_id(path))
                .collect::<Vec<_>>();
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_resume_does_not_spawn_or_mutate() {
        let dir = test_dir();
        let tools = tools(&dir);
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);

        assert!(session.resume(Vec::new(), t0).is_err());
        assert_eq!(session.phase, SessionPhase::Recording);
        assert!(fs::read_dir(&tools.segment_dir).unwrap().all(|entry| {
            let name = entry.unwrap().file_name();
            name == "fake-recorder"
        }));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_resume_stops_and_reaps_supplied_children() {
        let dir = test_dir();
        let script = dir.join("short-recorder");
        fs::write(
            &script,
            r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-f" ]; then out="$2"; shift 2; else shift; fi
done
trap 'exit 0' INT
: > "$out"
sleep 1
sleep 1
sleep 1
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        let tools = RecorderTools {
            wf_recorder: script,
            ffmpeg: PathBuf::from("ffmpeg"),
            segment_dir: dir.clone(),
        };
        let active = spawn_segment(
            &CaptureScope::Area(Geometry {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            }),
            "test",
            None,
            &tools,
        )
        .unwrap();
        assert!(wait_for(Duration::from_secs(1), || active[0].path.exists()));
        let pid = active[0].child.id();
        let path = active[0].path.clone();
        let t0 = Instant::now();
        let mut session = RecordingSession::new_for_test(t0);

        assert!(session.resume(active, t0).is_err());
        assert!(wait_for(Duration::from_millis(1500), || {
            !path.exists() && !process_exists(pid)
        }));
        assert_eq!(session.phase, SessionPhase::Recording);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_sibling_spawn_is_stopped_and_empty_outputs_are_removed() {
        let dir = test_dir();
        let script = dir.join("empty-recorder");
        fs::write(
            &script,
            r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-f" ]; then out="$2"; shift 2; else shift; fi
done
trap 'exit 0' INT
: > "$out"
while :; do sleep 1; done
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        let tools = RecorderTools {
            wf_recorder: script.clone(),
            ffmpeg: PathBuf::from("ffmpeg"),
            segment_dir: dir.clone(),
        };
        let mut spawns = 0;
        let mut first_pid = None;
        let mut first_path = None;

        assert!(
            spawn_segment_with(
                &CaptureScope::Outputs(vec!["DP-3".into(), "DP-1".into()]),
                "h264_nvenc",
                None,
                &tools,
                |program, args| {
                    spawns += 1;
                    if spawns == 2 {
                        Err(io::Error::new(io::ErrorKind::NotFound, "test failure"))
                    } else {
                        let path = PathBuf::from(
                            &args[args.iter().position(|arg| arg == "-f").unwrap() + 1],
                        );
                        let child = Command::new("sh")
                            .arg(program)
                            .args(args)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn()?;
                        first_pid = Some(child.id());
                        first_path = Some(path.clone());
                        assert!(wait_for(Duration::from_secs(1), || path.exists()));
                        Ok(child)
                    }
                }
            )
            .is_err()
        );
        let first_pid = first_pid.unwrap();
        let first_path = first_path.unwrap();
        assert!(wait_for(Duration::from_secs(2), || {
            !process_exists(first_pid) && !first_path.exists()
        }));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        assert!(script.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_stop_preserves_nonempty_segments() {
        let dir = test_dir();
        let path = dir.join("recoverable.mp4");
        let child = Command::new("sh")
            .args([
                "-c",
                "printf recoverable > \"$1\"; exit 7",
                "sh",
                path.to_str().unwrap(),
            ])
            .spawn()
            .unwrap();

        let result = StopChildrenJob {
            children: vec![ActiveRecorder {
                output: Some("DP-3".into()),
                path: path.clone(),
                child,
            }],
        }
        .wait();
        assert!(matches!(
            result,
            StopChildrenResult::Failed { kept, .. }
                if kept == vec![StoppedSegment { output: Some("DP-3".into()), path: path.clone() }]
        ));
        assert_eq!(fs::read(&path).unwrap(), b"recoverable");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stop_escalates_from_ignored_sigint_to_sigterm() {
        let dir = test_dir();
        let path = dir.join("forced-stop.mp4");
        let child = Command::new("sh")
            .args([
                "-c",
                "trap '' INT; trap 'exit 0' TERM; printf video > \"$1\"; while :; do :; done",
                "sh",
                path.to_str().unwrap(),
            ])
            .spawn()
            .unwrap();
        assert!(wait_for(Duration::from_secs(1), || path.exists()));
        let job = StopChildrenJob {
            children: vec![ActiveRecorder {
                output: None,
                path: path.clone(),
                child,
            }],
        };
        job.interrupt().unwrap();

        let started = Instant::now();
        let result = job.wait_with_timeouts(Duration::from_millis(50), Duration::from_millis(500));

        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(matches!(result, StopChildrenResult::Ready(_)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spawned_recorder_receives_parent_death_signal() {
        let dir = test_dir();
        let script = dir.join("pdeath-recorder");
        fs::write(
            &script,
            r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-f" ]; then out="$2"; shift 2; else shift; fi
done
exec python3 -c 'import ctypes,signal,sys,time; value=ctypes.c_int(); ctypes.CDLL(None).prctl(2,ctypes.byref(value),0,0,0); open(sys.argv[1],"w").write(str(value.value)); signal.signal(signal.SIGINT,lambda *_:sys.exit(0)); signal.signal(signal.SIGTERM,lambda *_:sys.exit(0)); time.sleep(100)' "$out"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        let tools = RecorderTools {
            wf_recorder: script,
            ffmpeg: PathBuf::from("ffmpeg"),
            segment_dir: dir.clone(),
        };

        let active = spawn_segment(
            &CaptureScope::Area(Geometry {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            }),
            "test",
            None,
            &tools,
        )
        .unwrap();
        assert!(wait_for(Duration::from_secs(1), || {
            fs::read(&active[0].path).is_ok_and(|bytes| bytes == b"15")
        }));
        assert_eq!(fs::read(&active[0].path).unwrap(), b"15");
        let job = StopChildrenJob { children: active };
        job.interrupt().unwrap();
        assert!(matches!(job.wait(), StopChildrenResult::Ready(_)));
        fs::remove_dir_all(dir).unwrap();
    }

    fn two_monitors() -> Vec<Monitor> {
        vec![
            Monitor {
                name: "DP-3".into(),
                description: "BenQ".into(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 1.0,
                focused: true,
            },
            Monitor {
                name: "DP-1".into(),
                description: "AOC".into(),
                x: 2560,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                focused: false,
            },
        ]
    }

    #[test]
    fn focused_start_plan_has_one_recorder_child() {
        let plan = start_plan(&crate::config::RecordingPrefs::default(), &two_monitors()).unwrap();
        assert_eq!(plan.outputs[0].name, "DP-3");
        assert_eq!(plan.outputs.len(), 1);
        assert_eq!(plan.notice, None);
    }

    #[test]
    fn named_start_plan_selects_the_configured_output() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Output("DP-1".into()),
            ..crate::config::RecordingPrefs::default()
        };
        let plan = start_plan(&prefs, &two_monitors()).unwrap();
        assert_eq!(plan.outputs[0].name, "DP-1");
        assert_eq!(plan.outputs.len(), 1);
        assert_eq!(plan.notice, None);
    }

    #[test]
    fn disconnected_start_plan_falls_back_to_focused_output() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Output("HDMI-A-9".into()),
            ..crate::config::RecordingPrefs::default()
        };
        let plan = start_plan(&prefs, &two_monitors()).unwrap();
        assert_eq!(plan.outputs[0].name, "DP-3");
        assert!(plan.notice.unwrap().contains("HDMI-A-9"));
    }

    #[test]
    fn both_separate_start_plan_has_one_recorder_child_per_output() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Both,
            both_mode: RecordBothMode::Separate,
            ..crate::config::RecordingPrefs::default()
        };
        let plan = start_plan(&prefs, &two_monitors()).unwrap();
        assert_eq!(plan.outputs.len(), 2);
        assert_eq!(plan.both_mode, RecordBothMode::Separate);
    }

    #[test]
    fn both_combined_start_plan_keeps_two_outputs_in_one_session() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Both,
            both_mode: RecordBothMode::Combined,
            ..crate::config::RecordingPrefs::default()
        };
        let plan = start_plan(&prefs, &two_monitors()).unwrap();
        assert_eq!(
            plan.outputs
                .iter()
                .map(|monitor| monitor.name.as_str())
                .collect::<Vec<_>>(),
            vec!["DP-3", "DP-1"]
        );
        assert_eq!(plan.outputs.len(), 2);
        assert_eq!(plan.both_mode, RecordBothMode::Combined);
        assert_eq!(plan.notice, None);
    }

    #[test]
    fn both_with_one_output_reports_fallback() {
        let prefs = crate::config::RecordingPrefs {
            default_target: crate::config::RecordDefaultTarget::Both,
            ..crate::config::RecordingPrefs::default()
        };
        let monitors = two_monitors();
        let plan = start_plan(&prefs, &monitors[..1]).unwrap();
        assert_eq!(plan.outputs.len(), 1);
        assert!(plan.notice.unwrap().contains("one"));
    }
}
