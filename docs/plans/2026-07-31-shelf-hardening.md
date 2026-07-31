# Shelf Hardening Implementation Plan

**Goal:** Close the five known shelf edge cases without changing the five-card viewport or the Linux capture UX.

**Approach:** Reuse the existing per-client IPC reader thread for PNG preparation, acknowledge `Add` only after the prepared card reaches the UI model, cap the hidden model at 20 cards and 256 MiB of temporary image data, use unique working capture paths while atomically retaining `last.png`, and use the animated layout for both rendering and pointer input.

## Task 1: Bound shelf resources and acknowledge ingestion

**Files:** `src/platform/linux/shelf/mod.rs`, `src/main.rs`

- [x] Add failing tests for count/byte eviction, invalid PNG preparation, and prepared add events.
- [x] Prepare PNGs in the existing client reader thread and send the result back through `DaemonEvent`.
- [x] Send `Response::ok` only after model insertion; return `Response::error` for decode/write failures.
- [x] Evict oldest hidden cards above 20 total cards or 256 MiB of temporary image PNGs, deleting only temporary files.
- [x] Make the capture CLI wait for and validate the daemon response.

## Task 2: Remove the default capture path race

**Files:** `src/platform/linux/paths.rs`, `src/main.rs`

- [x] Add failing tests proving default working paths are unique and `last.png` remains stable.
- [x] Capture to a unique cache path, consume that exact file, then atomically publish its bytes as `last.png`.
- [x] Remove the working file after synchronous consumers finish.

## Task 3: Keep interaction aligned with animation

**File:** `src/platform/linux/shelf/mod.rs`

- [x] Add failing tests for two simultaneous dismissals and hit-testing a moving card.
- [x] Render enough overflow cards for every active visible dismissal.
- [x] Hit-test hover, left press, and right press against the current animated layout.

## Task 4: Verify

- [x] Run focused shelf/path tests.
- [x] Run `cargo fmt --check`, `cargo test`, and a Linux `cargo check`.
- [x] Review `git diff --check` and the final focused diff.
