# Design: Shelf Redesign (uniform cards, 2 buttons, click-to-enlarge)

Date: 2026-05-31
Project: `/home/mt/projects/boltsnap`
Branch: `feat/screenshot-shelf`
Supersedes parts of `docs/superpowers/specs/2026-05-30-screenshot-shelf-design.md`
(thumbnail fit, hover buttons, click action).

## Motivation

The current shelf sizes each card to its own aspect-preserved thumbnail, so the
vertical stack has ragged widths/heights and looks unruly. The thumbnails also
carry a white border, three hover buttons (close / copy / edit), and a left-click
copies. The user wants a calmer, uniform column and a way to inspect the full
screenshot.

## Goals

1. **Uniform cards.** Every shelf card is the same fixed size; the column reads as
   a clean, aligned stack.
2. **No chrome.** No white border, no card background — just the screenshot image
   with rounded corners.
3. **Two buttons only.** Hover shows close (X) and edit (pencil). The copy button
   is removed.
4. **Click to enlarge.** Left-click opens a centered, full-image preview; right-click
   copies; drag still does drag-and-drop.

## Non-goals (YAGNI / out of scope)

- Blurred-fill backgrounds, hover timestamp (review m6), HiDPI/scaling (review m9).
- The parked code-review blockers B1 (`--edit -o` data loss) and B2 (fmt/clippy)
  are tracked separately and not part of this change.

## Decisions (confirmed with user)

- **Fixed card size:** `260 × 180` px for every card.
- **Image fit:** **crop-to-fill (cover).** The thumbnail is scaled to cover
  260×180 and center-cropped; overflow is cut. Cards become identical rectangles
  with no transparent gaps. The full image is always available via the enlarge
  view, copy, drag, and editor.
- **No white border;** rounded corners kept.
- **Buttons:** close (X) + edit (pencil) only.
- **Left-click** = enlarge, **right-click** = copy to clipboard, **drag** = DnD.
- **Enlarge view:** centered lightbox over the screen; **click anywhere or Esc**
  closes. X and edit buttons remain visible inside it.
- The **original screenshot is never modified.** Only the preview thumbnail is
  cropped; copy/drag/edit/enlarge all use the full original from the stored path.

## Components & changes

### `src/shelf/thumbnail.rs` — cover-crop to a fixed size
- Replace the fit-within logic with **cover + center-crop** to exact `CARD_W × CARD_H`.
- New constants `CARD_W = 260`, `CARD_H = 180` (replacing `MAX_W`/`MAX_H`).
- `scale = max(CARD_W / w, CARD_H / h)` (this **may upscale** small captures — the
  cost of a uniform grid; only affects the preview, never the original).
- After scaling, center-crop to exactly `CARD_W × CARD_H`.
- Pure and unit-testable: output dimensions are always `260×180`.

### `src/shelf/layout.rs` — fixed slots, two buttons
- Every `ThumbRect` is `CARD_W × CARD_H`; surface width = `pad*2 + CARD_W`,
  height = uniform stack. (No more "widest thumb" logic.)
- `Hit` enum drops `Copy`. Remaining: `Body`, `Edit`, `Close`.
- Icon strip is two slots: slot 0 = close (rightmost), slot 1 = edit.
- `hit()` returns `Close`/`Edit` for the button zones, else `Body`, each carrying
  the card id. (Right-click handling reads the id from any hit; see mod.rs.)

### `src/shelf/paint.rs` — drop border, drop copy glyph
- `blit_thumb_card`: remove the white-border ring (`CARD_BORDER`); keep the
  rounded-corner coverage. Thumbnails are already exactly card-sized.
- `draw_hover_icons`: draw two buttons (close, edit).
- `draw_glyph`: keep X (close) and pencil (edit); remove the copy glyph.

### `src/shelf/mod.rs` — pointer routing + enlarge lifecycle
- Pointer button handling:
  - **Right button** released over any card → copy that card's full image to the
    clipboard (reuses existing `copy_to_clipboard` path).
  - **Left button**, no drag, over `Body` → open the enlarge view for that card.
  - **Left button** over `Edit` → spawn editor (reload on save) — unchanged.
  - **Left button** over `Close` → remove card + delete its tempfile — unchanged.
  - **Left button + drag** → existing DnD source (image/png + uri-list, clipboard
    fallback) — unchanged.
- Owns the enlarge-view handle (open/close), delegating rendering/input to
  `preview.rs`.

### `src/shelf/preview.rs` (new) — the enlarge lightbox
- A **separate, on-demand overlay layer-surface** covering the focused output, so
  the shelf surface underneath is untouched (avoids the surface-recreate protocol
  risk from review M4). Created on left-click, destroyed on close.
- Renders a **dimmed translucent backdrop** plus the **full image fit to the
  screen** (whole image visible, aspect-preserved, centered).
- Requests keyboard interactivity so **Esc** closes; a click on the backdrop or
  on the image (i.e. anywhere that is **not** a button) also closes.
- Shows the **X** and **edit** buttons; clicking a button performs its action
  instead of the plain dismiss:
  - **X** = delete this screenshot (removes the card) and close the view.
  - **Edit** = open it in the editor (then close the view).
  - These match the card buttons' meaning, so behavior is consistent.
- Extract the fit-to-screen math (image WxH + screen WxH → scaled size + centered
  offset) as a **pure function** so it can be unit-tested without Wayland.

## Data flow

Capture → daemon ingests full PNG to a tempfile, model stores `{path, thumb,
source}` where `thumb` is now the 260×180 cover-crop. Every action resolves the
**full image via `path`**:
- right-click → `copy_to_clipboard(path)`
- drag → DnD reads `path`
- edit → editor on `path`
- left-click → preview decodes `path`, fits to screen, renders the lightbox

## Testing (TDD)

Pure/unit-testable pieces, each test written first:
- **thumbnail:** cover-crop yields exactly `260×180` for landscape, portrait,
  square, and tiny inputs; center-crop keeps the middle; aspect of the visible
  region matches the card.
- **layout:** all slots `260×180`; surface width `pad*2 + 260`; two-button hit
  zones (close rightmost, edit next); body hit; `Hit` has no `Copy`.
- **paint:** no white border (edge pixel is image color, corner transparent);
  exactly two hover buttons drawn; no copy glyph.
- **preview:** fit-to-screen math (scaled size ≤ screen, aspect preserved, centered
  offsets correct) for wide, tall, and small images.

Wayland surface lifecycle and pointer/keyboard wiring are validated manually on
Hyprland (see below).

## Manual validation (Hyprland/wlroots)

- Capture several shots of different aspect ratios → all cards identical 260×180,
  flush left, no border, no gaps.
- Hover → only X and pencil; no copy button.
- Left-click → centered full-image preview; Esc and click-anywhere close it.
- Right-click → clipboard has the full image.
- Drag into a native and an XWayland app → drops the full image; failed drop →
  clipboard fallback.
- Pencil (card and preview) → editor opens, save reloads the thumbnail.
- X (card and preview) → card and tempfile gone.

## Risks / notes

- Crop-to-fill **upscales** small captures to fill the card; acceptable for a
  preview, and the full image is one click away.
- The preview overlay adds a second layer-shell surface; its lifecycle must be
  clean (create/ack-configure/draw, destroy on close) to stay protocol-correct.
- Keyboard focus for Esc requires the overlay to request keyboard interactivity;
  ensure it releases focus on close.
