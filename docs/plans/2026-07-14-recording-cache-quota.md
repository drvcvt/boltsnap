# Recording Cache Quota Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound Boltsnap's temporary recording cache to 2 GiB without changing recording quality, frame rate, encoding, or direct file attachment behavior.

**Architecture:** Keep the existing disk-backed `rec_dir` and finished-file workflow. Add one filesystem-size check beside the existing free-space reserve, run it before recording starts and once per second while recording, and reuse the current recovery path to pause safely when the quota is reached.

**Tech Stack:** Rust, Cargo unit tests, existing Wayland recording daemon

## Global Constraints

- Temporary recording storage may use at most 2 GiB, apart from the bounded bytes written between one-second checks.
- Do not transcode, copy, resize, or lower the frame rate to enforce the quota.
- Do not silently delete files backing visible shelf cards.
- Preserve all pre-existing uncommitted work; do not stage or commit from this dirty worktree.

---

### Task 1: Enforce the temporary recording cache quota

**Files:**
- Modify: `src/record/finalize.rs`
- Modify: `src/shelf/mod.rs`
- Test: `src/record/finalize.rs`

**Interfaces:**
- Consumes: `crate::paths::rec_dir()`, the existing recording start checks, and the existing once-per-second recovery path.
- Produces: `check_recording_cache_limit(dir: &Path) -> Result<(), String>` and `RECORDING_CACHE_LIMIT_BYTES: u64`.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn recording_cache_accepts_files_below_two_gibibytes() {
    let dir = temp_dir("cache-below-limit");
    file(&dir.join("clip.mp4"), b"clip");
    assert!(check_recording_cache_limit(&dir).is_ok());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recording_cache_rejects_total_at_limit() {
    assert!(require_recording_cache_limit(RECORDING_CACHE_LIMIT_BYTES).is_err());
    assert!(require_recording_cache_limit(RECORDING_CACHE_LIMIT_BYTES - 1).is_ok());
}
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test recording_cache_`

Expected: compilation fails because the cache-quota functions and constant do not exist yet.

- [x] **Step 3: Write the minimal implementation**

```rust
pub const RECORDING_CACHE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn check_recording_cache_limit(dir: &Path) -> Result<(), String> {
    let bytes = fs::read_dir(dir)
        .map_err(|error| format!("read recording cache: {error}"))?
        .try_fold(0_u64, |total, entry| {
            let entry = entry.map_err(|error| format!("read recording cache entry: {error}"))?;
            let metadata = entry
                .metadata()
                .map_err(|error| format!("inspect recording cache entry: {error}"))?;
            Ok::<_, String>(total.saturating_add(metadata.is_file().then_some(metadata.len()).unwrap_or(0)))
        })?;
    require_recording_cache_limit(bytes)
}

fn require_recording_cache_limit(bytes: u64) -> Result<(), String> {
    if bytes < RECORDING_CACHE_LIMIT_BYTES {
        Ok(())
    } else {
        Err(format!("recording paused because the temporary recording cache reached 2 GiB ({bytes} bytes used); save or dismiss shelf recordings to free space"))
    }
}
```

Import `check_recording_cache_limit` in `src/shelf/mod.rs`, call it from `prepare_recording_cache`, and run it before `check_recording_reserve` in the existing once-per-second recording-space check.

- [x] **Step 4: Run focused and full tests**

Run: `cargo test recording_cache_`

Expected: both cache-quota tests pass.

Run: `cargo test`

Expected: the complete test suite passes.

- [x] **Step 5: Review without staging user work**

Run: `git diff --check && git diff -- src/record/finalize.rs src/shelf/mod.rs docs/plans/2026-07-14-recording-cache-quota.md`

Expected: no whitespace errors; the diff contains only the quota test, quota helper, and two integration hooks in addition to the user's pre-existing edits.
