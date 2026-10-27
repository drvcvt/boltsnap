//! The shelf's cards as the overlay draws them: where they stack, how they
//! look, how they come and go. Shared by the overlay and the settings
//! window's shelf preview, so the preview is the overlay's own painting
//! (settings plan, 5g V6).

pub mod layout;
pub mod paint;

/// What a card holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardKind {
    Image,
    Video,
}

/// How the cards look, from `[shelf]` in the config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardLook {
    /// 0 to 1: a card at rest; text and windows behind stay readable.
    pub opacity: f32,
    /// The hovered card is fully opaque.
    pub solid_on_hover: bool,
    /// A soft drop shadow behind every settled card.
    pub shadow: bool,
}

impl Default for CardLook {
    fn default() -> Self {
        Self {
            opacity: 0.8,
            solid_on_hover: true,
            shadow: true,
        }
    }
}

/// How long a new card takes to arrive, in ms.
pub const APPEAR_MS: u128 = 150;
/// How long a dismissed card takes to leave, in ms: a touch longer than the
/// appear so it reads as smooth rather than snappy.
pub const DISMISS_MS: u128 = 240;
/// Card scale at the far end of the appear animation.
pub const ANIM_SCALE_MIN: f32 = 0.88;

pub fn ease_out_cubic(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

/// (scale, opacity) of an arriving card, `eased` 0 to 1.
pub fn appear(eased: f32) -> (f32, f32) {
    (ANIM_SCALE_MIN + (1.0 - ANIM_SCALE_MIN) * eased, eased)
}

/// (scale, opacity) of a leaving card, `eased` 0 to 1: it shrinks toward
/// nothing in step with its collapsing slot, so it doesn't overlap the cards
/// sliding up, and fades.
pub fn dismiss(eased: f32) -> (f32, f32) {
    (1.0 - eased, 1.0 - eased)
}
