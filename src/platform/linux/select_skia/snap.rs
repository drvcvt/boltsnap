//! Snapping in the selector (snap plan, `docs/plans/selector/
//! 2026-10-27-snap-detection.md`): with Ctrl held the rectangle of the
//! current level under the pointer shows as the selection would, the wheel
//! steps a level out or in, a click takes it. The levels: what the image's
//! edges enclose (`selector::snap`), the visible Hyprland windows, the
//! screen under the pointer.

use serde_json::Value;

use super::{Mode, Selector};
use crate::selector::edit::Rect;
use crate::selector::snap::{self, Edges};

/// Touchpad scrolling arrives in pixels, many small steps: this much makes
/// one level, as one notch of a wheel does.
const SCROLL_STEP: f64 = 15.0;

#[derive(Default)]
pub(super) struct Snap {
    /// Ctrl is held.
    held: bool,
    /// The edges of the image, with what tells that image apart (its
    /// pixels' address and size): the frozen replay frame replaces it.
    edges: Option<((usize, u32, u32), Option<Edges>)>,
    /// The visible windows in surface pixels, asked once per selector.
    windows: Option<Vec<Rect>>,
    levels: Vec<Rect>,
    level: usize,
    scrolled: f64,
}

impl Selector {
    /// Ctrl went down or up; whether what shows changed.
    pub(super) fn set_snap_held(&mut self, held: bool) -> bool {
        if self.snap.held == held {
            return false;
        }
        self.snap.held = held;
        self.snap.scrolled = 0.0;
        if held {
            self.update_snap();
        }
        true
    }

    /// The rectangle shown in place of the selection while Ctrl is held.
    pub(super) fn snap_shown(&self) -> Option<Rect> {
        self.snap
            .held
            .then(|| self.snap.levels.get(self.snap.level).copied())
            .flatten()
    }

    /// The levels under the pointer now; a new set starts again innermost.
    /// Whether what shows changed.
    pub(super) fn update_snap(&mut self) -> bool {
        if !self.snap.held {
            return false;
        }
        let before = self.snap_shown();
        let inner = self.inner_boxes();
        if self.snap.windows.is_none() {
            let origin = self.output_origin.unwrap_or((0, 0));
            self.snap.windows = Some(hyprland_windows(origin, (self.surf_w, self.surf_h)));
        }
        let windows = self.snap.windows.as_deref().unwrap_or(&[]);
        let levels = snap::levels(&inner, windows, self.screen_at_cursor(), self.cursor);
        if levels != self.snap.levels {
            self.snap.levels = levels;
            self.snap.level = 0;
        }
        self.snap_shown() != before
    }

    /// A wheel or touchpad scroll of `steps` notches or `pixels` while Ctrl
    /// is held: up goes a level out, down a level in. Whether what shows
    /// changed.
    pub(super) fn scroll_snap(&mut self, steps: i32, pixels: f64) -> bool {
        if !self.snap.held || self.snap.levels.is_empty() {
            return false;
        }
        let step = if steps != 0 {
            self.snap.scrolled = 0.0;
            steps.signum()
        } else {
            self.snap.scrolled += pixels;
            if self.snap.scrolled.abs() < SCROLL_STEP {
                return false;
            }
            let step = self.snap.scrolled.signum() as i32;
            self.snap.scrolled = 0.0;
            step
        };
        let last = self.snap.levels.len() - 1;
        let level = if step < 0 {
            (self.snap.level + 1).min(last)
        } else {
            self.snap.level.saturating_sub(1)
        };
        let changed = level != self.snap.level;
        self.snap.level = level;
        changed
    }

    /// A click with Ctrl held: the shown rectangle becomes the selection.
    /// `None` when nothing shows, and the click goes on as any other.
    pub(super) fn take_snap(&mut self) -> Option<Rect> {
        let rect = self.snap_shown()?;
        self.mode = Mode::Editing { rect };
        self.interaction = None;
        Some(rect)
    }

    /// What the image's edges enclose around the pointer, in surface pixels;
    /// the edges are built the first time and again for a new image.
    fn inner_boxes(&mut self) -> Vec<Rect> {
        let Some(image) = self.image.as_ref() else {
            return Vec::new();
        };
        let (w, h) = image.dimensions();
        let id = (image.as_raw().as_ptr() as usize, w, h);
        if self.snap.edges.as_ref().map(|(seen, _)| *seen) != Some(id) {
            self.snap.edges = Some((id, Edges::new(image)));
        }
        let Some((_, Some(edges))) = self.snap.edges.as_ref() else {
            return Vec::new();
        };
        if self.surf_w == 0 || self.surf_h == 0 {
            return Vec::new();
        }
        // The image may have other pixels than the surface (a replay frame
        // in the video's size).
        let kx = f64::from(w) / f64::from(self.surf_w);
        let ky = f64::from(h) / f64::from(self.surf_h);
        let at = (
            (self.cursor.0 * kx).floor().clamp(0.0, f64::from(w - 1)) as u32,
            (self.cursor.1 * ky).floor().clamp(0.0, f64::from(h - 1)) as u32,
        );
        edges
            .around(at)
            .into_iter()
            .map(|(x0, y0, x1, y1)| Rect {
                x: f64::from(x0) / kx,
                y: f64::from(y0) / ky,
                w: f64::from(x1 - x0) / kx,
                h: f64::from(y1 - y0) / ky,
            })
            .collect()
    }

    /// The screen under the pointer: the whole surface when recording one
    /// output, else the captured monitor it is on.
    fn screen_at_cursor(&self) -> Rect {
        let whole = Rect {
            x: 0.0,
            y: 0.0,
            w: f64::from(self.surf_w),
            h: f64::from(self.surf_h),
        };
        if self.record_mode {
            return whole;
        }
        let (left, top) = self.output_origin.unwrap_or((0, 0));
        self.captured_monitors
            .iter()
            .map(|m| Rect {
                x: f64::from(m.origin.0 - left),
                y: f64::from(m.origin.1 - top),
                w: f64::from(m.logical_size.0),
                h: f64::from(m.logical_size.1),
            })
            .find(|r| r.contains(self.cursor))
            .unwrap_or(whole)
    }
}

/// The windows Hyprland shows now, in surface pixels; none off Hyprland.
fn hyprland_windows(origin: (i32, i32), surface: (u32, u32)) -> Vec<Rect> {
    let parse = |command| {
        super::super::hypr::json(command).and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    };
    match (parse("clients"), parse("monitors")) {
        (Some(clients), Some(monitors)) => visible_windows(&clients, &monitors, origin, surface),
        _ => Vec::new(),
    }
}

/// The windows on a workspace some monitor shows (its active or its
/// special one), mapped and not hidden, moved by `origin` (the surface's
/// top left in the layout) and cut to the surface.
pub(super) fn visible_windows(
    clients: &Value,
    monitors: &Value,
    origin: (i32, i32),
    (sw, sh): (u32, u32),
) -> Vec<Rect> {
    let shown: Vec<i64> = monitors
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| {
            [&m["activeWorkspace"]["id"], &m["specialWorkspace"]["id"]]
                .into_iter()
                .filter_map(Value::as_i64)
                .filter(|&id| id != 0)
        })
        .collect();
    let pair = |v: &Value| Some((v[0].as_f64()?, v[1].as_f64()?));
    clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["mapped"] == true && c["hidden"] != true)
        .filter(|c| {
            c["workspace"]["id"]
                .as_i64()
                .is_some_and(|id| shown.contains(&id))
        })
        .filter_map(|c| {
            let (x, y) = pair(&c["at"])?;
            let (w, h) = pair(&c["size"])?;
            let x0 = (x - f64::from(origin.0)).max(0.0);
            let y0 = (y - f64::from(origin.1)).max(0.0);
            let x1 = (x - f64::from(origin.0) + w).min(f64::from(sw));
            let y1 = (y - f64::from(origin.1) + h).min(f64::from(sh));
            (x1 > x0 && y1 > y0).then_some(Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::visible_windows;
    use crate::selector::edit::Rect;

    #[test]
    fn only_shown_windows_count_and_they_are_in_surface_pixels() {
        let monitors = json!([
            {"name": "DP-3", "activeWorkspace": {"id": 3}, "specialWorkspace": {"id": 0}},
            {"name": "DP-1", "activeWorkspace": {"id": 8}, "specialWorkspace": {"id": -98}},
        ]);
        let window = |ws: i64, at: [i64; 2], size: [i64; 2], mapped: bool, hidden: bool| json!({"workspace": {"id": ws}, "at": at, "size": size, "mapped": mapped, "hidden": hidden});
        let clients = json!([
            window(3, [10, 38], [1900, 1032], true, false),
            window(8, [1930, 38], [1900, 1032], true, false),
            // Another workspace, hidden, unmapped: none of them.
            window(1, [10, 38], [1900, 1032], true, false),
            window(3, [100, 100], [300, 300], true, true),
            window(3, [100, 100], [300, 300], false, false),
            // The special workspace on DP-1, partly off the right edge.
            window(-98, [3700, 500], [400, 300], true, false),
        ]);
        // The surface is DP-1 alone (record mode): its origin is 1920, 0.
        let rects = visible_windows(&clients, &monitors, (1920, 0), (1920, 1080));
        let r = |x, y, w, h| Rect { x, y, w, h };
        assert_eq!(
            rects,
            [
                r(10.0, 38.0, 1900.0, 1032.0),
                r(1780.0, 500.0, 140.0, 300.0)
            ]
        );
    }

    /// `SNAP_PNG=desktop.png SNAP_DIR=dir cargo test --bin boltsnap snap_probe
    /// -- --ignored --nocapture`: the levels at a few points of a real
    /// screen (its left 1920 x 1080), with `clients.json` and
    /// `monitors.json` from `hyprctl -j` in `SNAP_DIR`.
    #[test]
    #[ignore = "reads a screen capture from SNAP_PNG"]
    fn snap_probe() {
        use crate::selector::snap::{Edges, levels};
        let dir = std::path::PathBuf::from(std::env::var("SNAP_DIR").expect("SNAP_DIR"));
        let image = image::open(std::env::var("SNAP_PNG").expect("SNAP_PNG"))
            .unwrap()
            .crop_imm(0, 0, 1920, 1080)
            .into_rgba8();
        let read = |name: &str| -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(dir.join(name)).unwrap()).unwrap()
        };
        let windows = visible_windows(
            &read("clients.json"),
            &read("monitors.json"),
            (0, 0),
            (1920, 1080),
        );
        println!("windows: {windows:?}");
        let edges = Edges::new(&image).unwrap();
        let screen = Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        for point in [
            (400u32, 300u32),
            (960, 600),
            (1800, 900),
            (100, 1000),
            (960, 62),
        ] {
            let inner: Vec<Rect> = edges
                .around(point)
                .into_iter()
                .map(|(x0, y0, x1, y1)| Rect {
                    x: f64::from(x0),
                    y: f64::from(y0),
                    w: f64::from(x1 - x0),
                    h: f64::from(y1 - y0),
                })
                .collect();
            let at = (f64::from(point.0), f64::from(point.1));
            println!("{point:?}: inner {inner:?}");
            println!("   levels {:?}", levels(&inner, &windows, screen, at));
        }
    }
}
