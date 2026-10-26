//! The `/ui` gates over the real render of the toolbar: capitals on each
//! control's middle (gate 2), equal padding (gate 3), icon ink on the
//! capitals, one icon axis and one text edge per column (gate 8). Shared by
//! the unit tests (embedded face) and `selector_shots` (the user's face).

use boltsnap::app::measure::{Ink, ink_box};
use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, GAP_S, Rgb};
use boltsnap::ui::{Canvas, Rect};

use super::toolbar::{self, LABELS, Layout, Look, RECORD};

/// Capitals only: their top is the cap height, their bottom the baseline.
const CAPS: [&str; 4] = ["RECORD", "CLIP", "FRAME", "AUDIO"];
/// Behind the toolbar, unlike any of its colours.
const BACKDROP: Rgb = (0x05, 0x05, 0x05);

pub struct Control {
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
    pub controls: Vec<Control>,
    /// From the toolbar's left, top, bottom and right edge to the nearest
    /// control surface (on the pixels).
    pub padding: [f32; 4],
}

/// Draw the toolbar for `sel` in a `w`×`h` area with Clip there, Frame and
/// Audio on (`on`) or off, nothing under the pointer, and measure it.
pub fn measure(fonts: &Fonts, (w, h): (u32, u32), sel: (f32, f32, f32, f32), on: bool) -> Report {
    let metrics = toolbar::Metrics::new(fonts);
    let area = Rect::new(0.0, 0.0, w as f32, h as f32);
    let layout = toolbar::layout(sel, area, &metrics).expect("toolbar fits");
    let mut canvas = Canvas::new(w, h, 1).expect("canvas");
    canvas.clear(BACKDROP);
    let look = Look::at_rest(true, on, on, None);
    toolbar::draw_labelled(&mut canvas, fonts, &layout, &look, CAPS);
    let mut controls = Vec::new();
    for (i, rect) in layout.controls.iter().enumerate() {
        let control = &look.controls[i];
        let surface = toolbar::ladder(toolbar::step(i, control));
        let ink = toolbar::ink(i, control);
        let text_x = toolbar::text_x(*rect);
        let text = Rect::new(text_x, rect.y, rect.right() - text_x, rect.h);
        let caps = ink_box(&canvas, text, surface, ink).expect("label ink");
        let icon_ink = if i == RECORD { theme::ERROR } else { ink };
        let icon = ink_box(
            &canvas,
            toolbar::icon_rect(*rect).inset(-2.0),
            surface,
            icon_ink,
        )
        .expect("icon ink");
        controls.push(Control {
            label: LABELS[i],
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
        let fill = toolbar::ladder(toolbar::step(i, &all.controls[i]));
        ink_box(&lit, layout.hits[i], theme::PANEL, fill).expect("surface")
    };
    let (first, last, bottom) = (
        surface(0),
        surface(layout.columns - 1),
        surface(4 - layout.columns),
    );
    let b = layout.bounds;
    Report {
        layout,
        controls,
        padding: [
            first.left - b.x,
            first.top - b.y,
            b.bottom() - bottom.bottom,
            b.right() - last.right,
        ],
    }
}

/// Gate 2, 3 and 8 over `report`: the worst values, or what fails. `gap`
/// checks the room from the widest icon to the text, which needs Audio on:
/// its speaker with waves is the widest icon the toolbar has; the crossed
/// one is narrower and so has more room (the text edge does not move).
pub fn check(report: &Report, gap_check: bool) -> Result<String, String> {
    let mut caps = 0.0f32;
    let mut icons = 0.0f32;
    for c in &report.controls {
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
    let pad = report
        .padding
        .iter()
        .map(|p| (p - theme::GAP_XS).abs())
        .fold(0.0, f32::max);
    if pad > 0.5 {
        return Err(format!(
            "padding {:?} not {} all round",
            report.padding,
            theme::GAP_XS
        ));
    }
    if report.layout.controls.iter().any(|r| r.h != theme::CTL_H) {
        return Err("controls of different heights".into());
    }
    // Per column: one icon axis, one text edge.
    let columns = report.layout.columns;
    let (mut axis, mut edge, mut gap) = (0.0f32, 0.0f32, 0.0f32);
    for c in 0..columns {
        let group: Vec<&Control> = report.controls.iter().skip(c).step_by(columns).collect();
        let spread = |v: &mut dyn Iterator<Item = f32>| {
            let v: Vec<f32> = v.collect();
            v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
        };
        axis = axis.max(spread(&mut group.iter().map(|g| g.icon.middle_x())));
        edge = edge.max(spread(&mut group.iter().map(|g| g.text_x)));
    }
    // The toolbar's controls are one role: from the widest icon's ink to
    // the text is one GAP_S, a narrower icon has more room, never less.
    let widest = report
        .controls
        .iter()
        .map(|c| c.icon.right - c.text_x)
        .fold(f32::MIN, f32::max);
    if gap_check {
        gap = gap.max((-widest - GAP_S).abs());
    }
    if axis > 0.5 || edge > 0.5 || gap > 0.5 {
        return Err(format!(
            "icon axis spread {axis:.2}, text edge spread {edge:.2}, gap off GAP_S {gap:.2}"
        ));
    }
    Ok(format!(
        "{} columns: capitals worst {caps:.2} px, icon on capitals worst {icons:.2} px, \
         padding {:?}, icon axis spread {axis:.2}, text edge spread {edge:.2}, \
         widest icon to text off GAP_S by {}",
        columns,
        report.padding,
        if gap_check {
            format!("{gap:.2}")
        } else {
            "(not measured, Audio off)".into()
        }
    ))
}
