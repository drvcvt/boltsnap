# Hyprland XDND Compatibility Plugin Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an explicitly unstable, opt-in `hyprpm` plugin that sends the initial `XdndPosition` Hyprland currently omits after `XdndEnter`.

**Architecture:** Hook only `CX11DataDevice::sendEnter`, call Hyprland's original implementation, then send one motion event at the current pointer position relative to the destination surface. Keep this compatibility code outside Boltsnap's Rust runtime and fail plugin loading on ABI or hook mismatch.

**Tech Stack:** C++23, Hyprland Plugin API, `hyprpm`, Make, shell checks

## Global Constraints

- The plugin is opt-in and never installed or loaded by Boltsnap.
- Warn that it runs inside Hyprland, uses an unstable internal function hook, is x86_64-only, and may crash the compositor.
- Refuse ABI-mismatched Hyprland builds and missing or ambiguous hook symbols.
- Do not include the separate logical monitor-order patch.

---

### Task 1: Package and verify the compatibility plugin

**Files:**
- Create: `hyprpm.toml`
- Create: `contrib/hyprland-xdnd-fix/Makefile`
- Create: `contrib/hyprland-xdnd-fix/main.cpp`
- Create: `contrib/hyprland-xdnd-fix/README.md`
- Create: `contrib/hyprland-xdnd-fix/tests/check.sh`
- Modify: `README.md`

**Interfaces:**
- Consumes: `CX11DataDevice::sendEnter`, `CX11DataDevice::sendMotion`, the Hyprland plugin hash API, and the current pointer/surface geometry.
- Produces: `contrib/hyprland-xdnd-fix/boltsnap-xdnd-fix.so` registered as the `boltsnap-xdnd-fix` hyprpm plugin.

- [x] **Step 1: Write the failing package check**

  Add `tests/check.sh` that requires a parseable manifest, required safety warnings, exact hook filtering, original-enter-before-position ordering, and mismatch failures.

- [x] **Step 2: Run the check to verify it fails**

  Run: `contrib/hyprland-xdnd-fix/tests/check.sh`

  Expected: FAIL because the manifest, source, build file, and documentation do not exist yet.

- [x] **Step 3: Implement the minimal plugin and documentation**

  Hook the exact `CX11DataDevice::sendEnter` symbol, invoke its trampoline, derive local coordinates using the same surface-box calculation as Hyprland's drag-motion path, and immediately call `sendMotion`. Reject unsupported architecture, ABI mismatch, missing/ambiguous symbols, or failed hook activation.

- [x] **Step 4: Run focused and repository checks**

  Run: `contrib/hyprland-xdnd-fix/tests/check.sh`, `make -C contrib/hyprland-xdnd-fix`, `cargo fmt --check`, and `cargo test`.

  Expected: all commands pass without warnings.

- [x] **Step 5: Commit only plugin-related files**

  Stage the six plugin/plan files and `README.md`; leave unrelated worktree changes unstaged.
