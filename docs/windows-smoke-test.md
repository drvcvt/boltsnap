# Windows smoke test

Manual checklist for Windows 10/11. Run it before restoring Windows releases
and after any accepted Windows backend change. There is no automated UI
testing; this list is the coverage.

Setup: `cargo build --release`, then start the daemon (`boltsnap.exe daemon`
from a terminal to see stderr, or `boltsnap-background.exe` for the silent
production path).

## Capture

1. **PrintScreen** opens the Boltsnap area selector and keeps it open (not
   Snipping Tool). Draw a region → thumbnail appears bottom-left on the current
   monitor.
2. **Win+Shift+S** opens the same selector. After cancelling with Escape, the
   Start menu must NOT pop up.
3. `boltsnap window` and `boltsnap active-window` capture the right window.
4. `boltsnap full` captures all monitors as one image (check with mixed-DPI
   setups if available).
5. `boltsnap area --no-copy -o - > out.png` writes a valid PNG to stdout
   (external-editor piping path).

## Shelf

6. Body-click an image card → paste into an image target (e.g. Paint) works.
7. Body-click a video card → paste into Explorer inserts the file.
8. **Save** button → file appears in the save directory (`Pictures\Boltsnap`
   by default); **Close** removes the card; removing the last card hides the
   shelf.
9. Drag a card into **Explorer** AND into a **Chromium-based app** (Discord or
   a browser upload field) — both must accept the drop.

## Recording

10. **Alt+Shift+S** opens the record selector and keeps it open with the same
    stable brightness as the screenshot selector (including with HDR enabled),
    plus the REC pill and audio toggle but no frame checkbox. Confirm → controls
    popup appears top-center.
11. Popup: Pause/Resume updates the timer; **Save to shelf** adds a video card;
    **Save to disk** writes an .mp4 to the record directory; **Discard**
    removes everything.
12. `boltsnap stop` in a terminal finalizes an active recording to the shelf.
13. Play a saved recording: video is intact, audio present (system + mic per
    config), pause gaps are cut out.
14. Error balloons: with no recording active, run
    `boltsnap recording pause` triggered from a hotkey-like context (or click
    a stale controls popup) → a Boltsnap error balloon appears in the
    notification area.

## Daemon & tray

15. Tray menu: all four capture/record entries work; "Boltsnap beenden" exits
    the daemon.
16. Kill the daemon, restart it → orphaned `boltsnap-*` tempfiles from the
    previous run are cleaned (stderr reports counts when > 0).
17. NSIS installer: installs per-user without admin, daemon starts
    after setup and at next sign-in, uninstall removes it.
