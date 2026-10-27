"""Boltsnap's icons from the mark (construct.py), every measure from its
module s:

- Variant A, the tray and the app icon: the mark in `#ececec` with a `#121212`
  outline s/8 wide outside it, so it reads on dark and light bars. Recording
  colours the mark's own dot red, a pause amber.
- The dark variant for light surfaces: the mark in `#121212`, a white outline.
- Tray sizes are fitted to the pixel grid: s is a whole number of pixels and
  the mark's left and top edges sit on whole pixels, so the stem, its foot and
  the cap do not smear over two pixel columns (tray plan, 2026-10-27).

    python3 -m venv /tmp/mark && /tmp/mark/bin/pip install shapely
    /tmp/mark/bin/python assets/logo/icons.py

Needs rsvg-convert. Writes boltsnap-icon.svg and boltsnap-icon-dark.svg next
to this file, assets/icons/tray/{idle,recording,paused}-SIZE.png and
assets/linux/boltsnap-SIZE.png.
"""

import subprocess
import sys
import tempfile
from pathlib import Path

import construct as c

ROOT = Path(__file__).resolve().parents[2]
INK, DARK, WHITE = "#ececec", "#121212", "#ffffff"
DOTS = {"idle": None, "recording": "#ff453a", "paused": "#ffb020"}
TRAY_SIZES = (16, 20, 24, 32, 48, 64)
APP_SIZES = (64, 128, 256)
OUTLINE = c.S / 8
WIDTH, HEIGHT = 2 * c.R, 2 * c.R + c.S  # the mark: 3 s by 4 s


def fit(size):
    """(s in pixels, left, top) for the mark in a `size` square: the largest
    whole s whose mark (4 s high) and outline (s/8 each side) fit, the outline
    allowed to run up to 3/4 px past the edge (at 16 to 24 px it is under a
    pixel wide, and the mark then fills the height like other tray icons);
    left and top on whole pixels, as close to centred as they get."""
    s = int((size + 1.5) / (HEIGHT / c.S + 2 * OUTLINE / c.S))
    left = (size - 3 * s) // 2
    top = (size - 4 * s) // 2
    return s, left, top


def svg(size, fill, stroke, dot=None, placement=None):
    """The mark in a `size` square; `placement` (s, left, top) in pixels, or
    the mark and outline 7/8 of the side high, centred."""
    body, spot = c.mark()
    if placement is None:
        s = size * 7 / 8 / (HEIGHT / c.S + 2 * OUTLINE / c.S)
        left, top = (size - 3 * s) / 2, (size - 4 * s) / 2
    else:
        s, left, top = placement
    k = s / c.S
    paint = f'stroke="{stroke}" stroke-width="{2 * OUTLINE}" stroke-linejoin="round" paint-order="stroke"'
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">'
        f'<g transform="translate({left} {top}) scale({k})">'
        f'<path fill-rule="evenodd" fill="{fill}" {paint} d="{c.data(body)}"/>'
        f'<path fill="{dot or fill}" {paint} d="{c.data(spot)}"/>'
        "</g></svg>"
    )


def scalable(fill, stroke):
    """The mark with its outline as a free-standing SVG."""
    body, spot = c.mark()
    pad = OUTLINE
    paint = f'stroke="{stroke}" stroke-width="{2 * OUTLINE}" stroke-linejoin="round" paint-order="stroke"'
    return (
        "<!-- Built by icons.py from construct.py; edit those, not the paths. -->\n"
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{-pad:.0f} {-pad:.0f} {WIDTH + 2 * pad:.0f} {HEIGHT + 2 * pad:.0f}">\n'
        f'  <path fill-rule="evenodd" fill="{fill}" {paint} d="{c.data(body)}"/>\n'
        f'  <path fill="{fill}" {paint} d="{c.data(spot)}"/>\n'
        "</svg>\n"
    )


def png(doc, size, out):
    with tempfile.NamedTemporaryFile("w", suffix=".svg") as f:
        f.write(doc)
        f.flush()
        subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), f.name, "-o", str(out)], check=True)


def main():
    here = Path(__file__).resolve().parent
    (here / "boltsnap-icon.svg").write_text(scalable(INK, DARK))
    (here / "boltsnap-icon-dark.svg").write_text(scalable(DARK, WHITE))
    tray = ROOT / "assets/icons/tray"
    tray.mkdir(exist_ok=True)
    for size in TRAY_SIZES:
        for state, dot in DOTS.items():
            png(svg(size, INK, DARK, dot, fit(size)), size, tray / f"{state}-{size}.png")
    for size in APP_SIZES:
        png(svg(size, INK, DARK), size, ROOT / f"assets/linux/boltsnap-{size}.png")


if __name__ == "__main__":
    sys.exit(main())
