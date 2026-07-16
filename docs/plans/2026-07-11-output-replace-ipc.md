# Output-Aware Replace IPC Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route new shelf cards to the capture output and let Eddy replace an existing image or video card immediately on save.

**Architecture:** Extend the existing framed Unix-socket protocol without changing its envelope. `add` gains an optional `output`; `replace` uses a PNG payload for images and a filesystem path for videos. Boltsnap passes a card id when launching Eddy, and Eddy posts the replacement after a successful save.

**Tech Stack:** Rust 2024, Unix sockets/JSON, Qt 6/C++, existing Cargo and CTest suites.

## Global Constraints

- Keep old `add` frames valid when `output` is absent.
- Add no dependencies.
- Keep video transfer path-based; never load full video bytes into IPC memory.
- Reject replacement media that does not match the target card kind.

---

### Task 1: Output-aware Boltsnap `add`

**Files:**
- Modify: `src/ipc.rs`
- Modify: `src/capture.rs`
- Modify: `src/main.rs`
- Modify: `src/shelf/mod.rs`

**Interfaces:**
- Produces: `Request::Add { source, png, output: Option<String> }`.
- Produces: capture result `(Backend, Option<String>)` where the string is the selected Wayland output.

- [x] Add IPC round-trip tests proving `output: Some("DP-3")` survives and a legacy header without `output` decodes to `None`.
- [x] Run `cargo test ipc::tests` and verify the new test fails because `Request::Add` has no `output` field.
- [x] Add the optional field, return the capture output, and make the daemon prefer that name over its own focused-output lookup.
- [x] Run `cargo test` and verify all tests pass.

### Task 2: Boltsnap replacement handling

**Files:**
- Modify: `src/ipc.rs`
- Modify: `src/shelf/model.rs`
- Modify: `src/shelf/mod.rs`
- Modify: `src/editor.rs`

**Interfaces:**
- Produces: `Replacement::Image(Vec<u8>) | Replacement::Video(PathBuf)`.
- Produces: `Request::Replace { id: u64, media: Replacement }`.
- Consumes: `--boltsnap-card-id <id>` supported by Eddy in Task 3.

- [x] Add image/video replacement round-trip tests and model path replacement tests.
- [x] Run the focused Rust tests and verify they fail on the missing variants/method.
- [x] Implement image overwrite + thumbnail refresh and video path update + asynchronous first-frame refresh.
- [x] Pass the card id when Boltsnap opens Eddy and keep the old exit-time reload as compatibility fallback.
- [x] Run `cargo test` and verify all tests pass.

### Task 3: Eddy output and replacement client

**Files:**
- Modify: `/home/mt/projects/eddy/src/boltsnapipc.h`
- Modify: `/home/mt/projects/eddy/src/boltsnapipc.cpp`
- Modify: `/home/mt/projects/eddy/src/cli.h`
- Modify: `/home/mt/projects/eddy/src/cli.cpp`
- Modify: `/home/mt/projects/eddy/src/editorwindow.cpp`
- Modify: `/home/mt/projects/eddy/tests/test_boltsnapipc.cpp`
- Modify: `/home/mt/projects/eddy/tests/test_cli.cpp`

**Interfaces:**
- Consumes: `--boltsnap-card-id <u64>`.
- Produces: `add` headers with optional `output` and `replace` frames matching Task 2.

- [x] Add CLI and frame tests for card id, output-aware add, image replacement, and video replacement.
- [x] Run `ctest --test-dir build -R 'test_(cli|boltsnapipc)' --output-on-failure` and verify failures.
- [x] Implement the minimal builders/senders and post a replacement only after a successful explicit save.
- [x] Run the focused tests, then the full Eddy CTest suite.

### Task 4: Cross-repo verification

**Files:**
- Verify only; no new files.

- [x] Run `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` (strict Clippy exposes the repository's existing warning backlog).
- [x] Build Eddy, run `ctest --test-dir build --output-on-failure`, and run `git diff --check` in both repos (one unrelated Qt 6.11 toolbar geometry assertion remains red).
- [x] Inspect both diffs for protocol agreement and unrelated changes.
