# Shelf Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the boltsnap shelf a uniform column of fixed-size cards (no white border, crop-to-fill), reduce hover buttons to close + edit, make left-click open a centered full-image preview and right-click copy, and replace the "ghost" drag with a real, crisply-scaled translucent drag image.

**Architecture:** Pure pixel/geometry helpers (`thumbnail`, `paint`, `preview`) stay testable; the Wayland daemon (`shelf/mod.rs`) wires them. Thumbnails become exactly `260×180` (cover+crop), so the existing `Layout::compute` already yields a uniform stack. The enlarge view is a separate, on-demand overlay layer-surface with keyboard focus for Esc. The drag icon gets its own retained buffer so the shelf's `SlotPool` can no longer clobber it.

**Tech Stack:** Rust, `image` crate (resize/crop), `smithay-client-toolkit 0.19` (layer-shell, wl_shm `SlotPool`, pointer, keyboard, data-device DnD).

**Spec:** `docs/superpowers/specs/2026-05-31-shelf-redesign-design.md`

**Notes for the implementer:**
- This `Hyprland` runs `hyprlua`; animations are disabled by static `no_anim` rules already in place. Don't use `hyprctl keyword`.
- `~/.cargo/bin/boltsnap` is a symlink → `target/debug/boltsnap`. After each rebuild, **restart the daemon** (kill by PID, never `pkill -f`): `for p in $(pgrep -x boltsnap); do kill "$p"; done; rm -f "$XDG_RUNTIME_DIR/boltsnap.sock"; nohup ./target/debug/boltsnap daemon >/tmp/boltsnap-daemon.log 2>&1 &`
- Inspect styling without a compositor: `./target/debug/boltsnap __debug-render /tmp/x.png` then open the PNG.
- **Commit messages must NOT contain a `Co-Authored-By: Claude` trailer.**
- Bash output in this environment is flaky: prefer one command per turn; verify with `cargo test` exit status.
- There is a pre-existing uncommitted change in `src/shelf/mod.rs` (per-monitor placement). Leave it; it is unrelated and should be committed separately by the user.

---

## File Structure

- `src/shelf/thumbnail.rs` — **modify**: replace fit-within `make_thumbnail` + `MAX_W/MAX_H` with cover-crop `make_card_thumbnail` + `CARD_W=260`/`CARD_H=180`.
- `src/shelf/layout.rs` — **modify**: drop `Hit::Copy`; hover strip is two slots (close, edit).
- `src/shelf/paint.rs` — **modify**: drop the white border in `blit_thumb_card`; draw two hover buttons; drop the copy glyph; add pure `build_drag_icon`.
- `src/shelf/preview.rs` — **create**: pure `fit_centered` math + (later) the enlarge overlay render helper.
- `src/shelf/mod.rs` — **modify**: pointer routing (left body → preview, right → copy), drag-icon retained buffer, enlarge overlay surface lifecycle, keyboard (Esc).

Order of tasks keeps the build green and the app usable after every task. Tasks 1–5 are pure/TDD. Tasks 6–10 are Wayland wiring (compile + manual validation; unit-test the pure parts they call).

---

## Task 1: Cover-crop thumbnails to a fixed card size

**Files:**
- Modify: `src/shelf/thumbnail.rs`
- Modify caller: `src/shelf/mod.rs` (`ingest_png`, the `make_thumbnail(..., MAX_W, MAX_H)` call)

- [ ] **Step 1: Replace the tests in `src/shelf/thumbnail.rs`**

Replace the whole `#[cfg(test)] mod tests { … }` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_becomes_exact_card_size() {
        let src = RgbaImage::new(800, 400); // 2:1
        let t = make_card_thumbnail(&src, 260, 180);
        assert_eq!(t.dimensions(), (260, 180));
    }

    #[test]
    fn portrait_becomes_exact_card_size() {
        let src = RgbaImage::new(400, 800); // 1:2
        let t = make_card_thumbnail(&src, 260, 180);
        assert_eq!(t.dimensions(), (260, 180));
    }

    #[test]
    fn tiny_image_is_upscaled_to_card_size() {
        let src = RgbaImage::new(50, 30);
        let t = make_card_thumbnail(&src, 260, 180);
        assert_eq!(t.dimensions(), (260, 180));
    }

    #[test]
    fn center_crop_keeps_the_middle() {
        // Left half red, right half blue; cover-crop of a wide 4:1 image into a
        // 2:3-ish card keeps the central seam roughly centered.
        let mut src = RgbaImage::new(400, 100);
        for (x, _y, p) in src.enumerate_pixels_mut() {
            *p = if x < 200 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) };
        }
        let t = make_card_thumbnail(&src, 260, 180);
        // Just left of center is red-ish, just right is blue-ish.
        let left = t.get_pixel(120, 90).0;
        let right = t.get_pixel(140, 90).0;
        assert!(left[0] > left[2], "left of center should be reddish");
        assert!(right[2] > right[0], "right of center should be bluish");
    }
}
```

- [ ] **Step 2: Run the tests; verify they fail**

Run: `cargo test --lib shelf::thumbnail`
Expected: FAIL to compile / "cannot find function `make_card_thumbnail`".

- [ ] **Step 3: Replace the implementation in `src/shelf/thumbnail.rs`**

Replace the top of the file (the constants + `make_thumbnail`) with:

```rust
use image::RgbaImage;
use image::imageops::FilterType;

/// Fixed shelf card size in pixels. Every card is exactly this size so the shelf
/// reads as a uniform column. Tweak to resize the cards.
pub const CARD_W: u32 = 260;
pub const CARD_H: u32 = 180;

/// Scale `src` to *cover* (card_w, card_h) preserving aspect ratio, then
/// center-crop to exactly (card_w, card_h). May upscale small inputs — that is
/// the cost of a uniform grid, and only affects the preview; the original PNG is
/// never modified. Always returns an image of exactly card_w × card_h.
pub fn make_card_thumbnail(src: &RgbaImage, card_w: u32, card_h: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 || card_w == 0 || card_h == 0 {
        return RgbaImage::new(card_w.max(1), card_h.max(1));
    }
    let scale = (card_w as f32 / w as f32).max(card_h as f32 / h as f32);
    let nw = ((w as f32 * scale).round() as u32).max(card_w);
    let nh = ((h as f32 * scale).round() as u32).max(card_h);
    let scaled = image::imageops::resize(src, nw, nh, FilterType::Lanczos3);
    let x0 = (nw - card_w) / 2;
    let y0 = (nh - card_h) / 2;
    image::imageops::crop_imm(&scaled, x0, y0, card_w, card_h).to_image()
}
```

- [ ] **Step 4: Update the caller in `src/shelf/mod.rs`**

In `ingest_png`, change:

```rust
let thumb = make_thumbnail(&img, thumbnail::MAX_W, thumbnail::MAX_H);
```
to:
```rust
let thumb = make_card_thumbnail(&img, thumbnail::CARD_W, thumbnail::CARD_H);
```
And update the import near the top of `mod.rs`:
```rust
use crate::shelf::thumbnail::make_card_thumbnail;
```
(remove the old `make_thumbnail` import). The `thumbnail` module path (`thumbnail::CARD_W`) is already referenced via `crate::shelf::thumbnail`; if `mod.rs` uses a bare `thumbnail::` path, keep it — `thumbnail` is declared `pub mod thumbnail;` in `mod.rs`.

- [ ] **Step 5: Run tests + build; verify green**

Run: `cargo test --lib shelf::thumbnail && cargo build`
Expected: thumbnail tests PASS; build succeeds (the `__debug-render` and `debug_render` paths still compile).

- [ ] **Step 6: Commit**

```bash
git add src/shelf/thumbnail.rs src/shelf/mod.rs
git commit -m "feat(shelf): cover-crop thumbnails to a fixed 260x180 card"
```

---

## Task 2: Layout — drop the copy button, two-slot hover strip

**Files:**
- Modify: `src/shelf/layout.rs`

- [ ] **Step 1: Update the tests in `src/shelf/layout.rs`**

In `mod tests`, replace `hit_body_vs_icons_vs_outside` with:

```rust
#[test]
fn hit_body_vs_two_icons_vs_outside() {
    let c = cfg();
    let lay = Layout::compute(&[(7, 260, 180)], &c);
    let r = &lay.thumbs[0];
    // center of the thumb -> body
    let cx = (r.x + r.w / 2) as f64;
    let cy = (r.y + r.h / 2) as f64;
    assert_eq!(lay.hit(cx, cy, &c), Some(Hit::Body(7)));
    // close icon = rightmost slot
    let close_cx = (r.x + r.w - c.pad_icon - c.icon / 2) as f64;
    let icon_cy = (r.y + c.pad_icon + c.icon / 2) as f64;
    assert_eq!(lay.hit(close_cx, icon_cy, &c), Some(Hit::Close(7)));
    // edit icon = next slot to the left of close
    let edit_cx = (r.x + r.w - c.pad_icon - c.icon - c.icon_gap - c.icon / 2) as f64;
    assert_eq!(lay.hit(edit_cx, icon_cy, &c), Some(Hit::Edit(7)));
    // far outside
    assert_eq!(lay.hit(10_000.0, 10_000.0, &c), None);
}
```

- [ ] **Step 2: Run tests; verify they fail**

Run: `cargo test --lib shelf::layout`
Expected: FAIL — `Hit::Copy` still exists / the edit slot index differs, or a stale test references `Copy`.

- [ ] **Step 3: Update the `Hit` enum and `hit()`**

In `src/shelf/layout.rs`:

Change the enum to drop `Copy`:
```rust
#[derive(Debug, PartialEq, Eq)]
pub enum Hit {
    Body(u64),
    Edit(u64),
    Close(u64),
}
```

In `hit()`, change the icon loop to two slots (close rightmost = slot 0, edit = slot 1):
```rust
for (slot, make) in [(0u32, Hit::Close(r.id)), (1, Hit::Edit(r.id))] {
    let (ix, iy, iw, ih) = self.icon_rect(r, slot, cfg);
    if x >= ix as f64 && x < (ix + iw) as f64 && y >= iy as f64 && y < (iy + ih) as f64 {
        return Some(make);
    }
}
```
Update the doc comment above `icon_rect` to say `[edit][close], close rightmost`.

- [ ] **Step 4: Fix the two `mod.rs` match sites so the crate compiles**

Removing the `Copy` variant breaks two `match` sites in `mod.rs`. Fix them now
(routing to preview/right-click comes in Task 5; here just keep it compiling):

In `on_click`, drop the `Copy` arm (leave `Body` a no-op for now):
```rust
fn on_click(&mut self, hit: Hit) {
    match hit {
        Hit::Close(id) => self.remove_thumb(id),
        Hit::Edit(id) => self.spawn_editor(id),
        Hit::Body(_) => {}
    }
}
```
In `pointer_frame`'s left-`Press` arm, drop `Copy` from the id extraction:
```rust
let id = match hit {
    Hit::Body(id) | Hit::Edit(id) | Hit::Close(id) => id,
};
```
(`paint.rs` uses slot indices, not `Hit::Copy`, so it needs no change here.)

- [ ] **Step 5: Run tests; verify they pass**

Run: `cargo build && cargo test --lib shelf::layout`
Expected: build succeeds, layout tests PASS, no `Hit::Copy` references remain.

- [ ] **Step 6: Commit**

```bash
git add src/shelf/layout.rs src/shelf/mod.rs
git commit -m "feat(shelf): two-button hover strip, drop copy hit"
```

---

## Task 3: Paint — remove the white border, two buttons, drop copy glyph, add drag-icon builder

**Files:**
- Modify: `src/shelf/paint.rs`

- [ ] **Step 1: Update the border test**

Replace `card_rounds_corners_and_draws_white_border` with:

```rust
#[test]
fn card_rounds_corners_no_border() {
    let mut img = RgbaImage::new(20, 20);
    for p in img.pixels_mut() {
        *p = image::Rgba([10, 120, 240, 255]); // low R so we can tell it from white
    }
    let mut buf = vec![0u8; 20 * 20 * 4];
    blit_thumb_card(&mut buf, 20, 20, &img, 0, 0);
    // far corner is outside the radius -> transparent
    assert_eq!(buf[3], 0, "corner should be transparent");
    // centre is opaque, the thumbnail colour (low R), not white
    let c = ((10 * 20 + 10) * 4) as usize;
    assert!(buf[c + 3] > 250, "centre should be opaque");
    assert!(buf[c + 2] < 60, "centre R should be the thumbnail's");
    // left-edge midpoint is now the IMAGE colour, NOT a white border
    let e = ((10 * 20 + 0) * 4) as usize;
    assert!(buf[e + 3] > 200, "left edge should be (near) opaque image");
    assert!(buf[e + 2] < 60, "left edge R should be the image's, not white");
}
```

- [ ] **Step 2: Add a drag-icon builder test**

Add to `mod tests`:

```rust
#[test]
fn drag_icon_is_premultiplied_and_rounded() {
    let mut img = RgbaImage::new(40, 40);
    for p in img.pixels_mut() {
        *p = image::Rgba([200, 100, 50, 255]);
    }
    let buf = build_drag_icon(&img, 40, 40, 8.0, 0.85);
    assert_eq!(buf.len(), 40 * 40 * 4);
    // corner transparent
    assert_eq!(buf[3], 0, "corner alpha should be 0");
    // centre alpha ~ 0.85*255
    let c = ((20 * 40 + 20) * 4) as usize;
    assert!(buf[c + 3] > 200 && buf[c + 3] < 230, "got a={}", buf[c + 3]);
    // premultiplied: every colour channel <= alpha
    assert!(buf[c] <= buf[c + 3] && buf[c + 1] <= buf[c + 3] && buf[c + 2] <= buf[c + 3]);
}
```

- [ ] **Step 3: Run tests; verify they fail**

Run: `cargo test --lib shelf::paint`
Expected: FAIL — `build_drag_icon` undefined and the border test asserts new behavior.

- [ ] **Step 4: Rewrite `blit_thumb_card` (no border) and add `build_drag_icon`**

Replace the `CARD_BORDER` constant line and `blit_thumb_card` body. New constant block:
```rust
/// Thumbnail card corner radius, in pixels.
const CARD_RADIUS: f32 = 10.0;
```
(remove `const CARD_BORDER`.)

Replace `blit_thumb_card` with:
```rust
/// Composite an opaque RGBA thumbnail as a rounded card: rounded corners
/// (transparent outside the radius), no border. Output is premultiplied BGRA.
pub fn blit_thumb_card(canvas: &mut [u8], cw: u32, ch: u32, img: &RgbaImage, dx: u32, dy: u32) {
    let (iw, ih) = img.dimensions();
    let w = iw as f32;
    let h = ih as f32;
    let r = CARD_RADIUS.min(w / 2.0).min(h / 2.0);
    for sy in 0..ih {
        let py = dy + sy;
        if py >= ch { break; }
        for sx in 0..iw {
            let px = dx + sx;
            if px >= cw { break; }
            let cov = rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w, h, r);
            if cov <= 0.0 { continue; }
            let p = img.get_pixel(sx, sy).0;
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = (p[2] as f32 * cov).round().clamp(0.0, 255.0) as u8; // B
            canvas[idx + 1] = (p[1] as f32 * cov).round().clamp(0.0, 255.0) as u8; // G
            canvas[idx + 2] = (p[0] as f32 * cov).round().clamp(0.0, 255.0) as u8; // R
            canvas[idx + 3] = (cov * 255.0).round().clamp(0.0, 255.0) as u8; // A
        }
    }
}

/// Build a premultiplied-BGRA drag icon: scale `src` to (w,h) with Lanczos3,
/// round the corners, and apply a global `opacity` (0..=1). Returns w*h*4 bytes,
/// ready to copy into a wl_shm Argb8888 buffer.
pub fn build_drag_icon(src: &RgbaImage, w: u32, h: u32, radius: f32, opacity: f32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    if w == 0 || h == 0 { return buf; }
    let scaled = image::imageops::resize(src, w, h, image::imageops::FilterType::Lanczos3);
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    for sy in 0..h {
        for sx in 0..w {
            let cov = rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w as f32, h as f32, r) * opacity;
            if cov <= 0.0 { continue; }
            let p = scaled.get_pixel(sx, sy).0;
            let idx = ((sy * w + sx) * 4) as usize;
            buf[idx] = (p[2] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 1] = (p[1] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 2] = (p[0] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 3] = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    buf
}
```
Add `use image::RgbaImage;` is already present at the top of `paint.rs`.

- [ ] **Step 5: Two hover buttons; drop the copy glyph**

In `draw_hover_icons`, change `for slot in 0..3u32` to `for slot in 0..2u32`, and update the comment to "slot 0 close (rightmost), 1 edit".

In `draw_glyph`, change the `match slot` so slot 0 = X (unchanged) and slot 1 = pencil (move the pencil body from the old `_ =>` arm to `1 =>`), and delete the old copy arm:
```rust
match slot {
    0 => {
        // X
        stroke_line(canvas, cw, ch, x + lo, y + lo, x + hi, y + hi, hw, c, 1.0);
        stroke_line(canvas, cw, ch, x + hi, y + lo, x + lo, y + hi, hw, c, 1.0);
    }
    _ => {
        // edit: a pencil — diagonal body with a small nib corner at the lower-left
        let tipx = x + lo;
        let tipy = y + hi;
        stroke_line(canvas, cw, ch, tipx, tipy, x + hi, y + lo, hw, c, 1.0);
        let nib = (hi - lo) * 0.26;
        stroke_line(canvas, cw, ch, tipx, tipy, tipx + nib, tipy, hw, c, 1.0);
        stroke_line(canvas, cw, ch, tipx, tipy, tipx, tipy - nib, hw, c, 1.0);
    }
}
```
Delete the now-unused `stroke_rect` helper (it was only used by the copy glyph). If clippy later flags it as dead, that confirms removal.

- [ ] **Step 6: Run tests; verify pass**

Run: `cargo test --lib shelf::paint`
Expected: PASS for `card_rounds_corners_no_border`, `drag_icon_is_premultiplied_and_rounded`, and the existing `clear_zeros_buffer` / `fill_circle_blends_center_opaqueish` / `hovered_thumb_draws_icon_pixels`.

- [ ] **Step 7: Commit**

```bash
git add src/shelf/paint.rs
git commit -m "feat(shelf): borderless rounded cards, two buttons, drag-icon builder"
```

---

## Task 4: Preview fit-to-screen math (new module)

**Files:**
- Create: `src/shelf/preview.rs`
- Modify: `src/shelf/mod.rs` (add `pub mod preview;` near the other `pub mod` lines)

- [ ] **Step 1: Create `src/shelf/preview.rs` with the test first**

```rust
//! The centered full-image enlarge ("lightbox") view.

/// Fit an (img_w, img_h) image inside a (screen_w, screen_h) area with `margin`
/// px of breathing room on each side, preserving aspect ratio, centered.
/// Returns (draw_w, draw_h, off_x, off_y) in screen pixels.
pub fn fit_centered(
    img_w: u32,
    img_h: u32,
    screen_w: u32,
    screen_h: u32,
    margin: u32,
) -> (u32, u32, u32, u32) {
    if img_w == 0 || img_h == 0 || screen_w == 0 || screen_h == 0 {
        return (0, 0, screen_w / 2, screen_h / 2);
    }
    let avail_w = screen_w.saturating_sub(margin * 2).max(1);
    let avail_h = screen_h.saturating_sub(margin * 2).max(1);
    let scale = (avail_w as f32 / img_w as f32).min(avail_h as f32 / img_h as f32);
    let dw = ((img_w as f32 * scale).round() as u32).clamp(1, screen_w);
    let dh = ((img_h as f32 * scale).round() as u32).clamp(1, screen_h);
    let off_x = (screen_w - dw) / 2;
    let off_y = (screen_h - dh) / 2;
    (dw, dh, off_x, off_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_image_is_width_limited_and_centered() {
        let (dw, dh, ox, oy) = fit_centered(2000, 500, 1000, 1000, 0);
        assert_eq!(dw, 1000);
        assert_eq!(dh, 250);
        assert_eq!(ox, 0);
        assert_eq!(oy, 375);
    }

    #[test]
    fn tall_image_is_height_limited_and_centered() {
        let (dw, dh, ox, oy) = fit_centered(500, 2000, 1000, 1000, 0);
        assert_eq!(dw, 250);
        assert_eq!(dh, 1000);
        assert_eq!(ox, 375);
        assert_eq!(oy, 0);
    }

    #[test]
    fn margin_shrinks_the_drawable_area() {
        let (dw, dh, _, _) = fit_centered(1000, 1000, 1000, 1000, 100);
        assert!(dw <= 800 && dh <= 800, "got {dw}x{dh}");
    }
}
```

- [ ] **Step 2: Wire the module + run tests**

Add to `src/shelf/mod.rs` near `pub mod paint;`:
```rust
pub mod preview;
```
Run: `cargo test --lib shelf::preview`
Expected: PASS (the function and tests are in the same file, so they pass immediately — that's acceptable for a pure geometry helper written test-first in one step; verify each assertion matches by reading the output).

- [ ] **Step 3: Commit**

```bash
git add src/shelf/preview.rs src/shelf/mod.rs
git commit -m "feat(shelf): fit-centered math for the enlarge view"
```

---

## Task 5: Pointer routing — left body = enlarge (stub), right-click = copy

This task changes interactions: left-click on a card body opens the enlarge view
(stubbed to a log line here, implemented in Task 7), and right-click copies (works
fully now). `Hit::Copy` was already removed in Task 2.

**Files:**
- Modify: `src/shelf/mod.rs`

- [ ] **Step 1: Import `BTN_RIGHT`**

In the `seat::pointer` import line, add `BTN_RIGHT`:
```rust
pointer::{BTN_LEFT, BTN_RIGHT, PointerEvent, PointerEventKind, PointerHandler},
```

- [ ] **Step 2: Update `on_click` — Body opens preview, copy arm removed**

Replace `on_click` with:
```rust
fn on_click(&mut self, hit: Hit) {
    match hit {
        Hit::Close(id) => self.remove_thumb(id),
        Hit::Edit(id) => self.spawn_editor(id),
        Hit::Body(id) => self.open_preview(id),
    }
}
```

Add a temporary stub (replaced in Task 7) near `on_click`:
```rust
fn open_preview(&mut self, id: u64) {
    eprintln!("boltsnap: preview requested for {id} (not yet implemented)");
}
```

- [ ] **Step 3: Add the right-click copy helper**

Add a method on `Daemon`:
```rust
/// Copy the full image of the card under the cursor to the clipboard.
fn copy_card(&mut self, id: u64) {
    if let Some(t) = self.model.get(id) {
        let _ = crate::clipboard::copy_to_clipboard(&t.png_path, crate::Backend::Wayland);
    }
}
```

- [ ] **Step 4: Handle the right button in `pointer_frame`**

In `pointer_frame`'s `match ev.kind`, the `Press` arm currently matches `button == BTN_LEFT` and builds `PressState` with `id` extracted from `Hit::Body(id) | Hit::Edit(id) | Hit::Copy(id) | Hit::Close(id)`. Update the id extraction to drop `Copy`:
```rust
let id = match hit {
    Hit::Body(id) | Hit::Edit(id) | Hit::Close(id) => id,
};
```
Add a new arm for the right button (place it right after the `Press { .. } if button == BTN_LEFT` arm):
```rust
PointerEventKind::Press { button, .. } if button == BTN_RIGHT => {
    let (x, y) = self.pointer_pos;
    if let Some(hit) = self.layout.hit(x, y, &self.cfg) {
        let id = match hit {
            Hit::Body(id) | Hit::Edit(id) | Hit::Close(id) => id,
        };
        self.copy_card(id);
    }
}
```
(`self.pointer_pos` is the field the existing left-Press arm already uses.)

- [ ] **Step 5: Build + run the whole suite**

Run: `cargo build && cargo test`
Expected: build succeeds, all tests PASS (24 baseline minus removed/renamed plus new ones; no `Hit::Copy` references remain anywhere).

- [ ] **Step 6: Manual check**

Rebuild + restart the daemon (see header), capture a few shots:
- Cards are uniform 260×180, no white border, no gaps.
- Hover shows only X + pencil.
- Right-click a card → its image is on the clipboard (paste somewhere).
- Left-click a card → daemon log prints "preview requested …".
- Drag still works (ghost icon for now — fixed in Task 6).

- [ ] **Step 7: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): left-click opens preview (stub), right-click copies"
```

---

## Task 6: Real drag icon (retained buffer, borderless, translucent, centered hotspot)

**Files:**
- Modify: `src/shelf/mod.rs` (`Daemon` fields, `begin_drag`, `clear_drag`)

- [ ] **Step 1: Add a retained drag-icon pool field**

In `struct Daemon`, in the `// drag state` group, add:
```rust
/// A dedicated pool kept alive for the duration of a drag so the shelf's main
/// `pool` can't reuse the icon's slot and turn it into a "ghost".
drag_icon_pool: Option<SlotPool>,
```
In the constructor where the other drag fields are initialized (`drag_source: None,` etc.), add:
```rust
drag_icon_pool: None,
```

- [ ] **Step 2: Rebuild `begin_drag` to use the dedicated pool + `build_drag_icon`**

Replace the icon-building block inside `begin_drag` (the `if let Some(t) = self.model.get(id) { … }` that uses `self.pool.create_buffer` + `blit_thumb_card`) with:
```rust
if let Some(t) = self.model.get(id) {
    let (iw, ih) = t.thumb.dimensions();
    let bytes = crate::shelf::paint::build_drag_icon(&t.thumb, iw, ih, 10.0, 0.85);
    let stride = (iw * 4) as i32;
    if let Ok(mut pool) = SlotPool::new((iw * ih * 4) as usize, &self.shm) {
        if let Ok((buf, canvas)) = pool.create_buffer(
            iw as i32,
            ih as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            canvas.copy_from_slice(&bytes);
            // Centre the icon under the cursor (grab point under the pointer).
            icon.offset(-(iw as i32) / 2, -(ih as i32) / 2);
            let _ = buf.attach_to(&icon);
            icon.commit();
        }
        self.drag_icon_pool = Some(pool);
    }
}
```
Notes:
- `icon.offset(dx, dy)` is `wl_surface::offset`. If the SCTK/`wayland-client` version in use does not expose `offset` on the surface directly, attach with the offset instead: replace the `icon.offset(...)` + `buf.attach_to(&icon)` lines with the explicit attach:
  `icon.attach(Some(buf.wl_buffer()), -(iw as i32) / 2, -(ih as i32) / 2);` then `icon.commit();`. Verify against the wl_surface API actually available; keep whichever compiles.
- `canvas.copy_from_slice(&bytes)` requires `bytes.len() == canvas.len()` — both are `iw*ih*4`, so it holds.

- [ ] **Step 3: Free the dedicated pool in `clear_drag`**

In `clear_drag` (called by `cancelled` and `dnd_finished`), where it sets `self.icon_surface = None;`, add:
```rust
self.drag_icon_pool = None;
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: success. (No new unit tests here; the pure `build_drag_icon` is covered in Task 3.)

- [ ] **Step 5: Manual validation (the key acceptance check)**

Rebuild + restart daemon. Capture a shot, then drag the card into a file manager / chat / editor:
- The **real screenshot** sticks to the cursor, crisp and ~85% opaque, centered under the pointer — **no ghost**, no flicker when the shelf redraws.
- Drop into a native Wayland app and an XWayland app → the full image arrives.
- Drop onto empty desktop (failed drop) → clipboard fallback still fires.

If a faint icon still appears, debug with `WAYLAND_DEBUG=1` on the daemon and confirm the icon buffer isn't being released early; the dedicated pool should prevent it.

- [ ] **Step 6: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "fix(shelf): real drag image via retained buffer (no ghost), translucent + centered"
```

---

## Task 7: Enlarge view — overlay surface lifecycle + render

This replaces the Task 5 `open_preview` stub with a real centered lightbox on its
own overlay layer-surface. Keyboard (Esc) is added in Task 8; here, click-to-close
works.

**Files:**
- Modify: `src/shelf/mod.rs` (state, `open_preview`, `close_preview`, draw, `LayerShellHandler::configure`, `pointer_frame`)
- Modify: `src/shelf/preview.rs` (add a render helper)

- [ ] **Step 1: Add a preview render helper test in `src/shelf/preview.rs`**

```rust
#[cfg(test)]
mod render_tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn render_lightbox_dims_backdrop_and_draws_image() {
        let mut img = RgbaImage::new(100, 50);
        for p in img.pixels_mut() { *p = image::Rgba([0, 200, 0, 255]); }
        let (sw, sh) = (400u32, 300u32);
        let mut canvas = vec![0u8; (sw * sh * 4) as usize];
        render_lightbox(&mut canvas, sw, sh, &img, 20);
        // backdrop corner: semi-opaque dark (alpha > 0, not the image)
        assert!(canvas[3] > 0, "backdrop should be visible");
        // centre: the green image, opaque
        let c = ((sh / 2 * sw + sw / 2) * 4) as usize;
        assert!(canvas[c + 3] > 250, "image centre opaque");
        assert!(canvas[c + 1] > 150, "image centre green");
    }
}
```

- [ ] **Step 2: Run; verify fail**

Run: `cargo test --lib shelf::preview`
Expected: FAIL — `render_lightbox` undefined.

- [ ] **Step 3: Implement `render_lightbox` in `src/shelf/preview.rs`**

```rust
use image::RgbaImage;

/// Backdrop colour + opacity for the lightbox (premultiplied BGRA fill).
const BACKDROP: (u8, u8, u8) = (8, 8, 12);
const BACKDROP_A: f32 = 0.78;

/// Render the enlarge view into a premultiplied-BGRA `canvas` of (sw, sh):
/// a dimmed backdrop plus the full `img` fitted and centered with `margin`.
pub fn render_lightbox(canvas: &mut [u8], sw: u32, sh: u32, img: &RgbaImage, margin: u32) {
    // dim backdrop (premultiplied)
    let (br, bg, bb) = BACKDROP;
    let a = BACKDROP_A;
    for px in canvas.chunks_exact_mut(4) {
        px[0] = (bb as f32 * a) as u8;
        px[1] = (bg as f32 * a) as u8;
        px[2] = (br as f32 * a) as u8;
        px[3] = (a * 255.0) as u8;
    }
    let (iw, ih) = img.dimensions();
    let (dw, dh, ox, oy) = fit_centered(iw, ih, sw, sh, margin);
    if dw == 0 || dh == 0 { return; }
    let scaled = image::imageops::resize(img, dw, dh, image::imageops::FilterType::Lanczos3);
    for sy in 0..dh {
        let py = oy + sy;
        if py >= sh { break; }
        for sx in 0..dw {
            let pxn = ox + sx;
            if pxn >= sw { break; }
            let p = scaled.get_pixel(sx, sy).0; // opaque
            let idx = ((py * sw + pxn) * 4) as usize;
            canvas[idx] = p[2];
            canvas[idx + 1] = p[1];
            canvas[idx + 2] = p[0];
            canvas[idx + 3] = 255;
        }
    }
}
```

- [ ] **Step 4: Run; verify pass**

Run: `cargo test --lib shelf::preview`
Expected: PASS.

- [ ] **Step 5: Add preview state to `Daemon`**

Add fields (new `// preview` group):
```rust
// enlarge ("lightbox") view
preview: Option<PreviewState>,
```
And a struct near `PressState`:
```rust
/// State for the open enlarge view: its own overlay surface + a dedicated pool.
struct PreviewState {
    surface: LayerSurface,
    pool: SlotPool,
    image: image::RgbaImage,
    id: u64,
    size: (u32, u32),
}
```
Initialize `preview: None,` in the constructor.

- [ ] **Step 6: Implement `open_preview` / `close_preview`**

Replace the Task 5 stub `open_preview` with:
```rust
fn open_preview(&mut self, id: u64) {
    if self.preview.is_some() { return; }
    let (path, _) = match self.model.get(id) {
        Some(t) => (t.png_path.clone(), ()),
        None => return,
    };
    let img = match image::open(&path) {
        Ok(i) => i.to_rgba8(),
        Err(_) => return,
    };
    let qh = match self.qh.clone() { Some(q) => q, None => return };
    let surface = self.compositor.create_surface(&qh);
    let layer = self.layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("boltsnap-preview"),
        None, // current output
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, 0); // compositor fills the output; real size arrives in configure
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_exclusive_zone(-1);
    layer.commit();
    let pool = match SlotPool::new(256, &self.shm) { Ok(p) => p, Err(_) => return };
    self.preview = Some(PreviewState { surface: layer, pool, image: img, id, size: (0, 0) });
}

fn close_preview(&mut self) {
    // Dropping the LayerSurface unmaps/destroys it.
    self.preview = None;
}
```
Note: add `"boltsnap-preview"` to your hyprlua `no_anim` layer rule namespace later if the open/close animates; the existing rule matches `^boltsnap$` only.

- [ ] **Step 7: Draw the preview on configure**

In `LayerShellHandler::configure`, it currently configures the shelf surface. Make it branch on which surface configured. At the top of `configure`, handle the preview surface:
```rust
if let Some(pv) = self.preview.as_mut() {
    if pv.surface.wl_surface() == layer.wl_surface() {
        let (w, h) = (configure.new_size.0.max(1), configure.new_size.1.max(1));
        pv.size = (w, h);
        let stride = (w * 4) as i32;
        if let Ok((buffer, canvas)) = pv.pool.create_buffer(
            w as i32, h as i32, stride, wl_shm::Format::Argb8888,
        ) {
            let img = pv.image.clone();
            crate::shelf::preview::render_lightbox(canvas, w, h, &img, 48);
            let surface = pv.surface.wl_surface();
            surface.damage_buffer(0, 0, w as i32, h as i32);
            let _ = buffer.attach_to(surface);
            surface.commit();
        }
        return;
    }
}
```
(The existing shelf-configure code stays below this block.)

- [ ] **Step 8: Route pointer clicks on the preview surface to close**

At the top of `pointer_frame`'s per-event loop, the code does `if ev.surface != surface { continue; }` where `surface` is the shelf surface. Before that check, intercept preview-surface events:
```rust
if let Some(pv) = self.preview.as_ref() {
    let pv_surface = pv.surface.wl_surface().clone();
    if ev.surface == pv_surface {
        if let PointerEventKind::Press { button, .. } = ev.kind {
            if button == BTN_LEFT || button == BTN_RIGHT {
                // (Buttons inside the lightbox are added later; for now any
                // click closes.)
                self.close_preview();
                self.draw_if_ready();
            }
        }
        continue;
    }
}
```
> Buttons (X / edit) inside the lightbox: deferred to a follow-up (see "Deferred" at the end). The spec's preview X/edit are a nice-to-have on top of click/Esc-to-close; ship close-on-click first.

- [ ] **Step 9: Build + manual validate**

Run: `cargo build`, rebuild + restart daemon. Capture a shot, **left-click** the card:
- A centered, dimmed lightbox shows the **whole** screenshot fitted to the screen.
- **Click anywhere** closes it and returns to the shelf.
- The shelf underneath is intact after closing.

- [ ] **Step 10: Commit**

```bash
git add src/shelf/mod.rs src/shelf/preview.rs
git commit -m "feat(shelf): centered enlarge view on its own overlay surface (click to close)"
```

---

## Task 8: Esc closes the enlarge view (keyboard)

**Files:**
- Modify: `src/shelf/mod.rs` (SeatHandler keyboard capability, KeyboardHandler impl, delegate)

- [ ] **Step 1: Track the keyboard + capability**

Add a field to `Daemon` (near `pointer`):
```rust
keyboard: Option<wayland_client::protocol::wl_keyboard::WlKeyboard>,
```
Init `keyboard: None,` in the constructor.

In `SeatHandler::new_capability`, alongside the existing `Capability::Pointer` branch, add:
```rust
if cap == Capability::Keyboard && self.keyboard.is_none() {
    if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
        self.keyboard = Some(kb);
    }
}
```
And in `remove_capability`, release it:
```rust
if cap == Capability::Keyboard {
    if let Some(kb) = self.keyboard.take() { kb.release(); }
}
```

- [ ] **Step 2: Implement `KeyboardHandler`**

Add the import to the SCTK use block: `seat::keyboard::{KeyboardHandler, KeyEvent, Keysym, Modifiers, RawModifiers}` (verify exact names in SCTK 0.19; `Keysym` comes from `xkbcommon`/sctk re-export). Then:
```rust
impl KeyboardHandler for Daemon {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        // Esc closes the enlarge view.
        if event.keysym == Keysym::Escape && self.preview.is_some() {
            self.close_preview();
            self.draw_if_ready();
        }
    }
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: Modifiers, _: RawModifiers, _: u32) {}
}
```
Add `delegate_keyboard!(Daemon);` next to `delegate_pointer!(Daemon);`, and add `wl_keyboard` to the `wayland_client::protocol::{…}` import.
> The exact `KeyboardHandler` method signatures changed across SCTK versions. If they differ, run `cargo build`, read the trait error, and match the signatures the compiler reports. The behavior to implement is fixed: on `Keysym::Escape` with a preview open, call `close_preview()` + `draw_if_ready()`.

- [ ] **Step 3: Build + manual validate**

Run: `cargo build`, rebuild + restart daemon. Open a preview (left-click), press **Esc** → it closes. Click-to-close still works too.

- [ ] **Step 4: Commit**

```bash
git add src/shelf/mod.rs
git commit -m "feat(shelf): Esc closes the enlarge view"
```

---

## Task 9: Hygiene for touched files

The full repo `cargo fmt`/`clippy` cleanup is the parked B2; here just keep the new
code clean so it doesn't add noise.

- [ ] **Step 1: Format only the shelf files**

Run: `cargo fmt`
Then review the diff is limited to files this plan touched (plus any pre-existing
fmt drift the user may want separately — if `git diff` shows unrelated files, you
may `git checkout --` those to keep this change focused, or leave for B2).

- [ ] **Step 2: Clippy on the crate (don't fail the build, just read)**

Run: `cargo clippy --all-targets 2>&1 | grep -E 'shelf/(thumbnail|layout|paint|preview|mod)' || true`
Fix any *new* warnings in the files this plan changed (e.g. dead `stroke_rect` if you didn't delete it, needless clones). Pre-existing warnings elsewhere belong to B2.

- [ ] **Step 3: Full test suite**

Run: `cargo test`
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore(shelf): fmt + clippy for redesigned shelf files"
```

---

## Task 10: Full manual validation pass

- [ ] Rebuild (`cargo build`) + restart the daemon.
- [ ] Capture several shots of different aspect ratios (full, area, a tall window): all cards are identical 260×180, flush left, **no white border**, no gaps.
- [ ] Hover a card → only **X** and **pencil** show.
- [ ] **Left-click** → centered full-image lightbox; **Esc** and **click anywhere** close it; shelf intact after.
- [ ] **Right-click** → the full image is on the clipboard (paste to confirm).
- [ ] **Drag** a card into a native Wayland app and an XWayland app → the real screenshot (crisp, ~85% opaque, centered on cursor) drags; full image drops; failed drop → clipboard fallback.
- [ ] **Pencil** → editor opens; saving reloads that card's thumbnail.
- [ ] **X** → card and its tempfile are gone.
- [ ] `boltsnap __debug-render /tmp/x.png` → borderless rounded card renders.

---

## Self-Review (author check against the spec)

- **Spec coverage:** uniform 260×180 (T1) ✓; crop-to-fill (T1) ✓; no border + rounded (T3) ✓; two buttons X+edit (T2/T3) ✓; left=enlarge (T5/T7) ✓; right=copy (T5) ✓; centered lightbox + click/Esc close (T7/T8) ✓; real drag image, crisp + ~85% + hotspot, no ghost (T6) ✓; original never modified (all actions use `png_path`) ✓.
- **Deferred (explicitly out of this plan, matches spec "nice-to-have"):** X/edit **buttons inside** the lightbox (Task 7 ships click/Esc-to-close; the in-lightbox buttons can be a small follow-up reusing `draw_glyph` + a preview hit-test). Flag to the user at the end.
- **Type consistency:** `make_card_thumbnail`, `CARD_W/CARD_H`, `build_drag_icon`, `fit_centered`, `render_lightbox`, `PreviewState`, `open_preview/close_preview`, `copy_card`, `drag_icon_pool` are used consistently across tasks.
- **Library-version caveats (verify while implementing, behavior is fixed):** `wl_surface.offset` vs `attach(buffer, dx, dy)` hotspot API (T6); `KeyboardHandler` method signatures + `Keysym::Escape` path (T8); `configure.new_size` field name on `LayerSurfaceConfigure` (T7).
