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

## Task 5: No shelf surface while the shelf is empty (2026-10-22, implemented 2026-10-23, live open)

**Priority:** high for gaming on Hyprland; no effect on capture.

**Trigger / impact (reproduced 2026-10-22):** the daemon creates the shelf's OVERLAY layer at
startup (`place_on_output(None, …)`, `mod.rs:1266`) and keeps it for its whole life, 1x1 while no
card shows. Hyprland refuses to hand a fullscreen window the output alone while *any* overlay
layer exists on it (Hyprland 0.56.2 `src/output/Monitor.cpp` `isSolitaryBlocked`:
`!m_layerSurfaceLayers[OVERLAY].empty()` -> `SC_OVERLAYS`). Without a solitary client there is no
tearing (`isTearingBlocked` -> `TC_CANDIDATE`) and no direct scanout: every game frame is composited,
about one frame more input latency. Live: `hyprctl layers` listed `boltsnap 1 x 1` on DP-3 (pid of
`boltsnap daemon`) next to a fullscreen Brawlhalla with an `immediate` rule. mtmono's own 1x1
overlay was removed the same day; boltsnap's is the last one.

**Fix:** the shelf surface exists only while it shows something. It is already created on demand
(both add paths call `place_on_output`: card add `mod.rs:1890`, recording clips `mod.rs:3325`); only the
startup call and the missing teardown keep it alive.

### Behaviour

- Daemon start: no surface. The output roundtrip and `prep_shelf_compositor_rules()` stay.
- First card: `place_on_output` creates the surface on the requested/focused output as today; the
  first frame waits for `configure` as today after an output change.
- The appear animation (150 ms) is re-based to the first `configure` of a fresh surface, so it
  plays from its first frame instead of losing the configure roundtrip.
- Last card gone (dismiss animation finished, model empty, no animation, no drag): the surface is
  dropped in `draw()`; the card is already faded out, nothing visible changes.
- Next card: a new surface, on the then-focused output. (Not yet true for cards without an output, see S6 below.)

### Edge cases (checked in code, 83e1ae6; rechecked on ceab064)

| Case | Today | After |
|---|---|---|
| Drag of a card in progress | `start_drag` needs the shelf `wl_surface` as origin (`self.layer`, `mod.rs:2056`) | drop only when `drag_source` is none; a dragged card is still in the model |
| Card being saved | `remove_card_now` refuses while saving, card stays in the model | model not empty -> surface stays |
| Dismiss of the last card | removed in `tick_animations` after 240 ms, then `relayout` + `draw` | teardown happens in that `draw` |
| Frame callbacks | requested only while animations run; `frame()` matches the shelf surface | teardown requires no running animation, so no callback is lost |
| Pointer over the surface at teardown | `remove_card_now` already clears `hovered` / `press` for the card | unchanged; `pointer_frame` ignores events when `self.layer` is none |
| Output unplugged | `closed()` sets `layer = None` (A2 of the [2026-10-12 review](2026-10-12-recording-reliability-review.md)) | unchanged, the same state the teardown produces |
| Hyprland open/close animation of a new layer | n/a (surface lived forever) | user rule `static-layers` (`~/.config/hypr/hyprlua/rules.lua:193`, `no_anim` for all namespaces); boltsnap's own `hyprctl keyword layerrule` is rejected by the Lua config parser |
| Clipboard copy | own process (`clipboard.rs`, `Command::new(exe)`) | independent of the surface |
| Popups, markers, recording controls | own layer surfaces (`is_popup_surface`, `is_marker_surface`) | untouched |
| Socket on daemon restart | `remove_stale_socket` before bind (`mod.rs:1163`) | unchanged |

### Code (files and order)

`src/platform/linux/shelf/mod.rs` is far over the 1000-line limit (4032 lines on ceab064 after
the test split). So the logic goes into a new
`src/platform/linux/shelf/surface.rs`, and `mod.rs` does not grow (net 0 lines):

1. `surface.rs`: pure `shelf_surface_needed(cards: usize, animations: usize, dragging: bool) -> bool`
   with unit tests; `impl Daemon { fn release_empty_surface(&mut self) -> bool;
   fn mark_shelf_configured(&mut self) }`. Release clears `layer`, `output_name`,
   `shelf_configured`, `shelf_pending_draw`, `frame_requested`. `mark_shelf_configured` re-bases
   `Appear` animations when the surface was not configured before, then sets `shelf_configured`
   and clears `shelf_pending_draw`.
2. `mod.rs`: `mod surface;` (+1); drop `daemon.place_on_output(None, &qh);` (-1); first line of
   `draw()`: `if self.release_empty_surface() || !self.shelf_configured {` (0, the pending-draw
   flag it sets is reset by the next `place_on_output`); in `configure()` the two lines
   setting `shelf_configured` / `shelf_pending_draw` become `self.mark_shelf_configured();` (-1).

### Base and deploy

- Branch `lazy-shelf-surface` in worktree `~/projects/boltsnap-lazy-shelf`, based on **83e1ae6**
  (V2): the installed `~/.cargo/bin/boltsnap` (7.3 MB, 2026-09-26 00:13) was built right after it.
  Not based on HEAD (V9 restyle, never installed) and not on the uncommitted refactor, so the
  deployed binary differs from today's only by this task. Port to V9 and the refactor afterwards.
- 2026-10-23: the refactor was committed (d860e06..ceab064), so the change went straight onto
  main instead; the worktree branch is superseded. The binary built from main also carries the
  not yet installed V9, K5 and the 2026-10-12 waves, so the live checks below cover those too.
- Backup `~/.cargo/bin/boltsnap` -> `~/.cargo/bin/boltsnap.2026-09-26` (copy) before replacing it.
- Restart only while the shelf is empty and `boltsnap recording status` is `idle`: stop the
  daemon, start it as a Hyprland child (`hyprctl eval 'hl.exec_cmd("…/boltsnap daemon")'`) so it
  gets the session environment like the autostart.
- Rollback: copy the backup back, restart the same way.

### Checks

- [x] `cargo fmt --check`, `cargo test` (root 25 + 340, new tests for `shelf_surface_needed`),
  `cargo clippy --all-targets` without warnings (2026-10-23).
- [x] `git diff --check`; `mod.rs` 4032 -> 4031 lines.
- [ ] Live, after restart: `hyprctl layers` shows no `boltsnap` layer while idle.
- [ ] User: screenshot -> card appears with its animation; copy; drag a card into a browser; open
  in the editor; save; dismiss -> layer gone again; screenshot on the other monitor; start and stop
  a recording (controls still appear).
- [ ] With Brawlhalla fullscreen on DP-3: `hyprctl monitors -j` `solitaryBlockedBy` empty,
  `activelyTearing` true.

## Review 2026-10-23 (S1 to S7)

Read-only review of the shelf daemon, IPC and the Task 5 change on 9ef1cc0.
Nothing reproduced; every finding re-read against the code on 23.10.
("plausible": code path confirmed, runtime effect not proven). The Task 5
surface lifecycle itself was checked path by path and holds (every add calls
`place_on_output` before `draw`, release only inside `draw` on the event loop,
frame callbacks and stale `configure`/`closed` are safe). S5 and S6 are
regressions against the Task 5 goal and its behaviour text.

| ID | Prio | Trigger and impact | Evidence | Confidence | Fix and tradeoff |
| --- | --- | --- | --- | --- | --- |
| S1 | P2 | Any client on the socket sends `recording_thumb` with any path: the daemon reads it (up to 256 MiB) and **deletes it unconditionally**, no `same_user`, no path check. Can delete a live card's temp PNG or any file the user may delete. Nothing in the tree sends this request since July. | `linux/shelf/mod.rs:759-770`, `linux/shelf/thumbnail.rs:77-95` (`let _ = std::fs::remove_file(path);`) | verified | Remove the decode/delete path; keep the protocol variant only to reject it with an error. A capture client older than July keeps its placeholder. |
| S2 | P3 | Cards still take clicks and drags during their 240 ms dismiss animation; the temp file is then removed under a running drag or `xdg-open`. | `mod.rs:2300` `hit_test` over `animated_layout()`, `Layout::hit` never filters, `remove_card_now` ignores `drag_card` | verified path | `hit_test` skips ids with a `Disappear` animation. A click on a dismissing card does nothing. |
| S3 | P3 | Drag a temporary video card while its Save moves the file: the drag still offers the old path. | `mod.rs:2109` (path captured at drag start), `3133-3136` (`promote` updates only the model) | plausible | `begin_drag` refuses while `saves.is_saving(id)`. |
| S4 | P3 | A replay save, finalize or `add_video` during a drag does not end the drag; on another output `place_on_output` destroys the drag's origin surface, the compositor likely cancels, and the cancel path copies the image to the clipboard unasked. | `add_prepared` clears the drag (`mod.rs:1873-1875`), `add_finalized_cards` (`3280-3327`) does not | verified path, compositor reaction plausible | Same `clear_drag()` in `add_finalized_cards` when a card is added. |
| S5 | P3 | **Task 5 regression:** a drag whose end event never arrives (the code documents this, `mod.rs:1871-1872`, `2309-2312`) keeps the overlay surface alive after the last card is gone, so tearing and direct scanout stay blocked until the next screenshot. | `surface.rs:22-27` (`drag_source.is_some()` keeps the surface) | plausible | With an empty model and no animation, `clear_drag()` and release anyway; there is no card left to protect. |
| S6 | P3 | **Task 5 regression:** the release clears `output_name`, so a card without an output (editor `add_video`, legacy `add`, a failed Hyprland focus query) now lands on the **first** `wl_output` instead of the last used one. The behaviour text above promises the focused output. | `surface.rs:32`, `mod.rs:1686` (`requested.or(self.output_name…)`), `resolve_output_name` falls back to `available.first()` (`mod.rs:1006-1015`) | verified | `requested.or(self.focused_output).or(self.output_name)` in `place_on_output`; keep `output_name` across a release. `focused_output` is only as fresh as the last `refresh_focus`. |
| S7 | P3 | Parallel `add_video` imports each pass the cache capacity check before any copy lands and can exceed the 2 GiB cache together, which then blocks recordings. | `mod.rs:556-562`, `record/finalize.rs:726-741` | plausible | Give video imports their own `Limit::new(1)`. Imports then run one after another. |

Live open (not findings): whether Hyprland releases the shelf `wl_buffer` when
the layer is destroyed (otherwise each release cycle pins one `SlotPool` slot),
and what a compositor does with a drag whose origin surface is destroyed (S4).

### Plan

1. S6 and S5 first (regressions of Task 5, both in `surface.rs` plus one line
   in `place_on_output`; `mod.rs` must not grow). Tests: release keeps the
   last output; empty model with a stale drag releases.
2. S1 (delete the legacy path, reject the request; protocol test that the
   daemon answers an error and the file still exists).
3. S2, S4, S3 (one guard each, tests on the pure hit/drag decisions).
4. S7 with the next cache work.

Checks per step: `cargo fmt --check`, `cargo clippy --all-targets`,
`cargo test`; live after 1: card from the editor lands on the focused monitor,
`hyprctl layers` empty after the last card.

| ID | Status |
| --- | --- |
| S1 to S7 | documented, not implemented |
