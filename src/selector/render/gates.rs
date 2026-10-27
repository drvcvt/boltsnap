//! The `/ui` gates over the real render of the toolbar: capitals on each
//! label's middle (gate 2), equal padding and the group gaps (gate 3), icon
//! ink on the capitals and the icon-only controls' icons on their middle,
//! one icon axis and one text edge, `GAP_S` from the widest icon (gate 8).
//! Shared by the unit tests (embedded face) and `selector_shots` (the user's).

use boltsnap::app::measure::Ink;
use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, Rgb};
use boltsnap::ui::{Canvas, Rect};

use super::toolbar::{
    self, CTL, FRAME, GROUP_GAP, ICON_TEXT, IN_GAP, Layout, Look, PAD, READOUT, RECORD, SIDE,
};

/// Capitals only: their top is the cap height, their bottom the baseline.
const CAPS: [&str; 2] = ["RECORD", "CLIP"];
/// Behind the toolbar, unlike any of its colours.
const BACKDROP: Rgb = (0x05, 0x05, 0x05);
/// The screen the size is reserved for, and the size shown: the widest.
pub const VIDEO: (u32, u32) = (3840, 2160);

type Screen = (u32, u32);
type Selection = (f32, f32, f32, f32);

/// A screen and a selection for each arrangement.
pub const SHOTS: [(Screen, Selection); 4] = [
    ((1920, 1080), (612.0, 300.0, 760.0, 430.0)),
    ((260, 300), (40.0, 160.0, 180.0, 100.0)),
    ((180, 300), (20.0, 200.0, 140.0, 80.0)),
    ((140, 240), (20.0, 180.0, 100.0, 40.0)),
];

/// The edges of what `fg` drew over `bg` inside `region`, read on the
/// channel where the two differ most, partly covered rows and columns
/// counted by their coverage (as `app::measure::ink_box`, which reads red
/// only and so cannot see the red dot on the light primary surface).
fn ink(canvas: &Canvas, region: Rect, bg: Rgb, fg: Rgb) -> Option<Ink> {
    let (bgs, fgs) = ([bg.0, bg.1, bg.2], [fg.0, fg.1, fg.2]);
    let c = (0..3)
        .max_by_key(|&c| (fgs[c] as i32 - bgs[c] as i32).abs())
        .unwrap_or(0);
    let span = fgs[c] as f32 - bgs[c] as f32;
    let (w, h) = (
        canvas.pixmap.width() as usize,
        canvas.pixmap.height() as usize,
    );
    let data = canvas.pixmap.data();
    let clampu = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
    let (x0, x1) = (
        clampu(region.x.floor(), w),
        clampu(region.right().ceil(), w),
    );
    let (y0, y1) = (
        clampu(region.y.floor(), h),
        clampu(region.bottom().ceil(), h),
    );
    let at = |x: usize, y: usize| {
        ((data[(y * w + x) * 4 + c] as f32 - bgs[c] as f32) / span).clamp(0.0, 1.0)
    };
    let edges = |cover: Vec<(usize, f32)>| -> Option<(f32, f32)> {
        let lit: Vec<_> = cover.into_iter().filter(|(_, v)| *v > 0.05).collect();
        let (&(first, a), &(last, b)) = (lit.first()?, lit.last()?);
        Some((first as f32 + (1.0 - a), last as f32 + b))
    };
    let rows = (y0..y1)
        .map(|y| (y, (x0..x1).map(|x| at(x, y)).fold(0.0, f32::max)))
        .collect();
    let cols = (x0..x1)
        .map(|x| (x, (y0..y1).map(|y| at(x, y)).fold(0.0, f32::max)))
        .collect();
    let (top, bottom) = edges(rows)?;
    let (left, right) = edges(cols)?;
    Some(Ink {
        top,
        bottom,
        left,
        right,
    })
}

pub struct Labelled {
    pub label: &'static str,
    /// Space above the capitals minus space below them.
    pub caps_off: f32,
    /// Icon ink middle minus capitals middle.
    pub icon_off: f32,
    pub icon: Ink,
    pub text_x: f32,
}

pub struct Report {
    pub layout: Layout,
    pub labelled: Vec<Labelled>,
    /// Frame and Audio: icon ink middle minus the control's middle, x and y.
    pub icon_only: Vec<(f32, f32)>,
    /// From the toolbar's left, top and bottom edge to the nearest surface.
    pub padding: [f32; 3],
    /// From the toolbar's right edge to the size's ink, and the same for the
    /// record dot's ink from the left edge: the two optical insets.
    pub size_inset: Option<(f32, f32)>,
}

/// Draw the toolbar for `sel` in a `w`×`h` area with Clip there, Frame and
/// Audio on (`on`) or off, the widest size, nothing under the pointer, and
/// measure it.
pub fn measure(fonts: &Fonts, size: (u32, u32), sel: (f32, f32, f32, f32), on: bool) -> Report {
    measure_with(fonts, size, sel, on, true)
}

/// [`measure`], the toolbar with Clip or, with replay off, without.
pub fn measure_with(
    fonts: &Fonts,
    (w, h): (u32, u32),
    sel: (f32, f32, f32, f32),
    on: bool,
    clip: bool,
) -> Report {
    let metrics = toolbar::Metrics::new(fonts).with_size(fonts, VIDEO);
    let metrics = if clip {
        metrics
    } else {
        metrics.without_clip()
    };
    let area = Rect::new(0.0, 0.0, w as f32, h as f32);
    let layout = toolbar::layout(sel, area, &metrics).expect("toolbar fits");
    let mut canvas = Canvas::new(w, h, 1).expect("canvas");
    canvas.clear(BACKDROP);
    let mut look = Look::at_rest(true, on, on, None);
    look.size = Some(VIDEO);
    toolbar::draw_labelled(&mut canvas, fonts, &layout, &look, CAPS);
    let mut labelled = Vec::new();
    let mut icon_only = Vec::new();
    for (i, rect) in layout.controls.iter().enumerate() {
        if !layout.shown[i] {
            continue;
        }
        let control = &look.controls[i];
        let surface = toolbar::surface(i, control).unwrap_or(theme::PANEL);
        let ink_rgb = toolbar::ink(i, control);
        let icon_rgb = if i == RECORD {
            toolbar::mark(control)
        } else {
            ink_rgb
        };
        let icon = ink(
            &canvas,
            toolbar::icon_rect(i, *rect).inset(-2.0),
            surface,
            icon_rgb,
        )
        .expect("icon ink");
        if i >= FRAME {
            icon_only.push((
                icon.middle_x() - (rect.x + rect.w / 2.0),
                icon.middle_y() - (rect.y + rect.h / 2.0),
            ));
            continue;
        }
        let text_x = rect.x + layout.text_offset;
        let text = Rect::new(text_x, rect.y, rect.right() - text_x, rect.h);
        let caps = ink(&canvas, text, surface, ink_rgb).expect("label ink");
        labelled.push(Labelled {
            label: toolbar::LABELS[i],
            caps_off: (caps.top - rect.y) - (rect.bottom() - caps.bottom),
            icon_off: icon.middle_y() - caps.middle_y(),
            icon,
            text_x,
        });
    }
    // Padding: every control lit, so each has a surface to measure.
    let mut lit = Canvas::new(w, h, 1).expect("canvas");
    lit.clear(BACKDROP);
    let mut all = look;
    for control in &mut all.controls {
        control.hover = 1.0;
    }
    toolbar::draw(&mut lit, fonts, &layout, &all);
    let surface = |i: usize| {
        let fill = toolbar::surface(i, &all.controls[i]).expect("lit surface");
        ink(&lit, layout.hits[i], theme::PANEL, fill).expect("surface")
    };
    let b = layout.bounds;
    let lowest = (0..4)
        .filter(|&i| layout.shown[i])
        .max_by(|&a, &c| layout.controls[a].y.total_cmp(&layout.controls[c].y))
        .unwrap_or(0);
    let (first, bottom) = (surface(0), surface(lowest));
    let size_inset = layout.size.map(|rect| {
        let text = ink(&canvas, rect, theme::PANEL, theme::SUB).expect("size ink");
        let dot = labelled[0].icon;
        (b.right() - text.right, dot.left - b.x)
    });
    Report {
        layout,
        labelled,
        icon_only,
        padding: [
            first.left - b.x,
            first.top - b.y,
            b.bottom() - bottom.bottom,
        ],
        size_inset,
    }
}

/// A control that is on the toolbar (the others have an empty rectangle).
fn shown_rect(r: &Rect) -> bool {
    r.w > 0.0
}

/// Gate 2, 3 and 8 over `report`: the worst values, or what fails.
pub fn check(report: &Report) -> Result<String, String> {
    let (mut caps, mut icons, mut alone) = (0.0f32, 0.0f32, 0.0f32);
    for c in &report.labelled {
        if c.caps_off.abs() > 0.5 {
            return Err(format!(
                "{}: capitals {:+.2} px off the middle",
                c.label, c.caps_off
            ));
        }
        if c.icon_off.abs() > 0.5 {
            return Err(format!(
                "{}: icon {:+.2} px off the capitals",
                c.label, c.icon_off
            ));
        }
        caps = caps.max(c.caps_off.abs());
        icons = icons.max(c.icon_off.abs());
    }
    for &(dx, dy) in &report.icon_only {
        if dx.abs() > 0.5 || dy.abs() > 0.5 {
            return Err(format!(
                "icon-only control's icon off its middle by {dx:+.2}, {dy:+.2}"
            ));
        }
        alone = alone.max(dx.abs()).max(dy.abs());
    }
    let pad = report
        .padding
        .iter()
        .map(|p| (p - PAD).abs())
        .fold(0.0, f32::max);
    if pad > 0.5 {
        return Err(format!("padding {:?} not {PAD} all round", report.padding));
    }
    let l = &report.layout;
    let (controls, b) = (l.controls, l.bounds);
    let shown = |i: usize| l.shown[i];
    // Icon-only controls are square in one row; in more they may be widened
    // to fill theirs, and every row then ends at the right padding.
    let one_row = controls
        .iter()
        .all(|r| !shown_rect(r) || r.y == controls[0].y);
    if (0..4).any(|i| shown(i) && controls[i].h != CTL)
        || controls[FRAME..]
            .iter()
            .any(|r| if one_row { r.w != CTL } else { r.w < CTL })
    {
        return Err("controls of different heights, or an icon-only one not square".into());
    }
    if !one_row {
        let ends = (0..4)
            .filter(|&i| shown(i))
            .map(|i| controls[i])
            .chain(l.size.map(|r| Rect::new(r.x, r.y, r.w + SIDE, r.h)));
        let mut rows: Vec<(f32, f32)> = Vec::new();
        for r in ends {
            match rows.iter_mut().find(|(y, _)| *y == r.y) {
                Some(row) => row.1 = row.1.max(r.right()),
                None => rows.push((r.y, r.right())),
            }
        }
        if let Some((y, right)) = rows.iter().find(|(_, right)| *right != b.right() - PAD) {
            return Err(format!(
                "the row at {y} ends at {right}, not at the padding"
            ));
        }
    }
    // Groups: GROUP_GAP between the first group's end (Clip, or Record
    // without it) and Frame when they share a row, PAD within a group; the
    // size `SIDE` + PAD from the right edge.
    let end = if shown(1) { 1 } else { 0 };
    if controls[end].y == controls[2].y && controls[2].x - controls[end].right() != GROUP_GAP {
        return Err(format!(
            "{} to Frame is not the group gap",
            toolbar::LABELS[end]
        ));
    }
    if controls[3].x - controls[2].right() != IN_GAP {
        return Err("Frame to Audio is not the inner gap".into());
    }
    if let Some(size) = l.size {
        // In one row the size ends the toolbar, `SIDE` from its padding as a
        // label ends; on two rows its row is the shorter one.
        let ends = l.arrangement == 0 && b.right() - size.right() != SIDE + PAD;
        if size.x - controls[3].right() != GROUP_GAP
            || ends
            || size.right() + SIDE + PAD > b.right()
        {
            return Err(format!("size at {size:?} in {b:?}"));
        }
    } else if b.right() - controls.iter().map(|r| r.right()).fold(0.0, f32::max) < PAD {
        return Err("a control runs into the right padding".into());
    }
    // Stacked labelled controls share an icon axis and a text edge; from the
    // widest labelled icon's ink to the text is one GAP_S.
    let spread = |v: &[f32]| {
        v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
    };
    let stacked = controls[0].x == controls[1].x;
    let axis = if stacked {
        spread(
            &report
                .labelled
                .iter()
                .map(|c| c.icon.middle_x())
                .collect::<Vec<_>>(),
        )
    } else {
        0.0
    };
    let edge = if stacked {
        spread(&report.labelled.iter().map(|c| c.text_x).collect::<Vec<_>>())
    } else {
        0.0
    };
    let widest = report
        .labelled
        .iter()
        .map(|c| c.icon.right - c.text_x)
        .fold(f32::MIN, f32::max);
    let gap = -widest - ICON_TEXT;
    if axis > 0.5 || edge > 0.5 || gap.abs() > 0.5 {
        return Err(format!(
            "icon axis spread {axis:.2}, text edge spread {edge:.2}, gap off ICON_TEXT {gap:+.2}"
        ));
    }
    Ok(format!(
        "arrangement {}: capitals worst {caps:.2} px, icon on capitals worst {icons:.2} px, \
         icon-only icons off their middle worst {alone:.2} px, padding {:?}, size inset \
         {}, icon axis spread {axis:.2}, text edge spread {edge:.2}, widest icon to text \
         off ICON_TEXT by {gap:+.2}",
        l.arrangement,
        report.padding,
        report
            .size_inset
            .map_or("(no size)".into(), |(r, d)| format!(
                "{r:.2} px right vs record dot {d:.2} px left ({READOUT:?})"
            )),
    ))
}
