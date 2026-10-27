//! Pictures of what a page's settings do, following its controls as they
//! move (settings plan, 5d): the shelf on a screen (P1), the monitors a
//! recording takes (P2), the replay buffer on a timeline (P3). Surfaces
//! only: the window shows none of the user's own pictures (5e V5).

mod dims;
mod displays;
mod load;
mod replay;
mod shelf;

use std::time::Instant;

use super::App;
use crate::ui::{Canvas, Rect};

pub use dims::{Measure, measure as shelf_measure_of};
pub use displays::{
    caption_place as display_caption_place, every_caption as display_lines,
    name_places as display_name_places, places as display_places,
};
pub use load::ReplayLoad;
pub use replay::{
    bar as replay_bar_rect, cells as replay_cells, head as replay_head_rect, step as replay_step,
};

use super::pages::Screen;
use crate::ui::motion::Spring;
use crate::ui::text::Fonts;

/// What a preview block shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Shelf,
    /// The connected monitors, where a recording goes.
    Displays(Vec<Screen>),
    /// The replay buffer on a timeline; a fixed bitrate changes what fits.
    Replay {
        bitrate_kbps: Option<u32>,
    },
    /// What the running replay buffer costs, measured; `stale`: it runs
    /// with the settings from before the page's changes.
    Load {
        stale: bool,
    },
}

impl Preview {
    /// The block's height in a column `width` wide.
    pub fn height(&self, fonts: &Fonts, width: f32) -> f32 {
        match self {
            Preview::Shelf => shelf::height(width),
            Preview::Displays(screens) => displays::height(fonts, width, screens),
            Preview::Replay { .. } => replay::height(),
            Preview::Load { .. } => load::height(fonts, width),
        }
    }
}

/// Everything the previews move.
pub(super) struct State {
    play: shelf::Play,
    /// Per monitor how lit it is, and the bridge of a combined clip.
    lit: Vec<Spring>,
    bridge: Spring,
    /// The line under the monitors, the old one fading out.
    captions: Vec<(String, Spring)>,
    /// 0 to 1: the front card's measure shows; the edge being dragged.
    dims: Spring,
    drag: Option<dims::Drag>,
    /// The replay buffer's length, what its memory holds, what it holds now.
    length: Spring,
    holds: Spring,
    held: Spring,
    /// The buffer runs; its video bitrate, kbit/s.
    running: bool,
    kbps: u32,
    /// What the running buffer costs, as last measured.
    load: Option<ReplayLoad>,
}

impl State {
    pub(super) fn new(now: Instant) -> State {
        State {
            play: shelf::Play::new(now),
            lit: Vec::new(),
            bridge: Spring::settled(0.0, now),
            captions: Vec::new(),
            dims: Spring::settled(0.0, now),
            drag: None,
            length: Spring::settled(60.0, now),
            holds: Spring::settled(60.0, now),
            held: Spring::settled(0.0, now),
            running: false,
            kbps: 0,
            load: None,
        }
    }
}

impl App {
    /// Point the current page's previews at what its controls hold;
    /// `animate` false sets them (a page just shown).
    pub(super) fn sync_preview(&mut self, animate: bool, now: Instant) {
        self.sync_shelf(animate, now);
        self.sync_displays(animate, now);
        self.sync_replay(animate, now);
    }

    pub(super) fn preview_moving(&self, now: Instant) -> bool {
        self.shelf_moving(now)
            || self.displays_moving(now)
            || self.replay_moving(now)
            || !self.preview.dims.done(now)
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
            Preview::Replay { .. } => self.draw_replay(canvas, rect, now),
            Preview::Load { stale } => self.draw_load(canvas, rect, *stale),
        }
    }
}
