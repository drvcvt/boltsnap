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
- Next card: a new surface, on the then-focused output.

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
