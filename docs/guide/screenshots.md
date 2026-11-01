# Taking screenshots

```sh
boltsnap                                # area capture
boltsnap window                         # pick a window
boltsnap active-window                  # current focused window
boltsnap full                           # all monitors, copy

boltsnap full --no-copy -o /tmp/x.png   # write file, skip clipboard
boltsnap area --no-copy -o -            # PNG to stdout

boltsnap doctor                         # check helpers + capabilities
boltsnap settings [PAGE]                # settings window (Shelf, Screenshots, Recording, Replay, General)
```

The settings window also opens from the tray (**Settings…**) and from the app
launcher (**Boltsnap**). See [Settings window](configuration.md#settings-window).

Boltsnap does not bundle an editor. Linux shelf images open in the desktop's
default image app, or in the editor set with `[shelf] open_with` (Settings →
Screenshots → Open screenshots with). An image an editor saves over is kept in
the save folder when its card goes. Any other editor reads piped PNG output:

```sh
boltsnap area --no-copy -o - | eddy -f -
boltsnap area --no-copy -o - | satty --filename -
```

## The region selector

Screenshot region selection freezes all monitors and allows a single selection
across monitor boundaries, including offset and differently scaled outputs.
The saved image uses the highest output scale; gaps between monitors are black.
Recording and replay selection remain on their target monitor. If an output is
connected, disconnected, or rearranged during selection, retry the capture.

Hold **Ctrl** in the selector to snap: the rectangle under the pointer shows as
the selection would, the wheel steps a level out (up) or in (down), a click
takes it. The levels, innermost first: what the picture's own long straight
edges enclose (an embedded video, a panel; from the screenshot, or from the
frozen replay frame when clipping), the visible windows (Hyprland), the screen.

## Suggested keybinds

```
bind = , Print, exec, boltsnap area
bind = CTRL, Print, exec, boltsnap full
bind = ALT, Print, exec, boltsnap record
bind = ALT SHIFT, Print, exec, boltsnap replay save
```

On a keyboard without a Print key, the same binds on another key (for example
`Home`) work the same way; a bare `Home` then no longer reaches applications.
