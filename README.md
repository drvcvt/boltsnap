# Boltsnap

Fast Rust screenshot tool for Wayland (wlroots, Hyprland, sway) and X11 — no
`maim`, `import`, `slurp`, `xdotool`, `xwininfo`, `xclip` or other CLI helpers
required. Everything runs in-process.

- In-process capture: `libwayshot` on Wayland, `x11rb` on X11
- In-process selection overlay (drag a region) — replaces `slurp` / `slop`
- In-process clipboard: `wl-clipboard-rs` on Wayland, `arboard` on X11
- Pre-pushed compositor rules so the selector appears INSTANTLY (no fade-in
  animation on Hyprland or Sway)
- Built-in egui annotation editor, fully opt-in
- Pipe-friendly: `-o -` writes PNG to stdout

## Install

### Pre-built binaries (Linux x86_64)

Each tagged release on the
[GitHub releases page](https://github.com/drvcvt/boltsnap/releases) ships:

- `boltsnap-vX.Y.Z-x86_64-linux.tar.gz` — standalone binary
- `boltsnap_X.Y.Z_amd64.deb` — Debian / Ubuntu package
- `SHA256SUMS`

```sh
# .deb (Debian/Ubuntu)
curl -L -o boltsnap.deb "https://github.com/drvcvt/boltsnap/releases/latest/download/boltsnap_<VERSION>_amd64.deb"
sudo apt install ./boltsnap.deb

# Tarball (any glibc-based distro)
curl -L -o boltsnap.tar.gz "https://github.com/drvcvt/boltsnap/releases/latest/download/boltsnap-<VERSION>-x86_64-linux.tar.gz"
tar xf boltsnap.tar.gz
sudo install -m755 boltsnap-*/boltsnap /usr/local/bin/
```

### From source

```sh
cargo install --path .
```

### NixOS

A `flake.nix` is included.

```sh
nix run github:drvcvt/boltsnap            # one-shot run
nix profile install github:drvcvt/boltsnap # install to profile
nix develop                                # dev shell with cargo + libs ready
```

Without flakes:

```sh
nix-shell                                  # uses shell.nix
cargo build --release
```

The flake bakes RPATH so the binary finds wayland, libxkbcommon, vulkan-loader,
libGL, libdrm, libgbm and the xorg stack from the nix store at runtime.

**Don't run a `cargo build`'d binary directly on NixOS.** Errors like
`error while loading shared libraries: libgbm.so.1: cannot open shared object file`
mean the binary is being executed without nix-store libraries on `LD_LIBRARY_PATH`.
The binary needs the wrapper that the flake creates. Don't run boltsnap with
`sudo` either — sudo strips `LD_LIBRARY_PATH` and screenshot tools don't need
root anyway.

Backends:

| Backend | Capture        | Region select         | Clipboard          |
|---------|----------------|-----------------------|--------------------|
| Wayland | libwayshot     | in-process eframe     | wl-clipboard-rs    |
| X11     | x11rb GetImage | in-process eframe     | arboard            |

The only optional external is `hyprctl`, used for active-window geometry on
Hyprland. Other Wayland compositors fall back to the in-process selector for
window mode.

## Usage

```sh
boltsnap                                # area, copy PNG, remember as last
boltsnap area --edit                    # capture, then open editor
boltsnap --edit                         # open last screenshot in editor
boltsnap window                         # pick a window
boltsnap active-window                  # current focused window
boltsnap full                           # all monitors, copy

boltsnap full --no-copy -o /tmp/x.png   # write file, skip clipboard
boltsnap area --no-copy -o -            # PNG to stdout
boltsnap edit /tmp/x.png                # open existing image in editor

boltsnap doctor                         # check helpers + capabilities
```

The editor is opt-in. If you prefer something else:

```sh
boltsnap area --no-copy -o - | eddy -f -
boltsnap area --no-copy -o - | satty --filename -
```

## Suggested keybinds

```
bind = , Print, exec, boltsnap area
bind = SHIFT, Print, exec, boltsnap area --edit
bind = CTRL, Print, exec, boltsnap full
bind = $mod, Print, exec, boltsnap --edit
```

## Editor

Floating, always-on-top window. Annotations:

| Tool      | Key |
|-----------|-----|
| Move/pan  | `M` |
| Arrow     | `A` |
| Pen       | `P` |
| Box       | `R` |
| Highlight | `H` |
| Redact    | `X` |
| Blur      | `B` |

`Ctrl+Z` undo, `F1` help, `Esc` close, `Space`/`Enter` save and copy,
middle-mouse drag to pan, scroll to zoom.

## Build

```sh
cargo build --release
cargo test
```

## Screenshot shelf (Wayland / wlroots, e.g. Hyprland)

On Wayland, an interactive capture no longer copies-and-exits. Instead the
screenshot lands as a small floating **thumbnail in the bottom-left corner**
of the screen — a macOS-style shelf — and stays there until you use or dismiss
it. Multiple screenshots stack, newest on top.

```sh
boltsnap area        # capture a region -> appears in the shelf
boltsnap full        # whole screen -> shelf
boltsnap window      # pick a window -> shelf
```

Each thumbnail responds to:

- **Click** — copy the PNG to the clipboard (then paste with Ctrl+V).
- **Drag** — start a drag-and-drop into another app; the drop offers both the
  image (`image/png`) and a file path (`text/uri-list`) for maximum
  compatibility, including many XWayland apps. If the drop isn't accepted
  anywhere, the image is copied to the clipboard as a fallback.
- **Hover** then the icons: **✎** open in the annotation editor (the result
  updates the thumbnail), **⧉** copy, **✕** dismiss.

The shelf is served by a small long-lived daemon. It starts automatically on
the first Wayland capture; you don't need to set anything up. To start it
explicitly (or autostart it), run:

```sh
boltsnap daemon
# Hyprland autostart (optional): add to hyprland.conf
exec-once = boltsnap daemon
```

The shelf is **RAM-only**: its contents are cleared if the daemon restarts.
`boltsnap doctor` reports the Wayland session, whether the daemon is running,
and the socket path.

Flags still work: `--copy` also copies to the clipboard on capture, `-o PATH`
/ `--save` write a file (no shelf), and `-o -` streams PNG to stdout. **X11 is
unchanged** — it keeps the classic copy-to-clipboard one-shot behavior with no
shelf.

## Configuration

Create `~/.config/boltsnap/config.toml` to set persistent defaults:

```toml
# Directory where the shelf Save button writes timestamped PNGs.
# Default: ~/Bilder/boltsnap
save_dir = "~/Bilder/boltsnap"

# Annotation editor launched by the shelf card viewer and the --edit flag.
# Default: eddy
editor = "eddy"
```

Override precedence (highest to lowest):

1. CLI flag — `--save-dir DIR`, `--editor CMD`
2. Environment variable — `$BOLTSNAP_SAVE_DIR`, `$BOLTSNAP_EDITOR`
3. Config file — `~/.config/boltsnap/config.toml`
4. Built-in default

Clicking a shelf card opens the image in eddy (viewer + editor). The **Save**
button (top-left of a card) writes a timestamped PNG to the configured save
directory.

## Screen recording

```sh
boltsnap record
```

Opens the region selector in record mode. Draw a region, then click the **REC**
pill to start recording. A thin click-through border frames the captured area
while you use your PC normally. The shelf tray shows **●** + elapsed time and a
**Stop** button.

Hit **Stop** → **Confirm** to finish. The resulting `.mp4` appears in the shelf
as a **video card** (▶ badge); clicking it opens the file in **eddy**.

**Requires `wf-recorder`** (uses `wlr-screencopy` directly — no PipeWire,
no portal, no permission dialog).

```sh
# Arch / Manjaro
pacman -S wf-recorder
```

### Recording config keys

```toml
# ~/.config/boltsnap/config.toml

# Video codec passed to wf-recorder.
# Default: h264_nvenc (NVIDIA hardware encoding)
# Use libx264 if you have no NVENC GPU.
record_codec = "libx264"

# Directory where finished .mp4 files are saved.
# Default: same as save_dir
record_dir = "~/Videos/boltsnap"
```

`$BOLTSNAP_RECORD_CODEC` overrides `record_codec` from the environment.

### Suggested keybind

```
bind = ALT, Print, exec, boltsnap record
```

`boltsnap record` is a separate command from screenshot — bind it independently.

**v1 limitations:** single-take only (no pause); video only (no audio).

## License

MIT.
