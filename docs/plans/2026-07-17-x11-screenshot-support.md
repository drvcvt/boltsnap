# X11 Screenshot Support Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete Boltsnap's screenshot-only X11 support for modern TrueColor X servers: fullscreen, active-window, selected-window, editable area selection, editor handoff, and a clipboard selection that survives the foreground process.

**Architecture:** Keep the existing `Backend` dispatch and reuse `x11rb`, `image`, `tiny-skia`, and the selector's pure geometry/render helpers. Add one X11 protocol module and one X11 selector driver; do not abstract the working Wayland event loop or port the Wayland shelf/recording daemon.

**Tech Stack:** Rust 2024, `x11rb` core + `image` + `composite`, existing `image`/`tiny-skia`/`arboard`, Xephyr, Openbox, xdotool, xclip

## Global Constraints

- This is a plan only. Do not implement it until the user explicitly approves execution.
- Preserve all current uncommitted work. Execute later from a clean worktree or after the current changes are committed; never stage unrelated edits.
- Preserve every existing Wayland behavior and protocol path.
- Add no new runtime dependency. Enabling existing `x11rb` features is allowed.
- Scope is screenshots and clipboard only. X11 shelf, drag-and-drop, tray recording controls, and recording are explicit non-goals.
- Prefer native X11 operations: upload frozen bright/dim pixmaps once, then redraw selections with server-side copies and core drawing primitives.
- Support TrueColor X servers whose root visual can be decoded by `x11rb::image`; reject palette visuals with a precise error.
- `--instant` captures on first button release. Default area mode retains resize/move/confirm and Escape cancellation.
- X11 v1 may omit the Wayland selector's Alt magnifier and recording-only controls. Add them only if users request them after the base selector is stable.

---

## Review Evidence

### Current behavior observed in nested Xephyr

| Capability | Result | Evidence |
|---|---|---|
| Fullscreen capture, 24-bit TrueColor | Works | Produced a correct 938×1012 RGB PNG |
| Active-window capture | Works | `_NET_ACTIVE_WINDOW` produced the 364×238 client image |
| Clicked-window capture | Works when visible | Pointer grab produced the 364×260 decorated window |
| Occluded-window capture | Incorrect | Current root crop included the red window covering the selected blue window |
| Area capture | Missing | Returns `interactive region selection requires Wayland` |
| Clipboard after command exit | Broken | `xclip` reported `target image/png not available` |
| 16-bit TrueColor capture | Rejected | Current code assumes four storage bytes per pixel |
| Shelf and recording | Wayland-only | `src/shelf/mod.rs` binds Wayland globals; recording spawns `wf-recorder` |

Xephyr exposed Composite, MIT-SHM, RANDR, RENDER, XFIXES, XINERAMA, and XTEST. The XComposite specification states that automatic redirection provides off-screen hierarchy storage and that `NameWindowPixmap` exposes it; manual redirection is the exclusive mode, so this plan uses `Redirect::AUTOMATIC`: <https://www.x.org/archive/X11R7.5/doc/man/man3/Xcomposite.3.html>.

### Why full desktop-feature parity is excluded

- `src/shelf/mod.rs` is 3,842 lines and owns Wayland connection setup, layer surfaces, pointer/keyboard handling, data-device drag-and-drop, monitor discovery, recording overlays, popup controls, tray publication, IPC, and the recording lifecycle.
- An X11 shelf needs an override-redirect window, XShape input regions, Xdnd source handling, monitor placement, stacking policy, and a second event-loop integration.
- X11 recording needs FFmpeg `x11grab`, RandR monitor enumeration, an X11 recording border/popup, recorder-program abstraction, and backend-specific daemon routing.
- Full shelf + recording parity is approximately one to two focused engineering weeks and doubles the compositor/window-manager test matrix. Screenshot-only completion is approximately two to three focused days.

## File Map

- Create `src/x11.rs`: X11 connection helpers, generic pixel conversion, root/window capture, geometry clamping, active-window lookup, window picker, and optional XComposite capture.
- Create `src/select_x11.rs`: X11 override-redirect selector, grabs, event state, server-side pixmap redraws, and final crop.
- Modify `src/capture.rs`: retain backend orchestration; delegate X11 mechanics and area selection.
- Modify `src/select_skia/mod.rs`: expose only the already-pure `edit` and `render` modules as `pub(crate)`.
- Modify `src/clipboard.rs`: add a foreground X11 clipboard owner and spawn it for X11 copies.
- Modify `src/main.rs`: register the two X11 modules and the private clipboard-owner command; reject recording clearly on X11.
- Modify `src/paths.rs`: report the completed X11 capabilities accurately.
- Modify `Cargo.toml`: enable `x11rb`'s existing `image` and `composite` features.
- Create `scripts/test-x11-xephyr.sh`: repeatable nested X11 smoke test with isolated runtime state.
- Modify `README.md`: document exact X11 scope and Xephyr validation.

---

### Task 1: Make X11 pixel and window capture correct

**Files:**
- Create: `src/x11.rs`
- Modify: `src/main.rs`
- Modify: `src/capture.rs`
- Modify: `Cargo.toml`
- Test: `src/x11.rs`

**Interfaces:**
- Consumes: existing `CaptureMode`, `DynResult`, `x11rb`, and `image::RgbaImage`.
- Produces: `capture_root`, `capture_visible_rect`, `capture_window`, `active_window_id`, `pick_window_id`, `root_size`, and `native_image`.

- [ ] **Step 1: Write failing pure tests for pixel decoding and clipping**

Add tests around these exact interfaces:

```rust
pub(crate) fn clamp_to_root(
    rect: (i32, i32, u32, u32),
    root: (u32, u32),
) -> Option<(i16, i16, u16, u16)>;

fn rgba_from_x11_image(
    image: &x11rb::image::Image<'_>,
    layout: x11rb::image::PixelLayout,
) -> Result<RgbaImage, String>;
```

```rust
#[test]
fn clips_partially_offscreen_window_to_root() {
    assert_eq!(
        clamp_to_root((-10, -5, 100, 50), (800, 600)),
        Some((0, 0, 90, 45))
    );
    assert_eq!(clamp_to_root((900, 0, 10, 10), (800, 600)), None);
}

#[test]
fn decodes_rgb565_using_the_visual_masks() {
    use std::borrow::Cow;
    use x11rb::image::{BitsPerPixel, ColorComponent, Image, ImageOrder, PixelLayout, ScanlinePad};

    let layout = PixelLayout::new(
        ColorComponent::from_mask(0xf800).unwrap(),
        ColorComponent::from_mask(0x07e0).unwrap(),
        ColorComponent::from_mask(0x001f).unwrap(),
    );
    let mut image = Image::new(
        1, 1, ScanlinePad::Pad16, 16, BitsPerPixel::B16,
        ImageOrder::LsbFirst, Cow::Owned(vec![0, 0]),
    ).unwrap();
    image.put_pixel(0, 0, layout.encode((u16::MAX, 0, 0)));

    assert_eq!(rgba_from_x11_image(&image, layout).unwrap().as_raw(), &[255, 0, 0, 255]);
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test x11::tests --no-fail-fast`

Expected: compilation fails because `src/x11.rs` and its helpers do not exist.

- [ ] **Step 3: Move X11 protocol mechanics out of `capture.rs`**

Enable only the already-installed feature code:

```toml
x11rb = { version = "0.13", features = ["image", "composite"] }
```

Register `mod x11;` in `src/main.rs`. In `src/x11.rs`, use `x11rb::image::Image::get` plus the returned visual's `PixelLayout` instead of assuming little-endian BGRX with four bytes per pixel. Use `Image::allocate_native` and `PixelLayout::encode` for uploads so the same code handles 16-bit and 24-bit TrueColor servers.

Keep `src/capture.rs` responsible only for dispatch:

```rust
fn capture_x11(mode: CaptureMode, output: &Path, instant: bool) -> DynResult<()> {
    let image = match mode {
        CaptureMode::Full => crate::x11::capture_root()?,
        CaptureMode::Area => crate::select_x11::run_select(crate::x11::capture_root()?, instant)?
            .ok_or("selection cancelled")?,
        CaptureMode::Window => {
            let id = crate::x11::pick_window_id()?.ok_or("window selection cancelled")?;
            crate::x11::capture_window(id)?
        }
        CaptureMode::ActiveWindow => {
            let id = match crate::x11::active_window_id()? {
                Some(id) => id,
                None => crate::x11::pick_window_id()?.ok_or("window selection cancelled")?,
            };
            crate::x11::capture_window(id)?
        }
    };
    image::DynamicImage::ImageRgba8(image).to_rgb8().save(output)?;
    Ok(())
}
```

Pass `instant` from `capture()` into `capture_x11`; remove the old inline X11 functions only after their replacements compile.

- [ ] **Step 4: Capture redirected window storage when Composite is available**

Implement `capture_window(id)` with this order:

1. Query window geometry and clamp it for the fallback path.
2. Negotiate XComposite 0.4.
3. Redirect only the selected window hierarchy with `Redirect::AUTOMATIC`.
4. Name its pixmap, capture that drawable at `(0, 0)`, and free the pixmap.
5. Let connection teardown remove Boltsnap's automatic redirection.
6. If Composite is absent or the window cannot be redirected, fall back to the existing visible root crop and emit one concise stderr warning that occlusion may appear.

Do not request `Redirect::MANUAL`; the XComposite specification allows only one manual redirector and that would conflict with a desktop compositor.

- [ ] **Step 5: Run focused and full tests**

Run: `cargo test x11::tests --no-fail-fast`

Expected: pixel conversion and clipping tests pass.

Run: `cargo test`

Expected: the complete suite passes without Wayland regressions.

- [ ] **Step 6: Commit the isolated task**

From a clean worktree, stage only `Cargo.toml`, `src/main.rs`, `src/capture.rs`, and `src/x11.rs`, then commit as `fix: harden X11 window capture`.

---

### Task 2: Keep the X11 clipboard selection alive

**Files:**
- Modify: `src/clipboard.rs`
- Modify: `src/main.rs`
- Test: `scripts/test-x11-xephyr.sh` in Task 5

**Interfaces:**
- Consumes: the existing `arboard` dependency and detached-self pattern used by Wayland.
- Produces: `serve_x11_clipboard(path: &Path) -> DynResult<()>` and private command `__serve-x11-clipboard`.

- [ ] **Step 1: Reproduce the failing behavior in Xephyr**

Run the future harness through the clipboard assertion only.

Expected before the fix: Boltsnap exits successfully, then `xclip -selection clipboard -t image/png -o` fails because no selection owner remains.

- [ ] **Step 2: Add a foreground X11 clipboard owner**

Use arboard's existing Linux wait API; it serves requests until another owner replaces the selection:

```rust
pub fn serve_x11_clipboard(path: &Path) -> DynResult<()> {
    use arboard::SetExtLinux;

    let img = image::open(path)?.to_rgba8();
    let (width, height) = (img.width() as usize, img.height() as usize);
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| format!("X11 clipboard open failed: {e}"))?;
    clipboard
        .set()
        .wait()
        .image(arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Owned(img.into_raw()),
        })
        .map_err(|e| format!("X11 clipboard serve failed: {e}"))?;
    Ok(())
}
```

Change the X11 branch of `copy_to_clipboard` to spawn the current executable with `__serve-x11-clipboard`, exactly like the Wayland helper. Add the matching private command in `main.rs`. Remove the incorrect comment claiming `arboard::set_image` forks by itself.

- [ ] **Step 3: Verify ownership and replacement**

Run: `scripts/test-x11-xephyr.sh`

Expected: `xclip` retrieves a non-empty PNG after the foreground Boltsnap command exits; replacing the clipboard lets the helper exit.

- [ ] **Step 4: Commit the isolated task**

From a clean worktree, stage only `src/clipboard.rs` and `src/main.rs`, then commit as `fix: persist X11 clipboard images`.

---

### Task 3: Add an editable X11 area selector

**Files:**
- Create: `src/select_x11.rs`
- Modify: `src/select_skia/mod.rs`
- Modify: `src/main.rs`
- Modify: `src/capture.rs`
- Test: `src/select_x11.rs`
- Test: `scripts/test-x11-xephyr.sh`

**Interfaces:**
- Consumes: `select_skia::edit`, `select_skia::render`, `x11::native_image`, and a frozen `RgbaImage`.
- Produces: `run_select(image: RgbaImage, instant: bool) -> DynResult<Option<RgbaImage>>`.

- [ ] **Step 1: Expose only the platform-neutral selector helpers**

Change these two declarations and nothing else in the working Wayland selector:

```rust
pub(crate) mod edit;
pub(crate) mod render;
```

Do not extract or rewrite the Wayland event loop. Its protocol setup, frame callbacks, and recording controls remain in `select_skia/mod.rs`.

- [ ] **Step 2: Write failing X11 selector-state tests**

Define a small X11-only state machine:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Idle,
    Drawing { anchor: (f64, f64), now: (f64, f64) },
    Editing { rect: crate::select_skia::edit::Rect },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Redraw,
    Confirm(crate::select_skia::edit::Rect),
    Cancel,
    None,
}
```

Add tests that prove:

- a normalized rectangle is created for an up-left drag;
- a sub-four-pixel drag returns to `Idle`;
- `instant=true` confirms on first release;
- default mode enters `Editing` on first release;
- pressing inside without exceeding the three-pixel drag slop confirms;
- Escape cancels;
- resize and move remain clamped to the root dimensions.

- [ ] **Step 3: Verify selector tests fail**

Run: `cargo test select_x11::tests --no-fail-fast`

Expected: compilation fails because `src/select_x11.rs` and its state transitions do not exist.

- [ ] **Step 4: Create the native X11 overlay**

Implement `run_select` with one X11 connection and these resources:

```rust
struct Selector {
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    overlay: u32,
    bright: u32,
    dimmed: u32,
    back: u32,
    copy_gc: u32,
    dark_gc: u32,
    light_gc: u32,
    width: u16,
    height: u16,
    image: RgbaImage,
    mode: Mode,
    interaction: Option<Interaction>,
    instant: bool,
}
```

Initialization order:

1. Receive the frozen root screenshot before mapping any selector window.
2. Create three root-depth pixmaps: bright, fully dimmed, and back buffer.
3. Upload the bright image once. Build the dimmed version with the existing tiny-skia renderer and upload it once.
4. Create one full-root `InputOutput` window with `override_redirect=true` and masks for exposure, key press, button press/release, and pointer motion.
5. Map it, raise it, grab pointer and keyboard, and use the existing crosshair cursor construction.

Redraw order:

1. `CopyArea` the dimmed pixmap to the back pixmap.
2. If a selection exists, `CopyArea` its rectangle from the bright pixmap to the back pixmap.
3. Draw dark and white rectangle outlines plus eight handle squares with core X11 GCs.
4. `CopyArea` the completed back pixmap to the overlay and flush.

This avoids uploading a full 4K image on every pointer event. Connection teardown releases grabs and all X resources even on cancellation or error.

- [ ] **Step 5: Wire exact interaction semantics**

Translate X11 events into the state machine:

- Button 1 press starts drawing, resizing, moving, or click-to-confirm.
- `MotionNotify` updates the current operation using `edit::resize_rect` and `edit::move_rect`.
- Button 1 release confirms immediately only with `--instant`; otherwise it enters editable mode.
- Escape keysym `0xff1b` cancels.
- Return `0xff0d`, keypad Enter `0xff8d`, and Space `0x20` confirm an editable rectangle.
- Resolve keysyms with `GetKeyboardMapping`; do not compare layout-dependent raw keycodes.
- Crop the original frozen `RgbaImage`, not pixels read back from the overlay.

- [ ] **Step 6: Run focused, integration, and full tests**

Run: `cargo test select_x11::tests --no-fail-fast`

Expected: all state-machine tests pass.

Run: `scripts/test-x11-xephyr.sh`

Expected: automated drag creates the expected non-empty area PNG; Escape exits with the documented cancellation status.

Run: `cargo test`

Expected: complete suite passes.

- [ ] **Step 7: Commit the isolated task**

From a clean worktree, stage only `src/select_x11.rs`, `src/select_skia/mod.rs`, `src/main.rs`, and `src/capture.rs`, then commit as `feat: add X11 area selector`.

---

### Task 4: Make platform boundaries explicit

**Files:**
- Modify: `src/main.rs`
- Modify: `src/paths.rs`
- Modify: `README.md`
- Test: `src/main.rs`

**Interfaces:**
- Consumes: resolved `Backend` and existing usage/doctor output.
- Produces: honest X11 capability output and a clear Wayland-only recording error.

- [ ] **Step 1: Add a backend capability guard**

Use one helper at the command boundary:

```rust
fn require_wayland(backend: Backend, feature: &str) -> DynResult<()> {
    if backend.resolved()? == Backend::Wayland {
        Ok(())
    } else {
        Err(format!("{feature} currently requires Wayland").into())
    }
}
```

Call it at the beginning of `record_flow`. Do not start or contact the shelf daemon from an X11 recording command.

- [ ] **Step 2: Update help and doctor output**

- Remove the comment that `--instant` is Wayland-only.
- Report X11 area selection as available.
- Label shelf and recording as Wayland-only.
- Keep X11's default post-capture sink as clipboard; do not auto-start the Wayland shelf.

- [ ] **Step 3: Update README scope**

Document:

| X11 feature | Status after this plan |
|---|---|
| Full, area, selected window, active window | Supported |
| PNG clipboard | Supported through a short-lived background owner |
| External Eddy editor | Supported |
| Shelf and drag-and-drop | Wayland-only |
| Screen recording and controls | Wayland-only |

State that XComposite is used for unoccluded window contents when available and that visible-root cropping is the fallback.

- [ ] **Step 4: Run validation**

Run: `cargo test && git diff --check`

Expected: all tests pass and no whitespace errors are reported.

- [ ] **Step 5: Commit the isolated task**

From a clean worktree, stage only `src/main.rs`, `src/paths.rs`, and `README.md`, then commit as `docs: define X11 support boundaries`.

---

### Task 5: Add a repeatable Xephyr regression check

**Files:**
- Create: `scripts/test-x11-xephyr.sh`
- Modify: `README.md`

**Interfaces:**
- Consumes: built `target/debug/boltsnap`, Xephyr, Openbox, xterm, xdotool, xclip, and `file`.
- Produces: one command that verifies X11 without touching the user's real desktop or Wayland daemon.

- [ ] **Step 1: Add the isolated harness**

The script must:

```sh
#!/bin/sh
set -eu

display=${BOLTSNAP_X11_DISPLAY:-:98}
bin=${BOLTSNAP_BIN:-target/debug/boltsnap}
tmp=$(mktemp -d)
mkdir -m 700 "$tmp/runtime"

cleanup() {
    [ -n "${app_pid:-}" ] && kill "$app_pid" 2>/dev/null || true
    [ -n "${wm_pid:-}" ] && kill "$wm_pid" 2>/dev/null || true
    [ -n "${xephyr_pid:-}" ] && kill "$xephyr_pid" 2>/dev/null || true
    rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

run_x11() {
    env -u WAYLAND_DISPLAY DISPLAY="$display" XDG_SESSION_TYPE=x11 \
        XDG_RUNTIME_DIR="$tmp/runtime" "$@"
}

Xephyr "$display" -screen 1024x768x24 -ac -noreset >"$tmp/xephyr.log" 2>&1 &
xephyr_pid=$!
i=0
until run_x11 xdpyinfo >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -lt 50 ] || { cat "$tmp/xephyr.log"; exit 1; }
    sleep 0.1
done

run_x11 openbox >"$tmp/openbox.log" 2>&1 &
wm_pid=$!
run_x11 xterm -geometry 60x18+80+80 -title Boltsnap-X11-Test \
    -e sh -c 'printf "BOLTSNAP X11 XEPHYR TEST\n"; exec sh' &
app_pid=$!
sleep 0.5

run_x11 "$bin" full --backend x11 --no-copy -o "$tmp/full.png"
file "$tmp/full.png" | grep -q 'PNG image data'

win=$(run_x11 xdotool search --name Boltsnap-X11-Test | head -n 1)
run_x11 xdotool windowactivate --sync "$win"
run_x11 "$bin" active-window --backend x11 --no-copy -o "$tmp/active.png"
file "$tmp/active.png" | grep -q 'PNG image data'

run_x11 "$bin" window --backend x11 --no-copy -o "$tmp/window.png" &
shot_pid=$!
sleep 0.3
run_x11 xdotool mousemove 120 120 click 1
wait "$shot_pid"
file "$tmp/window.png" | grep -q 'PNG image data'

run_x11 "$bin" area --instant --backend x11 --no-copy -o "$tmp/area.png" &
shot_pid=$!
sleep 0.3
run_x11 xdotool mousemove 100 100 mousedown 1 mousemove 400 300 mouseup 1
wait "$shot_pid"
file "$tmp/area.png" | grep -q 'PNG image data'

run_x11 "$bin" full --backend x11
run_x11 xclip -selection clipboard -t image/png -o >"$tmp/clipboard.png"
file "$tmp/clipboard.png" | grep -q 'PNG image data'
printf 'replace' | run_x11 xclip -selection clipboard

printf 'Xephyr X11 smoke test passed\n'
```

The committed version may print the retained artifact directory when `BOLTSNAP_KEEP_X11_ARTIFACTS=1`; without that variable it must clean up all processes and files.

- [ ] **Step 2: Run the harness twice**

Run: `cargo build && scripts/test-x11-xephyr.sh && scripts/test-x11-xephyr.sh`

Expected: both runs print `Xephyr X11 smoke test passed`; no Xephyr, Openbox, xterm, clipboard-owner, or Boltsnap test processes remain.

- [ ] **Step 3: Run final quality gate**

Run: `cargo test && git diff --check`

Run: `ps -eo pid,cmd | grep -E '[X]ephyr :98|[o]penbox|[B]oltsnap-X11-Test'`

Expected: all Rust tests pass, diff check is clean, and the process check prints nothing.

- [ ] **Step 4: Commit the isolated task**

From a clean worktree, stage only `scripts/test-x11-xephyr.sh` and its README invocation, then commit as `test: cover X11 flows in Xephyr`.

---

## Go / No-Go Decision

Proceed with this plan if screenshot-only X11 support is valuable enough to maintain one additional selector driver and Xephyr smoke test. Stop after this plan; it gives a coherent X11 product without touching the Wayland daemon.

Do not proceed to full X11 shelf/recording parity unless there is a concrete user need. That work requires a separate design and implementation plan built around extracting daemon core state from Wayland presentation, adding an X11 window/Xdnd frontend, and replacing the `wf-recorder`-specific process layer with backend-specific recorder commands.

## Self-Review

- Spec coverage: existing X11 paths, clipboard lifetime, occlusion, area selection, diagnostics, docs, Xephyr automation, scope boundary, and cleanup are each assigned to a task.
- Placeholder scan: no deferred implementation markers are used inside the actionable tasks.
- Type consistency: `run_select`, X11 capture helpers, and the foreground clipboard-owner command have one stable signature throughout the plan.
- Regression boundary: Wayland modules expose two pure helper modules but otherwise keep their event loop and daemon unchanged.
