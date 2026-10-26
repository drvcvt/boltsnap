//! The recording selector's toolbar on the app's design system: Record,
//! Clip, Frame and Audio in one strip, a 2×2 grid or one column.

use boltsnap::ui::icons::Icon;
use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, CTL_H, GAP_S, GAP_XS, ICON, RADIUS, Rgb, TextStyle};
use boltsnap::ui::{Canvas, Rect};

use super::chrome::{Placement, SIDE, place};

pub const LABELS: [&str; 4] = ["Record", "Clip", "Frame", "Audio"];
pub const RECORD: usize = 0;
pub const CLIP: usize = 1;
pub const FRAME: usize = 2;
pub const AUDIO: usize = 3;

/// Room between the toolbar's edge and its controls, and between controls.
const PAD: f32 = GAP_XS;
/// Concentric with the toolbar's corners.
pub const INNER_RADIUS: f32 = RADIUS - PAD;

/// Each control's width in the face in use, measured once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    widths: [f32; 4],
}

impl Metrics {
    pub fn new(fonts: &Fonts) -> Self {
        Self {
            widths: LABELS.map(|label| {
                (SIDE + ICON + GAP_S + fonts.width(TextStyle::Label, label) + SIDE).ceil()
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub columns: usize,
    pub placement: Placement,
    pub bounds: Rect,
    /// Each control's surface.
    pub controls: [Rect; 4],
    /// Each control's target: its surface plus half the gap to a neighbour
    /// and the padding to the toolbar's edge, so the toolbar has no dead zone.
    pub hits: [Rect; 4],
}

impl Layout {
    /// The control under `point`.
    pub fn hit(&self, (x, y): (f32, f32)) -> Option<usize> {
        self.hits.iter().position(|rect| rect.contains(x, y))
    }
}

/// The toolbar for `sel` inside `area`: one strip if it fits with `GAP_S`
/// around it, else a 2×2 grid, else a single column; `None` when not even
/// that fits or the selection is empty.
pub fn layout(sel: (f32, f32, f32, f32), area: Rect, metrics: &Metrics) -> Option<Layout> {
    let (sx, sy, sw, sh) = sel;
    if ![sx, sy, sw, sh].iter().all(|v| v.is_finite()) || sw < 1.0 || sh < 1.0 {
        return None;
    }
    let (columns, widths) = [4usize, 2, 1].into_iter().find_map(|columns| {
        let mut widths = [0.0f32; 4];
        for (i, w) in metrics.widths.iter().enumerate() {
            widths[i % columns] = widths[i % columns].max(*w);
        }
        let rows = 4 / columns;
        let w = widths[..columns].iter().sum::<f32>() + (columns - 1) as f32 * PAD + 2.0 * PAD;
        let h = rows as f32 * CTL_H + (rows - 1) as f32 * PAD + 2.0 * PAD;
        (w + 2.0 * GAP_S <= area.w && h + 2.0 * GAP_S <= area.h).then_some((columns, widths))
    })?;
    let rows = 4 / columns;
    let width = widths[..columns].iter().sum::<f32>() + (columns + 1) as f32 * PAD;
    let height = rows as f32 * CTL_H + (rows + 1) as f32 * PAD;
    let (x, y, placement) = place(sel, (width, height), area);
    let bounds = Rect::new(x, y, width, height);
    let column_x = |c: usize| x + PAD + widths[..c].iter().sum::<f32>() + c as f32 * PAD;
    let row_y = |r: usize| y + PAD + r as f32 * (CTL_H + PAD);
    let controls: [Rect; 4] = std::array::from_fn(|i| {
        let (c, r) = (i % columns, i / columns);
        Rect::new(column_x(c), row_y(r), widths[c], CTL_H)
    });
    let hits = std::array::from_fn(|i| {
        let (c, r) = (i % columns, i / columns);
        let rect = controls[i];
        let left = if c == 0 { x } else { rect.x - PAD / 2.0 };
        let right = if c + 1 == columns {
            bounds.right()
        } else {
            rect.right() + PAD / 2.0
        };
        let top = if r == 0 { y } else { rect.y - PAD / 2.0 };
        let bottom = if r + 1 == rows {
            bounds.bottom()
        } else {
            rect.bottom() + PAD / 2.0
        };
        Rect::new(left, top, right - left, bottom - top)
    });
    Some(Layout {
        columns,
        placement,
        bounds,
        controls,
        hits,
    })
}

/// How one control looks this frame, each 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlLook {
    pub hover: f32,
    pub press: f32,
    /// Frame and Audio: switched on.
    pub on: f32,
    /// Clip: a replay buffer is there to clip from.
    pub enabled: f32,
}

/// How the toolbar looks this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub controls: [ControlLook; 4],
}

impl Look {
    /// The look without motion: `hovered`, the toggles' states, Clip there.
    pub fn at_rest(clip: bool, frame: bool, audio: bool, hovered: Option<usize>) -> Self {
        let f = |b: bool| f32::from(u8::from(b));
        Self {
            controls: std::array::from_fn(|i| ControlLook {
                hover: f(hovered == Some(i)),
                press: 0.0,
                on: f((i == FRAME && frame) || (i == AUDIO && audio)),
                enabled: f(i != CLIP || clip),
            }),
        }
    }
}

/// The steps a control's surface climbs on the toolbar: none, then the
/// app's control, hover and chosen surfaces.
const LADDER: [Rgb; 4] = [theme::PANEL, theme::RAISE, theme::HOVER, theme::SELECTED];

/// The surface at `step` (0 to 3), between the ladder's colours.
pub fn ladder(step: f32) -> Rgb {
    let step = step.clamp(0.0, 3.0);
    let low = (step.floor() as usize).min(2);
    theme::mix(LADDER[low], LADDER[low + 1], step - low as f32)
}

/// A control's step: where it rests (Record on the control surface, a
/// toggle that is on two steps up, like a segment's chosen part over its
/// track), one more under the pointer, one more while pressed.
pub fn step(index: usize, look: &ControlLook) -> f32 {
    let rest = match index {
        RECORD => 1.0,
        CLIP => 0.0,
        _ => 2.0 * look.on,
    };
    rest + (look.hover + look.press) * look.enabled
}

/// The label's and icon's colour.
pub fn ink(index: usize, look: &ControlLook) -> Rgb {
    match index {
        RECORD => theme::TEXT,
        CLIP => theme::mix(theme::DIM, theme::TEXT, look.enabled),
        _ => theme::mix(theme::SUB, theme::TEXT, look.on.max(look.hover)),
    }
}

/// The icon box of a control: `SIDE` in, centred on the capitals' middle.
pub fn icon_rect(control: Rect) -> Rect {
    Rect::new(
        control.x + SIDE,
        control.y + (control.h - ICON) / 2.0,
        ICON,
        ICON,
    )
}

/// Where a control's label starts.
pub fn text_x(control: Rect) -> f32 {
    control.x + SIDE + ICON + GAP_S
}

/// The toolbar's surface, each control's surface, icon and label.
pub fn draw(canvas: &mut Canvas, fonts: &Fonts, layout: &Layout, look: &Look) {
    draw_labelled(canvas, fonts, layout, look, LABELS);
}

/// [`draw`] with other words, for measuring capitals.
pub fn draw_labelled(
    canvas: &mut Canvas,
    fonts: &Fonts,
    layout: &Layout,
    look: &Look,
    labels: [&str; 4],
) {
    canvas.fill(layout.bounds, RADIUS, theme::PANEL, 1.0);
    for (i, rect) in layout.controls.iter().enumerate() {
        let control = &look.controls[i];
        let step = step(i, control);
        if step > 0.0 {
            canvas.fill(*rect, INNER_RADIUS, ladder(step), 1.0);
        }
        let ink = ink(i, control);
        let icon = icon_rect(*rect);
        match i {
            // The one colour on the toolbar: the primary action's mark.
            RECORD => Icon::Circle.fill(canvas, icon, theme::ERROR, 1.0),
            CLIP => Icon::Scissors.draw(canvas, icon, ink, 1.0),
            FRAME => Icon::SquareDashed.draw(canvas, icon, ink, 1.0),
            _ => {
                // Speaker with waves crossfades into the crossed-out one.
                Icon::Volume2.draw(canvas, icon, ink, control.on);
                Icon::VolumeX.draw(canvas, icon, ink, 1.0 - control.on);
            }
        }
        let baseline = fonts.baseline(TextStyle::Label, rect.y, rect.h);
        fonts.draw(
            canvas,
            TextStyle::Label,
            labels[i],
            (text_x(*rect), baseline),
            ink,
            1.0,
        );
    }
}
