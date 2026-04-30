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
boltsnap area --no-copy -o - | swappy -f -
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

## License

MIT.
