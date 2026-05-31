# Handoff: boltsnap macOS-style screenshot shelf (wlroots/Hyprland)

Date: 2026-05-31
Project: `/home/mt/projects/boltsnap`
Branch: `feat/screenshot-shelf` (28 commits over `origin/main`; **not pushed**)

## Goal / current status

Rebuild boltsnap (a Rust Wayland/X11 screenshot tool) to add a **macOS-style floating
"shelf"**: after a capture, a small thumbnail floats in the bottom-left corner; click=copy,
drag=DnD into apps, hover→edit/copy/close icons, ✕=dismiss. wlroots/Hyprland-only; X11 keeps
the old one-shot behaviour. Phase 2 (screen recording into the same shelf) is deferred.

**Implementation: complete and working.** Milestones A–G all done (see
`docs/superpowers/plans/2026-05-30-screenshot-shelf.md`). `cargo build` + `cargo build
--release` clean (0 errors, 0 warnings); `cargo test` = **24 passed, 0 failed**. Verified live
on Hyprland end-to-end (capture → shelf → click-copy; selector open/close animation removed).

**Working tree is NOT clean:** `src/shelf/mod.rs` has **uncommitted** changes (the per-monitor
placement + no-anim comment + ingest tweaks). Everything else is committed. The most recent
styling/monitor commit is `eaf290e`; the uncommitted mod.rs delta needs review + commit.

**Co-author policy (IMPORTANT):** NEVER add a `Co-Authored-By: Claude` trailer to commits in
this user's repos. All 28 branch commits are trailer-free; keep it that way.

## Files changed

### Rust source (boltsnap) — mostly committed; `src/shelf/mod.rs` has uncommitted edits
- `src/main.rs`
  - Refactored from one 2207-line file into modules (Milestone A).
  - `decide_post_capture()` + `PostCapture` enum: Wayland interactive capture → shelf (default,
    no auto-copy); `--copy` also copies; `-o`/`--save`/`-o -`/`--edit`/X11 keep old behaviour.
  - `capture_flow()` dispatches on `decide_post_capture` (this was silently lost once and
    re-fixed in `693c62a` — if you see "PostCapture never used" warnings, the wiring regressed).
  - Added hidden `__debug-render <png>` subcommand → `shelf::debug_render` (renders the shelf via
    the real draw path, no compositor needed; great for inspecting styling + detecting a stale binary).
  - `daemon` subcommand → `shelf::run_daemon`. `usage()` documents it.
- `src/ipc.rs` — Unix-socket frame protocol (`[u32 hdr_len][u32 payload_len][json][png]`),
  `Request::{Add,Reload,Ping}`, `socket_path()` ($XDG_RUNTIME_DIR/boltsnap.sock), `daemon_alive`,
  `ensure_daemon` (self-spawns `boltsnap daemon`), `send_to_shelf`. Unit-tested.
- `src/shelf/mod.rs` — **HAS UNCOMMITTED CHANGES.** SCTK daemon: wlr-layer-shell overlay,
  calloop event loop (Wayland fd + socket listener), `wl_shm` rendering, pointer hover/click,
  `wl_data_device` drag source (image/png + text/uri-list) + auto-copy fallback, editor spawn+reload.
  Uncommitted delta: `place_on_focused_output()` (per-capture monitor placement via
  `hyprctl monitors -j` → matching `wl_output`; `layer` is now `Option`), `add_png` calls it,
  and an expanded comment on `prep_shelf_compositor_rules` explaining the hyprlua no-op (below).
- `src/shelf/model.rs` — `ShelfModel` (add/remove/get/replace_thumb/newest_first). `Thumb.source`
  and `is_empty/len` are `#[allow(dead_code)]` (test-only). Committed.
- `src/shelf/thumbnail.rs` — `make_thumbnail` aspect-preserving downscale; `pub const MAX_W=200,
  MAX_H=144` (the shelf thumbnail size knob). Committed.
- `src/shelf/layout.rs` — stacking + hit-testing; `LayoutConfig` (icons shrunk to `icon:15`).
  Committed.
- `src/shelf/paint.rs` — BGRA compositing; `blit_thumb_card` (rounded corners + white border via
  SDF, `CARD_RADIUS=11`, `CARD_BORDER=2`); minimal circular hover buttons with anti-aliased
  glyphs (close X / copy / pencil). Committed.
- `src/capture.rs`, `src/select.rs`, `src/editor.rs`, `src/clipboard.rs`, `src/paths.rs` — moved
  out of main.rs in Milestone A. `paths::print_doctor` extended with shelf/daemon/socket status.
- `Cargo.toml` — added `smithay-client-toolkit = "0.19"`, `wayland-client = "0.31"`,
  `calloop = "0.13"`, `calloop-wayland-source = "0.3"` (all already transitive via winit; single
  `wayland-client v0.31.14` in the tree — verified no conflict). Committed.
- `README.md` — added "Screenshot shelf" section + `boltsnap daemon`. Committed.

### User's Hyprland config (OUTSIDE the repo) — applied, NOT under version control here
- `~/.config/hypr/hyprlua/rules.lua` (THE LIVE config; entrypoint `hyprland.lua` requires it):
  - `hl.window_rule({ match = { class = "^(boltsnap-select)$" }, no_anim = true })` — kills the
    selector overlay open/close animation. **User confirmed this works.**
  - `hl.layer_rule({ match = { namespace = "^boltsnap$" }, no_anim = true })` — kills the shelf
    layer animation.
- `~/.config/hypr/conf.d/rules.conf` (legacy mirror, plain-config form `noanim = true`) — kept in
  sync. Backups: `~/.config/hypr/conf.d/rules.conf.bak-boltsnap-*`.
- `~/.cargo/bin/boltsnap` is a **symlink → `/home/mt/projects/boltsnap/target/debug/boltsnap`**
  (so the user's Print keybind runs the dev build). Old v0.3.0 binary backed up at
  `~/.cargo/bin/boltsnap.v030.bak`.

## Files inspected (unchanged, for context)
- `~/.config/hypr/hyprland.lua` — confirms hyprlua entrypoint (`require("hyprlua.rules")`, etc.).
- `~/.config/hypr/conf.d/keybinds.conf` + `hyprlua/keybinds.lua` — Print/Shift+Print/Super+Shift+S
  → `boltsnap area`; Ctrl+Print → `boltsnap full`; Super+Print → `boltsnap --edit`.
- `/usr/share/hypr/stubs/hl.meta.lua` — hyprlua field whitelist; `HL.LayerRuleSpec` has `no_anim`.

## Key decisions / assumptions
- **Hybrid process model:** capture client self-spawns a long-lived `boltsnap daemon` over the
  unix socket; daemon also autostartable. Shelf is **RAM-only** (cleared on daemon restart).
- **eframe/winit can't do layer-shell or be a DnD source**, so the shelf uses raw
  **smithay-client-toolkit 0.19**. The selector overlay and editor stay eframe.
- **Hyprland here runs hyprlua (non-legacy parser).** TWO gotchas (documented in agent memory
  `hyprlua-no-anim-rule.md`):
  1. `hyprctl keyword ...` is a **silent no-op** (replies "can't work with non-legacy parsers",
     exits 0). So boltsnap's runtime rule pushes do nothing here — rules must be **static in the
     lua config**.
  2. hyprlua snake-cases keywords: `noanim` → field error, `animation="none"` → makes it WORSE.
     Correct is **`no_anim = true`**.
- **Dev-binary trap (cost ~6 turns):** the `~/.cargo/bin` symlink pointed at `target/release`
  while only `cargo build` (debug) was run → user tested stale code. Now symlink → `target/debug`;
  rebuild with `cargo build` and **restart the daemon** after each rebuild (it holds old code).
- **Per-capture monitor placement:** shelf must appear on the monitor focused *at capture time*
  (user has DP-1 + DP-3; main is DP-3), not the one focused at daemon start. Implemented in the
  uncommitted `place_on_focused_output`.
- Styling so far: thumbnail 200×144, 11px rounded corners, 2px white border, 15px circular
  buttons. **User still wants broader "style polish" — not yet started.**

## Commands run and results
- `cargo build` — 0 errors, 0 warnings.
- `cargo build --release` — 0 errors, 0 warnings.
- `cargo test` — **24 passed, 0 failed**.
- `hyprctl configerrors` — empty (0 errors) after the lua rule edits.
- Live: `boltsnap daemon` + `boltsnap full` → "Boltsnap sent full to shelf", daemon ingests PNG
  to `$TMPDIR/boltsnap-shelf-*.png` (note: this shell's `TMPDIR=/tmp/claude-1000`, not `/tmp`).
- `boltsnap __debug-render /tmp/card.png` — confirmed rounded corners + white border render.

## Open blockers / risks
- **Uncommitted `src/shelf/mod.rs`** (per-monitor + comment). Review and commit (no co-author trailer).
- **Shelf layer animation not yet user-confirmed.** Selector animation IS confirmed fixed. User
  should verify the thumbnail also pops in/out instantly now (layer_rule was just added).
- **Style polish requested but not done** — the next substantive task.
- **Not installed for real / no autostart yet.** Currently a symlink to `target/debug`. User
  earlier approved `cargo install --path .` + `exec-once = boltsnap daemon` in autostart.conf, but
  that was deferred until styling is locked. (If you switch to a real install, the symlink/rebuild
  workflow no longer applies — and the daemon must be restarted from the installed path.)
- **Bash tool output intermittently corrupts in this environment** (phantom/duplicate lines,
  cross-attributed parallel calls, truncation; `pkill -f` hangs → exit 144). Mitigation that
  worked: ONE command per turn, redirect to a `/tmp` file, then Read it; verify with `grep -c`;
  kill daemons by PID (`pgrep -x boltsnap` → `kill <pid>`), never `pkill -f`; avoid big parallel
  tool batches. For multi-line config/code edits prefer a Python heredoc with a count-guard.
- Commit history is coarser than the plan in spots (a tool-corruption episode forced two recovery
  commits: `1a68a99`, `693c62a`). Branch end-state is correct; history granularity is the only cost.

## Exact next steps
1. Review the uncommitted `src/shelf/mod.rs` diff (`git diff -- src/shelf/mod.rs`), then commit it
   (message describing per-monitor placement + hyprlua no-op note). **No co-author trailer.**
2. Ask the user to confirm the shelf thumbnail pop-in/out animation is gone (layer_rule no_anim).
3. Do the **style polish** the user queued. Inspect via `boltsnap __debug-render /tmp/x.png` and
   Read the PNG; iterate in `src/shelf/paint.rs` (+ `layout.rs` for spacing, `thumbnail.rs:MAX_W/H`
   for size). Rebuild with `cargo build`, then restart the daemon (kill by PID, relaunch
   `./target/debug/boltsnap daemon`).
4. When styling is locked: finalize — `cargo install --path .` (replaces the symlink with a real
   v0.4.2 binary), add `exec-once = boltsnap daemon` to `~/.config/hypr/conf.d/autostart.conf`
   (+ hyprlua/autostart.lua mirror), then run `superpowers:finishing-a-development-branch`.
5. Keep the Hyprland `no_anim` rules in `hyprlua/rules.lua` (live) AND `conf.d/rules.conf` (mirror)
   in sync if you touch them.

## Useful resume commands
```sh
cd /home/mt/projects/boltsnap
git status --short --branch
git diff -- src/shelf/mod.rs        # the uncommitted change
git log --oneline origin/main..HEAD | cat
cargo build && cargo test

# Restart the dev daemon after a rebuild (symlink -> target/debug):
for p in $(pgrep -x boltsnap); do kill "$p"; done; sleep 0.4
rm -f "$XDG_RUNTIME_DIR/boltsnap.sock"
nohup ./target/debug/boltsnap daemon >/tmp/boltsnap-daemon.log 2>&1 &

# Inspect shelf styling without a compositor:
./target/debug/boltsnap __debug-render /tmp/boltsnap-card.png   # then Read the PNG

# Trigger a real capture (or use the Print keybind):
./target/debug/boltsnap full        # whole screen -> shelf
./target/debug/boltsnap area        # drag-select -> shelf

# Hyprland sanity:
hyprctl configerrors
hyprctl monitors -j | grep -E '"name"|"focused"'
readlink ~/.cargo/bin/boltsnap      # should point at target/debug/boltsnap
```

## Reference docs (in-repo)
- Spec: `docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md`
- Plan: `docs/superpowers/plans/2026-05-30-screenshot-shelf.md`
- Agent memory (global, not in repo):
  `~/.claude/projects/-home-mt-projects-boltsnap/memory/` — `screenshot-shelf-project.md`,
  `shelf-execution-status.md`, `no-claude-coauthor.md`, `boltsnap-debug-release-symlink.md`,
  `hyprlua-no-anim-rule.md`.
