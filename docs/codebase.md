# Codebase inventory

Verified source baseline: `cf243b5` (2026-09-25), version 1.0.1 in
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
| Shelf | [`shelf/`](../src/shelf/mod.rs), [`linux/shelf/`](../src/platform/linux/shelf/mod.rs), [`tray.rs`](../src/platform/linux/tray.rs) | Shared model/layout/paint; Linux eventloop owns cards, controls, thumbnail jobs and recorder/replay lifecycle. RAM-only card state |
| Image transport and clipboard | [`protocol.rs`](../src/protocol.rs), [`linux/ipc.rs`](../src/platform/linux/ipc.rs), [`image_transfer.rs`](../src/platform/linux/image_transfer.rs), [`clipboard.rs`](../src/platform/linux/clipboard.rs) | Integrated: normal PNG plus CLI thumbnail (`add_thumb`); raw RGB memfd path is opt-in via `BOLTSNAP_RAW_TRANSFER=1`. EOF retry issue C1 remains |
| Recording and saves | [`record.rs`](../src/record.rs), [`record/session.rs`](../src/record/session.rs), [`finalize.rs`](../src/record/finalize.rs), [`audio.rs`](../src/record/audio.rs), [`linux/gsr.rs`](../src/platform/linux/gsr.rs), [`files.rs`](../src/platform/linux/files.rs) | Integrated Linux lifecycle, audio, pause/concat/compose, cache and no-replace video publication. Session/audio/finalize/sidecar modules are Linux-gated despite their `src/record/` location |
| Cursor | [`record/cursor.rs`](../src/record/cursor.rs), [`cursor_motion.rs`](../src/record/cursor_motion.rs), [`cursor_sidecar.rs`](../src/record/cursor_sidecar.rs), [`linux/cursor_track.rs`](../src/platform/linux/cursor_track.rs) | Integrated: EXT pointer feed, portable spring/frame clock and raw sidecars; plugin burns an arrow into video. No cursor-free copy or shape tracking |
| Cursor plugin | [`gsr_cursor/Cargo.toml`](../src/platform/linux/gsr_cursor/Cargo.toml), [`src/lib.rs`](../src/platform/linux/gsr_cursor/src/lib.rs) | Separate Linux cdylib, directly includes shared `cursor_motion.rs`; ordinary root Cargo commands do not build/test it |
| Replay | [`linux/replay/mod.rs`](../src/platform/linux/replay/mod.rs), [`worker/src/main.rs`](../src/platform/linux/replay/worker/src/main.rs), [`worker/Cargo.toml`](../src/platform/linux/replay/worker/Cargo.toml) | Integrated experimental Hyprland supervisor plus separate FFmpeg worker. Worker includes portable replay modules; root Cargo tests do not cover its media pipeline |
| Windows backend | [`windows/mod.rs`](../src/platform/windows/mod.rs), [`smoke checklist`](windows-smoke-test.md) | Integrated experimental native capture, IPC, shelf, recording and tray code; no current live acceptance or release artifacts. Gaps remain in the frozen [porting plan](../PORTING_PLAN.md#implementierungsstand) |

The normal recording path uses gsr when installed, otherwise wf-recorder. gsr
writes Matroska segments, which are remuxed to MP4 even for a single segment.
The direct-move optimization applies to an existing single MP4. Compatible
pause segments use stream copy; multi-stream Combined composition re-encodes.

Build registration is in [CI](../.github/workflows/ci.yml),
[release packaging](../.github/workflows/release.yml) and
[`flake.nix`](../flake.nix). CI separately registers root Linux/Windows tests,
libway consumer tests, the replay worker and the cursor plugin. This is coverage
configuration, not a claim that remote CI ran in this review.
The Nix package includes the cursor plugin; tarball/Debian packaging currently
includes neither plugin nor replay worker. The Nix package version is still
`0.4.3`, unlike Cargo's `1.0.1`; that packaging metadata is stale.

## Open review findings

Rechecked against `cf243b5`. The [fix plan](plans/2026-09-25-review-fixes.md) is
the canonical issue record, with causes, proposed fixes, implementation sequence
and regression/live checks. Only A1 is implemented; 14 defects remain open.
The deeper review reproduced G2/G3 in an isolated source snapshot and G5 with
synthetic media. Its scripts are versioned next to the plan. Older isolated
repros remain local ignored artifacts; no live daemon races were triggered.

| Priority / ID | Trigger, consequence and current code evidence |
| --- | --- |
| P1 / A2 | Two image-card Saves in the same second choose the same filename. [`save_card`](../src/platform/linux/shelf/mod.rs) calls `fs::copy` without no-replace protection, overwriting the earlier screenshot. [`paths::local_timestamp`](../src/platform/linux/paths.rs) only supplies second precision |
| P1 / A3 | Close-animation completion or cache eviction calls `remove_card_now` while the video Save thread may not yet have opened its source. [`shelf/mod.rs`](../src/platform/linux/shelf/mod.rs) removes the card/file and its `saving_cards` entry; [`Thumb::delete_file_on_dismiss`](../src/shelf/model.rs) deletes temporary media and sidecar. This can lose the only recording before Save starts |
| P1 / E1 | Multi-stream Combined composition succeeds, then Disk Save fails. [`finalize_recording`](../src/record/finalize.rs) returns the composed clip under key `None`; the shelf restores it and [`finalize_failed`](../src/record/session.rs) permits Resume. New named-output groups mix with `None`, so the next `combined_layout` fails with `combined recording is missing monitor layout data` |
| P2 / A4 | [`promote_recording`](../src/record/finalize.rs) always chooses a `.mp4` destination through `unique_recording_path`, even for replay Matroska. Bytes are unchanged. [`begin_drag`](../src/platform/linux/shelf/mod.rs) also advertises all videos as `video/mp4`; its cancelled-drag callback treats only MP4 as video and sends MKV into the image clipboard fallback |
| P2 / B1 | [`prepare_shelf_video`](../src/platform/linux/shelf/mod.rs) validates through `metadata` but then hard-links the supplied path. A relative symlink becomes invalid in the cache directory while import reports success. Regular-file hardlinks also remain sensitive to in-place writes to the original |
| P2 / C1 | [`ipc::add_with`](../src/platform/linux/ipc.rs) retries a mutating `add_thumb` as `add` on every `UnexpectedEof`, including a truncated ACK after successful insertion. That can create two cards for one screenshot |
| P2 / D1 | [`DataSourceHandler::send_request`](../src/platform/linux/shelf/mod.rs) uses `fs::read` for a requested byte transfer. A large video is fully resident in daemon RAM until the receiver finishes; the background thread does not bound memory |
| P2 / D2 | Image [`save_card`](../src/platform/linux/shelf/mod.rs) performs directory creation and copying in the Wayland/IPC eventloop. Slow storage blocks shelf interaction and recording controls |
| P1 / G1 | [`run_daemon`](../src/platform/linux/shelf/mod.rs) removes the socket and cached media after a nonexclusive alive check, before binding. Concurrent starts can remove a newly active daemon's socket and recordings |
| P2 / G2 | [`export::create_partial`](../src/platform/linux/replay/worker/src/export.rs) ends remux partial names in `.partial`; [`is_orphan_rec_file`](../src/platform/linux/paths.rs) only recognizes replay names ending in `.mkv`. Failed partials survive startup and consume the cache quota |
| P2 / G3 | [`process::output_with_interval`](../src/platform/linux/replay/process.rs) joins pipe reader threads without a deadline. A finished child with an output-inheriting descendant bypasses the timeout; reproduced at 1.202 s for a 40-ms limit |
| P2 / G4 | [`RecordingWatch` and `publish_recording_snapshot`](../src/platform/linux/shelf/mod.rs) retain disconnected idle clients until a status change and impose no subscription limit, allowing file descriptors to accumulate |
| P2 / G5 | [`live.rs`](../src/platform/linux/replay/worker/src/live.rs) retains a valid frozen snapshot while reporting `busy=false`; after input failure, [`watchdog`](../src/platform/linux/replay/mod.rs) sees neither ready nor busy and replaces the session despite the held selection |
| P3 / H1 | [`flake.nix`](../flake.nix) hardcodes main package version `0.4.3` while Cargo declares `1.0.1` |

A1 is **Integrated / Automated green**: [`files::copy_noreplace`](../src/platform/linux/files.rs)
uses PID/sequence part names, `create_new`, sync-before-publish and no-replace
rename. Tests cover concurrent copies, another operation's partial file, read
failure, non-UTF-8 paths and the finalizer's copy/move behavior. This fixes part
file collisions, not image overwrites or card lifetime races. A real shelf Save
to another physical disk is still owed.

F1 is a behavior decision, not an approved fix: [`Sources::sample` and
`Spring::apply`](../src/record/cursor_motion.rs) reset the spring when Leave
arrives before the next tracker becomes visible. Seamless monitor handover is
not guaranteed. The README now states that limitation.

## Partial and deferred work

- Raw image IPC is Integrated but opt-in; its full-path performance did not
  justify making it the default. See [measurements](screenshot-performance.md).
- libway GPU capture is a library capability, not an integrated Boltsnap
  recording backend. Screenshots use CPU buffers; recording uses external
  recorders. See [libway integration](libway.md).
- GPU-only replay crop is Unavailable for release use: its recorded pixel
  comparison failed. CPU crop plus the selected encoder remains active.
- gsr stdout-before-exit saving is Integrated in `cf243b5`, with four automated
  stop/reaper tests. The saved-path line can finish a segment before process
  teardown; reaping continues in the background. Current end-to-end latency
  and live acceptance remain unmeasured here. Direct libway RGB canvas remains
  planned and unimplemented. See the [performance plan](plans/2026-09-24-performance-save-and-screenshot.md).
- All open findings above remain planned. The fix plan's policy recommendations are
  not accepted product decisions. Windows work remains frozen.

## Verification at this baseline

All deeper-review builds used an isolated archive snapshot with separate target
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
release build, Nix build or long-running soak was performed in this review.
The ignored `live_smooth_cursor_recording` test was not run. Dated
[cursor implementation notes](plans/2026-09-24-live-cursor-plugin.md#5-implementation-notes)
record user acceptance of area cursor playback and narrower live checks of
scaled output/replay/Combined capture. They do not establish current blanket
live acceptance. Monitor transitions, save error recovery, slow-disk Save/Close,
broader compositor/hardware coverage and playback presentation timing remain
open. [Replay validation](replay.md#verification-and-limits) records its own
limited native fullscreen run.
