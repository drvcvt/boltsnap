# Screenshot-Shelf Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a macOS-style floating screenshot shelf to boltsnap on wlroots/Hyprland: captured screenshots appear as small thumbnails in the bottom-left corner, where they can be clicked to copy, dragged into other apps, edited, or dismissed.

**Architecture:** Hybrid process model. A long-lived `boltsnap daemon` owns a `wlr-layer-shell` overlay surface (rendered with raw `wl_shm` via smithay-client-toolkit) and holds thumbnails in RAM. The capture client (`boltsnap area|full|window`) captures as before, then sends the PNG over a Unix socket to the daemon, self-spawning it if absent. Pure logic (IPC framing, shelf model, thumbnail scaling, layout/hit-testing, pixel compositing) is unit-tested; the SCTK/Wayland glue is verified manually on Hyprland. The existing eframe selection-overlay and annotation-editor are reused unchanged. X11 keeps the old one-shot behavior.

**Tech Stack:** Rust 2024, `smithay-client-toolkit = "0.19"` (layer-shell + `wl_data_device` drag-source + `wl_shm`, already in the lockfile via winit), `wayland-client = "0.31"`, `calloop` (SCTK default feature, for integrating the socket listener with the Wayland event loop), `image` (scaling/compositing), `serde_json` (IPC headers). Existing: eframe/egui, libwayshot, wl-clipboard-rs, x11rb, arboard.

**Reference:** `docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md`

**Verified API facts (from compile-checked research against sctk 0.19.2 / wayland-client 0.31.14):**
- `wl_shm::Format::Argb8888` is little-endian `0xAARRGGBB` → **bytes in memory are `[B,G,R,A]` (BGRA), alpha premultiplied**.
- Drag source: `DataDeviceManagerState::create_drag_and_drop_source(qh, mimes, DndAction)` → `DragSource`; start with `source.start_drag(&data_device, &origin_surface, icon, serial)` where `serial` is the `PointerEventKind::Press { serial, .. }` value. `DragSource`/`DataDevice` have `Drop` impls — **keep them in state**.
- `DataSourceHandler::send_request(.., mime, fd: WritePipe)` → `File::from(OwnedFd::from(fd))`, write bytes, drop to close. `dnd_dropped` = success, `cancelled` = no-target (**auto-copy fallback trigger**), `dnd_finished` = safe to drop source.
- Pointer: `PointerEvent { surface, position:(f64,f64) /*surface-local*/, kind }`, `PointerEventKind::{Enter,Leave,Motion,Press,Release,Axis}`, `BTN_LEFT=0x110`. Position read from `event.position`, not the kind.
- Layer: `LayerShell::bind` → `create_layer_surface(qh, surface, Layer::Overlay, Some("boltsnap"), None)`; `set_anchor(Anchor::BOTTOM|Anchor::LEFT)`, `set_margin(t,r,b,l)`, `set_size(w,h)`, `set_keyboard_interactivity(None)`, `set_exclusive_zone(-1)`; initial bufferless `commit()` triggers first `configure`; SCTK acks configure for you.
- Shm: `SlotPool::create_buffer(w,h,stride,Argb8888) -> (Buffer, &mut [u8])`; `buffer.attach_to(surface)`; if `pool.canvas(&buffer)` returns `None` the compositor still holds it → allocate a second buffer (double-buffer).

---

## File Structure

This refactors the single 2207-line `src/main.rs` into focused modules, then adds the shelf. Files that change together live together.

**Refactor (Milestone A) — move existing code, no behavior change:**
- `src/main.rs` — shrinks to: CLI types (`Args`, `Backend`, `CaptureMode`), `parse_args`, `main`/`run` routing, and `mod` declarations. Keeps the `#[cfg(test)]` parser tests.
- `src/capture.rs` — Wayland (libwayshot) + X11 (x11rb) capture, geometry/hyprctl helpers, `strip_uniform_border`, `flatten_to_rgb`. Owns the `parse_hypr_geometry` test.
- `src/select.rs` — the eframe selection overlay (`SelectApp`, `run_select_with_parallel_capture`, `prep_compositor_for`).
- `src/editor.rs` — the eframe annotation editor (`EditorApp`, `Tool`, `Annotation`, `run_editor`, `render_annotations` + drawing primitives + color consts + phosphor glyphs). Owns the `render_*` tests.
- `src/clipboard.rs` — `copy_to_clipboard`, `serve_wayland_clipboard`.
- `src/paths.rs` — cache/temp/last-screenshot path helpers, `timestamp`, `has_cmd`, `print_doctor`, `self_test`.

**New (Milestones B–G):**
- `src/ipc.rs` — Unix-socket frame protocol + `Request`/`Response` + `socket_path()` + client `send_to_shelf`/`ensure_daemon`.
- `src/shelf/mod.rs` — daemon entry `run_daemon()`, the `Daemon` SCTK state struct, calloop wiring, all Wayland handler impls, drag lifecycle, click/hover/copy/close/edit handling.
- `src/shelf/model.rs` — `Thumb`, `ShelfModel` (add/remove/get/replace/iter). Pure, unit-tested.
- `src/shelf/thumbnail.rs` — `make_thumbnail`. Pure, unit-tested.
- `src/shelf/layout.rs` — `LayoutConfig`, `ThumbRect`, `Layout`, `Hit`, `compute`, `hit`. Pure, unit-tested.
- `src/shelf/paint.rs` — `draw_shelf` + BGRA compositing helpers. Pixel helpers unit-tested.

---

## Milestone A — Refactor main.rs into modules (no behavior change)

Goal: identical behavior, `cargo test` and `cargo build` green, smaller files. Do these one module at a time; after each, build + test + commit. **Do not change any logic** — move code verbatim, add `pub`/`use`/`mod` as needed.

### Task A1: Establish module skeleton

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add empty module declarations and a shared result type**

At the top of `src/main.rs`, just below the existing `use` block, add:

```rust
mod capture;
mod clipboard;
mod editor;
mod ipc;
mod paths;
mod select;
mod shelf;

pub type DynResult<T> = Result<T, Box<dyn std::error::Error>>;
```

Remove the old `type DynResult<T> = ...` line (it becomes the `pub` one above). Create empty placeholder files so the crate compiles:

```rust
// src/capture.rs, src/clipboard.rs, src/editor.rs, src/select.rs, src/paths.rs
// (each starts empty; filled in later tasks)
```

```rust
// src/ipc.rs — empty for now
```

```rust
// src/shelf/mod.rs
pub mod layout;
pub mod model;
pub mod paint;
pub mod thumbnail;
```

```rust
// src/shelf/model.rs, layout.rs, paint.rs, thumbnail.rs — empty for now
```

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles (empty modules are legal; existing code still in main.rs).

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "refactor: add module skeleton for shelf work"
```

### Task A2: Move clipboard code

**Files:**
- Create: `src/clipboard.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Move functions**

Cut `serve_wayland_clipboard` (main.rs ~731) and `copy_to_clipboard` (~748) into `src/clipboard.rs`. Add at the top of `clipboard.rs`:

```rust
use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Backend, DynResult};
```

Make both functions `pub`. In `main.rs`, replace call sites with `crate::clipboard::copy_to_clipboard(...)` / `crate::clipboard::serve_wayland_clipboard(...)` (or add `use crate::clipboard::{copy_to_clipboard, serve_wayland_clipboard};` near the top and leave call sites unqualified).

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS (same tests as before).

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "refactor: move clipboard code to clipboard.rs"
```

### Task A3: Move path/doctor helpers

**Files:**
- Create: `src/paths.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Move functions**

Cut into `src/paths.rs`: `target_path`, `edit_output_path`, `cache_dir`, `last_pointer_path`, `remember_last_screenshot`, `last_screenshot_path`, `normalize_path`, `ensure_file`, `default_save_path`, `temp_png`, `timestamp`, `has_cmd`, `print_doctor`, `self_test`. Add at top:

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Args, DynResult};
```

Make each `pub`. Note `print_doctor`/`self_test` reference `has_cmd` and capture — keep them here for now; `print_doctor` will be extended in Milestone G. In `main.rs` add `use crate::paths::*;` (or qualify call sites).

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "refactor: move path/doctor helpers to paths.rs"
```

### Task A4: Move capture code

**Files:**
- Create: `src/capture.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Move functions and their test**

Cut into `src/capture.rs`: `capture`, `flatten_to_rgb`, `strip_uniform_border`, `capture_x11*` family, `x11_capture_root`, `x11_active_window_id`, `x11_window_geometry`, `x11_pick_window_id`, `capture_wayland` and any `capture_*_bytes` helpers, `pick_focused_wl_output`, `parse_geometry`, `hyprland_active_window_geometry`, `parse_hypr_window_geometry`, `geometry_from_json_arrays`, `run_capture`. Also move the `#[cfg(test)] mod` containing `parse_hypr_geometry` (and any capture-related tests) — place a `#[cfg(test)] mod tests { ... }` at the bottom of `capture.rs` with just those tests.

Add at top of `capture.rs`:

```rust
use std::path::Path;
use std::process::Command;

use image::{DynamicImage, Rgba, RgbaImage, imageops};
use serde_json::Value;

use crate::{Backend, CaptureMode, DynResult};
use crate::select::run_select_with_parallel_capture;
```

Make the public-facing ones `pub` (`capture`, `strip_uniform_border`, and any `capture_*_bytes` used by `capture_to_stdout`).

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS, including the moved `parse_hypr_geometry` test.

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "refactor: move capture code to capture.rs"
```

### Task A5: Move selection overlay

**Files:**
- Create: `src/select.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Move functions**

Cut into `src/select.rs`: `SelectApp` (struct + impls + `eframe::App`), `run_select_with_parallel_capture`, `prep_compositor_for`. Add at top:

```rust
use std::process::Command;
use std::sync::{Arc, Mutex};

use eframe::egui;
use image::RgbaImage;

use crate::DynResult;
```

Make `run_select_with_parallel_capture` and `prep_compositor_for` `pub`.

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "refactor: move selection overlay to select.rs"
```

### Task A6: Move editor + render

**Files:**
- Create: `src/editor.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Move functions and tests**

Cut into `src/editor.rs`: the `MONO_*` / `SIDEBAR_W` etc. consts, `Tool`, `ActionIcon`, `thin_separator`, `tool_glyph`, `action_glyph`, `Annotation`, `EditorApp` (+ impls + `eframe::App`), `run_editor`, `render_annotations`, `blur_rect`, `draw_thick_line`, `draw_disc`, `draw_rect_outline`, `fill_rect`, `fill_rect_alpha`, `rect_bounds`, `put_pixel_checked`. Move the `#[cfg(test)] mod` tests for `render_*` (redaction/arrow/blur) into a tests module at the bottom of `editor.rs`. Add at top:

```rust
use std::path::Path;
use std::thread::JoinHandle;

use eframe::egui;
use image::{Rgba, RgbaImage, imageops};

use crate::{Backend, DynResult};
use crate::clipboard::copy_to_clipboard;
use crate::select::prep_compositor_for;
```

Make `run_editor` and `render_annotations` `pub`.

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS, including `render_redaction_blacks_region`, `render_arrow_draws_end`, `render_blur_changes_noisy_region`.

- [ ] **Step 3: Verify main.rs is now small**

Run: `wc -l src/main.rs`
Expected: roughly 200–260 lines (CLI types, parse_args, run/main routing, parser tests).

- [ ] **Step 4: Commit**

```bash
git add src/
git commit -m "refactor: move editor + annotation rendering to editor.rs"
```

---

## Milestone B — Pure logic + IPC (fully unit-tested, no Wayland yet)

### Task B1: IPC frame protocol

**Files:**
- Create/replace: `src/ipc.rs`
- Test: in `src/ipc.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

```rust
// src/ipc.rs
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{\"cmd\":\"add\"}", &[1, 2, 3, 4]).unwrap();
        let mut cur = Cursor::new(buf);
        let (header, payload) = read_frame(&mut cur).unwrap();
        assert_eq!(header, b"{\"cmd\":\"add\"}");
        assert_eq!(payload, vec![1, 2, 3, 4]);
    }

    #[test]
    fn request_add_roundtrip() {
        let req = Request::Add { source: "area".into(), png: vec![9, 8, 7] };
        let bytes = req.encode();
        let mut cur = Cursor::new(bytes);
        match Request::read(&mut cur).unwrap() {
            Request::Add { source, png } => {
                assert_eq!(source, "area");
                assert_eq!(png, vec![9, 8, 7]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn request_ping_and_reload_roundtrip() {
        let mut cur = Cursor::new(Request::Ping.encode());
        assert!(matches!(Request::read(&mut cur).unwrap(), Request::Ping));
        let mut cur = Cursor::new(Request::Reload { id: 42 }.encode());
        assert!(matches!(Request::read(&mut cur).unwrap(), Request::Reload { id: 42 }));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ipc 2>&1 | head` (will fail to compile: `write_frame`/`Request` undefined).
Expected: compile error / FAIL.

- [ ] **Step 3: Implement the protocol**

Add above the tests module in `src/ipc.rs`:

```rust
#[derive(Debug)]
pub enum Request {
    Add { source: String, png: Vec<u8> },
    Reload { id: u64 },
    Ping,
}

/// Frame = [u32 BE header_len][u32 BE payload_len][header bytes][payload bytes].
pub fn write_frame<W: Write>(w: &mut W, header: &[u8], payload: &[u8]) -> io::Result<()> {
    w.write_all(&(header.len() as u32).to_be_bytes())?;
    w.write_all(&(payload.len() as u32).to_be_bytes())?;
    w.write_all(header)?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut len4 = [0u8; 4];
    r.read_exact(&mut len4)?;
    let hlen = u32::from_be_bytes(len4) as usize;
    r.read_exact(&mut len4)?;
    let plen = u32::from_be_bytes(len4) as usize;
    let mut header = vec![0u8; hlen];
    r.read_exact(&mut header)?;
    let mut payload = vec![0u8; plen];
    r.read_exact(&mut payload)?;
    Ok((header, payload))
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Request::Add { source, png } => {
                let header = json!({ "cmd": "add", "source": source });
                write_frame(&mut buf, header.to_string().as_bytes(), png).unwrap();
            }
            Request::Reload { id } => {
                let header = json!({ "cmd": "reload", "id": id });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
            Request::Ping => {
                let header = json!({ "cmd": "ping" });
                write_frame(&mut buf, header.to_string().as_bytes(), &[]).unwrap();
            }
        }
        buf
    }

    pub fn read<R: Read>(r: &mut R) -> io::Result<Request> {
        let (header, payload) = read_frame(r)?;
        let v: Value = serde_json::from_slice(&header)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        match v.get("cmd").and_then(|c| c.as_str()) {
            Some("add") => Ok(Request::Add {
                source: v.get("source").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                png: payload,
            }),
            Some("reload") => Ok(Request::Reload {
                id: v.get("id").and_then(|i| i.as_u64()).unwrap_or(0),
            }),
            Some("ping") => Ok(Request::Ping),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown cmd: {other:?}"),
            )),
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib ipc`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ipc.rs
git commit -m "feat(ipc): unix-socket frame protocol + Request encode/decode"
```

### Task B2: Socket path + client send/ensure-daemon

**Files:**
- Modify: `src/ipc.rs`

- [ ] **Step 1: Write the failing test**

Add to the tests module:

```rust
#[test]
fn socket_path_uses_runtime_dir() {
    // Save/restore to avoid clobbering other tests' env.
    let prev = std::env::var("XDG_RUNTIME_DIR").ok();
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/user/test") };
    assert_eq!(socket_path(), PathBuf::from("/run/user/test/boltsnap.sock"));
    match prev {
        Some(v) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) },
        None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --lib ipc::tests::socket_path_uses_runtime_dir`
Expected: FAIL (`socket_path` undefined).

- [ ] **Step 3: Implement**

Add to `src/ipc.rs`:

```rust
pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("boltsnap.sock");
        }
    }
    std::env::temp_dir().join("boltsnap.sock")
}

/// True if a daemon answers a Ping on the socket.
pub fn daemon_alive() -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
            let _ = s.write_all(&Request::Ping.encode());
            // We don't require a structured reply; a successful connect+write is enough.
            true
        }
        Err(_) => false,
    }
}

/// Connect to the daemon, self-spawning `boltsnap daemon` if none is running.
fn ensure_daemon() -> io::Result<UnixStream> {
    if let Ok(s) = UnixStream::connect(socket_path()) {
        return Ok(s);
    }
    // No daemon: spawn one detached.
    let exe = std::env::current_exe()?;
    Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Poll for it to come up (~1s).
    for _ in 0..100 {
        if let Ok(s) = UnixStream::connect(socket_path()) {
            return Ok(s);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "daemon did not start"))
}

/// Send a request to the shelf daemon, starting it if needed.
pub fn send_to_shelf(req: Request) -> io::Result<()> {
    let mut stream = ensure_daemon()?;
    stream.write_all(&req.encode())?;
    stream.flush()?;
    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib ipc`
Expected: all PASS. (`ensure_daemon`/`send_to_shelf` are covered later by the manual daemon test.)

- [ ] **Step 5: Commit**

```bash
git add src/ipc.rs
git commit -m "feat(ipc): socket_path, daemon_alive, ensure_daemon, send_to_shelf"
```

### Task B3: Shelf model

**Files:**
- Create/replace: `src/shelf/model.rs`

- [ ] **Step 1: Write the failing test**

```rust
// src/shelf/model.rs
use std::path::PathBuf;
use image::RgbaImage;

#[cfg(test)]
mod tests {
    use super::*;

    fn img() -> RgbaImage { RgbaImage::new(2, 2) }

    #[test]
    fn add_assigns_unique_ids_newest_first() {
        let mut m = ShelfModel::new();
        let a = m.add(PathBuf::from("/tmp/a.png"), img(), "area".into());
        let b = m.add(PathBuf::from("/tmp/b.png"), img(), "full".into());
        assert_ne!(a, b);
        let ids: Vec<u64> = m.newest_first().map(|t| t.id).collect();
        assert_eq!(ids, vec![b, a]); // newest first
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn remove_returns_thumb_and_shrinks() {
        let mut m = ShelfModel::new();
        let a = m.add(PathBuf::from("/tmp/a.png"), img(), "area".into());
        let removed = m.remove(a).unwrap();
        assert_eq!(removed.png_path, PathBuf::from("/tmp/a.png"));
        assert!(m.is_empty());
        assert!(m.remove(a).is_none());
    }

    #[test]
    fn replace_thumb_swaps_image_keeps_id() {
        let mut m = ShelfModel::new();
        let a = m.add(PathBuf::from("/tmp/a.png"), RgbaImage::new(2, 2), "area".into());
        assert!(m.replace_thumb(a, RgbaImage::new(4, 4)));
        assert_eq!(m.get(a).unwrap().thumb.dimensions(), (4, 4));
        assert!(!m.replace_thumb(999, RgbaImage::new(1, 1)));
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --lib shelf::model`
Expected: FAIL (`ShelfModel` undefined).

- [ ] **Step 3: Implement**

Add above the tests module:

```rust
pub struct Thumb {
    pub id: u64,
    pub png_path: PathBuf,
    pub thumb: RgbaImage,
    pub source: String,
}

#[derive(Default)]
pub struct ShelfModel {
    thumbs: Vec<Thumb>, // index 0 = newest
    next_id: u64,
}

impl ShelfModel {
    pub fn new() -> Self {
        Self { thumbs: Vec::new(), next_id: 1 }
    }

    /// Insert a new thumbnail at the top of the shelf; returns its id.
    pub fn add(&mut self, png_path: PathBuf, thumb: RgbaImage, source: String) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.thumbs.insert(0, Thumb { id, png_path, thumb, source });
        id
    }

    pub fn remove(&mut self, id: u64) -> Option<Thumb> {
        let pos = self.thumbs.iter().position(|t| t.id == id)?;
        Some(self.thumbs.remove(pos))
    }

    pub fn get(&self, id: u64) -> Option<&Thumb> {
        self.thumbs.iter().find(|t| t.id == id)
    }

    pub fn replace_thumb(&mut self, id: u64, thumb: RgbaImage) -> bool {
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == id) {
            t.thumb = thumb;
            true
        } else {
            false
        }
    }

    pub fn is_empty(&self) -> bool { self.thumbs.is_empty() }
    pub fn len(&self) -> usize { self.thumbs.len() }

    /// Iterate newest-first (top of the shelf first).
    pub fn newest_first(&self) -> impl Iterator<Item = &Thumb> {
        self.thumbs.iter()
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib shelf::model`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/shelf/model.rs
git commit -m "feat(shelf): ShelfModel add/remove/get/replace, newest-first"
```

### Task B4: Thumbnail scaling

**Files:**
- Create/replace: `src/shelf/thumbnail.rs`

- [ ] **Step 1: Write the failing test**

```rust
// src/shelf/thumbnail.rs
use image::RgbaImage;
use image::imageops::FilterType;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_in_box_and_preserves_aspect() {
        let src = RgbaImage::new(800, 400); // 2:1
        let t = make_thumbnail(&src, 170, 120);
        let (w, h) = t.dimensions();
        assert!(w <= 170 && h <= 120, "got {w}x{h}");
        // 2:1 aspect: width is the limiting dim -> 170x85
        assert_eq!((w, h), (170, 85));
    }

    #[test]
    fn does_not_upscale_small_images() {
        let src = RgbaImage::new(50, 30);
        let t = make_thumbnail(&src, 170, 120);
        assert_eq!(t.dimensions(), (50, 30));
    }

    #[test]
    fn tall_image_limited_by_height() {
        let src = RgbaImage::new(200, 800); // 1:4
        let t = make_thumbnail(&src, 170, 120);
        let (w, h) = t.dimensions();
        assert!(h <= 120 && w <= 170);
        assert_eq!((w, h), (30, 120));
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --lib shelf::thumbnail`
Expected: FAIL (`make_thumbnail` undefined).

- [ ] **Step 3: Implement**

```rust
/// Downscale `src` to fit within (max_w, max_h), preserving aspect ratio.
/// Never upscales: images already smaller are returned at original size.
pub fn make_thumbnail(src: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return src.clone();
    }
    let scale = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    if scale >= 1.0 {
        return src.clone();
    }
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    image::imageops::resize(src, nw, nh, FilterType::Triangle)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib shelf::thumbnail`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/shelf/thumbnail.rs
git commit -m "feat(shelf): make_thumbnail aspect-preserving downscale"
```

### Task B5: Layout + hit-testing

**Files:**
- Create/replace: `src/shelf/layout.rs`

- [ ] **Step 1: Write the failing test**

```rust
// src/shelf/layout.rs

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LayoutConfig { LayoutConfig::default() }

    #[test]
    fn stacks_newest_on_top_and_sizes_surface() {
        let c = cfg();
        // newest-first: id 2 (170x100) on top, id 1 (160x90) below
        let lay = Layout::compute(&[(2, 170, 100), (1, 160, 90)], &c);
        assert_eq!(lay.thumbs.len(), 2);
        // top thumb at y = pad
        assert_eq!(lay.thumbs[0].id, 2);
        assert_eq!(lay.thumbs[0].y, c.pad);
        // second thumb below first + gap
        assert_eq!(lay.thumbs[1].id, 1);
        assert_eq!(lay.thumbs[1].y, c.pad + 100 + c.gap);
        // surface width = pad*2 + widest thumb
        assert_eq!(lay.width, c.pad * 2 + 170);
        // surface height = pad*2 + 100 + gap + 90
        assert_eq!(lay.height, c.pad * 2 + 100 + c.gap + 90);
    }

    #[test]
    fn hit_body_vs_icons_vs_outside() {
        let c = cfg();
        let lay = Layout::compute(&[(7, 170, 100)], &c);
        let r = &lay.thumbs[0];
        // center of the thumb -> body
        let cx = (r.x + r.w / 2) as f64;
        let cy = (r.y + r.h / 2) as f64;
        assert_eq!(lay.hit(cx, cy, &c), Some(Hit::Body(7)));
        // close icon is the rightmost icon in the top-right strip
        let close_cx = (r.x + r.w - c.pad_icon - c.icon / 2) as f64;
        let icon_cy = (r.y + c.pad_icon + c.icon / 2) as f64;
        assert_eq!(lay.hit(close_cx, icon_cy, &c), Some(Hit::Close(7)));
        // far outside
        assert_eq!(lay.hit(10_000.0, 10_000.0, &c), None);
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --lib shelf::layout`
Expected: FAIL (types undefined).

- [ ] **Step 3: Implement**

```rust
#[derive(Clone, Copy)]
pub struct LayoutConfig {
    pub pad: u32,      // outer padding inside the surface
    pub gap: u32,      // vertical gap between thumbs
    pub icon: u32,     // icon square size
    pub icon_gap: u32, // gap between icons
    pub pad_icon: u32, // inset of the icon strip from the thumb's top-right corner
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { pad: 12, gap: 10, icon: 22, icon_gap: 5, pad_icon: 6 }
    }
}

pub struct ThumbRect {
    pub id: u64,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Hit {
    Body(u64),
    Edit(u64),
    Copy(u64),
    Close(u64),
}

pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub thumbs: Vec<ThumbRect>,
}

impl Layout {
    /// `sizes` is newest-first: (id, thumb_w, thumb_h). Stacks top-to-bottom.
    pub fn compute(sizes: &[(u64, u32, u32)], cfg: &LayoutConfig) -> Layout {
        let widest = sizes.iter().map(|(_, w, _)| *w).max().unwrap_or(0);
        let mut thumbs = Vec::with_capacity(sizes.len());
        let mut y = cfg.pad;
        for (i, (id, w, h)) in sizes.iter().enumerate() {
            if i > 0 {
                y += cfg.gap;
            }
            thumbs.push(ThumbRect { id: *id, x: cfg.pad, y, w: *w, h: *h });
            y += *h;
        }
        let width = if sizes.is_empty() { 1 } else { cfg.pad * 2 + widest };
        let height = if sizes.is_empty() { 1 } else { y + cfg.pad };
        Layout { width, height, thumbs }
    }

    /// Icon strip lives at the thumb's top-right: [edit][copy][close], close rightmost.
    fn icon_rect(&self, r: &ThumbRect, slot_from_right: u32, cfg: &LayoutConfig) -> (u32, u32, u32, u32) {
        let right = r.x + r.w - cfg.pad_icon;
        let x = right - (slot_from_right + 1) * cfg.icon - slot_from_right * cfg.icon_gap;
        let y = r.y + cfg.pad_icon;
        (x, y, cfg.icon, cfg.icon)
    }

    pub fn hit(&self, x: f64, y: f64, cfg: &LayoutConfig) -> Option<Hit> {
        for r in &self.thumbs {
            let inside = x >= r.x as f64
                && x < (r.x + r.w) as f64
                && y >= r.y as f64
                && y < (r.y + r.h) as f64;
            if !inside {
                continue;
            }
            // icons: slot 0 = close (rightmost), slot 1 = copy, slot 2 = edit
            for (slot, make) in [
                (0u32, Hit::Close(r.id)),
                (1, Hit::Copy(r.id)),
                (2, Hit::Edit(r.id)),
            ] {
                let (ix, iy, iw, ih) = self.icon_rect(r, slot, cfg);
                if x >= ix as f64 && x < (ix + iw) as f64 && y >= iy as f64 && y < (iy + ih) as f64 {
                    return Some(make);
                }
            }
            return Some(Hit::Body(r.id));
        }
        None
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib shelf::layout`
Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/shelf/layout.rs
git commit -m "feat(shelf): layout stacking + pointer hit-testing"
```

### Task B6: BGRA compositing primitives

**Files:**
- Create/replace: `src/shelf/paint.rs`

- [ ] **Step 1: Write the failing test**

```rust
// src/shelf/paint.rs
use image::RgbaImage;

use crate::shelf::layout::{Layout, LayoutConfig};
use crate::shelf::model::ShelfModel;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_opaque_red_is_bgra() {
        let mut buf = vec![0u8; 4]; // 1x1
        fill_rect_bgra(&mut buf, 1, 1, 0, 0, 1, 1, 255, 0, 0, 255);
        // premultiplied opaque red -> B=0 G=0 R=255 A=255
        assert_eq!(buf, vec![0, 0, 255, 255]);
    }

    #[test]
    fn blit_straight_alpha_premultiplies() {
        // a 1x1 source: half-transparent white (straight alpha)
        let mut src = RgbaImage::new(1, 1);
        src.put_pixel(0, 0, image::Rgba([255, 255, 255, 128]));
        let mut buf = vec![0u8; 4]; // transparent dst
        blit_rgba(&mut buf, 1, 1, &src, 0, 0);
        // premultiplied: 255*128/255 = 128 for each color, A=128
        assert_eq!(buf, vec![128, 128, 128, 128]);
    }

    #[test]
    fn clear_zeros_buffer() {
        let mut buf = vec![9u8; 16];
        clear(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --lib shelf::paint`
Expected: FAIL (functions undefined).

- [ ] **Step 3: Implement primitives**

```rust
/// Set the whole canvas to transparent (0,0,0,0).
pub fn clear(canvas: &mut [u8]) {
    for b in canvas.iter_mut() {
        *b = 0;
    }
}

/// Fill an axis-aligned rect with a straight-alpha color, writing premultiplied BGRA.
/// (Source-over onto whatever is there is overkill for solid UI chrome; we overwrite.)
pub fn fill_rect_bgra(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    let x1 = (x + w).min(cw);
    let y1 = (y + h).min(ch);
    let (pr, pg, pb) = premul(r, g, b, a);
    for py in y..y1 {
        for px in x..x1 {
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = pb;
            canvas[idx + 1] = pg;
            canvas[idx + 2] = pr;
            canvas[idx + 3] = a;
        }
    }
}

/// Composite a straight-alpha RGBA image onto the canvas at (dx,dy) using source-over.
pub fn blit_rgba(canvas: &mut [u8], cw: u32, ch: u32, img: &RgbaImage, dx: u32, dy: u32) {
    let (iw, ih) = img.dimensions();
    for sy in 0..ih {
        let py = dy + sy;
        if py >= ch {
            break;
        }
        for sx in 0..iw {
            let px = dx + sx;
            if px >= cw {
                break;
            }
            let p = img.get_pixel(sx, sy).0;
            let (sr, sg, sb, sa) = (p[0], p[1], p[2], p[3]);
            let idx = ((py * cw + px) * 4) as usize;
            if sa == 255 {
                canvas[idx] = sb;
                canvas[idx + 1] = sg;
                canvas[idx + 2] = sr;
                canvas[idx + 3] = 255;
            } else if sa == 0 {
                // leave dst
            } else {
                // source-over with premultiplied dst
                let (spr, spg, spb) = premul(sr, sg, sb, sa);
                let inv = 255u32 - sa as u32;
                let db = canvas[idx] as u32;
                let dg = canvas[idx + 1] as u32;
                let dr = canvas[idx + 2] as u32;
                let da = canvas[idx + 3] as u32;
                canvas[idx] = (spb as u32 + db * inv / 255) as u8;
                canvas[idx + 1] = (spg as u32 + dg * inv / 255) as u8;
                canvas[idx + 2] = (spr as u32 + dr * inv / 255) as u8;
                canvas[idx + 3] = (sa as u32 + da * inv / 255) as u8;
            }
        }
    }
}

#[inline]
fn premul(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    let a = a as u32;
    (
        (r as u32 * a / 255) as u8,
        (g as u32 * a / 255) as u8,
        (b as u32 * a / 255) as u8,
    )
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib shelf::paint`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/shelf/paint.rs
git commit -m "feat(shelf): BGRA compositing primitives (clear/fill/blit)"
```

### Task B7: draw_shelf assembly

**Files:**
- Modify: `src/shelf/paint.rs`

- [ ] **Step 1: Write the failing test**

Add to `paint.rs` tests:

```rust
    #[test]
    fn draw_shelf_fills_thumb_region_nontransparent() {
        use std::path::PathBuf;
        let mut model = ShelfModel::new();
        let mut t = RgbaImage::new(40, 30);
        for p in t.pixels_mut() { *p = image::Rgba([10, 20, 200, 255]); }
        let id = model.add(PathBuf::from("/tmp/x.png"), t, "area".into());
        let cfg = LayoutConfig::default();
        let sizes: Vec<(u64, u32, u32)> =
            model.newest_first().map(|t| (t.id, t.thumb.width(), t.thumb.height())).collect();
        let layout = Layout::compute(&sizes, &cfg);
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        draw_shelf(&mut canvas, layout.width, layout.height, &layout, &model, Some(id), &cfg);
        // a pixel in the middle of the (only) thumb must be non-transparent
        let r = &layout.thumbs[0];
        let px = r.x + r.w / 2;
        let py = r.y + r.h / 2;
        let idx = ((py * layout.width + px) * 4) as usize;
        assert!(canvas[idx + 3] > 0, "thumb body should be opaque");
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --lib shelf::paint::tests::draw_shelf_fills_thumb_region_nontransparent`
Expected: FAIL (`draw_shelf` undefined).

- [ ] **Step 3: Implement**

```rust
const ICON_BG: (u8, u8, u8, u8) = (20, 20, 28, 220);
const ICON_CLOSE_BG: (u8, u8, u8, u8) = (40, 16, 16, 230);
const GLYPH: (u8, u8, u8, u8) = (240, 240, 245, 255);

/// Render the whole shelf: each thumbnail, plus hover icons on the hovered thumb.
pub fn draw_shelf(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    layout: &Layout,
    model: &ShelfModel,
    hovered: Option<u64>,
    cfg: &LayoutConfig,
) {
    clear(canvas);
    for r in &layout.thumbs {
        if let Some(thumb) = model.get(r.id) {
            blit_rgba(canvas, cw, ch, &thumb.thumb, r.x, r.y);
        }
        if hovered == Some(r.id) {
            draw_hover_icons(canvas, cw, ch, r, cfg);
        }
    }
}

fn draw_hover_icons(canvas: &mut [u8], cw: u32, ch: u32, r: &super::layout::ThumbRect, cfg: &LayoutConfig) {
    // slot 0 close (rightmost), 1 copy, 2 edit — mirror Layout::icon_rect math.
    for slot in 0..3u32 {
        let right = r.x + r.w - cfg.pad_icon;
        let x = right - (slot + 1) * cfg.icon - slot * cfg.icon_gap;
        let y = r.y + cfg.pad_icon;
        let bg = if slot == 0 { ICON_CLOSE_BG } else { ICON_BG };
        fill_rect_bgra(canvas, cw, ch, x, y, cfg.icon, cfg.icon, bg.0, bg.1, bg.2, bg.3);
        draw_glyph(canvas, cw, ch, slot, x, y, cfg.icon);
    }
}

/// Crude vector glyphs: 0=close (X), 1=copy (two squares), 2=edit (diagonal stroke).
fn draw_glyph(canvas: &mut [u8], cw: u32, ch: u32, slot: u32, x: u32, y: u32, s: u32) {
    let m = s / 5; // margin
    match slot {
        0 => {
            // X: two diagonals
            for i in m..(s - m) {
                put(canvas, cw, ch, x + i, y + i);
                put(canvas, cw, ch, x + i, y + (s - 1 - i));
            }
        }
        1 => {
            // copy: two overlapping square outlines
            stroke_rect(canvas, cw, ch, x + m, y + m, s - 2 * m - m, s - 2 * m - m);
            stroke_rect(canvas, cw, ch, x + 2 * m, y + 2 * m, s - 2 * m - m, s - 2 * m - m);
        }
        _ => {
            // edit: a single diagonal pencil stroke
            for i in m..(s - m) {
                put(canvas, cw, ch, x + i, y + (s - 1 - i));
            }
        }
    }
}

fn put(canvas: &mut [u8], cw: u32, ch: u32, x: u32, y: u32) {
    if x < cw && y < ch {
        let idx = ((y * cw + x) * 4) as usize;
        let (r, g, b, a) = GLYPH;
        let (pr, pg, pb) = premul(r, g, b, a);
        canvas[idx] = pb;
        canvas[idx + 1] = pg;
        canvas[idx + 2] = pr;
        canvas[idx + 3] = a;
    }
}

fn stroke_rect(canvas: &mut [u8], cw: u32, ch: u32, x: u32, y: u32, w: u32, h: u32) {
    for i in 0..w {
        put(canvas, cw, ch, x + i, y);
        put(canvas, cw, ch, x + i, y + h.saturating_sub(1));
    }
    for j in 0..h {
        put(canvas, cw, ch, x, y + j);
        put(canvas, cw, ch, x + w.saturating_sub(1), y + j);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib shelf::paint`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/shelf/paint.rs
git commit -m "feat(shelf): draw_shelf assembly with hover icons"
```

---

## Milestone C — Daemon entry + CLI wiring (compiles; capture routes to shelf)

### Task C1: Add SCTK dependencies and verify no version conflict

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add deps**

In `Cargo.toml` `[dependencies]` add:

```toml
smithay-client-toolkit = "0.19"
wayland-client = "0.31"
```

- [ ] **Step 2: Verify resolution + build**

Run: `cargo build 2>&1 | tail -20`
Expected: builds. Then confirm a single wayland-client major:
Run: `cargo tree -i wayland-client | head`
Expected: one `wayland-client v0.31.x` node (shared with winit). If a second major appears, STOP and reconcile versions before continuing.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add smithay-client-toolkit + wayland-client for the shelf"
```

### Task C2: decide_post_capture routing function

**Files:**
- Modify: `src/main.rs` (the `Args` struct + add the function + tests)

- [ ] **Step 1: Add `copy_explicit` to Args**

In `Args` add field `copy_explicit: bool` (default `false` in `Default`). In `parse_args`, set `args.copy_explicit = true;` in BOTH the `"--copy"` and `"--no-copy"` arms (in addition to setting `copy`).

- [ ] **Step 2: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `main.rs`:

```rust
    fn args_with(f: impl FnOnce(&mut Args)) -> Args {
        let mut a = Args::default();
        f(&mut a);
        a
    }

    #[test]
    fn wayland_bare_goes_to_shelf_no_copy() {
        let a = args_with(|_| {});
        assert!(matches!(
            decide_post_capture(&a, Backend::Wayland),
            PostCapture::Shelf { copy: false }
        ));
    }

    #[test]
    fn wayland_copy_flag_shelf_plus_copy() {
        let a = args_with(|a| { a.copy = true; a.copy_explicit = true; });
        assert!(matches!(
            decide_post_capture(&a, Backend::Wayland),
            PostCapture::Shelf { copy: true }
        ));
    }

    #[test]
    fn x11_bare_is_copy_only() {
        let a = args_with(|_| {});
        assert!(matches!(decide_post_capture(&a, Backend::X11), PostCapture::CopyOnly));
    }

    #[test]
    fn edit_and_file_and_stdout_take_priority() {
        let e = args_with(|a| a.edit = true);
        assert!(matches!(decide_post_capture(&e, Backend::Wayland), PostCapture::Edit));
        let f = args_with(|a| a.output = Some(std::path::PathBuf::from("/tmp/x.png")));
        assert!(matches!(decide_post_capture(&f, Backend::Wayland), PostCapture::File { .. }));
        let s = args_with(|a| a.output = Some(std::path::PathBuf::from("-")));
        assert!(matches!(decide_post_capture(&s, Backend::Wayland), PostCapture::Stdout));
    }
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test --lib decide_post_capture 2>&1 | head`
Expected: FAIL (undefined).

- [ ] **Step 4: Implement**

Add to `main.rs`:

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum PostCapture {
    Stdout,
    Edit,
    File { copy: bool },
    Shelf { copy: bool },
    CopyOnly,
}

/// Decide what to do with a freshly captured screenshot. `backend` is already resolved.
pub fn decide_post_capture(args: &Args, backend: Backend) -> PostCapture {
    if is_stdout_target(args) {
        return PostCapture::Stdout;
    }
    if args.edit {
        return PostCapture::Edit;
    }
    if args.output.is_some() || args.save {
        return PostCapture::File { copy: args.copy };
    }
    match backend {
        Backend::Wayland => PostCapture::Shelf { copy: args.copy_explicit && args.copy },
        _ => PostCapture::CopyOnly,
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib`
Expected: all PASS (including existing parser tests).

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: decide_post_capture routing (shelf default on wayland)"
```

### Task C3: Wire capture_flow to the shelf

**Files:**
- Modify: `src/capture.rs` (or wherever `capture_flow` now lives — keep it in `main.rs` if it routes; this plan keeps `capture_flow` in `main.rs`)

- [ ] **Step 1: Rewrite `capture_flow` to use the decision**

Replace the body of `capture_flow` (main.rs) with:

```rust
fn capture_flow(args: &Args) -> DynResult<()> {
    let mode = CaptureMode::parse(&args.command)?;
    if is_stdout_target(args) {
        return capture_to_stdout(mode, args.backend);
    }
    let resolved_backend = args.backend.resolved()?;

    let output = if args.edit {
        edit_output_path(args).unwrap_or_else(|| temp_png("shot"))
    } else {
        target_path(args)
    };
    let resolved = crate::capture::capture(mode, &output, args.backend)?;
    crate::capture::strip_uniform_border(&output)?;

    match decide_post_capture(args, resolved_backend) {
        PostCapture::Stdout => unreachable!("handled above"),
        PostCapture::Edit => {
            // run_editor(image_path: PathBuf, output_path: Option<PathBuf>, copy_after: bool, backend: Backend)
            //   -> DynResult<PathBuf>  (returns the final saved path)
            let final_path = crate::editor::run_editor(
                output.clone(),
                edit_output_path(args),
                args.copy,
                resolved,
            )?;
            remember_last_screenshot(&final_path)?;
            println!("captured -> {}", final_path.display());
        }
        PostCapture::File { copy } => {
            if copy {
                crate::clipboard::copy_to_clipboard(&output, resolved)?;
            }
            remember_last_screenshot(&output)?;
            println!("captured -> {}", output.display());
            if copy {
                println!("copied to clipboard");
            }
        }
        PostCapture::CopyOnly => {
            crate::clipboard::copy_to_clipboard(&output, resolved)?;
            remember_last_screenshot(&output)?;
            println!("captured -> {}", output.display());
            println!("copied to clipboard");
        }
        PostCapture::Shelf { copy } => {
            remember_last_screenshot(&output)?;
            if copy {
                crate::clipboard::copy_to_clipboard(&output, resolved)?;
            }
            let png = std::fs::read(&output)?;
            crate::ipc::send_to_shelf(crate::ipc::Request::Add {
                source: mode.label().to_string(),
                png,
            })?;
            println!("captured -> shelf");
            if copy {
                println!("copied to clipboard");
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS. (The shelf branch needs a running daemon to do anything visible; that's verified in Milestone D.)

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "feat: route wayland interactive captures to the shelf"
```

### Task C4: `daemon` command routing + stub `run_daemon`

**Files:**
- Modify: `src/main.rs` (routing), `src/shelf/mod.rs`

- [ ] **Step 1: Add a temporary stub so routing compiles**

In `src/shelf/mod.rs` add:

```rust
use crate::DynResult;

pub fn run_daemon() -> DynResult<()> {
    eprintln!("boltsnap daemon: not yet implemented");
    Ok(())
}
```

In `main.rs` `run()` match, add a `"daemon" => crate::shelf::run_daemon(),` arm. Add `daemon` to the `usage()` text.

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles. `cargo run -- daemon` prints the stub line and exits.

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "feat: route `boltsnap daemon` to shelf::run_daemon stub"
```

---

## Milestone D — Layer-shell shelf rendering (manual verification on Hyprland)

From here, tasks are verified by **running on a live Hyprland session** (no compositor-free unit test is possible). Each task still ends with `cargo build` + `cargo test` (must stay green) and a precise manual check.

> **Worker note:** These tasks build one big SCTK state struct incrementally. The full handler set must be present for the `delegate_*!` macros to satisfy their trait bounds, so Task D1 lands the entire scaffold with empty handler bodies, and later tasks fill them in. The code below is grounded in the compile-verified API reference in the plan header.

### Task D1: SCTK scaffold — Daemon state, globals, delegates, empty handlers

**Files:**
- Replace: `src/shelf/mod.rs`

- [ ] **Step 1: Write the full scaffold**

Replace `src/shelf/mod.rs` with (handlers present, bodies minimal; rendering wired in D2):

```rust
pub mod layout;
pub mod model;
pub mod paint;
pub mod thumbnail;

use std::os::unix::net::UnixListener;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer::WlPointer, wl_seat::WlSeat, wl_surface::WlSurface},
    Connection, QueueHandle,
};

use crate::shelf::layout::{Layout, LayoutConfig};
use crate::shelf::model::ShelfModel;
use crate::DynResult;

pub struct Daemon {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    pointer: Option<WlPointer>,

    model: ShelfModel,
    layout: Layout,
    cfg: LayoutConfig,
    width: u32,
    height: u32,
    hovered: Option<u64>,
    exit: bool,
}

pub fn run_daemon() -> DynResult<()> {
    // Single-instance: if a daemon already answers, do nothing.
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    let sock = crate::ipc::socket_path();
    let _ = std::fs::remove_file(&sock); // clear stale socket
    let _listener = UnixListener::bind(&sock)?; // wired into calloop in Task C-> D5
    drop(_listener); // placeholder; real listener added in Task D5

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<Daemon>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let pool = SlotPool::new(256 * 256 * 4, &shm)?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("boltsnap"),
        None,
    );
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
    layer.set_margin(0, 0, 24, 24);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(-1);
    layer.set_size(1, 1);
    layer.commit();

    let cfg = LayoutConfig::default();
    let mut daemon = Daemon {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        pointer: None,
        model: ShelfModel::new(),
        layout: Layout::compute(&[], &cfg),
        cfg,
        width: 1,
        height: 1,
        hovered: None,
        exit: false,
    };

    loop {
        event_queue.blocking_dispatch(&mut daemon)?;
        if daemon.exit {
            break;
        }
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

impl CompositorHandler for Daemon {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Daemon {
    fn output_state(&mut self) -> &mut OutputState { &mut self.output_state }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Daemon {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 != 0 { self.width = configure.new_size.0; }
        if configure.new_size.1 != 0 { self.height = configure.new_size.1; }
        self.draw(qh);
    }
}

impl SeatHandler for Daemon {
    fn seat_state(&mut self) -> &mut SeatState { &mut self.seat_state }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: WlSeat, cap: Capability) {
        if cap == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl PointerHandler for Daemon {
    fn pointer_frame(&mut self, _: &Connection, _qh: &QueueHandle<Self>, _: &WlPointer, _events: &[PointerEvent]) {
        // filled in Milestone E
    }
}

impl ShmHandler for Daemon {
    fn shm_state(&mut self) -> &mut Shm { &mut self.shm }
}

impl ProvidesRegistryState for Daemon {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry_state }
    registry_handlers![OutputState, SeatState];
}

impl Daemon {
    fn draw(&mut self, _qh: &QueueHandle<Self>) {
        // filled in Task D2
    }
}

delegate_compositor!(Daemon);
delegate_output!(Daemon);
delegate_shm!(Daemon);
delegate_seat!(Daemon);
delegate_pointer!(Daemon);
delegate_layer!(Daemon);
delegate_registry!(Daemon);
```

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: compiles (this is the critical compile-gate for the SCTK API). Tests still green.

- [ ] **Step 3: Manual smoke**

Run: `cargo run -- daemon` in a Hyprland session.
Expected: process stays alive (event loop running), no panic. Nothing visible yet (size 1x1, empty draw). Ctrl-C to stop.

- [ ] **Step 4: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): SCTK layer-shell daemon scaffold (compiles, runs)"
```

### Task D2: Render the shelf buffer in `draw`

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Implement `draw` + a `relayout` helper**

Replace the `impl Daemon { fn draw ... }` block with:

```rust
impl Daemon {
    /// Recompute layout from the model and resize the layer surface to match.
    fn relayout(&mut self) {
        let sizes: Vec<(u64, u32, u32)> = self
            .model
            .newest_first()
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        self.layout = Layout::compute(&sizes, &self.cfg);
        self.layer.set_size(self.layout.width, self.layout.height);
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let stride = (w * 4) as i32;
        let needed = (w * h * 4) as usize;
        // grow the pool if necessary by recreating the buffer each draw (simple + safe)
        let (buffer, canvas) = match self.pool.create_buffer(
            w as i32,
            h as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        debug_assert_eq!(canvas.len(), needed);

        crate::shelf::paint::draw_shelf(
            canvas, w, h, &self.layout, &self.model, self.hovered, &self.cfg,
        );

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        let _ = buffer.attach_to(surface);
        self.layer.commit();
        let _ = qh; // frame callbacks not needed; we redraw on demand
    }
}
```

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): render thumbnails into the layer-shell buffer"
```

### Task D3: Add-thumb pipeline (decode PNG → thumbnail → model → redraw)

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Add an `add_png` method**

Add to `impl Daemon`:

```rust
    /// Ingest a PNG: persist a daemon-owned temp copy, scale a thumbnail, show it.
    fn add_png(&mut self, png: &[u8], source: &str, qh: &QueueHandle<Self>) {
        let img = match image::load_from_memory(png) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("boltsnap daemon: bad PNG: {e}");
                return;
            }
        };
        // daemon-owned temp file for editor + drag uri-list
        let path = crate::paths::temp_png("shelf");
        if let Err(e) = std::fs::write(&path, png) {
            eprintln!("boltsnap daemon: temp write failed: {e}");
            return;
        }
        let thumb = crate::shelf::thumbnail::make_thumbnail(&img, 170, 120);
        self.model.add(path, thumb, source.to_string());
        self.relayout();
        self.draw(qh);
    }
```

(Make `crate::paths::temp_png` `pub` if it is not already.)

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS (method is unused until the socket listener calls it in D5 — allow the dead-code warning or call it from D5 in the same change).

- [ ] **Step 3: Commit**

```bash
git add src/
git commit -m "feat(shelf): add_png ingest pipeline (decode/scale/store/redraw)"
```

### Task D4: Switch the event loop to calloop and integrate the socket listener

**Files:**
- Modify: `Cargo.toml` (only if calloop/calloop-wayland-source aren't reachable as SCTK reexports), `src/shelf/mod.rs`

- [ ] **Step 1: Confirm calloop reexports**

SCTK reexports calloop. Use `smithay_client_toolkit::reexports::calloop` and `::calloop_wayland_source`. Verify by building after the next step. (If they are not reexported in this SCTK build, add `calloop = "0.13"` and `calloop-wayland-source = "0.3"` to `Cargo.toml` matching SCTK 0.19's versions, then `cargo tree -i calloop` to confirm a single version.)

- [ ] **Step 2: Replace the blocking loop with a calloop loop + listener source**

In `run_daemon`, remove the placeholder `_listener`/`drop` lines and the `loop { event_queue.blocking_dispatch ... }`. Replace the tail of `run_daemon` (after `daemon` is constructed) with:

```rust
    use smithay_client_toolkit::reexports::calloop::{
        EventLoop, Interest, Mode, PostAction, generic::Generic,
    };
    use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;

    let listener = UnixListener::bind(&sock)?;
    listener.set_nonblocking(true)?;

    let mut event_loop: EventLoop<Daemon> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    WaylandSource::new(conn.clone(), event_queue)
        .insert(handle.clone())
        .map_err(|e| format!("insert wayland source: {e}"))?;

    let source = Generic::new(listener, Interest::READ, Mode::Level);
    handle
        .insert_source(source, |_readiness, listener, daemon: &mut Daemon| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => daemon.handle_client(stream),
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        eprintln!("boltsnap daemon: accept error: {e}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| format!("insert listener source: {e}"))?;

    // The redraw after handle_client needs the queue handle; stash it on the daemon.
    daemon.qh = Some(qh.clone());

    while !daemon.exit {
        event_loop
            .dispatch(std::time::Duration::from_millis(250), &mut daemon)
            .map_err(|e| format!("dispatch: {e}"))?;
    }
    let _ = std::fs::remove_file(&sock);
    Ok(())
```

Move `let _listener = UnixListener::bind(&sock)?; drop(...)` lines out (we bind once here). Bind the socket AFTER `daemon_alive()` returns false and after `remove_file`. Keep the earlier `remove_file(&sock)` for staleness.

Add a `qh: Option<QueueHandle<Daemon>>` field to `Daemon` (init `qh: None`), since calloop callbacks get `&mut Daemon` without the handle.

- [ ] **Step 3: Add `handle_client`**

Add to `impl Daemon`:

```rust
    fn handle_client(&mut self, mut stream: std::os::unix::net::UnixStream) {
        use std::io::Write;
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let req = match crate::ipc::Request::read(&mut stream) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("boltsnap daemon: bad request: {e}");
                return;
            }
        };
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return,
        };
        match req {
            crate::ipc::Request::Ping => {
                let _ = stream.write_all(b"PONG");
            }
            crate::ipc::Request::Add { source, png } => {
                self.add_png(&png, &source, &qh);
            }
            crate::ipc::Request::Reload { id } => {
                self.reload(id, &qh);
            }
        }
    }

    fn reload(&mut self, id: u64, qh: &QueueHandle<Self>) {
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        if let Ok(img) = image::open(&path) {
            let thumb = crate::shelf::thumbnail::make_thumbnail(&img.to_rgba8(), 170, 120);
            self.model.replace_thumb(id, thumb);
            self.relayout();
            self.draw(qh);
        }
    }
```

(`Request::read` returning `Ping`/`Add`/`Reload` matches Task B1.)

- [ ] **Step 4: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 5: Manual end-to-end (the milestone payoff)**

In a Hyprland session, two terminals:
- Terminal 1: `cargo run -- daemon`
- Terminal 2: `cargo run -- area` then drag-select a region.

Expected: a thumbnail of your selection appears in the **bottom-left corner**, floating above other windows, without stealing focus. Run `cargo run -- area` again → a second thumbnail stacks **above** the first. Also verify self-spawn: stop the daemon (Ctrl-C in T1), then in T2 run `cargo run -- area` — a daemon should auto-start and the thumbnail should appear.

- [ ] **Step 6: Commit**

```bash
git add src/ Cargo.toml Cargo.lock
git commit -m "feat(shelf): calloop loop + unix-socket listener -> live thumbnails"
```

---

## Milestone E — Pointer: hover highlight + click-to-copy + close

### Task E1: Hover tracking

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Implement hover in `pointer_frame`**

Replace `PointerHandler::pointer_frame` body:

```rust
    fn pointer_frame(&mut self, _: &Connection, _qh: &QueueHandle<Self>, _: &WlPointer, events: &[PointerEvent]) {
        use smithay_client_toolkit::seat::pointer::PointerEventKind::*;
        let surface = self.layer.wl_surface().clone();
        let mut redraw = false;
        for ev in events {
            if ev.surface != surface {
                continue;
            }
            let (x, y) = ev.position;
            match ev.kind {
                Leave { .. } => {
                    if self.hovered.is_some() {
                        self.hovered = None;
                        redraw = true;
                    }
                }
                Enter { .. } | Motion { .. } => {
                    let now = match self.layout.hit(x, y, &self.cfg) {
                        Some(crate::shelf::layout::Hit::Body(id))
                        | Some(crate::shelf::layout::Hit::Edit(id))
                        | Some(crate::shelf::layout::Hit::Copy(id))
                        | Some(crate::shelf::layout::Hit::Close(id)) => Some(id),
                        None => None,
                    };
                    if now != self.hovered {
                        self.hovered = now;
                        redraw = true;
                    }
                }
                _ => {}
            }
        }
        if redraw {
            if let Some(qh) = self.qh.clone() {
                self.draw(&qh);
            }
        }
    }
```

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Manual**

Run daemon + capture one shot. Move the mouse over the thumbnail.
Expected: the three icons (close/copy/edit) appear while hovering and disappear on leave.

- [ ] **Step 4: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): hover highlight reveals thumbnail icons"
```

### Task E2: Press/release tracking + click semantics (copy / close)

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Add press-state fields**

Add to `Daemon`: `press: Option<PressState>,` and define near the top:

```rust
struct PressState {
    id: u64,
    hit: crate::shelf::layout::Hit,
    x: f64,
    y: f64,
    serial: u32,
    dragging: bool,
}
```

Init `press: None` in the constructor.

- [ ] **Step 2: Handle Press/Release in `pointer_frame`**

Extend the `match ev.kind` with (add these arms before `_ => {}`):

```rust
                Press { button, serial, .. } if button == smithay_client_toolkit::seat::pointer::BTN_LEFT => {
                    if let Some(hit) = self.layout.hit(x, y, &self.cfg) {
                        let id = match hit {
                            crate::shelf::layout::Hit::Body(i)
                            | crate::shelf::layout::Hit::Edit(i)
                            | crate::shelf::layout::Hit::Copy(i)
                            | crate::shelf::layout::Hit::Close(i) => i,
                        };
                        self.press = Some(PressState { id, hit, x, y, serial, dragging: false });
                    }
                }
                Release { button, .. } if button == smithay_client_toolkit::seat::pointer::BTN_LEFT => {
                    if let Some(p) = self.press.take() {
                        if !p.dragging {
                            self.on_click(p.hit, &mut redraw);
                        }
                    }
                }
```

Import `BTN_LEFT` (already via the path used above) and ensure `PointerEventKind` import covers `Press`/`Release`.

- [ ] **Step 3: Implement `on_click`**

Add to `impl Daemon`:

```rust
    fn on_click(&mut self, hit: crate::shelf::layout::Hit, redraw: &mut bool) {
        use crate::shelf::layout::Hit;
        match hit {
            Hit::Body(id) | Hit::Copy(id) => {
                if let Some(t) = self.model.get(id) {
                    let path = t.png_path.clone();
                    if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland) {
                        eprintln!("boltsnap daemon: copy failed: {e}");
                    }
                }
            }
            Hit::Close(id) => {
                if let Some(t) = self.model.remove(id) {
                    let _ = std::fs::remove_file(&t.png_path);
                    if self.hovered == Some(id) {
                        self.hovered = None;
                    }
                    self.relayout();
                    *redraw = true;
                }
            }
            Hit::Edit(id) => {
                self.spawn_editor(id);
            }
        }
    }
```

Add a temporary stub so it compiles (real one in Milestone G):

```rust
    fn spawn_editor(&mut self, _id: u64) {
        // implemented in Milestone G
    }
```

- [ ] **Step 4: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 5: Manual**

Daemon + one shot. Click the thumbnail body → paste (Ctrl+V) into a text field; the image should paste. Hover + click the ⧉ icon → also copies. Hover + click ✕ → thumbnail disappears.

- [ ] **Step 6: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): click=copy, copy icon, close icon"
```

---

## Milestone F — Drag-and-drop source + auto-copy fallback

### Task F1: Data device manager + drag state + companion handlers

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Add imports, fields, binding, and the two companion handlers**

Add imports:

```rust
use smithay_client_toolkit::{
    data_device_manager::{
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{DataSourceHandler, DragSource},
        DataDeviceManagerState, WritePipe,
    },
    delegate_data_device,
};
use wayland_client::protocol::{
    wl_data_device::WlDataDevice, wl_data_device_manager::DndAction, wl_data_source::WlDataSource,
};
```

Add fields to `Daemon`:

```rust
    ddm: DataDeviceManagerState,
    data_device: Option<DataDevice>,
    drag_source: Option<DragSource>,
    drag_path: Option<std::path::PathBuf>,
    drop_ok: bool,
```

Bind in `run_daemon` (after the other binds):

```rust
    let ddm = DataDeviceManagerState::bind(&globals, &qh)?;
```

and set `ddm, data_device: None, drag_source: None, drag_path: None, drop_ok: false,` in the constructor.

Acquire the data device in `SeatHandler::new_capability` (alongside the pointer branch):

```rust
        if self.data_device.is_none() {
            self.data_device = Some(self.seat_state_data_device(qh, &seat));
        }
```

Add the helper:

```rust
    fn seat_state_data_device(&self, qh: &QueueHandle<Self>, seat: &WlSeat) -> DataDevice {
        self.ddm.get_data_device(qh, seat)
    }
```

Add the companion handler impls (empty bodies are valid for a pure source) and the delegate:

```rust
impl DataDeviceHandler for Daemon {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _x: f64, _y: f64, _: &WlSurface) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _x: f64, _y: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for Daemon {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}

delegate_data_device!(Daemon);
```

> Note: confirm the exact `DataDeviceHandler`/`DataOfferHandler` method signatures against `cargo doc -p smithay-client-toolkit --no-deps` for the resolved 0.19.x; the names above match the verified reference, but if the build complains about a signature, adjust the parameter list to match the compiler error (bodies stay empty).

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): bind data device manager + companion DnD handlers"
```

### Task F2: Start drag on motion threshold; serve data; auto-copy fallback

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Begin drag in `pointer_frame` Motion**

In the `Enter { .. } | Motion { .. }` arm, after the hover logic, add drag-start detection:

```rust
                    // drag start: left button held, moved past threshold, press began on a body
                    if let Some(p) = self.press.as_mut() {
                        if !p.dragging {
                            let dx = x - p.x;
                            let dy = y - p.y;
                            if (dx * dx + dy * dy) > 36.0 {
                                if matches!(p.hit, crate::shelf::layout::Hit::Body(_)) {
                                    p.dragging = true;
                                    let id = p.id;
                                    let serial = p.serial;
                                    self.begin_drag(id, serial);
                                }
                            }
                        }
                    }
```

(Threshold 6px → 36 squared.)

- [ ] **Step 2: Implement `begin_drag`**

Add to `impl Daemon`:

```rust
    fn begin_drag(&mut self, id: u64, serial: u32) {
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return,
        };
        let device = match self.data_device.as_ref() {
            Some(d) => d,
            None => return,
        };
        let source = self.ddm.create_drag_and_drop_source(
            &qh,
            ["image/png", "text/uri-list"],
            DndAction::Copy,
        );
        let origin = self.layer.wl_surface();
        source.start_drag(device, origin, None, serial);
        self.drag_path = Some(path);
        self.drop_ok = false;
        self.drag_source = Some(source);
    }
```

- [ ] **Step 3: Implement `DataSourceHandler`**

```rust
impl DataSourceHandler for Daemon {
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: Option<String>) {}

    fn send_request(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource, mime: String, fd: WritePipe) {
        use std::io::Write;
        use std::os::fd::OwnedFd;
        let is_ours = self.drag_source.as_ref().map(|d| d.inner() == source).unwrap_or(false);
        if !is_ours {
            return;
        }
        let Some(path) = self.drag_path.clone() else { return; };
        let mut file = std::fs::File::from(OwnedFd::from(fd));
        match mime.as_str() {
            "text/uri-list" => {
                let abs = std::fs::canonicalize(&path).unwrap_or(path);
                let uri = format!("file://{}\r\n", abs.display());
                let _ = file.write_all(uri.as_bytes());
            }
            _ => {
                if let Ok(bytes) = std::fs::read(&path) {
                    let _ = file.write_all(&bytes);
                }
            }
        }
        // file drops here -> fd closed -> EOF to reader
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drop_ok = true;
    }

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drag_source = None;
        self.drag_path = None;
        self.press = None;
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        // No valid target took the drop -> auto-copy fallback.
        if !self.drop_ok {
            if let Some(path) = self.drag_path.clone() {
                if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland) {
                    eprintln!("boltsnap daemon: fallback copy failed: {e}");
                }
            }
        }
        self.drag_source = None;
        self.drag_path = None;
        self.press = None;
    }

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}
```

- [ ] **Step 4: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 5: Manual**

Daemon + one shot. Press on the thumbnail body and drag into:
- a native Wayland app that accepts image paste/drop (e.g. a file manager window, or a chat input) — the image or file should drop.
- Drag and release over empty desktop → nothing accepts → verify Ctrl+V now pastes the image (auto-copy fallback fired).

- [ ] **Step 6: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): wl_data_device drag source (png+uri) + auto-copy fallback"
```

### Task F3: Drag icon surface (the thumbnail follows the cursor)

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Create an icon surface and pass it to start_drag**

Add a field `icon_surface: Option<WlSurface>` and a `CompositorState` handle (store `compositor` in `Daemon` as `compositor: CompositorState`). In `begin_drag`, before `start_drag`, build a small icon buffer from the thumb and attach it to a fresh surface:

```rust
        // build a drag icon from the thumbnail
        let icon = self.compositor.create_surface(&qh);
        if let Some(t) = self.model.get(id) {
            let (iw, ih) = t.thumb.dimensions();
            let stride = (iw * 4) as i32;
            if let Ok((buf, canvas)) = self.pool.create_buffer(
                iw as i32, ih as i32, stride,
                wayland_client::protocol::wl_shm::Format::Argb8888,
            ) {
                crate::shelf::paint::clear(canvas);
                crate::shelf::paint::blit_rgba(canvas, iw, ih, &t.thumb, 0, 0);
                let _ = buf.attach_to(&icon);
                icon.commit();
            }
        }
        // ... then:
        source.start_drag(device, origin, Some(&icon), serial);
        self.icon_surface = Some(icon);
```

Store `compositor` in the constructor (`compositor,`) and init `icon_surface: None`. Clear `self.icon_surface = None;` in both `dnd_finished` and `cancelled`.

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Manual**

Drag a thumbnail → a small image of the screenshot should follow the cursor during the drag.

- [ ] **Step 4: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): thumbnail drag icon follows the cursor"
```

---

## Milestone G — Editor integration, doctor, docs

### Task G1: Editor integration (✎ opens editor; result reloads thumbnail)

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Implement `spawn_editor`**

Replace the stub:

```rust
    fn spawn_editor(&mut self, id: u64) {
        let path = match self.model.get(id) {
            Some(t) => t.png_path.clone(),
            None => return,
        };
        // Run the editor in a thread; overwrite the temp PNG in place; then ask the
        // daemon to reload that thumbnail via its own socket.
        std::thread::spawn(move || {
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(_) => return,
            };
            let status = std::process::Command::new(exe)
                .arg("edit")
                .arg(&path)
                .arg("-o")
                .arg(&path)
                .arg("--no-copy")
                .status();
            if matches!(status, Ok(s) if s.success()) {
                let _ = crate::ipc::send_to_shelf(crate::ipc::Request::Reload { id });
            }
        });
    }
```

(`send_to_shelf` connects to the already-running daemon; `ensure_daemon` will just connect. The daemon's `handle_client` handles `Reload` via Task D4's `reload`.)

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Manual**

Daemon + one shot. Hover, click ✎ → the annotation editor opens with that screenshot. Draw something, save (Space/Enter), close. The thumbnail in the shelf should update to the annotated version.

- [ ] **Step 4: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): edit icon opens editor and reloads the thumbnail"
```

### Task G2: doctor checks for shelf

**Files:**
- Modify: `src/paths.rs` (where `print_doctor` lives)

- [ ] **Step 1: Extend `print_doctor`**

Add to `print_doctor` (after existing output):

```rust
    // Shelf / Wayland layer-shell checks.
    let on_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
    println!("wayland session:   {}", if on_wayland { "yes" } else { "no" });
    println!(
        "shelf daemon:      {}",
        if crate::ipc::daemon_alive() { "running" } else { "not running" }
    );
    let sock = crate::ipc::socket_path();
    println!("shelf socket:      {}", sock.display());
```

(A full `zwlr_layer_shell_v1` capability probe requires a Wayland roundtrip; the daemon's own `LayerShell::bind` already fails loudly if it's missing, so doctor reports session + daemon status, which is the actionable info.)

- [ ] **Step 2: Build + test**

Run: `cargo build && cargo test`
Expected: PASS.

- [ ] **Step 3: Manual**

Run: `cargo run -- doctor`
Expected: shows wayland session yes/no, daemon running/not, socket path.

- [ ] **Step 4: Commit**

```bash
git add src/paths.rs
git commit -m "feat(doctor): report wayland session, shelf daemon, socket path"
```

### Task G3: README + usage update

**Files:**
- Modify: `README.md`, `src/main.rs` (`usage()`)

- [ ] **Step 1: Update `usage()`**

Add lines to the usage string:

```
  boltsnap daemon                         run the screenshot shelf (auto-started on demand)
```

And note that on Wayland the default now sends to the shelf.

- [ ] **Step 2: Update README**

Add a "Screenshot Shelf (Wayland/Hyprland)" section to `README.md` documenting: the shelf behavior, that `boltsnap area` on Wayland now puts the shot in the shelf (click=copy, drag=DnD, ✎ edit, ✕ dismiss), `--copy` to also auto-copy, `-o`/`--save`/`-o -` keep file/stdout behavior, X11 unchanged, optional `exec-once = boltsnap daemon` for autostart, and that the shelf is RAM-only (cleared on daemon restart).

- [ ] **Step 3: Build + test + commit**

Run: `cargo build && cargo test`
Expected: PASS.

```bash
git add README.md src/main.rs
git commit -m "docs: document the screenshot shelf and daemon"
```

### Task G4: Full regression + manual acceptance pass

**Files:** none (verification only)

- [ ] **Step 1: Automated gate**

Run: `cargo test`
Expected: all unit tests pass (ipc, shelf::model, shelf::thumbnail, shelf::layout, shelf::paint, decide_post_capture, existing parser/render/hypr tests).

Run: `cargo build --release`
Expected: clean release build.

- [ ] **Step 2: Manual acceptance on Hyprland**

Verify each spec behavior:
- [ ] `boltsnap area` (no daemon running) → daemon self-starts, thumbnail appears bottom-left.
- [ ] Second `boltsnap area` → stacks above the first.
- [ ] Click body → Ctrl+V pastes image elsewhere.
- [ ] Hover → icons appear; ⧉ copies; ✕ dismisses (and frees its temp file).
- [ ] Drag body into a Wayland app → image/file drops.
- [ ] Drag to empty desktop → Ctrl+V still pastes (auto-copy fallback).
- [ ] ✎ → editor opens, edit, save → thumbnail updates.
- [ ] `boltsnap full --no-copy -o /tmp/x.png` → writes file, no shelf, no clipboard (unchanged).
- [ ] `boltsnap area --no-copy -o -` → PNG to stdout (unchanged).
- [ ] `boltsnap --edit` → opens last screenshot in editor (unchanged).
- [ ] X11 session: `boltsnap area` → copies to clipboard, no shelf (unchanged).
- [ ] `boltsnap doctor` → reports shelf status.
- [ ] Restart daemon → shelf is empty (RAM-only confirmed).

- [ ] **Step 3: Commit any fixes found, then finalize**

```bash
git add -A
git commit -m "test: manual acceptance pass for the screenshot shelf"
```

---

## Notes for the implementer

- **SCTK signature drift:** the handler trait method signatures (especially `DataDeviceHandler`/`DataOfferHandler`) are pinned to the verified 0.19.x reference. If the resolved patch version differs and the compiler reports a signature mismatch, adjust the parameter list to match the error — keep the bodies as specified. Run `cargo doc -p smithay-client-toolkit --no-deps --open` to read the exact resolved API.
- **Double buffering:** `draw` recreates the buffer each frame from the `SlotPool`, which transparently allocates a new slot when the compositor still holds the previous one. If you see flicker or "buffer busy" errors under rapid updates, switch to keeping two `Buffer`s and `pool.canvas(&buf)`-checking for `None` as in the SCTK `data_device` example.
- **Surface scale / HiDPI:** v1 renders at logical pixels (scale 1). On HiDPI outputs the shelf will look soft. A follow-up can read the output scale in `CompositorHandler::scale_factor_changed`, multiply buffer dimensions, and set `surface.set_buffer_scale`.
- **Multi-monitor:** the layer surface is created with `output: None`, so the compositor places it on the current output. Pinning to the focused monitor is a follow-up if needed.
- **Phase 2 (recording)** reuses this whole pipeline: a recording produces a file, the client sends an `Add`-like frame (extend `Request` with a `kind`/`mime`), the daemon stores a video-backed thumbnail, and drag offers `video/*` + `uri-list`. Out of scope here.
