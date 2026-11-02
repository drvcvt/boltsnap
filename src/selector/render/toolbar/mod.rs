//! The recording selector's toolbar (variant A, compact K2): [Record, Clip]
//! · [Frame, Audio] · the size of the clip, one row if it fits, else two or
//! three. Every measure is a formula of the theme's tokens, below.

use boltsnap::ui::icons::Icon;
use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, GAP_S, GAP_XS, Rgb, TextStyle};
use boltsnap::ui::{Canvas, Rect};

use super::chrome::size_text;

mod layout;
pub use layout::{Layout, Metrics, layout};

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

/// Record's dot, the toolbar's one colour: it lights up with its surface,
/// an eighth of the way to `TEXT` per step above rest, so it keeps 3:1 on
/// the hover and pressed surfaces too.
pub fn mark(look: &ControlLook) -> Rgb {
    let up = (step(RECORD, look) - 1.0).clamp(0.0, 2.0);
    theme::mix(theme::ERROR, theme::TEXT, 0.125 * up)
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
            // A control widened to fill a row may be an odd pixel wider.
            control.x + ((control.w - ICON_BOX) / 2.0).floor(),
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
        if !layout.shown[i] {
            continue;
        }
        let control = &look.controls[i];
        if let Some(fill) = surface(i, control) {
            canvas.fill(*rect, CTL_RADIUS, fill, 1.0);
        }
        let ink = ink(i, control);
        let icon = icon_rect(i, *rect);
        match i {
            // The one colour on the toolbar: the primary action's mark.
            RECORD => Icon::Circle.fill(canvas, icon, mark(control), 1.0),
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
