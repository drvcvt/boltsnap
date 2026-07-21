# Experimental Hyprland XDND workaround

> [!CAUTION]
> This plugin is experimental and intentionally opt-in. It uses an unstable
> internal C++ function hook, runs inside the Hyprland process, and can crash
> the compositor. Hyprland function hooks currently work only on x86_64. A
> Hyprland update can break the plugin even when it compiled successfully.

Hyprland currently sends `XdndEnter` when a Wayland drag enters an XWayland
window, but sends `XdndPosition` only after another pointer-motion event. If the
button is released first, applications such as Electron can ignore the drop.
This plugin hooks only `CX11DataDevice::sendEnter`, calls the original method,
then sends one position event using the current pointer position relative to the
destination surface.

This does **not** patch or replace the Hyprland binary and does not include the
separate logical monitor-order fix. Boltsnap never installs or loads it
automatically.

## Install with hyprpm

Install a compiler and the development dependencies required by `hyprpm`, then:

```sh
hyprpm add https://github.com/drvcvt/boltsnap
hyprpm enable boltsnap-xdnd-fix
hyprpm reload
```

The plugin refuses to load when its build-time Hyprland ABI differs from the
running compositor or when the internal hook cannot be identified exactly.

## Disable immediately if Hyprland becomes unstable

```sh
hyprpm disable boltsnap-xdnd-fix
hyprpm reload
```

Remove the plugin once the equivalent fix is available in the installed
Hyprland release.
