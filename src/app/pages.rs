//! The settings pages with the values in effect (settings plan, table under
//! "Aufbau"). The platform fills [`Prefs`] from the config; Phase 5 swaps
//! that for the daemon's state and makes the rows write.

use std::time::Instant;

use super::{Block, Page, Row};
use crate::ui::icons::Icon;
use crate::ui::theme;
use crate::ui::widgets::{Dropdown, Part, Segment, Slider, Stepper, Switch, TextField, Widget};

/// The values the pages show, in plain types.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub card_width: u32,
    pub visible_cards: u32,
    pub corner: Corner,
    pub save_dir: String,
    pub both_combined: bool,
    pub fps: u32,
    pub cursor: Cursor,
    pub show_frame: bool,
    pub audio: bool,
    pub audio_source: AudioSource,
    pub record_dir: String,
    pub disk_to_shelf: bool,
    pub shelf_to_disk_after: Option<u64>,
    pub replay_autostart: bool,
    pub replay_seconds: u64,
    pub replay_memory_mib: usize,
    pub replay_fps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    System,
    Mellow,
    Quick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSource {
    SystemAndMic,
    Mic,
    System,
}

const FRAME_RATES: [u32; 5] = [30, 60, 120, 144, 240];
const SHELF_TO_DISK: [Option<u64>; 5] = [None, Some(30), Some(60), Some(300), Some(900)];
const REPLAY_MEMORY_MIB: [usize; 5] = [256, 512, 1024, 2048, 4096];

fn row(icon: Icon, label: &str, hint: Option<&str>, control: Box<dyn Widget>) -> Block {
    Block::Row(Row {
        icon,
        label: label.to_owned(),
        hint: hint.map(str::to_owned),
        control,
        enabled: true,
    })
}

/// A dropdown over `choices`, with the value in effect added when it is
/// none of them, so the page never shows a value that is not the real one.
fn choose<T: PartialEq + Copy>(
    choices: &[T],
    current: T,
    label: impl Fn(T) -> String,
    now: Instant,
) -> Box<Dropdown> {
    let mut values = choices.to_vec();
    if !values.contains(&current) {
        values.push(current);
    }
    let chosen = values.iter().position(|v| *v == current).unwrap_or(0);
    Box::new(Dropdown::new(
        values.into_iter().map(label).collect(),
        chosen,
        now,
    ))
}

fn fps(fps: u32) -> String {
    format!("{fps}\u{a0}fps")
}

fn duration(seconds: Option<u64>) -> String {
    match seconds {
        None => "Off".into(),
        Some(s) if s.is_multiple_of(60) => format!("{}\u{a0}min", s / 60),
        Some(s) => format!("{s}\u{a0}s"),
    }
}

fn mebibytes(mib: usize) -> String {
    if mib.is_multiple_of(1024) {
        format!("{}\u{a0}GiB", mib / 1024)
    } else {
        format!("{mib}\u{a0}MiB")
    }
}

fn switch(on: bool, now: Instant) -> Box<Switch> {
    Box::new(Switch::new(on, now))
}

fn segment(parts: Vec<Part>, chosen: usize, now: Instant) -> Box<Segment> {
    Box::new(Segment::new(parts, chosen, now))
}

/// Until the rows write (Phase 5), every page says so.
fn preview() -> Block {
    Block::Banner {
        icon: Icon::Info,
        text: "Preview: changes here are not saved yet".into(),
        color: theme::SUB,
    }
}

fn shelf(prefs: &Prefs, now: Instant) -> Page {
    let corners = [
        (Corner::TopLeft, Icon::ArrowUpLeft),
        (Corner::TopRight, Icon::ArrowUpRight),
        (Corner::BottomLeft, Icon::ArrowDownLeft),
        (Corner::BottomRight, Icon::ArrowDownRight),
    ];
    let corner = corners
        .iter()
        .position(|(c, _)| *c == prefs.corner)
        .unwrap_or(2);
    Page {
        name: "Shelf".into(),
        icon: Icon::Layers,
        blocks: vec![
            preview(),
            Block::Heading("Cards".into()),
            row(
                Icon::Scaling,
                "Card size",
                None,
                Box::new(Slider::new(prefs.card_width.into(), (140, 380, 10), "px")),
            ),
            row(
                Icon::GalleryVerticalEnd,
                "Cards shown",
                None,
                Box::new(Stepper::new(prefs.visible_cards.into(), 1, 8, now)),
            ),
            row(
                Icon::Scan,
                "Corner",
                None,
                segment(
                    corners.iter().map(|(_, icon)| Part::icon(*icon)).collect(),
                    corner,
                    now,
                ),
            ),
        ],
    }
}

fn screenshots(prefs: &Prefs) -> Page {
    Page {
        name: "Screenshots".into(),
        icon: Icon::Image,
        blocks: vec![
            preview(),
            Block::Heading("Saving".into()),
            row(
                Icon::Folder,
                "Save folder",
                None,
                Box::new(TextField::new(&prefs.save_dir)),
            ),
        ],
    }
}

fn recording(prefs: &Prefs, now: Instant) -> Page {
    let cursor = [Cursor::System, Cursor::Mellow, Cursor::Quick]
        .iter()
        .position(|c| *c == prefs.cursor)
        .unwrap_or(0);
    let source = [
        AudioSource::SystemAndMic,
        AudioSource::Mic,
        AudioSource::System,
    ]
    .iter()
    .position(|s| *s == prefs.audio_source)
    .unwrap_or(0);
    let next = Some("Applies to the next recording");
    Page {
        name: "Recording".into(),
        icon: Icon::Video,
        blocks: vec![
            preview(),
            Block::Heading("Video".into()),
            row(
                Icon::Gauge,
                "Frame rate",
                next,
                choose(&FRAME_RATES, prefs.fps, fps, now),
            ),
            row(
                Icon::Monitor,
                "Both displays",
                None,
                segment(
                    vec![Part::text("Separate"), Part::text("Combined")],
                    usize::from(prefs.both_combined),
                    now,
                ),
            ),
            row(
                Icon::MousePointer2,
                "Cursor",
                None,
                segment(
                    vec![
                        Part::text("System"),
                        Part::text("Mellow"),
                        Part::text("Quick"),
                    ],
                    cursor,
                    now,
                ),
            ),
            row(
                Icon::SquareDashed,
                "Show frame",
                Some("Outline around a recorded region"),
                switch(prefs.show_frame, now),
            ),
            Block::Heading("Audio".into()),
            row(Icon::Volume2, "Audio", next, switch(prefs.audio, now)),
            row(
                Icon::Mic,
                "Source",
                None,
                segment(
                    vec![
                        Part::text("System and mic"),
                        Part::text("Mic"),
                        Part::text("System"),
                    ],
                    source,
                    now,
                ),
            ),
            Block::Heading("Saving".into()),
            row(
                Icon::Folder,
                "Video folder",
                None,
                Box::new(TextField::new(&prefs.record_dir)),
            ),
            row(
                Icon::HardDriveDownload,
                "Show saved videos on the shelf",
                None,
                switch(prefs.disk_to_shelf, now),
            ),
            row(
                Icon::Clock,
                "Keep long shelf saves",
                Some("From this length they go to the video folder"),
                choose(&SHELF_TO_DISK, prefs.shelf_to_disk_after, duration, now),
            ),
        ],
    }
}

fn replay(prefs: &Prefs, now: Instant) -> Page {
    let restart = Some("Applies when replay restarts");
    Page {
        name: "Replay".into(),
        icon: Icon::RotateCcwClock,
        blocks: vec![
            preview(),
            Block::Heading("Buffer".into()),
            row(
                Icon::Power,
                "Start with Boltsnap",
                None,
                switch(prefs.replay_autostart, now),
            ),
            row(
                Icon::Timer,
                "Length",
                restart,
                Box::new(Slider::new(prefs.replay_seconds as i64, (5, 600, 5), "s")),
            ),
            row(
                Icon::MemoryStick,
                "Memory",
                restart,
                choose(&REPLAY_MEMORY_MIB, prefs.replay_memory_mib, mebibytes, now),
            ),
            row(
                Icon::Gauge,
                "Frame rate",
                restart,
                choose(&FRAME_RATES, prefs.replay_fps, fps, now),
            ),
        ],
    }
}

pub fn pages(prefs: &Prefs, now: Instant) -> Vec<Page> {
    vec![
        shelf(prefs, now),
        screenshots(prefs),
        recording(prefs, now),
        replay(prefs, now),
    ]
}
