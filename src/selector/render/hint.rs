//! Drawing the toolbar as one fading layer, and the short lines beside it:
//! why Clip cannot be pressed, and the icon-only controls' names.

use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, GAP_S, GAP_XS};
use boltsnap::ui::{Canvas, Rect};

use super::chrome::Placement;
use super::toolbar::{self, Layout, Look};
use super::toolbar::{BAR_H, BAR_RADIUS, PAD, SIDE, TEXT};

/// A line beside the toolbar is a toolbar with one control that has no
/// surface: `BAR_H` tall, `BAR_RADIUS`, its text where a label sits.
const INSET: f32 = PAD + SIDE;

/// What `paint` draws inside `bounds`, at `alpha` as one layer: drawn on its
/// own and laid over the frame, so surfaces inside it do not show through
/// each other while it fades. `paint` gets `bounds`' origin to draw from.
fn faded(canvas: &mut Canvas, bounds: Rect, alpha: f32, paint: impl FnOnce(&mut Canvas, f32, f32)) {
    if alpha >= 1.0 {
        paint(canvas, 0.0, 0.0);
        return;
    }
    let Some(mut layer) = Canvas::new(bounds.w.ceil() as u32, bounds.h.ceil() as u32, 1) else {
        return;
    };
    paint(&mut layer, -bounds.x, -bounds.y);
    canvas.pixmap.draw_pixmap(
        bounds.x as i32,
        bounds.y as i32,
        layer.pixmap.as_ref(),
        &tiny_skia::PixmapPaint {
            opacity: alpha.clamp(0.0, 1.0),
            ..Default::default()
        },
        tiny_skia::Transform::identity(),
        None,
    );
}

/// The toolbar at `alpha`, as one layer.
pub fn draw_faded(canvas: &mut Canvas, fonts: &Fonts, layout: &Layout, look: &Look, alpha: f32) {
    faded(canvas, layout.bounds, alpha, |canvas, dx, dy| {
        toolbar::draw(canvas, fonts, &layout.translated(dx, dy), look);
    });
}

/// Where a line about control `anchor` goes: left with it, `GAP_XS` beside
/// the toolbar on the side away from the selection (above a toolbar above
/// it, below one below or inside it), else the other side, `GAP_S` inside
/// `area`; cut with an ellipsis when too long.
pub fn hint_rect(
    fonts: &Fonts,
    layout: &Layout,
    anchor: usize,
    text: &str,
    area: Rect,
) -> (Rect, String) {
    let room = (area.w - 2.0 * GAP_S - 2.0 * INSET).max(0.0);
    let text = fonts.ellipsize(TEXT, text, room).into_owned();
    let w = (2.0 * INSET + fonts.width(TEXT, &text)).ceil();
    let b = layout.bounds;
    let above = b.y - GAP_XS - BAR_H;
    let below = b.bottom() + GAP_XS;
    let (top, bottom) = (area.y + GAP_S, area.bottom() - GAP_S - BAR_H);
    let y = match layout.placement {
        Placement::Above if above >= top => above,
        Placement::Above if below <= bottom => below,
        _ if below <= bottom => below,
        _ if above >= top => above,
        _ => below.clamp(top, bottom.max(top)),
    };
    let x = layout.controls[anchor].x.clamp(
        area.x + GAP_S,
        (area.right() - GAP_S - w).max(area.x + GAP_S),
    );
    (Rect::new(x.round(), y.round(), w, BAR_H), text)
}

/// The line, in a small surface of the toolbar's kind, at `alpha`.
pub fn draw_hint(canvas: &mut Canvas, fonts: &Fonts, rect: Rect, text: &str, alpha: f32) {
    faded(canvas, rect, alpha, |canvas, dx, dy| {
        let rect = Rect::new(rect.x + dx, rect.y + dy, rect.w, rect.h);
        canvas.fill(rect, BAR_RADIUS, theme::PANEL, 1.0);
        let baseline = fonts.baseline(TEXT, rect.y, rect.h);
        fonts.draw(
            canvas,
            TEXT,
            text,
            (rect.x + INSET, baseline),
            theme::SUB,
            1.0,
        );
    });
}

/// Where the icon-only controls' names go: one place for all of them, with
/// the first of the group, as wide as the widest name, so a swap from one
/// control to the other or a change of word crossfades only the words; the
/// surface neither moves nor changes size.
pub fn names_rect(fonts: &Fonts, layout: &Layout, area: Rect) -> Rect {
    let names = [toolbar::FRAME, toolbar::AUDIO]
        .into_iter()
        .flat_map(|i| [true, false].map(|on| toolbar::name(i, on)))
        .flatten();
    let widest = names
        .max_by(|a, b| fonts.width(TEXT, a).total_cmp(&fonts.width(TEXT, b)))
        .unwrap_or_default();
    hint_rect(fonts, layout, toolbar::FRAME, widest, area).0
}

/// The names, each at its opacity, on one surface as opaque as the most
/// visible of them.
pub fn draw_names(canvas: &mut Canvas, fonts: &Fonts, rect: Rect, names: &[(&str, f32)]) {
    let most = names.iter().map(|(_, a)| *a).fold(0.0, f32::max);
    if most <= 0.0 {
        return;
    }
    faded(canvas, rect, most, |canvas, dx, dy| {
        let rect = Rect::new(rect.x + dx, rect.y + dy, rect.w, rect.h);
        canvas.fill(rect, BAR_RADIUS, theme::PANEL, 1.0);
        let baseline = fonts.baseline(TEXT, rect.y, rect.h);
        for (text, alpha) in names {
            let at = (rect.x + INSET, baseline);
            fonts.draw(canvas, TEXT, text, at, theme::SUB, alpha / most);
        }
    });
}
