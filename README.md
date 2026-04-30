# Boltsnap

Fast Rust screenshot tool for Wayland (wlroots, Hyprland, sway) and X11.

- In-process capture via `libwayshot` on Wayland — no `grim` shell-out
- ~7-14× faster than `wayshot`, `grim`, `flameshot` on the same hardware
- Built-in egui annotation editor, fully opt-in
- Pipe-friendly: `-o -` writes PNG to stdout, drop-in for `grim`/`maim`

## Install

```sh
cargo install --path .
```

### NixOS

A `flake.nix` is included. Either:

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
libGL, libdrm and the xorg stack from the nix store at runtime.

Runtime helpers:

| Backend | Capture | Region select | Clipboard |
|---------|---------|---------------|-----------|
| Wayland | in-process (libwayshot) | `slurp` | in-process (wl-clipboard-rs) |
| X11     | `maim` (or ImageMagick `import`) | maim/slop | `xclip` |

`hyprctl` is used for window enumeration on Hyprland. Other Wayland
compositors fall back to area selection for window mode.

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
