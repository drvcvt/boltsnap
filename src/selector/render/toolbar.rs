//! The recording selector's toolbar (variant A, compact K2): [Record, Clip]
//! · [Frame, Audio] · the size of the clip, one row if it fits, else two or
//! three. Every measure is a formula of the theme's tokens, below.

use boltsnap::ui::icons::Icon;
use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, GAP_S, GAP_XS, Rgb, STROKE, TextStyle};
use boltsnap::ui::{Canvas, Rect};

use super::chrome::{Placement, place, size_text};

pub const LABELS: [&str; 4] = ["Record", "Clip", "Frame", "Audio"];
pub const RECORD: usize = 0;
pub const CLIP: usize = 1;
pub const FRAME: usize = 2;
pub const AUDIO: usize = 3;

/// Every word on the toolbar, its size and the lines beside it: one step,
/// the settings rows' (capitals 10).
pub const TEXT: TextStyle = TextStyle::Label;
/// The size's style (the same step).
pub const READOUT: TextStyle = TEXT;
/// A control's height: the capitals plus `GAP_S` above and below, so the
/// side room `(CTL − cap) / 2` is `GAP_S` too (as much beside the text as
/// above it) and `CTL − cap` is even, the capitals exactly in the middle.
pub const CTL: f32 = TEXT.cap() + 2.0 * GAP_S;
/// Room beside a label, and before the toolbar's end after the size.
pub const SIDE: f32 = (CTL - TEXT.cap()) / 2.0;
/// The toolbar's rim: half a spacing step, so the toolbar hugs its controls;
/// it is the group's outline, not a surface of its own.
pub const PAD: f32 = GAP_XS / 2.0;
/// Between the controls of one group: the rim, so every inner gap is the
/// same as the edge they are set in.
pub const IN_GAP: f32 = PAD;
/// Between groups: the inner gap plus one spacing step, so a group's end
/// reads as exactly one step more.
pub const GROUP_GAP: f32 = IN_GAP + GAP_XS;
/// A control's corners: its side room minus the rim, so a control's corner
/// sits inside the toolbar's as its edge sits inside the toolbar's edge;
/// whole pixels at any text step (a quarter of `CTL` would be 6.5 here).
pub const CTL_RADIUS: f32 = SIDE - PAD;
/// The toolbar's corners, concentric with its controls': inner = outer −
/// rim, both through the same squircle mapping.
pub const BAR_RADIUS: f32 = CTL_RADIUS + PAD;
/// The toolbar's height in one row, and the lines' beside it.
pub const BAR_H: f32 = CTL + 2.0 * PAD;
/// An icon's box: the theme's icon beside `Label` text, exactly as the
/// settings rows pair them (never larger than the text's nominal size).
pub const ICON_BOX: f32 = theme::ICON;
/// From the widest labelled icon's ink to its label: one spacing step.
pub const ICON_TEXT: f32 = GAP_S;

/// The labelled controls' icons and whether they are filled.
const LABELLED_ICONS: [(Icon, bool); 2] = [(Icon::Circle, true), (Icon::Scissors, false)];

/// Right edge of an icon's ink in its `ICON_BOX`: its outline's box, plus
/// half the stroke for a stroked icon.
fn ink_right(icon: Icon, filled: bool) -> f32 {
    let right = icon.path().map_or(24.0, |path| path.bounds().right());
    right * ICON_BOX / 24.0 + if filled { 0.0 } else { STROKE / 2.0 }
}

/// What a row of the toolbar holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Control(usize),
    Size,
}

impl Item {
    /// [Record, Clip] · [Frame, Audio] · size.
    fn group(self) -> u8 {
        match self {
            Item::Control(i) if i < FRAME => 0,
            Item::Control(_) => 1,
            Item::Size => 2,
        }
    }
}

use Item::{Control as C, Size as S};

/// The arrangements, the first that fits wins: one row; one row without
/// the size (it goes first); two rows; three rows without the size. Two
/// rows without the size would be as wide as two rows with it, so it is
/// not one.
const ARRANGEMENTS: [&[&[Item]]; 4] = [
    &[&[C(0), C(1), C(2), C(3), S]],
    &[&[C(0), C(1), C(2), C(3)]],
    &[&[C(0), C(1)], &[C(2), C(3), S]],
    &[&[C(0)], &[C(1)], &[C(2), C(3)]],
];

/// The gap between two neighbours, across or down.
fn gap(a: Item, b: Item) -> f32 {
    if a.group() == b.group() {
        IN_GAP
    } else {
        GROUP_GAP
    }
}

/// Each control's width in the face in use, measured once, and the room
/// the size takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    widths: [f32; 4],
    /// From a labelled control's left edge to its label: `SIDE`, then the
    /// widest labelled icon's ink rounded to a whole pixel, then `GAP_S`.
    text_offset: f32,
    /// The widest size the screen allows, digits in cells; `None` hides it.
    size: Option<f32>,
}

impl Metrics {
    pub fn new(fonts: &Fonts) -> Self {
        let ink = LABELLED_ICONS
            .iter()
            .map(|&(icon, filled)| ink_right(icon, filled))
            .fold(0.0, f32::max);
        let text_offset = SIDE + ink.round() + ICON_TEXT;
        let labelled = |label| (text_offset + fonts.width(TEXT, label) + SIDE).ceil();
        Self {
            widths: [labelled(LABELS[0]), labelled(LABELS[1]), CTL, CTL],
            text_offset,
            size: None,
        }
    }

    /// Room for the size of a clip of a `video`-sized screen: the widest any
    /// width and height up to the screen's are set in (only the values with
    /// the most digits can be widest), so the toolbar keeps its width.
    pub fn with_size(mut self, fonts: &Fonts, video: (u32, u32)) -> Self {
        let widest = |most: u32| {
            let most = most.max(1);
            let from = 10u32.pow(most.ilog10());
            (from..=most)
                .map(|n| fonts.width(READOUT, &n.to_string()))
                .fold(0.0, f32::max)
        };
        let sign = fonts.width(READOUT, "\u{a0}×\u{a0}");
        self.size = Some((widest(video.0) + sign + widest(video.1)).ceil());
        self
    }

    fn width(&self, item: Item) -> Option<f32> {
        match item {
            Item::Control(i) => Some(self.widths[i]),
            // The text, then `SIDE` before the edge, as a label ends.
            Item::Size => self.size.map(|w| w + SIDE),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Which of the arrangements, 0 for one row with the size.
    pub arrangement: usize,
    pub placement: Placement,
    pub bounds: Rect,
    /// Each control's surface.
    pub controls: [Rect; 4],
    /// Each control's target: its surface plus half the gap to a neighbour,
    /// out to the toolbar's edge where it has none, so no control has a dead
    /// zone around it. The size is no control and takes no input.
    pub hits: [Rect; 4],
    /// Where the size's text goes, its reserved width; none if it is left out.
    pub size: Option<Rect>,
    /// From a labelled control's left edge to its label.
    pub text_offset: f32,
}

impl Layout {
    /// The same toolbar `dx`, `dy` further.
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        let shift = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        Self {
            bounds: shift(self.bounds),
            controls: self.controls.map(shift),
            hits: self.hits.map(shift),
            size: self.size.map(shift),
            ..*self
        }
    }

    /// The control under `point`.
    pub fn hit(&self, (x, y): (f32, f32)) -> Option<usize> {
        self.hits.iter().position(|rect| rect.contains(x, y))
    }
}

/// The rows of an arrangement with what of it there is (the size only when
/// the screen size is known).
fn rows(arrangement: usize, metrics: &Metrics) -> Vec<Vec<(Item, f32)>> {
    ARRANGEMENTS[arrangement]
        .iter()
        .map(|row| {
            row.iter()
                .filter_map(|&item| Some((item, metrics.width(item)?)))
                .collect()
        })
        .collect()
}

/// Width and height of `rows` with `PAD` around them.
fn extent(rows: &[Vec<(Item, f32)>]) -> (f32, f32) {
    let width = rows
        .iter()
        .map(|row| {
            let gaps: f32 = row.windows(2).map(|p| gap(p[0].0, p[1].0)).sum();
            row.iter().map(|(_, w)| w).sum::<f32>() + gaps
        })
        .fold(0.0, f32::max);
    let gaps: f32 = rows.windows(2).map(|p| gap(p[0][0].0, p[1][0].0)).sum();
    (
        width + 2.0 * PAD,
        rows.len() as f32 * CTL + gaps + 2.0 * PAD,
    )
}

/// The toolbar for `sel` inside `area`, `GAP_S` from its edges; `None` when
/// not even three rows fit or the selection is empty.
pub fn layout(sel: (f32, f32, f32, f32), area: Rect, metrics: &Metrics) -> Option<Layout> {
    let (sx, sy, sw, sh) = sel;
    if ![sx, sy, sw, sh].iter().all(|v| v.is_finite()) || sw < 1.0 || sh < 1.0 {
        return None;
    }
    let (arrangement, rows, (width, height)) = (0..ARRANGEMENTS.len()).find_map(|a| {
        let rows = rows(a, metrics);
        let (w, h) = extent(&rows);
        (w + 2.0 * GAP_S <= area.w && h + 2.0 * GAP_S <= area.h).then_some((a, rows, (w, h)))
    })?;
    let (x, y, placement) = place(sel, (width, height), area);
    let bounds = Rect::new(x, y, width, height);
    let mut controls = [Rect::default(); 4];
    let mut hits = [Rect::default(); 4];
    let mut size = None;
    let mut top = y + PAD;
    for (r, row) in rows.iter().enumerate() {
        let above = (r > 0).then(|| gap(rows[r - 1][0].0, row[0].0));
        let below = rows.get(r + 1).map(|next| gap(row[0].0, next[0].0));
        let mut left = x + PAD;
        for (k, &(item, w)) in row.iter().enumerate() {
            let rect = Rect::new(left, top, w, CTL);
            let after = row.get(k + 1).map(|&(next, _)| gap(item, next));
            match item {
                Item::Control(i) => {
                    let before = (k > 0).then(|| gap(row[k - 1].0, item));
                    let l = before.map_or(bounds.x, |g| rect.x - g / 2.0);
                    let rt = after.map_or(bounds.right(), |g| rect.right() + g / 2.0);
                    let t = above.map_or(bounds.y, |g| rect.y - g / 2.0);
                    let b = below.map_or(bounds.bottom(), |g| rect.bottom() + g / 2.0);
                    controls[i] = rect;
                    hits[i] = Rect::new(l, t, rt - l, b - t);
                }
                Item::Size => size = Some(Rect::new(rect.x, rect.y, w - SIDE, CTL)),
            }
            left += w + after.unwrap_or(0.0);
        }
        top += CTL + below.unwrap_or(0.0);
    }
    Some(Layout {
        arrangement,
        placement,
        bounds,
        controls,
        hits,
        size,
        text_offset: metrics.text_offset,
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
    /// The clip's size in pixels, if the selection makes one.
    pub size: Option<(u32, u32)>,
}

impl Look {
    /// The look without motion: `hovered`, the toggles' states, Clip there.
    #[allow(dead_code)] // Tests and the selector bench.
    pub fn at_rest(clip: bool, frame: bool, audio: bool, hovered: Option<usize>) -> Self {
        let f = |b: bool| f32::from(u8::from(b));
        Self {
            controls: std::array::from_fn(|i| ControlLook {
                hover: f(hovered == Some(i)),
                press: 0.0,
                on: f((i == FRAME && frame) || (i == AUDIO && audio)),
                enabled: f(i != CLIP || clip),
            }),
            size: None,
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

/// A control's step on the ladder: Record rests on the control surface
/// (the only resting surface, with the only colour: the one primary), Clip
/// on none, a toggle that is on two steps up (like a segment's chosen part
/// over its track); one more under the pointer, one more while pressed.
pub fn step(index: usize, look: &ControlLook) -> f32 {
    let rest = match index {
        RECORD => 1.0,
        CLIP => 0.0,
        _ => 2.0 * look.on,
    };
    rest + (look.hover + look.press) * look.enabled
}

/// A control's surface, `None` where there is none.
pub fn surface(index: usize, look: &ControlLook) -> Option<Rgb> {
    let step = step(index, look);
    (step > 0.0).then(|| ladder(step))
}

/// The label's and icon's colour.
pub fn ink(index: usize, look: &ControlLook) -> Rgb {
    match index {
        RECORD => theme::TEXT,
        CLIP => theme::mix(theme::DIM, theme::TEXT, look.enabled),
        _ => theme::mix(theme::SUB, theme::TEXT, look.on.max(look.hover)),
    }
}

/// A control's icon box: `SIDE` in for a labelled one, centred in an
/// icon-only one; on whole pixels either way.
pub fn icon_rect(index: usize, control: Rect) -> Rect {
    let y = control.y + (control.h - ICON_BOX) / 2.0;
    if index >= FRAME {
        Rect::new(
            control.x + (control.w - ICON_BOX) / 2.0,
            y,
            ICON_BOX,
            ICON_BOX,
        )
    } else {
        Rect::new(control.x + SIDE, y, ICON_BOX, ICON_BOX)
    }
}

/// The name an icon-only control shows under the pointer, with its state.
pub fn name(index: usize, on: bool) -> Option<&'static str> {
    match (index, on) {
        (FRAME, true) => Some("Frame on"),
        (FRAME, false) => Some("Frame off"),
        (AUDIO, true) => Some("Audio on"),
        (AUDIO, false) => Some("Audio off"),
        _ => None,
    }
}

/// The toolbar's surface, each control's surface, icon and label, the size.
pub fn draw(canvas: &mut Canvas, fonts: &Fonts, layout: &Layout, look: &Look) {
    draw_labelled(canvas, fonts, layout, look, [LABELS[0], LABELS[1]]);
}

/// [`draw`] with other words on the labelled controls, for measuring.
pub fn draw_labelled(
    canvas: &mut Canvas,
    fonts: &Fonts,
    layout: &Layout,
    look: &Look,
    labels: [&str; 2],
) {
    canvas.fill(layout.bounds, BAR_RADIUS, theme::PANEL, 1.0);
    for (i, rect) in layout.controls.iter().enumerate() {
        let control = &look.controls[i];
        if let Some(fill) = surface(i, control) {
            canvas.fill(*rect, CTL_RADIUS, fill, 1.0);
        }
        let ink = ink(i, control);
        let icon = icon_rect(i, *rect);
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
        if i < FRAME {
            let baseline = fonts.baseline(TEXT, rect.y, rect.h);
            let at = (rect.x + layout.text_offset, baseline);
            fonts.draw(canvas, TEXT, labels[i], at, ink, 1.0);
        }
    }
    if let (Some(rect), Some(size)) = (layout.size, look.size) {
        let baseline = fonts.baseline(READOUT, rect.y, rect.h);
        // Drawn as the face sets it: Outfit's digits are proportional and
        // ab_glyph applies no `tnum`, so equal cells would gape around a
        // "1". The room is reserved for the widest (see `with_size`), so the
        // toolbar never changes width; only the end of the text moves.
        let at = (rect.x, baseline);
        fonts.draw(canvas, READOUT, &size_text(size), at, theme::SUB, 1.0);
    }
}
