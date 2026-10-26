//! Measuring what was drawn, for the `/ui` gates: how far the capitals of
//! every control's text sit from its vertical middle. Lives outside the tests
//! so the real UI font can be measured too (`boltsnap settings --measure`).

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
