//! Pictures of what a page's settings do, following its controls as they
//! move (settings plan, 5d): the shelf on a screen (P1), the monitors a
//! recording takes (P2), the replay buffer on a timeline (P3).

mod displays;
mod shelf;

use std::time::Instant;

use super::App;
use crate::ui::{Canvas, Rect};

pub use displays::places as display_places;
pub use shelf::{screen, shelf_cards};

use super::pages::Screen;
use crate::ui::motion::Spring;

/// What a preview block shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Shelf,
    /// The connected monitors, where a recording goes.
    Displays(Vec<Screen>),
}

impl Preview {
    /// The block's height in a column `width` wide.
    pub fn height(&self, width: f32) -> f32 {
        match self {
            Preview::Shelf => shelf::height(width),
            Preview::Displays(screens) => displays::height(width, screens),
        }
    }
}

/// Everything the previews move.
pub(super) struct State {
    cards: Vec<shelf::Card>,
    shape: Option<shelf::ShelfShape>,
    /// Per monitor how lit it is, and the bridge of a combined clip.
    lit: Vec<Spring>,
    bridge: Spring,
}

impl State {
    pub(super) fn new(now: Instant) -> State {
        State {
            cards: Vec::new(),
            shape: None,
            lit: Vec::new(),
            bridge: Spring::settled(0.0, now),
        }
    }
}

impl App {
    /// Point the current page's previews at what its controls hold;
    /// `animate` false sets them (a page just shown).
    pub(super) fn sync_preview(&mut self, animate: bool, now: Instant) {
        self.sync_shelf(animate, now);
        self.sync_displays(animate, now);
    }

    pub(super) fn preview_moving(&self, now: Instant) -> bool {
        self.shelf_moving(now) || self.displays_moving(now)
    }

    pub(super) fn draw_preview(
        &self,
        canvas: &mut Canvas,
        preview: &Preview,
        rect: Rect,
        now: Instant,
    ) {
        match preview {
            Preview::Shelf => self.draw_shelf(canvas, rect, now),
            Preview::Displays(_) => self.draw_displays(canvas, rect, now),
        }
    }
}
