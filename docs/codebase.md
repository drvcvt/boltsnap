# Codebase inventory

Verified source baseline: `eaf0b13` (2026-09-25), version 1.0.1 in
[`Cargo.toml`](../Cargo.toml). This is the maintained navigation and status map;
module declarations and manifests remain authoritative. Dated plans and benchmark
reports describe their own revisions, not the current acceptance state.

`Integrated` means the runtime/build path exists. `Automated green` means the
checks named below passed for this baseline. `Live accepted` requires a recorded
real interaction; automated media or offscreen GL tests do not establish it.
Linux is supported. Windows remains experimental, unmaintained and frozen.

## Runtime and build owners

| Component | Canonical files and consumers | Boundary and limitations |
| --- | --- | --- |
| CLI and configuration | [`main.rs`](../src/main.rs), [`config.rs`](../src/config.rs) dispatch capture, daemon, recording and replay commands | Main binary; selects native implementations through `platform` |
| Portable library | [`lib.rs`](../src/lib.rs) exports [`image.rs`](../src/image.rs), [`protocol.rs`](../src/protocol.rs), [`replay/`](../src/replay/mod.rs) | Image geometry, IPC framing/states, replay settings/ring/time/crop/wire; not the entire application |
| Platform registration | [`platform/mod.rs`](../src/platform/mod.rs), [`linux/mod.rs`](../src/platform/linux/mod.rs), [`windows/mod.rs`](../src/platform/windows/mod.rs) | Module-level target selection; native dependencies are target-specific in Cargo |
| Screenshots | [`linux/capture.rs`](../src/platform/linux/capture.rs), [`portal.rs`](../src/platform/linux/portal.rs), [`vendor/libway`](../vendor/README.md) | Integrated: Wayland EXT/WLR CPU capture plus scoped portal fallback; X11 via x11rb. No X11 area selector or shelf |
| Frozen desktop selector | [`selector/`](../src/selector/mod.rs), [`linux/select_skia/`](../src/platform/linux/select_skia/mod.rs) | Shared geometry/rendering; Linux Wayland input and SHM surfaces. Screenshot regions span outputs; recording/replay regions use one target output |
| Shelf | [`shelf/`](../src/shelf/mod.rs), [`linux/shelf/`](../src/platform/linux/shelf/mod.rs), [`tray.rs`](../src/platform/linux/tray.rs) | Shared model/layout/paint; Linux eventloop owns cards, controls, thumbnail jobs and recorder/replay lifecycle. RAM-only card state. The shelf layer surface exists only while it shows a card ([`surface.rs`](../src/platform/linux/shelf/surface.rs), [plan](plans/2026-07-31-shelf-hardening.md#task-5-no-shelf-surface-while-the-shelf-is-empty-2026-10-22-implemented-2026-10-23-live-open)) |
| Image transport and clipboard | [`protocol.rs`](../src/protocol.rs), [`linux/ipc.rs`](../src/platform/linux/ipc.rs), [`image_transfer.rs`](../src/platform/linux/image_transfer.rs), [`clipboard.rs`](../src/platform/linux/clipboard.rs) | Integrated: normal PNG plus CLI thumbnail (`add_thumb`); raw RGB memfd path is opt-in via `BOLTSNAP_RAW_TRANSFER=1`. Each screenshot is sent once; a daemon older than `add_thumb` needs a restart |
| Recording and saves | [`record.rs`](../src/record.rs), [`record/session.rs`](../src/record/session.rs), [`finalize.rs`](../src/record/finalize.rs), [`audio.rs`](../src/record/audio.rs), [`linux/gsr.rs`](../src/platform/linux/gsr.rs), [`files.rs`](../src/platform/linux/files.rs) | Integrated Linux lifecycle, audio, pause/concat/compose, cache and no-replace video publication. Session/audio/finalize/sidecar modules are Linux-gated despite their `src/record/` location |
| Cursor | [`record/cursor.rs`](../src/record/cursor.rs), [`cursor_motion.rs`](../src/record/cursor_motion.rs), [`cursor_sidecar.rs`](../src/record/cursor_sidecar.rs), [`linux/cursor_track.rs`](../src/platform/linux/cursor_track.rs) | Integrated: EXT pointer feed, portable spring/frame clock and raw sidecars; plugin burns an arrow into video. No cursor-free copy or shape tracking |
| Cursor plugin | [`gsr_cursor/Cargo.toml`](../src/platform/linux/gsr_cursor/Cargo.toml), [`src/lib.rs`](../src/platform/linux/gsr_cursor/src/lib.rs) | Separate Linux cdylib, directly includes shared `cursor_motion.rs`; ordinary root Cargo commands do not build/test it |
| Replay | [`linux/replay/mod.rs`](../src/platform/linux/replay/mod.rs), [`worker/src/main.rs`](../src/platform/linux/replay/worker/src/main.rs), [`worker/Cargo.toml`](../src/platform/linux/replay/worker/Cargo.toml) | Integrated experimental Hyprland supervisor plus separate FFmpeg worker. Worker includes portable replay modules; root Cargo tests do not cover its media pipeline |
| Windows backend | [`windows/mod.rs`](../src/platform/windows/mod.rs), [`smoke checklist`](windows-smoke-test.md) | Integrated experimental native capture, IPC, shelf, recording and tray code; no current live acceptance or release artifacts. Gaps remain in the frozen [porting plan](../PORTING_PLAN.md#implementierungsstand) |

The normal recording path uses gsr when installed, otherwise wf-recorder. gsr
writes Matroska segments, which are remuxed to MP4 even for a single segment.
Matroska stores frame times in whole milliseconds; the remux of gsr segments
uses the recording's frame rate as video timescale, which puts every frame
back on its exact slot ([`build_concat_args`](../src/record/finalize.rs)).
The direct-move optimization applies to an existing single MP4. Compatible
pause segments use stream copy; multi-stream Combined composition re-encodes.
The stop wait finishes a gsr segment on gsr's saved-path stdout line and reaps
the process in the background ([`StopChildrenJob`](../src/record/session.rs)).

Build registration is in [CI](../.github/workflows/ci.yml),
[release packaging](../.github/workflows/release.yml) and
[`flake.nix`](../flake.nix). CI separately registers root Linux/Windows tests,
libway consumer tests, the replay worker and the cursor plugin. This is coverage
configuration, not a claim that remote CI ran in this review.
The Nix package includes the cursor plugin; tarball/Debian packaging currently
includes neither plugin nor replay worker. The Nix package reads its version
from `Cargo.toml`.

## Open review findings

All fifteen defects from the 2026-09-25 reviews are fixed in code, one commit
each (A1 in `a430ceb`, the rest from `da75eda` to `bf784fc`). The
[fix plan](plans/2026-09-25-review-fixes.md) is the canonical record: causes,
decisions, what each fix does, the tests that cover it and the checks that
remain. The build of `b11e5b2` is installed and running. Live-checked so far:
a second daemon start exits without touching the running one (G1). The other
live checks in the plan's section 5 are open.

| ID | Fixed behavior |
| --- | --- |
| G1 | [`run_daemon`](../src/platform/linux/shelf/mod.rs) takes an `flock` on `boltsnap.lock` before log rotation and cleanup; only a real socket is removed as stale |
| A2 | Image saves publish through [`copy_noreplace`](../src/platform/linux/files.rs) under a fresh `-N` name; existing files and symlinks stay |
| A3 | A card being saved stays until the save ends; close waits for success, eviction skips it ([`shelf/save.rs`](../src/platform/linux/shelf/save.rs)) |
| E1 | After a failed save the recording offers Save and Discard only; the popup reads SAVE FAILED ([`session.rs`](../src/record/session.rs)) |
| A4 | Saved cards keep their container extension; drags offer the file's MIME type; a cancelled video drag never reaches the image clipboard |
| B1 | Video import copies the checked file handle into the cache instead of hard-linking the path |
| C1 | [`add_to_shelf`](../src/platform/linux/ipc.rs) sends once; a lost answer is reported with the path of `last.png`. An older daemon needs a restart |
| D1 | Drag transfers stream the file |
| D2 | Image saves run off the event loop |
| G2 | Startup cleanup removes `clip-*.{mp4,mkv}.<pid>.<seq>.partial` remux leftovers |
| G3 | [`process::output_with_interval`](../src/platform/linux/replay/process.rs) reads both pipes with `poll` under one deadline |
| G4 | Closed recording watchers are dropped before each new watch; at most 16 stay open |
| G5 | The worker reports `selection_active`; watchdog and manual Start leave an open selection alone |
| H1 | [`flake.nix`](../flake.nix) reads the package version from `Cargo.toml` (not evaluated: no Nix here) |

A1 is **Integrated / Automated green**: [`files::copy_noreplace`](../src/platform/linux/files.rs)
uses PID/sequence part names, `create_new`, sync-before-publish and no-replace
rename. Tests cover concurrent copies, another operation's partial file, read
failure, non-UTF-8 paths and the finalizer's copy/move behavior. It fixes part
file collisions; image overwrites and card lifetime races were A2 and A3. A real shelf Save
to another physical disk is still owed.

F1 is a behavior decision, not an approved fix: [`Sources::sample` and
`Spring::apply`](../src/record/cursor_motion.rs) reset the spring when Leave
arrives before the next tracker becomes visible. Seamless monitor handover is
not guaranteed. The README now states that limitation.

The [recording and replay review of 2026-10-12](plans/2026-10-12-recording-reliability-review.md)
is the record for its findings. Fixed in code, not yet checked live:
[`record/recover.rs`](../src/record/recover.rs) saves segments a dead daemon
left; a closed shelf output no longer ends the daemon;
[`shelf/watch.rs`](../src/platform/linux/shelf/watch.rs) pauses on a stalled
gsr segment and keeps room to remux; disk saves remux into `record_dir`; replay
captures at a constant bitrate sized to its buffer, picks its codec through
`gsr --info`, and runs exports at background priority. The plan's section 4
lists what is still open, including the recording feedback in wave 3.

## Partial and deferred work

- Raw image IPC is Integrated but opt-in; its full-path performance did not
  justify making it the default. See [measurements](screenshot-performance.md).
- libway GPU capture is a library capability, not an integrated Boltsnap
  recording backend. Screenshots use CPU buffers; recording uses external
  recorders. See [libway integration](libway.md).
- GPU-only replay crop is Unavailable for release use: its recorded pixel
  comparison failed. CPU crop plus the selected encoder remains active.
- gsr stdout-before-exit saving is Integrated (`cf243b5`) and was measured once
  live: 145 ms from Save to card for a 14 s 120 FPS recording, previously
  0.33-0.40 s. Direct libway RGB canvas remains planned and unimplemented. See
  the [performance plan](plans/2026-09-24-performance-save-and-screenshot.md).
- Exact frame times for saved recordings are Integrated (`b11e5b2`) and live
  accepted by the user (cursor reported clean, 1684 frames on the 8.333 ms
  grid). Replay clips are MP4 with the capture's exact frame times (`1/fps`
  time base kept, see [replay](replay.md)); synthetic worker tests check the
  grid, live replay saves are not yet checked. Occasional stalls during playback in Eddy (separate repository,
  Qt Multimedia) are reported by the user and not investigated.
- The review fixes above are code-complete; their live checks are open except
  G1. Policy decisions are recorded in the fix plan. Windows work remains frozen.

## Verification at this baseline

On `eaf0b13`: `cargo fmt --check`, `nice -n 19 cargo test --locked -j 4`
(**23 library + 322 binary tests**, three ignored) and
`cargo clippy --locked --all-targets` without warnings. The ignored FFmpeg test
`remuxed_gsr_segments_keep_exact_frame_times` passed on `b11e5b2`. The replay
worker passed `cargo fmt --check`, `cargo test --locked` (24), a debug build,
`tests/probe.py`, `tests/live.py` and the G5 freeze repro on `aed223f`. Release
builds of `boltsnap` (`b11e5b2`) and the replay worker (`abe1363`) are
installed in `~/.cargo/bin`; the cursor plugin and `boltsnap-background` are
unchanged since `c8cf56b`. Not run: Windows target, Nix, cursor plugin crate.

Earlier, all deeper-review builds used an isolated archive snapshot with separate target
outputs because `src/record/session.rs` was being edited concurrently. On
`cf243b5`, `cargo fmt --check` and `cargo test --locked -j 4` passed: **23 library
+ 303 binary tests**, two ignored. A worker debug build also passed. New G2/G3
Rust repros and the G5 synthetic worker repro then failed as expected; see the
[recorded evidence](plans/2026-09-25-review-fixes.md#6-vertieftes-review-und-tatsächlich-ausgeführte-checks).
No product source was changed for those repros in the shared working tree.

The following supplementary checks passed earlier on `a430ceb`, 2026-09-25.
They are last-recorded results, not a claim of rerunning them on the newer commit:

| Check actually run | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo test --locked` | 23 library + 299 binary tests passed; two ignored by default |
| `cargo check --locked` | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo test --locked synthetic_video_produces_a_small_rgba_thumbnail -- --ignored` | One synthetic FFmpeg test passed |
| `python3 tools/sync-libway.py --check` | Snapshot hashes verified; library/consumer suites not rerun |
| Cursor plugin: separate manifest fmt, `cargo test --locked`, then `cargo test --locked -- --ignored` | 19 standard + two offscreen EGL/GL tests passed |
| Replay worker: separate manifest fmt, `cargo test --locked`, `cargo build --locked` | 22 tests and debug build passed; existing unused `output_setup` warning remains |
| Worker `tests/probe.py` and `tests/live.py` with the freshly built debug worker | Synthetic decode, frame/audio identity, budgets, overwrite protection, B-frame rejection, pipe input, freeze/crop, concurrent status, export exclusion and stale IDs passed |

Root logs are `/tmp/boltsnap-review-20260925-{tests,check,clippy}.log`; companion
and media logs share that prefix. Synthetic media artifacts are
`/tmp/boltsnap-replay-test.t35afzoz` and `/tmp/boltsnap-replay-live.9fvu5bjm`.
These local temporary artifacts are not part of the repository.

No live desktop capture, daemon replacement, installation, Windows tests,
release build, Nix build or long-running soak was performed in that deep review.
The ignored `live_smooth_cursor_recording` test was not run. Dated
[cursor implementation notes](plans/2026-09-24-live-cursor-plugin.md#5-implementation-notes)
record user acceptance of area cursor playback and narrower live checks of
scaled output/replay/Combined capture. They do not establish current blanket
live acceptance. Monitor transitions, save error recovery, slow-disk Save/Close,
broader compositor/hardware coverage and playback presentation timing remain
open. [Replay validation](replay.md#verification-and-limits) records its own
limited native fullscreen run.
