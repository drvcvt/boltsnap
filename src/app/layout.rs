//! Where everything goes at a window size: the navigation column, the page's
//! column of groups, each row's parts. Rows put their control below the name
//! when both do not fit side by side (settings plan, Phase 4b). Crossing a
//! threshold is a state change: the navigation opens and closes and rows
//! stack on springs (Phase 4c).

use std::time::{Duration, Instant};

use super::{App, Block, Row};
use crate::ui::Rect;
use crate::ui::motion::Spring;
use crate::ui::theme::TextStyle;
use crate::ui::theme::{
    CONTENT_MAX, CTL_H, GAP_L, GAP_M, GAP_S, GAP_XL, GAP_XS, GLIDE, ICON, LABEL_MIN,
    NAV_NARROW_BELOW, NAV_NARROW_W, NAV_W, ROW_H,
};
use crate::ui::widgets::{self, Widget};

/// A text line: a group heading, a row's name above its hint.
pub const LINE: f32 = 16.0;
const HINT_LINE: f32 = 14.0;
/// A name or hint keeps to this many lines; more ends with an ellipsis.
const TEXT_LINES: usize = 3;
/// The line pitch of a name or hint that takes more than one line: capitals
/// 10 and 9 px, at least 4 px from a descender to the next line's capitals.
const NAME_PITCH: f32 = 18.0;
const HINT_PITCH: f32 = 16.0;

/// Where a row's parts go.
#[derive(Clone, Debug)]
pub struct RowParts {
    pub icon: Rect,
    /// The name's first line.
    pub label: Rect,
    /// The hint's first line.
    pub hint: Option<Rect>,
    pub control: Rect,
    /// Everything the name and hint may take up; their lines sit centred in it.
    pub text: Rect,
    pub names: Vec<String>,
    pub hints: Vec<String>,
    pub name_pitch: f32,
    pub hint_pitch: f32,
    /// The reset button, in the text's room so the control stays put.
    pub reset: Option<Rect>,
}

/// A row's name and hint, broken into lines for one width.
#[derive(Clone, Debug)]
struct RowText {
    names: Vec<String>,
    hints: Vec<String>,
}

impl RowText {
    fn name_pitch(&self) -> f32 {
        if self.names.len() > 1 {
            NAME_PITCH
        } else {
            LINE
        }
    }

    fn hint_pitch(&self) -> f32 {
        if self.hints.len() > 1 {
            HINT_PITCH
        } else {
            HINT_LINE
        }
    }

    fn height(&self) -> f32 {
        self.names.len() as f32 * self.name_pitch() + self.hints.len() as f32 * self.hint_pitch()
    }
}

/// One way to lay a row out: beside its control, or stacked over it.
struct Arrangement {
    text: Rect,
    control: Rect,
    reset: Option<Rect>,
    lines: RowText,
    height: f32,
}

/// The room a reset button takes from the text.
const RESET_ROOM: f32 = CTL_H + GAP_S;

impl RowParts {
    /// Where the reset button is, or would be: `GAP_S` left of the control
    /// beside it, at the right end of the name's line over a stacked one.
    /// Also for a button still fading out after its key went back.
    pub fn reset_place(&self) -> Rect {
        if self.control.y >= self.text.bottom() {
            Rect::new(self.control.right() - CTL_H, self.text.y, CTL_H, CTL_H)
        } else {
            Rect::new(self.control.x - GAP_S - CTL_H, self.control.y, CTL_H, CTL_H)
        }
    }
}

/// The page at the current size and scroll, in window coordinates.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub title: Rect,
    /// One per block: a heading's line, a banner, a row.
    pub blocks: Vec<Rect>,
    /// The surface behind each run of rows.
    pub panels: Vec<Rect>,
    /// From the top of the page to below its last block, unscrolled.
    pub height: f32,
}

/// Far enough ahead that every layout spring is at rest: the slowest,
/// `GLIDE`, settles within ten of its durations.
pub const AT_REST: Duration = Duration::from_secs(10);

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Between two rectangles, on whole pixels (see [`App::column`]).
fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect::new(
        lerp(from.x, to.x, t).round(),
        lerp(from.y, to.y, t).round(),
        lerp(from.w, to.w, t).round(),
        lerp(from.h, to.h, t).round(),
    )
}

/// To `to` on `GLIDE`, or at once.
fn move_to(spring: &mut Spring, to: f32, animate: bool, now: Instant) {
    if animate {
        spring.retarget(to, GLIDE, now);
    } else {
        spring.set(to, now);
    }
}

/// The row's inner line, `GAP_M` in from the group's sides.
fn inner(rect: Rect) -> Rect {
    Rect::new(rect.x + GAP_M, rect.y, rect.w - 2.0 * GAP_M, rect.h)
}

/// Where the name starts, after the row's icon.
fn text_x(inner: Rect) -> f32 {
    inner.x + ICON + GAP_S
}

/// The room for the control beside the name.
fn room_beside(inner: Rect) -> f32 {
    inner.right() - text_x(inner) - LABEL_MIN - GAP_M
}

/// The control's width beside the name, or `None` when it does not fit
/// there and the row stacks.
fn beside(row: &Row, fonts: &crate::ui::text::Fonts, inner: Rect) -> Option<f32> {
    let room = room_beside(inner);
    let width = row.control.width(fonts, room);
    (width <= room).then_some(width)
}

impl App {
    fn row_text(&self, row: &Row, width: f32) -> RowText {
        RowText {
            names: self
                .fonts
                .wrap(TextStyle::Label, &row.label, width, TEXT_LINES),
            hints: row.hint.as_deref().map_or_else(Vec::new, |hint| {
                self.fonts.wrap(TextStyle::Hint, hint, width, TEXT_LINES)
            }),
        }
    }

    /// The control on the right, the text in the room left of it; the row
    /// grows when the text takes more than `ROW_H` allows.
    fn side(&self, row: &Row, rect: Rect) -> Arrangement {
        let line = inner(rect);
        let x = text_x(line);
        let width = row.control.width(&self.fonts, room_beside(line));
        // A control too wide for this row still leaves the name `LABEL_MIN`
        // to wrap in: that arrangement only shows while the row stacks, and
        // a name broken letter by letter would make the row jump first.
        let room = (line.right() - width - GAP_M - x).max(LABEL_MIN);
        let text_w = if row.reset { room - RESET_ROOM } else { room };
        let lines = self.row_text(row, text_w);
        let height = ROW_H.max(lines.height() + 2.0 * GAP_S);
        let control = Rect::new(
            line.right() - width,
            rect.y + (height - CTL_H) / 2.0,
            width,
            CTL_H,
        );
        Arrangement {
            text: Rect::new(x, rect.y, text_w, height),
            reset: row
                .reset
                .then(|| Rect::new(control.x - GAP_S - CTL_H, control.y, CTL_H, CTL_H)),
            control,
            lines,
            height,
        }
    }

    /// The text over the whole width, the control under it.
    fn stacked(&self, row: &Row, rect: Rect) -> Arrangement {
        let line = inner(rect);
        let x = text_x(line);
        let full = line.right() - x;
        let text_w = if row.reset { full - RESET_ROOM } else { full };
        let lines = self.row_text(row, text_w);
        let text = Rect::new(x, rect.y + GAP_S, text_w, CTL_H.max(lines.height()));
        let width = if row.control.stretches() {
            full
        } else {
            row.control.width(&self.fonts, full).min(full)
        };
        Arrangement {
            control: Rect::new(line.right() - width, text.bottom() + GAP_S, width, CTL_H),
            // On the name's line, at the right end.
            reset: row
                .reset
                .then(|| Rect::new(line.right() - CTL_H, text.y, CTL_H, CTL_H)),
            height: text.h + CTL_H + 3.0 * GAP_S,
            text,
            lines,
        }
    }

    /// Where the navigation is going: icons only below `NAV_NARROW_BELOW`.
    pub fn narrow(&self) -> bool {
        self.size.0 < NAV_NARROW_BELOW
    }

    /// 0 with icons only, 1 with names.
    pub fn nav_open(&self, now: Instant) -> f32 {
        self.nav_open.value(now).clamp(0.0, 1.0)
    }

    pub fn nav_w(&self, now: Instant) -> f32 {
        lerp(NAV_NARROW_W, NAV_W, self.nav_open(now))
    }

    /// A navigation entry's icon, at one place in both states (settings
    /// plan, 4h): closed, the entry is a `CTL_H` square with the icon in its
    /// middle; open, the icon keeps that inset, the same as above and below
    /// it. Only the name comes and goes.
    pub fn nav_icon(&self, index: usize, now: Instant) -> Rect {
        let inset = (CTL_H - ICON) / 2.0;
        widgets::icon_box(self.nav_rect(index, now), GAP_S + inset, ICON)
    }

    /// Where navigation names start, `GAP_S` after the icon; a closing
    /// navigation fades and cuts them there.
    pub fn nav_text_x(&self) -> f32 {
        GAP_S + (CTL_H - ICON) / 2.0 + ICON + GAP_S
    }

    /// A navigation entry's surface. Entries follow each other with no gap
    /// in their hit areas ([`App::nav_slot`]); the surface sits inside.
    pub fn nav_rect(&self, index: usize, now: Instant) -> Rect {
        let slot = self.nav_slot(index, now);
        Rect::new(slot.x, slot.y + GAP_XS / 2.0, slot.w, CTL_H)
    }

    /// An entry's hit area. The first entry's surface is on the title's line.
    pub fn nav_slot(&self, index: usize, now: Instant) -> Rect {
        let pitch = CTL_H + GAP_XS;
        Rect::new(
            GAP_S,
            GAP_XL - GAP_XS / 2.0 + index as f32 * pitch,
            self.nav_w(now) - 2.0 * GAP_S,
            pitch,
        )
    }

    /// The page's column right of a navigation `nav_w` wide: centred, at most
    /// `CONTENT_MAX` wide, `GAP_XL` from the edges at least.
    fn column_beside(&self, nav_w: f32) -> (f32, f32) {
        let area = self.size.0 - nav_w;
        let w = (area - 2.0 * GAP_XL).clamp(0.0, CONTENT_MAX);
        (nav_w + (area - w) / 2.0, w)
    }

    /// On whole pixels, like everything that moves the page: text sits on
    /// whole pixels, and what carries it must move in the same steps.
    pub fn column(&self, now: Instant) -> (f32, f32) {
        let (x, w) = self.column_beside(self.nav_w(now));
        (x.round(), w.round())
    }

    /// Point the layout's springs at the window's size: the navigation opens
    /// or closes, each row sits beside its name or stacks. A row new to the
    /// springs (a page just shown) starts where it belongs.
    pub(super) fn reshape(&mut self, now: Instant) {
        let animate = !self.reduced_motion;
        let open = if self.narrow() { 0.0 } else { 1.0 };
        move_to(&mut self.nav_open, open, animate, now);
        let nav_w = if self.narrow() { NAV_NARROW_W } else { NAV_W };
        let line = inner(Rect::new(0.0, 0.0, self.column_beside(nav_w).1, ROW_H));
        let stacked: Vec<f32> = self.pages[self.current]
            .blocks
            .iter()
            .map(|block| match block {
                Block::Row(row) if beside(row, &self.fonts, line).is_none() => 1.0,
                _ => 0.0,
            })
            .collect();
        for (index, to) in stacked.into_iter().enumerate() {
            match self.stacked.get_mut(index) {
                Some(spring) => move_to(spring, to, animate, now),
                None => self.stacked.push(Spring::settled(to, now)),
            }
        }
    }

    /// The layout `scroll` pixels down the page.
    pub fn layout_at(&self, scroll: f32, now: Instant) -> Layout {
        let (x, w) = self.column(now);
        // Scroll glides in fractions; the page moves in whole pixels so its
        // text never shifts against its surfaces.
        let top = (-scroll).round();
        let mut y = GAP_XL;
        let title = Rect::new(x, top + y, w, CTL_H);
        y += CTL_H + GAP_L;
        let mut layout = Layout {
            title,
            ..Layout::default()
        };
        let mut panel: Option<f32> = None;
        let mut first = true;
        let blocks = &self.page().blocks;
        for (index, block) in blocks.iter().enumerate() {
            let in_panel = matches!(block, Block::Row(_));
            if !in_panel && let Some(start) = panel.take() {
                y += GAP_XS;
                layout.panels.push(Rect::new(x, top + start, w, y - start));
            }
            if !first && panel.is_none() {
                // A heading sits close to its group; everything else is a
                // group apart.
                let after_heading = matches!(blocks[index - 1], Block::Heading(_));
                y += if after_heading { GAP_S } else { GAP_XL };
            }
            let h = match block {
                Block::Heading(_) => LINE,
                Block::Banner { text, .. } => widgets::banner_height(&self.fonts, w, text),
                Block::Row(row) => {
                    if panel.is_none() {
                        panel = Some(y);
                        y += GAP_XS;
                    }
                    let t = self.stacked.get(index).map_or(0.0, |s| s.value(now));
                    let probe = Rect::new(x, 0.0, w, 0.0);
                    let (side, stacked) = (self.side(row, probe), self.stacked(row, probe));
                    lerp(side.height, stacked.height, t.clamp(0.0, 1.0)).round()
                }
            };
            layout.blocks.push(Rect::new(x, top + y, w, h));
            y += h;
            first = false;
        }
        if let Some(start) = panel {
            y += GAP_XS;
            layout.panels.push(Rect::new(x, top + start, w, y - start));
        }
        layout.height = y + GAP_XL;
        layout
    }

    pub fn layout(&self, now: Instant) -> Layout {
        self.layout_at(self.scroll.offset(now), now)
    }

    pub fn block_rects(&self, now: Instant) -> Vec<Rect> {
        self.layout(now).blocks
    }

    /// A row's parts. How far it has stacked is in its height, which the
    /// layout mixes between its two arrangements' heights from the row's
    /// spring.
    pub fn row_parts(&self, rect: Rect, row: &Row) -> RowParts {
        let line = inner(rect);
        let (side, stacked) = (self.side(row, rect), self.stacked(row, rect));
        let span = stacked.height - side.height;
        let t = if span.abs() < 0.5 {
            // Both as tall: which one the row is in, it says itself.
            if beside(row, &self.fonts, line).is_some() {
                0.0
            } else {
                1.0
            }
        } else {
            ((rect.h - side.height) / span).clamp(0.0, 1.0)
        };
        let (text, control) = if t == 0.0 {
            (side.text, side.control)
        } else if t == 1.0 {
            (stacked.text, stacked.control)
        } else {
            (
                lerp_rect(side.text, stacked.text, t),
                lerp_rect(side.control, stacked.control, t),
            )
        };
        let reset = match (side.reset, stacked.reset) {
            (Some(a), Some(b)) if t > 0.0 && t < 1.0 => Some(lerp_rect(a, b, t)),
            (a, b) => {
                if t < 0.5 {
                    a
                } else {
                    b
                }
            }
        };
        let lines = if t < 0.5 { side.lines } else { stacked.lines };
        let (name_pitch, hint_pitch) = (lines.name_pitch(), lines.hint_pitch());
        let top = (text.y + (text.h - lines.height()) / 2.0).round();
        let label = Rect::new(text.x, top, text.w, name_pitch);
        let hint = (!lines.hints.is_empty()).then(|| {
            Rect::new(
                text.x,
                top + lines.names.len() as f32 * name_pitch,
                text.w,
                hint_pitch,
            )
        });
        RowParts {
            icon: widgets::icon_box(label, line.x, ICON),
            label,
            hint,
            control,
            text,
            names: lines.names,
            hints: lines.hints,
            name_pitch,
            hint_pitch,
            reset,
        }
    }

    /// Where lists may open: the page area, off the navigation.
    pub(super) fn bounds(&self, now: Instant) -> Rect {
        let x = self.nav_w(now);
        Rect::new(x, 0.0, self.size.0 - x, self.size.1).inset(GAP_S)
    }

    /// The open list, if a control has one: (block, list rect).
    pub(super) fn open_overlay(&self, now: Instant) -> Option<(usize, Rect)> {
        self.overlay_where(now, |control| control.overlay_open())
    }

    /// The list being drawn, also while it fades out.
    pub(super) fn visible_overlay(&self, now: Instant) -> Option<(usize, Rect)> {
        self.overlay_where(now, |_| true)
    }

    fn overlay_where(
        &self,
        now: Instant,
        wanted: impl Fn(&dyn Widget) -> bool,
    ) -> Option<(usize, Rect)> {
        let rects = self.block_rects(now);
        self.page()
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match block {
                Block::Row(row) if wanted(row.control.as_ref()) => {
                    let anchor = self.row_parts(rects[index], row).control;
                    let list = row
                        .control
                        .overlay(anchor, self.bounds(now), &self.fonts, now)?;
                    Some((index, list))
                }
                _ => None,
            })
    }
}
