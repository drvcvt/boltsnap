//! Measuring what was drawn, for the `/ui` gates: how far the capitals of
//! every control's text sit from its vertical middle, and where each row's
//! icon ink sits against its name (gate 8). Lives outside the tests so the
//! real UI font can be measured too (`boltsnap settings --measure`).

use std::time::Instant;

use super::{App, Bind, Block, Page, Row};
use crate::ui::icons::Icon;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, Rgb};
use crate::ui::widgets::{Dropdown, Part, Segment, TextField, Widget};
use crate::ui::{Canvas, Rect};

/// Capitals only: their top is the cap height, their bottom the baseline.
const CAPS: &str = "HHHH";
const SCALE: u32 = 2;

/// Top and bottom of the `TEXT` ink over `background` inside `region`, in
/// logical pixels, with partly covered rows counted by their coverage.
pub fn ink(canvas: &Canvas, region: Rect, background: Rgb) -> Option<(f32, f32)> {
    let scale = canvas.scale;
    let width = canvas.pixmap.width() as usize;
    let data = canvas.pixmap.data();
    let span = theme::TEXT.0 as f32 - background.0 as f32;
    let (x0, x1) = (
        (region.x * scale).floor() as usize,
        (region.right() * scale).ceil() as usize,
    );
    let (y0, y1) = (
        (region.y * scale).floor() as usize,
        (region.bottom() * scale).ceil() as usize,
    );
    let coverage = |y: usize| -> f32 {
        (x0..x1.min(width))
            .map(|x| (data[(y * width + x) * 4] as f32 - background.0 as f32) / span)
            .fold(0.0, f32::max)
            .clamp(0.0, 1.0)
    };
    let rows: Vec<(usize, f32)> = (y0..y1.min(canvas.pixmap.height() as usize))
        .map(|y| (y, coverage(y)))
        .filter(|(_, c)| *c > 0.05)
        .collect();
    let (&(first, top_cov), &(last, bottom_cov)) = (rows.first()?, rows.last()?);
    let top = first as f32 + (1.0 - top_cov);
    let bottom = last as f32 + bottom_cov;
    Some((top / scale, bottom / scale))
}

/// The edges of what `foreground` drew over `background` inside `region`,
/// in logical pixels, partly covered rows and columns counted by coverage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl Ink {
    pub fn middle_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }

    pub fn middle_x(&self) -> f32 {
        (self.left + self.right) / 2.0
    }
}

pub fn ink_box(canvas: &Canvas, region: Rect, background: Rgb, foreground: Rgb) -> Option<Ink> {
    let scale = canvas.scale;
    let (width, height) = (
        canvas.pixmap.width() as usize,
        canvas.pixmap.height() as usize,
    );
    let data = canvas.pixmap.data();
    let span = foreground.0 as f32 - background.0 as f32;
    let x0 = ((region.x * scale).floor().max(0.0) as usize).min(width);
    let x1 = ((region.right() * scale).ceil().max(0.0) as usize).min(width);
    let y0 = ((region.y * scale).floor().max(0.0) as usize).min(height);
    let y1 = ((region.bottom() * scale).ceil().max(0.0) as usize).min(height);
    let at = |x: usize, y: usize| {
        ((data[(y * width + x) * 4] as f32 - background.0 as f32) / span).clamp(0.0, 1.0)
    };
    let edges = |cover: Vec<(usize, f32)>| -> Option<(f32, f32)> {
        let lit: Vec<_> = cover.into_iter().filter(|(_, c)| *c > 0.05).collect();
        let (&(first, a), &(last, b)) = (lit.first()?, lit.last()?);
        Some((first as f32 + (1.0 - a), last as f32 + b))
    };
    let rows = (y0..y1)
        .map(|y| (y, (x0..x1).map(|x| at(x, y)).fold(0.0, f32::max)))
        .collect();
    let columns = (x0..x1)
        .map(|x| (x, (y0..y1).map(|y| at(x, y)).fold(0.0, f32::max)))
        .collect();
    let (top, bottom) = edges(rows)?;
    let (left, right) = edges(columns)?;
    Some(Ink {
        top: top / scale,
        bottom: bottom / scale,
        left: left / scale,
        right: right / scale,
    })
}

/// Where a row's icon sits against its name (gate 8).
#[derive(Clone, Debug)]
pub struct IconRow {
    pub page: String,
    pub label: String,
    /// Icon ink middle minus capitals middle; the gate is ±0.5.
    pub dy: f32,
    pub icon: Ink,
    /// Where the name is drawn from.
    pub text_x: f32,
}

/// Every row of `pages` and every navigation entry, names set in capitals,
/// drawn at `size` and scale 2.
pub fn icon_rows(fonts: Fonts, mut pages: Vec<Page>, size: (f32, f32)) -> Vec<IconRow> {
    for page in &mut pages {
        page.name = page.name.to_uppercase();
        for block in &mut page.blocks {
            if let Block::Row(row) = block {
                row.label = row.label.to_uppercase();
            }
        }
    }
    let now = Instant::now();
    let settled = now + std::time::Duration::from_secs(2);
    let names: Vec<String> = pages.iter().map(|page| page.name.clone()).collect();
    let mut app = App::new(fonts, pages, size, now);
    let mut out = Vec::new();
    for (index, name) in names.iter().enumerate() {
        app.show_named(name, now);
        let mut canvas =
            Canvas::new(size.0 as u32, size.1 as u32, SCALE).expect("canvas for the measure");
        app.draw(&mut canvas, settled);
        let layout = app.layout(settled);
        for (block, rect) in app.page().blocks.iter().zip(&layout.blocks) {
            let Block::Row(row) = block else { continue };
            let parts = app.row_parts(*rect, row);
            let around = parts.icon.inset(-2.0);
            let (Some(icon), Some(caps)) = (
                ink_box(&canvas, around, theme::PANEL, theme::SUB),
                ink_box(&canvas, parts.label, theme::PANEL, theme::TEXT),
            ) else {
                continue;
            };
            out.push(IconRow {
                page: name.clone(),
                label: row.label.clone(),
                dy: icon.middle_y() - caps.middle_y(),
                icon,
                text_x: parts.label.x,
            });
        }
        // The navigation entry of this page, on its selection surface.
        let slot = app.nav_rect(index, settled);
        let around = Rect::new(slot.x, slot.y, theme::GAP_M + theme::ICON + 3.0, slot.h);
        let text = Rect::new(
            around.right() + 2.0,
            slot.y,
            slot.w - around.w - 2.0,
            slot.h,
        );
        if let (Some(icon), Some(caps)) = (
            ink_box(&canvas, around, theme::RAISE, theme::TEXT),
            ink_box(&canvas, text, theme::RAISE, theme::TEXT),
        ) {
            out.push(IconRow {
                page: "navigation".into(),
                label: name.clone(),
                dy: icon.middle_y() - caps.middle_y(),
                icon,
                text_x: app.nav_text_x(),
            });
        }
    }
    out
}

/// Gate 8 over `rows`: each icon's ink on its capitals' middle (±0.5), and
/// per role (all rows of every page, the navigation) the icons on one axis,
/// the names on one edge, `GAP_S` (±0.5) from the widest icon's ink to the
/// names. Per role, not per page: a page with only narrow icons must not move
/// its names. The worst values, or what fails.
pub fn icon_gate(rows: &[IconRow]) -> Result<String, String> {
    let spread = |values: &mut dyn Iterator<Item = f32>| {
        let values: Vec<f32> = values.collect();
        let max = values.iter().copied().fold(f32::MIN, f32::max);
        let min = values.iter().copied().fold(f32::MAX, f32::min);
        (min, max - min)
    };
    let (mut dy, mut axis, mut edge, mut gap) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for row in rows {
        if row.dy.abs() > 0.5 {
            return Err(format!(
                "{} {}: icon {:+.2} px off the capitals",
                row.page, row.label, row.dy
            ));
        }
        dy = dy.max(row.dy.abs());
    }
    for group in ["rows", "navigation"] {
        let rows: Vec<&IconRow> = rows
            .iter()
            .filter(|row| (row.page == "navigation") == (group == "navigation"))
            .collect();
        let (_, a) = spread(&mut rows.iter().map(|row| row.icon.middle_x()));
        let (text, e) = spread(&mut rows.iter().map(|row| row.text_x));
        let widest = rows
            .iter()
            .map(|row| row.icon.right)
            .fold(f32::MIN, f32::max);
        let g = text - widest - theme::GAP_S;
        if a > 0.5 || e > 0.5 || g.abs() > 0.5 {
            return Err(format!(
                "{group}: icon axis spread {a:.2}, name edge spread {e:.2}, gap off GAP_S by {g:+.2}"
            ));
        }
        (axis, edge, gap) = (axis.max(a), edge.max(e), gap.max(g.abs()));
    }
    Ok(format!(
        "worst icon off the capitals {dy:.2} px, axis spread {axis:.2}, edge spread {edge:.2}, gap off GAP_S {gap:.2}"
    ))
}

/// `rect`'s top-to-capitals gap minus its baseline-to-bottom gap.
fn offset(canvas: &Canvas, region: Rect, rect: Rect, background: Rgb) -> Option<f32> {
    let (top, bottom) = ink(canvas, region, background)?;
    Some((top - rect.y) - (rect.bottom() - bottom))
}

fn probe_row(label: &str, control: Box<dyn Widget>) -> Block {
    Block::Row(Row {
        icon: Icon::Info,
        label: label.to_owned(),
        hint: None,
        control,
        enabled: true,
        bind: Bind::None,
    })
}

/// Every kind of text a control or row draws, set in capitals, with how far
/// its capitals sit off the middle (logical px; the gate is ±0.5).
pub fn cap_offsets(fonts: Fonts) -> Vec<(&'static str, f32)> {
    let now = Instant::now();
    let page = Page {
        name: CAPS.into(),
        icon: Icon::Info,
        blocks: vec![
            Block::Banner {
                icon: Icon::Info,
                text: CAPS.into(),
                color: theme::SUB,
            },
            probe_row(CAPS, Box::new(Dropdown::new(vec![CAPS.into()], 0, now))),
            probe_row(
                CAPS,
                Box::new(Segment::new(
                    vec![Part::text(CAPS), Part::text("hh")],
                    0,
                    now,
                )),
            ),
            probe_row(CAPS, Box::new(TextField::new(CAPS))),
        ],
    };
    let size = (760.0, 520.0);
    let app = App::new(fonts, vec![page], size, now);
    let mut canvas =
        Canvas::new(size.0 as u32, size.1 as u32, SCALE).expect("canvas for the measure");
    let settled = now + std::time::Duration::from_secs(2);
    app.draw(&mut canvas, settled);
    let layout = app.layout(settled);
    let mut out = Vec::new();
    // The navigation entry, text only (the icon sits left of it).
    let nav = app.nav_rect(0, settled);
    let text = Rect::new(nav.x + 40.0, nav.y, nav.w - 40.0, nav.h);
    out.extend(offset(&canvas, text, nav, theme::RAISE).map(|o| ("navigation", o)));
    let banner = layout.blocks[0].inset(theme::GAP_M);
    let banner_text = Rect::new(banner.x + 30.0, banner.y, banner.w - 30.0, banner.h);
    out.extend(offset(&canvas, banner_text, banner, theme::PANEL).map(|o| ("banner", o)));
    for (index, name) in [(1, "dropdown"), (2, "segment"), (3, "text field")] {
        let Block::Row(row) = &app.page().blocks[index] else {
            unreachable!()
        };
        let parts = app.row_parts(layout.blocks[index], row);
        let row_rect = layout.blocks[index];
        out.extend(offset(&canvas, parts.label, row_rect, theme::PANEL).map(|o| ("row name", o)));
        // The control's text: left part for a segment, before the chevron
        // for a dropdown, the field's text.
        let control = parts.control;
        let region = match name {
            "segment" => Rect::new(control.x, control.y, control.w / 2.0, control.h),
            "dropdown" => Rect::new(control.x, control.y, control.w - 32.0, control.h),
            _ => control,
        };
        let background = if name == "segment" {
            theme::SELECTED
        } else {
            theme::RAISE
        };
        out.extend(offset(&canvas, region, control, background).map(|o| (name, o)));
    }
    out
}
