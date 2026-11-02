//! Where the toolbar's controls go: the arrangements, each control's width
//! in the face in use, and the layout for a selection.

use boltsnap::ui::Rect;
use boltsnap::ui::icons::Icon;
use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{GAP_S, STROKE};

use super::super::chrome::{Placement, place};
use super::{
    CLIP, CTL, FRAME, GROUP_GAP, ICON_BOX, ICON_TEXT, IN_GAP, LABELS, PAD, READOUT, SIDE, TEXT,
};

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
    /// Clip is on the toolbar: not when no replay buffer runs.
    clip: bool,
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
            clip: true,
        }
    }

    /// The toolbar without Clip: replay is off, so there is nothing to clip
    /// from and no reason to offer it (selector plan, Nachtrag 27.10.).
    pub fn without_clip(mut self) -> Self {
        self.clip = false;
        self
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
            Item::Control(CLIP) if !self.clip => None,
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
    /// Which controls are on the toolbar; the others have empty rectangles
    /// and take no input.
    pub shown: [bool; 4],
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
/// the screen size is known, Clip only with a replay buffer); a row left
/// empty goes.
fn rows(arrangement: usize, metrics: &Metrics) -> Vec<Vec<(Item, f32)>> {
    ARRANGEMENTS[arrangement]
        .iter()
        .map(|row| {
            row.iter()
                .filter_map(|&item| Some((item, metrics.width(item)?)))
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
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

/// With more than one row, every row ends at the right padding, so the
/// toolbar keeps one edge all round: the size takes a row's rest where it
/// stands (its text stays at its start), else the row's controls share it in
/// whole pixels, the last taking a pixel left over.
fn justify(mut rows: Vec<Vec<(Item, f32)>>, inner: f32) -> Vec<Vec<(Item, f32)>> {
    if rows.len() < 2 {
        return rows;
    }
    for row in &mut rows {
        let gaps: f32 = row.windows(2).map(|p| gap(p[0].0, p[1].0)).sum();
        let rest = inner - gaps - row.iter().map(|(_, w)| w).sum::<f32>();
        if rest <= 0.0 {
            continue;
        }
        if let Some(size) = row.iter_mut().find(|(item, _)| *item == Item::Size) {
            size.1 += rest;
            continue;
        }
        let count = row.len() as f32;
        let share = (rest / count).floor();
        for (_, w) in row.iter_mut() {
            *w += share;
        }
        if let Some((_, w)) = row.last_mut() {
            *w += rest - share * count;
        }
    }
    rows
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
    let rows = justify(rows, width - 2.0 * PAD);
    let (x, y, placement) = place(sel, (width, height), area);
    let bounds = Rect::new(x, y, width, height);
    let mut controls = [Rect::default(); 4];
    let mut shown = [false; 4];
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
                    shown[i] = true;
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
        shown,
        hits,
        size,
        text_offset: metrics.text_offset,
    })
}
