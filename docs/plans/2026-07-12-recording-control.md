# Recording Control, Tray, and Quickshell Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Boltsnap's permanent recording pill with a persisted optional frame, a real pause/resume session, state-aware IPC and controls, native tray fullscreen choices, safe Shelf/Disk save semantics, and an IPC-only Quickshell widget immediately left of VPN.

**Architecture:** The shelf daemon remains the single recording owner. Pure recording state and finalization live under `src/record/`; `src/shelf/mod.rs` only coordinates Wayland surfaces, children, workers, shelf cards, and IPC. The tray runs through `ksni` and sends lightweight actions into the existing calloop loop. Quickshell starts one long-lived `boltsnap recording watch --json` process and never imports Boltsnap internals or video data.

**Tech Stack:** Rust 2024, calloop 0.13, smithay-client-toolkit 0.19, `wf-recorder`, FFmpeg, `ksni` 0.3.5 blocking API, QML/Quickshell 0.3.0 (`Process`, `SplitParser`), existing Tiny-Skia/Mono.Sdf UI.

## Global Constraints

- Approved behavior is defined by `docs/specs/2026-07-12-recording-control-design.md`; audio and Eddy timeline editing stay out of scope.
- Boltsnap must run without Quickshell. Quickshell only consumes the public CLI/IPC contract and never receives video bytes.
- The Wayland/calloop thread must never wait for a recorder, FFmpeg, copy, sync, or merge operation.
- Ordinary pause/resume finalization must use stream copy; combined multi-monitor output is the only re-encoding path.
- Permanent disk recordings must never be duplicated for a shelf card or deleted when that card closes.
- Only paths created inside Boltsnap's recording cache may be cleaned automatically.
- Quickshell changes follow `/home/mt/.config/quickshell/mono/AGENTS.md`: root `qmldir` registration, `import ".."` from subfolders, popout content inside `bar/Bar.qml`, hot reload only, and no `pkill` testing.
- Rust work uses TDD, `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and one reviewed commit per task.

---

## Execution precondition

The current Boltsnap worktree contains unrelated, already-started output-aware Add/Replace IPC changes in `src/capture.rs`, `src/editor.rs`, `src/ipc.rs`, `src/main.rs`, `src/shelf/mod.rs`, and `src/shelf/model.rs`, plus `docs/plans/2026-07-11-output-replace-ipc.md`. Preserve them. Before implementation, either explicitly checkpoint that work or create a clean implementation worktree containing it as a baseline. Never use `git add -A`; stage named files or recording-only hunks and inspect `git diff --cached` before every commit. Quickshell is a separate repository and therefore gets separate commits.

## Public contracts to keep stable

The CLI contract is:

```text
boltsnap record
boltsnap recording status --json
boltsnap recording watch --json
boltsnap recording show-controls
boltsnap recording pause
boltsnap recording resume
boltsnap recording save-shelf
boltsnap recording save-disk
boltsnap recording discard
```

The existing Alt+Print binding continues to invoke `boltsnap record`. That command opens the selector only when idle; in every other public state it sends `show-controls`. Keep `boltsnap stop` as a compatibility alias for `recording save-shelf`; remove it from the recommended keybinding documentation.

Every status/watch line has this stable JSON shape:

```json
{"state":"recording","elapsed_ms":83000,"scope":"area","outputs":["DP-3"],"actions_enabled":true,"error":null}
```

Allowed public state strings are exactly `idle`, `recording`, `paused`, and `finalizing`. Allowed scope strings are exactly `none`, `area`, `output`, and `both`.

---

### Task 1: Persist recording preferences and resolve monitor targets

**Files:**

- Modify: `src/config.rs`
- Modify: `src/record.rs`
- Modify: `README.md`

**Interfaces:**

- Consumes: existing `Config::parse`, `config_path`, and `wf_recorder_*_args`.
- Produces: `RecordingPrefs`, `RecordDefaultTarget`, `RecordBothMode`, `Monitor`, `Config::recording_prefs`, `save_recording_prefs`, `parse_hyprland_monitors`, and `resolve_record_outputs` for Tasks 2, 3, 4, 7, and 8.

- [ ] Add failing config tests for defaults, all four TOML keys, an individual output target, malformed values falling back safely, unknown-key preservation, and atomic write/read round trips.

```rust
#[test]
fn recording_prefs_default_to_focused_separate_and_visible() {
    assert_eq!(
        Config::default().recording_prefs(),
        RecordingPrefs {
            default_target: RecordDefaultTarget::Focused,
            both_mode: RecordBothMode::Separate,
            show_frame: true,
            disk_add_to_shelf: true,
        }
    );
}

#[test]
fn recording_prefs_write_preserves_unknown_keys() {
    let path = std::env::temp_dir().join(format!(
        "boltsnap-prefs-{}-{}.toml",
        std::process::id(),
        crate::paths::timestamp()
    ));
    std::fs::write(&path, "editor = \"eddy\"\ncustom = 7\n").unwrap();
    let prefs = RecordingPrefs {
        default_target: RecordDefaultTarget::Output("DP-3".into()),
        both_mode: RecordBothMode::Combined,
        show_frame: false,
        disk_add_to_shelf: false,
    };
    save_recording_prefs_at(&path, &prefs).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("custom = 7"));
    assert_eq!(Config::parse(&written).recording_prefs(), prefs);
    std::fs::remove_file(path).unwrap();
}
```

- [ ] Run `cargo test recording_prefs_` and expect compilation to fail because `RecordingPrefs` and its persistence functions do not exist yet.

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordDefaultTarget {
    Focused,
    Output(String),
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordBothMode {
    Separate,
    Combined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingPrefs {
    pub default_target: RecordDefaultTarget,
    pub both_mode: RecordBothMode,
    pub show_frame: bool,
    pub disk_add_to_shelf: bool,
}

impl Default for RecordingPrefs {
    fn default() -> Self {
        Self {
            default_target: RecordDefaultTarget::Focused,
            both_mode: RecordBothMode::Separate,
            show_frame: true,
            disk_add_to_shelf: true,
        }
    }
}
```

The defaults are `Focused`, `Separate`, `true`, and `true`. Parse and write `record_default_target`, `record_both_mode`, `record_show_frame`, and `record_disk_add_to_shelf` in the existing root TOML table.

- [ ] Implement `Config::recording_prefs()`, `save_recording_prefs(&RecordingPrefs)`, and the testable `save_recording_prefs_at(path, prefs)`. Read an existing TOML table, update only the four owned keys, write a sibling `.tmp-<pid>` file, `sync_all`, and rename it over the config. Create the parent directory first. On malformed existing TOML, return an error instead of destroying the file.

- [ ] Add failing `src/record.rs` tests for focused, named-output, disconnected-output, both-output, and one-monitor fallback resolution from a Hyprland monitor JSON fixture.

```rust
const MONITORS: &[u8] = br#"[
  {"name":"DP-3","description":"BenQ","x":0,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true},
  {"name":"DP-1","description":"AOC","x":2560,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false}
]"#;

#[test]
fn disconnected_default_falls_back_to_focused_output() {
    let monitors = parse_hyprland_monitors(MONITORS).unwrap();
    let (resolved, notice) = resolve_record_outputs(
        &RecordDefaultTarget::Output("HDMI-A-9".into()),
        &monitors,
    ).unwrap();
    assert_eq!(resolved.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["DP-3"]);
    assert!(notice.unwrap().contains("HDMI-A-9"));
}

#[test]
fn both_keeps_connected_output_order() {
    let monitors = parse_hyprland_monitors(MONITORS).unwrap();
    let (resolved, notice) = resolve_record_outputs(&RecordDefaultTarget::Both, &monitors).unwrap();
    assert_eq!(resolved.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["DP-3", "DP-1"]);
    assert_eq!(notice, None);
}
```

- [ ] Run `cargo test disconnected_default_falls_back` and expect compilation to fail because monitor parsing and resolution are not implemented.

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub focused: bool,
}

pub fn parse_hyprland_monitors(json: &[u8]) -> Result<Vec<Monitor>, String>;
pub fn resolve_record_outputs(
    target: &RecordDefaultTarget,
    monitors: &[Monitor],
) -> Result<(Vec<Monitor>, Option<String>), String>;
```

Return an optional human-readable fallback notice alongside the resolved monitors. Preserve Hyprland's array order for menu stability; use the focused monitor for `Focused` and for a disconnected named target.

- [ ] Update the README config example and document the exact defaults and accepted values (`focused`, `output:<name>`, `both`; `separate`, `combined`).

- [ ] Run `cargo test config::tests`, `cargo test record::tests`, and then `cargo test`; expect all new and existing tests to pass.

- [ ] Commit with `git commit -m "feat: persist recording target preferences"`.

---

### Task 2: Add the selector frame checkbox and carry it through IPC

**Files:**

- Modify: `src/select_skia/mod.rs`
- Modify: `src/select_skia/render.rs`
- Modify: `src/ipc.rs`
- Modify: `src/main.rs`

**Interfaces:**

- Consumes: `RecordingPrefs` and `save_recording_prefs` from Task 1.
- Produces: `RecordSelectionResult`, `record_frame_checkbox_rect`, and `Request::StartRecording { x, y, w, h, show_frame }` for Task 6.

- [ ] Add failing renderer tests asserting that the checkbox rect is directly beside the REC pill, stays on-screen at every edge, draws checked/unchecked states, and does not overlap the REC hit zone. Give the helper the exact signature `record_frame_checkbox_rect(sel: (f32, f32, f32, f32), surf_w: u32, surf_h: u32) -> Option<(f64, f64, f64, f64)>`.

```rust
#[test]
fn frame_checkbox_sits_beside_rec_without_overlap() {
    let sel = (100.0, 100.0, 800.0, 500.0);
    let rec = rec_pill_rect(sel, 1920, 1080).unwrap();
    let check = record_frame_checkbox_rect(sel, 1920, 1080).unwrap();
    assert!(check.0 >= rec.0 + rec.2);
    assert!(check.0 + check.2 <= 1920.0);
    assert!(check.1 >= 0.0 && check.1 + check.3 <= 1080.0);
}

#[test]
fn start_recording_missing_frame_field_defaults_true() {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, br#"{"cmd":"record","x":1,"y":2,"w":3,"h":4}"#, &[]).unwrap();
    assert!(matches!(
        Request::read(&mut std::io::Cursor::new(bytes)).unwrap(),
        Request::StartRecording { show_frame: true, .. }
    ));
}
```

- [ ] Run `cargo test frame_checkbox_sits_beside_rec` and expect compilation to fail because the checkbox helper does not exist.

```rust
pub struct RecordSelectionResult {
    pub rect: Option<edit::Rect>,
    pub show_frame: bool,
}

pub fn run_select_record(initial_show_frame: bool) -> DynResult<RecordSelectionResult>;
```

- [ ] Add `show_frame` to selector state. A click in `record_frame_checkbox_rect(sel, self.surf_w, self.surf_h)` toggles only the checkbox; a click in the existing REC pill confirms. Return the final checkbox value even when Escape cancels so the preference change still persists.

- [ ] Extend `Request::StartRecording` with `show_frame: bool`, update its JSON encoder/decoder, and add a backward-compatibility decode test where missing `show_frame` defaults to `true`.

- [ ] Change idle `record_flow` to load `RecordingPrefs`, pass `show_frame` into the selector, persist a changed value after the selector closes, and send it with `StartRecording`. Do not change screenshot selector behavior.

- [ ] Run `cargo test select_skia::render::tests`, `cargo test ipc::tests`, and then `cargo test`; expect all tests to pass.

- [ ] Commit with `git commit -m "feat: persist the recording frame selector"`.

---

### Task 3: Build the recording session state machine and real pause segments

**Files:**

- Modify: `src/record.rs`
- Create: `src/record/session.rs`

**Interfaces:**

- Consumes: `Geometry`, `Monitor`, `RecordBothMode`, and the existing `wf_recorder_*_args`.
- Produces: `PublicRecordingState`, `RecordingAction`, `SessionPhase`, `CaptureScope`, `ActiveRecorder`, `RecordingSession`, `RecorderTools`, `StopChildrenJob`, and `StopChildrenResult` for Tasks 4–8.

- [ ] Add failing tests for every legal and illegal transition, active-only elapsed time across two pauses, frozen paused elapsed time, one segment per output per resume, and public snapshot mapping during short internal transitions.

```rust
#[test]
fn elapsed_excludes_paused_wall_time() {
    let t0 = Instant::now();
    let mut s = RecordingSession::new_for_test(t0);
    s.begin_pause(t0 + Duration::from_secs(10)).unwrap();
    s.finish_pause(Vec::new()).unwrap();
    assert_eq!(s.elapsed_at(t0 + Duration::from_secs(50)), Duration::from_secs(10));
    s.resume(Vec::new(), t0 + Duration::from_secs(50)).unwrap();
    assert_eq!(s.elapsed_at(t0 + Duration::from_secs(55)), Duration::from_secs(15));
}

#[test]
fn invalid_resume_does_not_mutate_recording_session() {
    let t0 = Instant::now();
    let mut s = RecordingSession::new_for_test(t0);
    assert!(s.resume(Vec::new(), t0).is_err());
    assert_eq!(s.phase, SessionPhase::Recording);
    assert_eq!(s.active_elapsed, Duration::ZERO);
}
```

`new_for_test` is a `#[cfg(test)]` constructor creating an area session with no child processes; production uses `RecordingSession::new(scope, monitors, codec, both_mode, show_frame, active, now)`.

- [ ] Run `cargo test elapsed_excludes_paused_wall_time` and expect compilation to fail because `record::session` does not exist.

```rust
pub mod session;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicRecordingState { Idle, Recording, Paused, Finalizing }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingAction { Pause, Resume, SaveShelf, SaveDisk, Discard }

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

#[derive(Debug)]
pub struct ActiveRecorder {
    pub output: Option<String>,
    pub path: PathBuf,
    pub child: std::process::Child,
}

pub struct RecordingSession {
    pub phase: SessionPhase,
    pub scope: CaptureScope,
    pub monitors: Vec<Monitor>,
    pub codec: String,
    pub both_mode: RecordBothMode,
    pub show_frame: bool,
    pub completed: BTreeMap<Option<String>, Vec<PathBuf>>,
    pub active: Vec<ActiveRecorder>,
    pub active_elapsed: Duration,
    pub active_started: Option<Instant>,
    pub last_error: Option<String>,
}
```

Expose methods `elapsed_at(now)`, `begin_pause(now)`, `finish_pause(completed)`, `resume(active, now)`, `begin_finalize(now)`, `finalize_failed(error)`, and `can_accept(action)`. `Pausing` publishes `paused/actions_enabled=false`; `Discarding` and `Finalizing` publish `finalizing/actions_enabled=false`.

- [ ] Implement `spawn_segment(scope, codec, tools)`. `RecorderTools` defaults to `wf-recorder` and `ffmpeg` but accepts explicit executable paths in tests. Start one child for an area/single output and one child per output for `Both`; if any spawn fails, SIGINT and reap already-started siblings off-thread and remove their empty paths.

- [ ] Implement a worker input/output pair for stopping children:

```rust
pub struct StopChildrenJob { pub children: Vec<ActiveRecorder> }
pub struct StoppedSegment { pub output: Option<String>, pub path: PathBuf }
pub enum StopChildrenResult { Ready(Vec<StoppedSegment>), Failed { kept: Vec<PathBuf>, error: String } }
```

The caller sends SIGINT immediately, then the worker waits. Never call `Child::wait` on the calloop thread.

- [ ] In tests, create executable fake recorder shell scripts under a unique temp directory. The fake traps SIGINT, writes a recognizable segment marker, and exits zero. Verify two pause/resume cycles retain three ordered segments and an invalid resume does not spawn.

- [ ] Run `cargo test record::session::tests` and expect all session/lifecycle tests to pass.

- [ ] Commit with `git commit -m "feat: model paused recording sessions"`.

---

### Task 4: Implement lossless segment finalization and safe file ownership

**Files:**

- Modify: `src/record.rs`
- Create: `src/record/finalize.rs`
- Modify: `src/paths.rs`
- Modify: `src/shelf/model.rs`

**Interfaces:**

- Consumes: `RecorderTools`, `Monitor`, `RecordBothMode`, and ordered session segments from Task 3.
- Produces: `SaveDestination`, `FinalizeRequest`, `FinalizedClip`, `FinalizeFailure`, `finalize_recording`, and shelf `FileLifetime` for Tasks 6, 7, and 11.

- [ ] Add failing finalizer tests for the one-segment fast path, concat-demuxer `-c copy` arguments, concat-list escaping, separate two-output grouping, combined layout coordinates, codec quality arguments, insufficient space, failed FFmpeg preservation, same-filesystem rename, and cross-filesystem copy failure preservation.

```rust
#[test]
fn concat_uses_demuxer_and_stream_copy() {
    let args = build_concat_args(Path::new("/tmp/list.txt"), Path::new("/tmp/out.mp4"));
    assert_eq!(args, vec![
        "-y", "-f", "concat", "-safe", "0", "-i", "/tmp/list.txt",
        "-c", "copy", "/tmp/out.mp4"
    ]);
}

#[test]
fn combined_layout_uses_hyprland_positions() {
    let filter = build_xstack_filter(&[
        Monitor { name: "DP-3".into(), description: "BenQ".into(), x: 0, y: 0, width: 2560, height: 1440, scale: 1.0, focused: true },
        Monitor { name: "DP-1".into(), description: "AOC".into(), x: 2560, y: 0, width: 1920, height: 1080, scale: 1.0, focused: false },
    ]).unwrap();
    assert!(filter.contains("xstack=inputs=2:layout=0_0|2560_0:fill=black"));
}

#[test]
fn nvenc_combined_quality_is_visually_lossless() {
    assert_eq!(quality_args("h264_nvenc"), vec![
        "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "16", "-b:v", "0"
    ]);
}
```

- [ ] Run `cargo test concat_uses_demuxer_and_stream_copy` and expect compilation to fail because `record::finalize` does not exist.

```rust
pub enum SaveDestination { Shelf, Disk(PathBuf) }

pub struct FinalizeRequest {
    pub segments: BTreeMap<Option<String>, Vec<PathBuf>>,
    pub monitors: Vec<Monitor>,
    pub both_mode: RecordBothMode,
    pub codec: String,
    pub destination: SaveDestination,
}

pub struct FinalizedClip {
    pub output: Option<String>,
    pub path: PathBuf,
    pub permanent: bool,
}

pub struct FinalizeFailure {
    pub error: String,
    pub recoverable_segments: BTreeMap<Option<String>, Vec<PathBuf>>,
}

pub fn finalize_recording(req: FinalizeRequest, tools: &RecorderTools)
    -> Result<Vec<FinalizedClip>, FinalizeFailure>;
```

- [ ] Implement one-segment finalization without invoking FFmpeg. For multiple compatible pause segments, write a concat list inside the recording cache and run `ffmpeg -f concat -safe 0 -i <list> -c copy <temp-output>`. Delete source segments and the list only after a non-empty final output succeeds.

- [ ] Implement combined mode after per-output concat. Normalize Hyprland logical positions by the largest monitor scale so high-DPI streams are never downscaled. Build one `scale=<tile-width>:<tile-height>` filter per input and an `xstack` layout containing every normalized coordinate, for example `xstack=inputs=2:layout=0_0|3840_0:fill=black`. Use:

```text
*_nvenc: -preset p5 -tune hq -rc vbr -cq 16 -b:v 0
libx264/libx265: -preset veryfast -crf 16
other codecs: -q:v 2
```

Delete separate finalized inputs only after the combined output succeeds. Keep them on every error.

- [ ] Before concat or composition, use `libc::statvfs` on the destination directory. Require free bytes greater than the sum of all source sizes plus `max(64 MiB, source_sum / 20)`. The one-segment rename path skips this duplicate-space check.

- [ ] Implement permanent destination names as `boltsnap-YYYY-MM-DD_HH-MM-SS[-<sanitized-output>][-N].mp4`. On the same filesystem, rename. Across filesystems, copy to a `.part-<pid>` sibling, `sync_all`, rename the part, then remove the source. Never remove a source after a failed copy, sync, or rename.

- [ ] Add shelf file ownership:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileLifetime { Temporary, Permanent }
```

Store it on `Thumb`. Add `add_kind_with_lifetime`; keep existing `add`/`add_kind` defaulting to `Temporary`. Dismiss animation deletes only `Temporary` paths. Saving an already permanent card is a no-op with “Already saved” feedback. Add tests proving permanent files survive card dismissal.

- [ ] Run `cargo test record::finalize::tests`, `cargo test shelf::model::tests`, `cargo test paths::tests`, and then `cargo test`; expect all tests to pass.

- [ ] Commit with `git commit -m "feat: finalize recording segments without duplicate loss"`.

---

### Task 5: Add request/response recording IPC and the public CLI

**Files:**

- Modify: `src/ipc.rs`
- Modify: `src/main.rs`

**Interfaces:**

- Consumes: `PublicRecordingState` and `RecordingAction` from Task 3 and the start request from Task 2.
- Produces: `RecordingSnapshot`, framed `Response`, `call_daemon`, and `watch_recording` for Tasks 6, 9, and 10.

- [ ] Add failing IPC round-trip tests for status, watch, show-controls, every control action, multi-output start, response success/error, snapshot JSON, and newline framing.

```rust
#[test]
fn recording_control_roundtrips_all_actions() {
    for action in [
        RecordingAction::Pause,
        RecordingAction::Resume,
        RecordingAction::SaveShelf,
        RecordingAction::SaveDisk,
        RecordingAction::Discard,
    ] {
        let decoded = Request::read(&mut Cursor::new(
            Request::RecordingControl { action }.encode()
        )).unwrap();
        assert!(matches!(decoded, Request::RecordingControl { action: got } if got == action));
    }
}

#[test]
fn recording_snapshot_json_uses_stable_public_names() {
    let line = RecordingSnapshot {
        state: PublicRecordingState::Paused,
        elapsed_ms: 83_000,
        scope: "both".into(),
        outputs: vec!["DP-3".into(), "DP-1".into()],
        actions_enabled: true,
        error: None,
    }.to_json_line();
    assert_eq!(line, "{\"state\":\"paused\",\"elapsed_ms\":83000,\"scope\":\"both\",\"outputs\":[\"DP-3\",\"DP-1\"],\"actions_enabled\":true,\"error\":null}\n");
}
```

- [ ] Run `cargo test recording_control_roundtrips_all_actions` and expect compilation to fail because the recording control request variants do not exist.

```rust
pub enum Request {
    // existing variants remain
    RecordingStatus,
    RecordingWatch,
    ShowRecordingControls,
    RecordingControl { action: RecordingAction },
    StartRecordingOutputs { names: Vec<String>, combined: bool },
}

pub struct RecordingSnapshot {
    pub state: PublicRecordingState,
    pub elapsed_ms: u64,
    pub scope: String,
    pub outputs: Vec<String>,
    pub actions_enabled: bool,
    pub error: Option<String>,
}
```

- [ ] Keep the existing framed socket for requests. Add framed JSON `Response { ok, error, snapshot }` for status/control calls. After a framed `RecordingWatch`, switch that connection to raw newline-delimited snapshot JSON. Add `call_daemon(req) -> io::Result<Response>` and `watch_recording() -> io::Result<UnixStream>`; preserve `send_to_shelf` for fire-and-forget legacy calls.

- [ ] Extend `Args` with `tail: Vec<String>` and `json: bool` without changing the existing `image` behavior. Implement the exact CLI contract listed at the top. `status` prints one JSON object; `watch` writes each received line to stdout and flushes. Invalid transitions print the daemon error and exit non-zero.

- [ ] Make `record_flow` query status first. Idle opens the selector; recording/paused/finalizing sends `ShowRecordingControls`. Map legacy `stop` to `SaveShelf` and update `usage()`.

- [ ] Add parser tests for every CLI form and a local `UnixListener` test proving that watch forwards multiple lines without buffering until process exit.

- [ ] Run `cargo test ipc::tests`, `cargo test parser_`, and then `cargo test`; expect all tests to pass.

- [ ] Commit with `git commit -m "feat: expose recording control IPC"`.

---

### Task 6: Replace the permanent pill with the centered Boltsnap control popup

**Files:**

- Modify: `src/shelf/recording.rs`
- Modify: `src/shelf/paint.rs`
- Modify: `src/shelf/mod.rs`

**Interfaces:**

- Consumes: the session, finalizer, ownership, and IPC contracts from Tasks 2–5.
- Produces: daemon-owned recording lifecycle, nonblocking watchers, `DaemonEvent`, and the centered popup used by every Boltsnap control entry point.

- [ ] Replace `RecPhase`, `IndButton`, indicator constants, pill hit tests, and pill paint tests with a fixed centered popup layout and tests:

```rust
#[test]
fn finalizing_popup_has_no_live_buttons() {
    for y in 0..POPUP_H {
        for x in 0..POPUP_W {
            assert_eq!(
                popup_hit(PublicRecordingState::Finalizing, false, x as f64, y as f64),
                None
            );
        }
    }
}

#[test]
fn recording_and_paused_share_the_pause_resume_cell() {
    let (x, y, w, h) = pause_resume_rect();
    let point = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
    assert_eq!(popup_hit(PublicRecordingState::Recording, true, point.0, point.1), Some(PopupButton::PauseResume));
    assert_eq!(popup_hit(PublicRecordingState::Paused, true, point.0, point.1), Some(PopupButton::PauseResume));
}
```

- [ ] Run `cargo test finalizing_popup_has_no_live_buttons` and expect compilation to fail because the new popup layout does not exist.

```rust
pub const POPUP_W: u32 = 408;
pub const POPUP_H: u32 = 148;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupButton { PauseResume, SaveShelf, SaveDisk, Discard }

pub fn popup_hit(state: PublicRecordingState, enabled: bool, x: f64, y: f64)
    -> Option<PopupButton>;
```

The popup header displays `Recording`, `Paused`, or `Saving…`; recording shows Pause, paused shows Resume, and finalizing disables every button. Use the existing shelf font/drawing primitives. Keep the black marker painter unchanged.

- [ ] In `Daemon`, replace indicator surface/pool/configured fields with an optional popup surface/pool/configured flag. Create it on demand on the currently focused output as `Layer::Overlay`, with no anchors (centered), `KeyboardInteractivity::Exclusive`, namespace `boltsnap-recording-controls`, and no exclusive zone.

- [ ] Route pointer clicks to `RecordingAction`, and make Escape drop only the popup. Closing the popup surface must also drop only the popup. Closing the optional marker surface must remove only the frame; it must never discard the recording.

- [ ] Remove the old permanent indicator creation, drawing, ticking, Stop/Confirm/Cancel flow, and all old pill comments/tests. Start a region marker only when `show_frame` is true. Pause and finalization remove the marker; resume recreates it only when enabled.

- [ ] Add a `calloop::channel::channel::<DaemonEvent>()` source. Send stop/wait/finalization results from workers back through it; handle them on the daemon thread to mutate the model and surfaces. No worker may call Wayland APIs.

```rust
enum AfterStop {
    Pause,
    Save(SaveDestination),
    Discard,
}

enum DaemonEvent {
    ChildrenStopped { after: AfterStop, result: StopChildrenResult },
    Finalized(Result<Vec<FinalizedClip>, FinalizeFailure>),
}
```

- [ ] Make request handling return responses and retain watch streams as nonblocking `UnixStream`s. `publish_recording_snapshot` writes the initial state, every state/error change, and each new whole elapsed second. Drop a watcher on `WouldBlock`, broken pipe, or any write error; never retry synchronously.

- [ ] Keep the existing `try_wait` health check, but change its failure path: preserve every non-empty finished/current cache segment, move the session to recoverable `Paused`, set `last_error`, remove the marker, notify, and publish. Never route an unexpected recorder exit through discard cleanup.

- [ ] Wire actions:

  - Pause: move children to a stop worker, phase `Pausing`, then `Paused` on success.
  - Resume: spawn the next segment set and return to `Recording`.
  - Shelf/Disk Save: stop active children if needed, enter `Finalizing`, run `finalize_recording`, then create cards/files.
  - Discard: stop active children off-thread, delete only Boltsnap cache paths, close UI, publish idle.
  - Finalize failure: retain paths, return to recoverable `Paused`, set `last_error`, notify, and republish.

- [ ] For each finalized shelf card, add the existing placeholder immediately after worker success and start the existing thumbnail worker. Disk Save adds permanent cards only when `record_disk_add_to_shelf` is enabled; those cards reference the permanent path and use `FileLifetime::Permanent`.

- [ ] Run `cargo test shelf::recording::tests`, `cargo test shelf::paint::tests`, `cargo test record::session::tests`, `cargo test ipc::tests`, and then `cargo test`; expect all tests to pass.

- [ ] Under a Wayland session, run `boltsnap daemon` and verify: no permanent pill appears; `boltsnap record` while active opens a centered popup; Escape closes only the popup; reopening works; pause freezes elapsed time; discard returns to idle.

- [ ] Commit with `git commit -m "feat: add state-aware recording controls"`.

---

### Task 7: Support fullscreen defaults, both-display modes, and fallback

**Files:**

- Modify: `src/record/session.rs`
- Modify: `src/record/finalize.rs`
- Modify: `src/shelf/mod.rs`
- Modify: `src/main.rs`

**Interfaces:**

- Consumes: `resolve_record_outputs`, `RecordingSession`, and `finalize_recording`.
- Produces: `start_plan`, `start_recording_outputs`, and `start_default_recording` for the tray in Task 8.

```rust
pub struct StartPlan {
    pub outputs: Vec<Monitor>,
    pub both_mode: RecordBothMode,
    pub notice: Option<String>,
}

pub fn start_plan(prefs: &RecordingPrefs, monitors: &[Monitor]) -> Result<StartPlan, String>;
```

- [ ] Add daemon tests around a pure `start_plan(prefs, monitors)` helper for focused, named, disconnected, both/separate, both/combined, and one-monitor fallback cases. Assert the fallback notice and the number of recorder children.

```rust
fn two_monitors() -> Vec<Monitor> {
    vec![
        Monitor { name: "DP-3".into(), description: "BenQ".into(), x: 0, y: 0, width: 2560, height: 1440, scale: 1.0, focused: true },
        Monitor { name: "DP-1".into(), description: "AOC".into(), x: 2560, y: 0, width: 1920, height: 1080, scale: 1.0, focused: false },
    ]
}

#[test]
fn both_combined_start_plan_keeps_two_outputs_in_one_session() {
    let prefs = RecordingPrefs {
        default_target: RecordDefaultTarget::Both,
        both_mode: RecordBothMode::Combined,
        show_frame: false,
        disk_add_to_shelf: true,
    };
    let monitors = two_monitors();
    let plan = start_plan(&prefs, &monitors).unwrap();
    assert_eq!(plan.outputs.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["DP-3", "DP-1"]);
    assert_eq!(plan.both_mode, RecordBothMode::Combined);
    assert_eq!(plan.notice, None);
}

#[test]
fn both_with_one_output_reports_fallback() {
    let prefs = RecordingPrefs { default_target: RecordDefaultTarget::Both, ..RecordingPrefs::default() };
    let monitors = parse_hyprland_monitors(br#"[{"name":"DP-3","description":"BenQ","x":0,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true}]"#).unwrap();
    let plan = start_plan(&prefs, &monitors).unwrap();
    assert_eq!(plan.outputs.len(), 1);
    assert!(plan.notice.unwrap().contains("one connected display"));
}
```

- [ ] Run `cargo test both_combined_start_plan` and expect compilation to fail because `start_plan` is not implemented.

- [ ] Replace the single `start_recording_output(name)` path with `start_recording_outputs(monitors, both_mode)`. Capture each output with `wf-recorder -o <name>` into its own segment path. Keep all children under one logical session and pause/resume/discard them together.

- [ ] Implement `start_default_recording`: reload preferences, parse current `hyprctl monitors -j`, resolve outputs, notify any fallback, and start the resolved plan. The tray and `boltsnap record full` both call this same daemon action; no target resolution is duplicated in CLI code.

- [ ] Separate mode returns one clip/card per output. Combined mode calls the tested `xstack` finalizer and returns one clip/card. Include output names in separate filenames and card source labels.

- [ ] Run `cargo test start_plan`, `cargo test record::session::tests`, `cargo test record::finalize::tests`, and then `cargo test`; expect all tests to pass.

- [ ] Manually record both displays for 20 seconds in separate mode, pause/resume once, Shelf Save, and verify two playable clips at native source quality. Repeat combined mode and verify one clip follows the Hyprland arrangement with black only in real layout gaps.

- [ ] Commit with `git commit -m "feat: record configured fullscreen targets"`.

---

### Task 8: Add the always-visible native tray menu

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/tray.rs`
- Modify: `src/main.rs`
- Modify: `src/shelf/mod.rs`

**Interfaces:**

- Consumes: `RecordingPrefs`, `Monitor`, `PublicRecordingState`, and `start_default_recording`.
- Produces: `TraySnapshot`, `TrayAction`, `BoltsnapTray`, and its blocking handle; no later Rust task adds a second tray abstraction.

- [ ] Add `ksni = { version = "0.3.5", features = ["blocking"] }`. Keep its default Tokio feature because `ksni`'s blocking implementation requires either the Tokio or async-io runtime feature.

- [ ] Add pure menu snapshot tests before implementing the DBus adapter:

```rust
#[test]
fn tray_menu_exposes_outputs_modes_and_checkmarks() {
    let snapshot = TraySnapshot {
        prefs: RecordingPrefs {
            default_target: RecordDefaultTarget::Output("DP-3".into()),
            both_mode: RecordBothMode::Combined,
            show_frame: false,
            disk_add_to_shelf: true,
        },
        monitors: vec![
            Monitor { name: "DP-3".into(), description: "BenQ".into(), x: 0, y: 0, width: 2560, height: 1440, scale: 1.0, focused: true },
            Monitor { name: "DP-1".into(), description: "AOC".into(), x: 2560, y: 0, width: 1920, height: 1080, scale: 1.0, focused: false },
        ],
        state: PublicRecordingState::Idle,
    };
    let model = menu_model(&snapshot);
    assert_eq!(model.start_region_enabled, true);
    assert_eq!(model.default_labels, vec!["BenQ (DP-3)", "AOC (DP-1)", "Both displays"]);
    assert_eq!(model.default_selected, 0);
    assert_eq!(model.both_mode_selected, 1);
    assert_eq!(model.show_frame, false);
    assert_eq!(model.disk_add_to_shelf, true);
}
```

- [ ] Run `cargo test tray_menu_exposes_outputs_modes_and_checkmarks` and expect compilation to fail because `src/tray.rs` and `menu_model` do not exist.

```rust
#[derive(Clone, Debug)]
pub struct TraySnapshot {
    pub prefs: RecordingPrefs,
    pub monitors: Vec<Monitor>,
    pub state: PublicRecordingState,
}

#[derive(Clone, Debug)]
pub enum TrayAction {
    StartRegion,
    StartDefault,
    SetDefaultTarget(RecordDefaultTarget),
    SetBothMode(RecordBothMode),
    SetShowFrame(bool),
    SetDiskAddToShelf(bool),
}

#[cfg(test)]
struct TrayMenuModel {
    start_region_enabled: bool,
    default_labels: Vec<String>,
    default_selected: usize,
    both_mode_selected: usize,
    show_frame: bool,
    disk_add_to_shelf: bool,
}

#[cfg(test)]
fn menu_model(snapshot: &TraySnapshot) -> TrayMenuModel;
```

Assert labels, enabled states, selected monitor/both radio entry, both-mode radio entry, and both checkmarks. Start entries are disabled outside idle; settings remain enabled.

- [ ] Implement `BoltsnapTray: ksni::Tray` with id/title `boltsnap`, icon name `camera-video`, `Status::Active`, and this menu order:

```text
Start region recording
Start fullscreen recording
Default monitor > [connected outputs, Both displays]
Both displays mode > [Separate clips, Combined clip]
Show recording frame
Video: Move to shelf after Disk Save
```

Use `StandardItem`, `SubMenu`, `RadioGroup`, and `CheckmarkItem`. Every callback updates only its local snapshot and performs a nonblocking `calloop::channel::Sender::send(TrayAction)`; no config, process, or Wayland operation runs in the menu callback.

- [ ] Spawn the blocking tray once during daemon startup via `TrayMethods::spawn()`. Keep its `ksni::blocking::Handle` in `Daemon`; update it after output events, preference changes, and recording state changes. If no StatusNotifierWatcher is available, log once and continue running the daemon.

- [ ] Handle tray actions on the calloop thread. `StartRegion` spawns the current executable as detached `boltsnap record`; `StartDefault` invokes the daemon's shared default-start path. Setting actions atomically persist preferences, update marker visibility immediately when applicable, publish the new tray snapshot, and notify on persistence failure.

Extend Task 6's event enum in this task rather than creating another channel:

```rust
enum DaemonEvent {
    ChildrenStopped { after: AfterStop, result: StopChildrenResult },
    Finalized(Result<Vec<FinalizedClip>, FinalizeFailure>),
    Tray(TrayAction),
}
```

- [ ] In `OutputHandler::{new_output,update_output,output_destroyed}`, rebuild monitor name/description entries and update the tray. Do not restart the tray service.

- [ ] Run `cargo test tray::tests` and then `cargo test`; expect all tests to pass.

- [ ] Run `cargo build` and inspect the real tray host: the icon remains present while idle, output names/descriptions are current, radio/check items persist across daemon restart, and right-click start actions work.

- [ ] Commit with `git commit -m "feat: add recording controls to the tray"`.

---

### Task 9: Add the Quickshell recording service and popout components

**Repository:** `/home/mt/.config/quickshell/mono`

**Files:**

- Create: `services/BoltsnapRecordingService.qml`
- Create: `popouts/BoltsnapRecordingPopout.qml`
- Create: `popouts/BoltsnapRecordingContent.qml`
- Create: `tests/tst_BoltsnapRecordingService.qml`
- Modify: `qmldir`

**Interfaces:**

- Consumes: the exact CLI and JSON schema from Task 5.
- Produces: observable `BoltsnapRecordingService` properties/methods plus `BoltsnapRecordingPopout` and `BoltsnapRecordingContent` for Task 10.

- [ ] Create the failing QML service test. The production service exposes `property bool watchEnabled: true` solely to make process ownership controllable; its watch `Process.running` binds to that property.

```qml
import QtQuick
import QtTest
import mono

TestCase {
    name: "BoltsnapRecordingService"

    BoltsnapRecordingService {
        id: service
        watchEnabled: false
    }

    function test_snapshot_updates_observable_state() {
        service.applySnapshot('{"state":"paused","elapsed_ms":83000,"scope":"both","outputs":["DP-3","DP-1"],"actions_enabled":true,"error":null}')
        compare(service.state, "paused")
        compare(service.elapsedMs, 83000)
        compare(service.elapsedText, "01:23")
        compare(service.active, true)
        compare(service.outputs.length, 2)
    }

    function test_malformed_snapshot_keeps_last_good_state() {
        service.applySnapshot('{broken')
        compare(service.state, "paused")
        compare(service.elapsedMs, 83000)
    }
}
```

- [ ] Run `qmltestrunner -input tests/tst_BoltsnapRecordingService.qml -import /usr/lib/qt6/qml -import /home/mt/.config/quickshell/mono` and expect failure because `BoltsnapRecordingService` is not registered yet.

- [ ] Implement `BoltsnapRecordingService` as a process-only `Scope`. It owns these observable properties: `state`, `elapsedMs`, `scope`, `outputs`, `actionsEnabled`, `error`, `ready`, plus derived `active` and `elapsedText`.

```qml
Process {
    id: watchProcess
    running: true
    command: ["boltsnap", "recording", "watch", "--json"]
    stdout: SplitParser {
        splitMarker: "\n"
        onRead: line => root.applySnapshot(line)
    }
    onExited: watchRestart.restart()
}
```

Parse one small JSON object per line with `JSON.parse`. Ignore malformed lines while retaining the last good state. Restart after 1500 ms. Do not use a polling timer.

- [ ] Add `pauseOrResume()`, `saveShelf()`, `saveDisk()`, and `discard()` methods. Each creates a one-shot `Process` running the corresponding public CLI command and destroys it on exit. While `actionsEnabled` is false, methods return without spawning.

- [ ] Implement the standard popout shim with `IpcHandler target: "boltsnapRecording"` and `open/show/hide/toggle`. It owns only opening state and delegates to `bar.openBoltsnapRecording(centerX)`.

- [ ] Implement content without a Rectangle/card background. Expose `contentHeight` from a compact `ColumnLayout`; show state, frozen/running time, scope/output summary, Pause/Resume, Shelf Save, Disk Save, and destructive Discard. Bind every button's disabled state to `actionsEnabled`; show `Saving…` when finalizing. Use existing `Btn`, `Theme`, and `Anim` through `import ".."`.

- [ ] Register all three components in root `qmldir` with their folder paths.

- [ ] Run:

```bash
cd /home/mt/.config/quickshell/mono
qmllint -I /usr/lib/qt6/qml -I /home/mt/.config/quickshell/mono \
  services/BoltsnapRecordingService.qml \
  popouts/BoltsnapRecordingPopout.qml \
  popouts/BoltsnapRecordingContent.qml
```

Expect no syntax/type errors from the new files.

- [ ] Run `qmltestrunner -input tests/tst_BoltsnapRecordingService.qml -import /usr/lib/qt6/qml -import /home/mt/.config/quickshell/mono` and expect `Totals: 4 passed, 0 failed, 0 skipped, 0 blacklisted` (init/cleanup plus the two test functions).

- [ ] Commit in the Quickshell repository with `git commit -m "feat: consume boltsnap recording IPC"`.

---

### Task 10: Put the Quickshell widget immediately left of VPN

**Repository:** `/home/mt/.config/quickshell/mono`

**Files:**

- Modify: `shell.qml`
- Modify: `bar/Bar.qml`

**Interfaces:**

- Consumes: the three registered QML components from Task 9 and existing Bar morph helpers.
- Produces: the visible recording module and Bar-owned recording popout; no Boltsnap-specific state is added to `QuietService`.

- [ ] Before editing, run the ordering check below and expect a non-zero exit because `boltsnapRecordingModule` does not exist yet.

```bash
cd /home/mt/.config/quickshell/mono
test "$(rg -n 'id: boltsnapRecordingModule' bar/Bar.qml | cut -d: -f1)" \
  -lt "$(rg -n 'id: vpnModule' bar/Bar.qml | cut -d: -f1)"
```

- [ ] Instantiate `BoltsnapRecordingService` and `BoltsnapRecordingPopout` in `shell.qml`; inject both into `Bar`. Wire the shim's `bar` reference through Bar's existing `Component.onCompleted` block.

- [ ] Extend the bar popout state machine with `openBoltsnapRecording`, width `360`, fallback height `220`, the `rawActivePopoutHeight` case, and a lazy `Loader` for `BoltsnapRecordingContent` using the exact existing opacity/scale/transform/x/y bindings.

- [ ] Replace the old anonymous `quietService.recordingActive` pulsing-dot item with a `Module` immediately before `vpnModule`:

```qml
visible: barRoot.boltsnapRecordingService?.active ?? false
label: barRoot.boltsnapRecordingService?.state === "finalizing"
    ? "Saving…"
    : "●  " + (barRoot.boltsnapRecordingService?.elapsedText ?? "00:00")
fgColor: barRoot.boltsnapRecordingService?.state === "recording"
    ? Theme.error
    : Theme.warning
```

Recording is red; paused is dim amber with frozen time; finalizing says `Saving…`. Clicking toggles the recording popout. Do not alter `QuietService`; it still detects other recorders for auto-quiet.

- [ ] Add a `Connections` binding that closes the recording popout when the service becomes idle. Do not hide or modify recording state when the user merely closes the popout.

- [ ] Run `qmllint` over the two new components plus `shell.qml` and `bar/Bar.qml`. Then allow hot reload; do not kill Quickshell. Find its PID with `pgrep -af "qs.*-c mono" | grep -v zsh` and inspect `qs log --pid <pid> | tail -40` for `Configuration Loaded` and no binding errors.

- [ ] Re-run the exact ordering check above and expect exit status 0.

- [ ] Start a recording from the Boltsnap tray and verify the widget is directly left of VPN, increments once per second, freezes amber on pause, exposes all four controls, shows `Saving…` during finalization, and disappears only after idle. Stop Quickshell temporarily and verify Boltsnap recording/tray operation continues independently; restart through the user's normal service, not `pkill`.

- [ ] Commit in the Quickshell repository with `git commit -m "feat: add boltsnap recording widget"`.

---

### Task 11: Run end-to-end storage, duration, and regression validation

**Files:**

- Modify: `README.md`
- Modify: `docs/specs/2026-07-12-recording-control-design.md` only if implementation details intentionally differ; record the final behavior, not a changelog.

**Interfaces:**

- Consumes: the completed Rust and Quickshell behavior from Tasks 1–10.
- Produces: a verified release candidate and final user-facing documentation; no new runtime API is introduced here.

- [ ] Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`. Fix only recording-related warnings/regressions; preserve unrelated working-tree changes.

- [ ] Run a fake-tool lifecycle test covering start → pause → resume → Shelf Save and start → pause → Disk Save. Assert recorder children are reaped, one-segment saves skip FFmpeg, pause saves use `-c copy`, temporary inputs disappear only after success, and failed finalization remains retryable.

- [ ] Record a five-minute single-monitor clip and inspect `ps`, playback smoothness, resolution, frame pacing, elapsed time, and daemon responsiveness. Confirm no per-frame IPC or shell process spawning occurs.

- [ ] Validate the storage matrix:

| Action | Final path | Shelf card | On card close |
|---|---|---|---|
| Shelf Save | Boltsnap cache | yes, temporary | file deleted |
| Disk Save + toggle on | `record_dir` | yes, same permanent path | file retained |
| Disk Save + toggle off | `record_dir` | no | file retained |
| Discard | none | no | all cache segments deleted |
| Failed save | cache segments retained | no new card | retry/discard available |

- [ ] Restart the daemon after a deliberately abandoned cache fixture and verify only Boltsnap-created cache files are cleaned; place a permanent video in `record_dir` and verify it is untouched.

- [ ] Re-test screenshots on both monitors and Eddy open/replace flows so the pre-existing output-aware IPC work is not regressed.

- [ ] Update README keybinding, CLI, tray, config, save semantics, pause behavior, and Quickshell integration sections. State clearly that combined dual-monitor mode alone re-encodes and targets visually lossless quality.

- [ ] Commit Boltsnap documentation/final fixes with `git commit -m "docs: document recording control workflow"`.

## Final acceptance checklist

- [ ] Alt+Print is a state-aware toggle: selector when idle, controls otherwise.
- [ ] The selector frame toggle and every tray setting survive daemon restart.
- [ ] The permanent recording pill is gone; the frame is the only optional persistent overlay.
- [ ] Pause excludes paused wall time and concatenates compatible segments with stream copy.
- [ ] Separate Both mode creates two clips; Combined creates one correctly arranged clip.
- [ ] Shelf Save is temporary; Disk Save is permanent; permanent shelf cards never duplicate or delete the disk file.
- [ ] Watch IPC is nonblocking and Quickshell is only a client.
- [ ] Tray recording continues without Quickshell.
- [ ] All Rust tests, clippy, qmllint, real tray checks, and manual multi-monitor checks pass.
