# The screenshot shelf

On supported Wayland compositors (Hyprland, Sway, Niri), an interactive capture
appears as a small floating **thumbnail in the bottom-left corner** (or the
corner set under `[shelf]`, see [Configuration](configuration.md)). The
experimental Windows backend contains the same shelf flow, but it is not
currently verified. Multiple screenshots stack with the newest one on top.

```sh
boltsnap area        # capture a region -> appears in the shelf
boltsnap full        # whole screen -> shelf
boltsnap window      # pick a window -> shelf
```

Each thumbnail responds to:

- **Click** — on Linux, open an image with `[shelf] open_with` (default: the desktop's default app) and a video in the default app; on Windows, copy the image or video file reference.
- **Right-click** (Linux) — copy the image or video file reference.
- **Drag** — start a drag-and-drop into another app; the drop offers both the
  image (`image/png`) and a file path (`text/uri-list`) for maximum
  compatibility, including many XWayland apps. If the drop isn't accepted
  anywhere, the image is copied to the clipboard as a fallback.
- **Hover** then the icons: **Save** writes the media to disk, **✕** dismisses it.

The shelf is served by a small long-lived daemon. It starts automatically on
the first Wayland capture and at sign-in after a Windows installer setup. To
start it explicitly (or autostart it), run:

```sh
boltsnap daemon
# Hyprland autostart (optional): add to hyprland.conf
exec-once = boltsnap daemon
```

The shelf state is **RAM-only** and is cleared if the daemon restarts. Image
files use the system temporary directory; large video files use the disk-backed
Boltsnap cache and are removed when their cards are dismissed or the daemon is
restarted.
`boltsnap doctor` reports the Wayland session, whether the daemon is running,
and the socket path.

Other tools can put media on the shelf through that socket. Each message is a
frame: header length and payload length as big-endian `u32`, then a JSON
header, then the payload.

- `{"cmd":"add","source":"…","output":"DP-1"}` with PNG bytes as payload.
- `{"cmd":"add_video","path":"/abs/clip.mp4","source":"…","output":"DP-1"}`
  with no payload. The shelf imports the file into its cache and answers with a
  frame whose header is `{"ok":true,"path":"…"}` (or `"ok":false` with an
  `"error"`). Linux only; the unmaintained Windows backend answers with an
  error.

`output` is optional and picks the monitor the shelf appears on.

Video import copies the file (a symlink is followed) into the shelf cache
before the answer, so later changes to the original do not reach the card.
Keep the file unchanged while the import runs; a copy that comes out shorter
than the file was is rejected.

Flags still work: `--copy` also copies to the clipboard on capture, `-o PATH`
/ `--save` write a file (no shelf), and `-o -` streams PNG to stdout. **X11 is
unchanged** — it keeps the classic copy-to-clipboard one-shot behavior with no
shelf.
