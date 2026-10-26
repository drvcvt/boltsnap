"""Builds boltsnap-mark.svg from a few shapes, every size from one module.

    python3 -m venv /tmp/mark && /tmp/mark/bin/pip install shapely
    /tmp/mark/bin/python assets/logo/construct.py > assets/logo/boltsnap-mark.svg

The module is the stem's width s. O is the bowl's centre; y grows downwards.
- Bowl: the superellipse |x/R|^n + |y/R|^n = 1 with R = 3/2 s and n = 2.4.
- Stem: its left edge is the bowl's left edge, its foot the bowl's foot, its
  top a half circle one s above the bowl. The frame is 3 s by 4 s.
- Cap, counter and the dot's moat all have radius s/2; the counter sits on O,
  so the bowl's wall right of it is exactly s thick.
- Dot: the moat shrunk by phi, on the 45 degree diagonal through O, flush with
  the bowl's outline, so the moat bites the corner open.
- The bowl is mirror-symmetric about O's horizontal axis, so where the stem's
  right edge meets it at the top (the valley) and at the foot (the dent) are
  mirror images. Both, the stem's foot corner and the bite's tips are rounded
  with s/8.
"""

import math

from shapely.geometry import Point, Polygon, box
from shapely.ops import unary_union

PHI = (1 + 5**0.5) / 2
S = 120.0  # the module in view box units: the mark is 360 x 480
R, N = 1.5 * S, 2.4
HALF, ROUND = S / 2, S / 8
DOT = HALF / PHI
ARC = 128  # segments per quarter circle


def bowl(steps=1024):
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, s = math.cos(t), math.sin(t)
        pts.append(
            (R * math.copysign(abs(c) ** (2 / N), c), R * math.copysign(abs(s) ** (2 / N), s))
        )
    return Polygon(pts)


def outline_at(angle):
    """The bowl's distance from O along `angle`."""
    c, s = abs(math.cos(angle)), abs(math.sin(angle))
    return R / (c**N + s**N) ** (1 / N)


def mark():
    top = -R - S
    stem = box(-R, top + HALF, -R + S, R)
    cap = Point(-R + HALF, top + HALF).buffer(HALF, ARC)
    foot = box(-R, R - HALF, -R + HALF, R).difference(Point(-R + HALF, R - HALF).buffer(HALF, ARC))
    stem = unary_union([stem, cap]).difference(foot)
    shape = unary_union([bowl(), stem]).buffer(ROUND, ARC).buffer(-ROUND, ARC)
    angle = -math.pi / 4
    d = outline_at(angle) - DOT
    centre = (d * math.cos(angle), d * math.sin(angle))
    shape = shape.difference(Point(0, 0).buffer(HALF, ARC))
    shape = shape.difference(Point(*centre).buffer(HALF, ARC))
    shape = shape.buffer(-ROUND, ARC).buffer(ROUND, ARC)
    return shape.simplify(0.05), Point(*centre).buffer(DOT, ARC).simplify(0.05)


def data(geom):
    ox, oy = R, R + S  # the frame's top left corner to the origin

    def ring(coords):
        pts = list(coords)[:-1]
        return "M" + " L".join(f"{x + ox:.2f} {y + oy:.2f}" for x, y in pts) + " Z"

    return " ".join([ring(geom.exterior.coords)] + [ring(i.coords) for i in geom.interiors])


if __name__ == "__main__":
    body, dot = mark()
    w, h = 2 * R, 2 * R + S
    print(f"""<!-- The Boltsnap mark, built by construct.py next to this file; edit
     that, not the paths. Two paths: the body (counter as a hole, even-odd)
     and the dot. -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0f} {h:.0f}">
  <path fill-rule="evenodd" d="{data(body)}"/>
  <path d="{data(dot)}"/>
</svg>""")
