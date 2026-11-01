# Installing Boltsnap

## Linux x86_64

Each tagged release on the
[GitHub releases page](https://github.com/drvcvt/boltsnap/releases) ships:

- `boltsnap-vX.Y.Z-x86_64-linux.tar.gz` — standalone binary
- `boltsnap_X.Y.Z-1_amd64.deb` — Debian / Ubuntu package
- `SHA256SUMS`

```sh
# .deb (Debian/Ubuntu)
curl -L -o boltsnap.deb "https://github.com/drvcvt/boltsnap/releases/latest/download/boltsnap_<VERSION>-1_amd64.deb"
sudo apt install ./boltsnap.deb

# Tarball (any glibc-based distro with Mesa's libgbm)
curl -L -o boltsnap.tar.gz "https://github.com/drvcvt/boltsnap/releases/latest/download/boltsnap-<VERSION>-x86_64-linux.tar.gz"
tar xf boltsnap.tar.gz
sudo install -m755 boltsnap-*/boltsnap /usr/local/bin/
```

The binary links `libgbm.so.1`, which every Wayland compositor already
installs. The Vulkan loader (`libvulkan.so.1`) is optional: without it,
screenshots use shared memory instead of GPU readback.

## From source

Wayland screenshots use our unpublished `libway` 0.1 library. A source snapshot
is included under `vendor/libway`, so a normal checkout needs no sibling project.
See [library development and synchronization](../../vendor/README.md) and the
[integration validation](../libway.md).

Build dependencies on Debian and Ubuntu:

```sh
sudo apt install pkg-config build-essential \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev wayland-protocols \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libgl1-mesa-dev libegl1-mesa-dev libgbm-dev libfontconfig1-dev
```

Then build and install into `~/.cargo/bin`:

```sh
cargo install --path .
```

The `.deb` and the Nix package install a launcher entry with the Boltsnap
icon. After `cargo install` or with the tarball, add it from the checkout or
the unpacked tarball; `sed` writes the binary's full path, since
`~/.cargo/bin` is often not on the desktop session's `PATH`:

```sh
mkdir -p ~/.local/share/applications
sed "s|^Exec=boltsnap|Exec=$(command -v boltsnap)|" assets/linux/boltsnap.desktop \
  > ~/.local/share/applications/boltsnap.desktop
for size in 64 128 256; do
  install -Dm644 assets/linux/boltsnap-$size.png \
    ~/.local/share/icons/hicolor/${size}x${size}/apps/boltsnap.png
done
```

## Arch Linux

The tagged release binaries can lag behind `main` — for the current shelf and
screen-recording features, build from source:

```sh
sudo pacman -S --needed rust wayland libxkbcommon mesa pkgconf base-devel gpu-screen-recorder
git clone https://github.com/drvcvt/boltsnap
cd boltsnap
cargo install --path .        # -> ~/.cargo/bin/boltsnap
```

## NixOS

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

The flake bakes RPATH so the binary finds wayland, libxkbcommon,
libGL, libdrm, libgbm and the xorg stack from the nix store at runtime.

**Don't run a `cargo build`'d binary directly on NixOS.** Errors like
`error while loading shared libraries: libgbm.so.1: cannot open shared object file`
mean the binary is being executed without nix-store libraries on `LD_LIBRARY_PATH`.
The binary needs the wrapper that the flake creates. Don't run boltsnap with
`sudo` either — sudo strips `LD_LIBRARY_PATH` and screenshot tools don't need
root anyway.

## Windows 10/11 (experimental, unmaintained)

Windows release artifacts are paused because there is no maintainer available
to run the required real-system smoke tests. The backend remains available for
source builds without support guarantees.

To build on Windows, install the Rust MSVC toolchain, Visual Studio Build Tools
with **Desktop development with C++**, and a current Windows SDK:

```powershell
git clone https://github.com/drvcvt/boltsnap
cd boltsnap
cargo build --release
.\target\release\boltsnap.exe area
```

Maintainers can build the installer with:

```powershell
.\packaging\windows\build-nsis.ps1
```

## Platform support

| Backend | Status                     | Capture        | Region select         | Clipboard          |
|---------|----------------------------|----------------|-----------------------|--------------------|
| Wayland | Supported                  | libway + portal fallback | across monitors, tiny-skia | wl-clipboard-rs    |
| X11     | Supported                  | x11rb GetImage | unavailable           | arboard            |
| Windows | Experimental, unmaintained | DXGI + WGC     | in-process tiny-skia  | Win32 + OLE        |

Replay capture currently has an experimental Hyprland adapter. It additionally
requires the separate replay worker, FFmpeg and GPU Screen Recorder with native
KMS permission. X11, other Wayland compositors and Windows have no replay adapter
yet. See [setup, controls and limitations](../replay.md).

## Wayland compatibility

Boltsnap supports Wayland protocols, not a specific compositor framework:

| Feature | Required protocol |
|---------|-------------------|
| Direct screenshots | `ext-image-copy-capture-v1` + `ext-image-capture-source-v1`, or `wlr-screencopy-unstable-v1` |
| Recording via gpu-screen-recorder | KMS capture (gsr-kms-server); the wf-recorder fallback needs `wlr-screencopy-unstable-v1` |
| Region selector and screenshot shelf | `wlr-layer-shell-unstable-v1` |
| Clipboard | `ext-data-control-v1` or `wlr-data-control-unstable-v1` |

Compositors Boltsnap has been run on:

| Stack | Tested compositors |
|-------|--------------------|
| Hyprland / Aquamarine | Hyprland |
| wlroots | Sway |
| Smithay | Niri |

Other Wayland compositors may work if they expose the required protocols;
using wlroots or Smithay alone is not a compatibility guarantee.

Screenshots read back from dma-bufs through Vulkan when the driver supports
it and fall back to shared memory otherwise; the daemon logs the path it uses
once (`screenshots read back via Vulkan`). Screenshot capture and clipboard
handling need no CLI helpers. `hyprctl` is
optional and supplies active-window geometry on Hyprland; other Wayland
compositors fall back to the in-process selector for window mode. Recording
uses GPU Screen Recorder (wf-recorder when it is not installed) and FFmpeg.
