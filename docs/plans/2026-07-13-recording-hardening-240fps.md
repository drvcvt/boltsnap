# Recording Hardening and 240 FPS Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make BoltSnap record native-resolution 240 FPS video by default while preventing shelf-save overwrite/duplication, disk exhaustion, orphaned recorders, unbounded shutdown, malformed IPC allocation, and detached-daemon split ownership.

**Architecture:** Keep the existing `wf-recorder` segment and background-finalization design. Add small pure helpers at the current shared boundaries (`record` argv, `record::finalize` file promotion/free-space query, `record::session` child lifecycle, `ipc` decoding), then call them from the daemon event loop without blocking it.

**Tech Stack:** Rust stdlib, libc, existing wf-recorder/FFmpeg, calloop, existing systemd user unit; no new dependencies.

## Global Constraints

- Native capture resolution and constant 240 FPS are the default.
- Default `h264_nvenc` quality is preset `p5`, tune `hq`, VBR with CQ 16.
- Ordinary pause/resume concatenation remains `-c copy`.
- The Wayland/calloop thread never waits for recorder shutdown, FFmpeg, or video file copies.
- Keep at least 2 GiB free on the recording-cache filesystem.
- No user file is overwritten and no temporary source is deleted before verified promotion success.

---

### Task 1: Capture profile

**Files:**
- Modify/Test: `src/record.rs`

**Interfaces:**
- Produces: `wf_recorder_args` and `wf_recorder_output_args` containing the shared 240 FPS profile.

- [ ] Add assertions that both argv builders contain `-r 240` and, for `h264_nvenc`, repeated `-p` values `preset=p5`, `tune=hq`, `rc=vbr`, and `cq=16`.
- [ ] Run `cargo test --bin boltsnap record::tests::wf_ -- --nocapture`; expect the new assertions to fail.
- [ ] Add one private `capture_profile_args(codec: &str) -> Vec<String>` and chain it into both existing builders. Non-NVENC codec overrides receive only `-r 240`.
- [ ] Re-run the focused tests; expect pass.

### Task 2: Disk reserve

**Files:**
- Modify/Test: `src/record/finalize.rs`
- Modify: `src/shelf/mod.rs`

**Interfaces:**
- Produces: `pub const RECORDING_DISK_RESERVE_BYTES: u64` and `pub fn available_space(path: &Path) -> Result<u64, String>`.
- Consumes: the existing 250 ms daemon tick and recording start functions.

- [ ] Add a test that `available_space` returns a positive value for a real temporary directory.
- [ ] Run the test and verify it fails because the helper does not exist.
- [ ] Move the existing `statvfs` calculation into `available_space`; retain `ensure_free_space` on top of it.
- [ ] Add start preflight checks and a once-per-second field/tick check. At reserve exhaustion, route through the existing `AfterStop::Recover` path with an explicit disk-space error.
- [ ] Run recording/finalize/shelf tests; expect pass.

### Task 3: Bounded recorder lifecycle and diagnostics

**Files:**
- Modify/Test: `src/record/session.rs`
- Modify/Test: `src/record/finalize.rs`

**Interfaces:**
- Produces: bounded `StopChildrenJob::wait`, Linux parent-death handling in `spawn_segment`, and FFmpeg error text containing concise stderr.

- [ ] Add a fake recorder test that ignores SIGINT and exits on SIGTERM, using test-only short deadlines, and assert `wait` completes.
- [ ] Run it and verify the current unbounded implementation fails/times out.
- [ ] Poll `try_wait`; after the graceful deadline send SIGTERM, then SIGKILL after the kill deadline. Preserve non-empty output regardless of non-success status while reporting the forced stop.
- [ ] In the child `pre_exec`, set `PR_SET_PDEATHSIG=SIGTERM` and reject a changed parent race. Inherit recorder stderr.
- [ ] Add an FFmpeg failure test whose fake executable writes a marker to stderr; change execution to `-hide_banner -loglevel error` plus captured output and include bounded stderr in the error.
- [ ] Run focused lifecycle/finalize tests; expect pass.

### Task 4: Safe asynchronous shelf promotion

**Files:**
- Modify/Test: `src/record/finalize.rs`
- Modify/Test: `src/shelf/model.rs`
- Modify: `src/shelf/mod.rs`

**Interfaces:**
- Produces: `pub fn promote_recording(source: &Path, dir: &Path, output: Option<&str>) -> Result<PathBuf, String>` and `ShelfModel::promote(id, path)`.
- Produces daemon event: `CardPromoted { id, result }`.

- [ ] Add tests proving promotion never overwrites an existing same-second-style destination, removes the temporary source only after success, and changes a model card to permanent exactly once.
- [ ] Run focused tests and verify failure.
- [ ] Reuse `unique_recording_path` plus the existing no-replace move/cross-filesystem copy helper; do not add a second copy implementation.
- [ ] Replace synchronous `save_card` copy with one worker thread and handle completion on `DaemonEvent`; update path/lifetime only on success.
- [ ] Run model/finalize/shelf tests; expect pass.

### Task 5: IPC and daemon ownership

**Files:**
- Modify/Test: `src/ipc.rs`
- Modify/Test: `src/shelf/mod.rs`

**Interfaces:**
- Frame limits: 64 KiB header, 256 MiB payload.
- `ensure_daemon` first invokes `systemctl --user start boltsnap-daemon.service`, then retains the detached fallback.

- [ ] Add decoder tests for oversized declared lengths, out-of-range coordinates, and zero width/height.
- [ ] Run them and verify current decoding accepts or attempts allocation.
- [ ] Reject lengths before allocation and parse geometry with checked conversions/non-zero validation.
- [ ] Add a command-construction helper test for the systemd start command; call it before detached spawn and keep fallback behavior.
- [ ] Validate start requests again in the daemon before launching `wf-recorder`.
- [ ] Run IPC/shelf tests; expect pass.

### Task 6: Verification and deployment

**Files:**
- No source changes expected.

- [ ] Run `cargo fmt --check`, `cargo check --all-targets`, and `cargo test --all-targets -- --test-threads=1`.
- [ ] Run the parallel suite repeatedly to verify fake-executable tests no longer mask regressions with `ETXTBSY`.
- [ ] Build release, install the binary, stop any detached daemon/watch leftovers, and start `boltsnap-daemon.service` so systemd owns the deployed process.
- [ ] Verify `systemctl --user is-active boltsnap-daemon.service`, `boltsnap recording status --json`, tray presence, and exactly one current Quickshell watch client.
