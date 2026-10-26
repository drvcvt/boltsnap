//! The settings pages with the values in effect (settings plan, table under
//! "Aufbau"). The platform fills [`Prefs`] from the config; every row is
//! bound to the key it writes (Phase 5b).

use std::time::Instant;

use super::{Bind, Block, Encode, Page, Row};
use crate::settings;
use crate::ui::icons::Icon;
use crate::ui::theme;
use crate::ui::widgets::{
    Dropdown, IconButton, Part, Segment, Slider, Stepper, Switch, TextField, Value, Widget,
};

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
    /// The buffer is capturing, starting, or up but stalled.
    pub replay_running: bool,
    /// Up, but no video is arriving.
    pub replay_stalled: bool,
    /// Up with other settings than the file has now: a restart applies them.
    pub replay_stale: bool,
    /// A recording runs; recording settings apply to the next one.
    pub recording: bool,
    /// The keys `config.toml` sets; their rows offer a reset.
    pub set_keys: Vec<String>,
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

/// Shown on top of every page: why a write failed, or why nothing can be
/// written right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    /// The rows are off: no daemon, or a `config.toml` that does not parse.
    pub locked: bool,
}

const FRAME_RATES: [u32; 5] = [30, 60, 120, 144, 240];
const SHELF_TO_DISK: [Option<u64>; 5] = [None, Some(30), Some(60), Some(300), Some(900)];
const REPLAY_MEMORY_MIB: [usize; 5] = [256, 512, 1024, 2048, 4096];

fn row(
    icon: Icon,
    label: &str,
    hint: Option<&str>,
    (control, bind): (Box<dyn Widget>, Bind),
) -> Block {
    Block::Row(Row {
        icon,
        label: label.to_owned(),
        hint: hint.map(str::to_owned),
        control,
        enabled: true,
        bind,
        reset: false,
    })
}

fn key(key: &'static str, encode: impl Fn(&Value) -> Option<settings::Value> + 'static) -> Bind {
    Bind::Key(key, Box::new(encode) as Encode)
}

fn int(value: &Value) -> Option<settings::Value> {
    match value {
        Value::Int(n) => Some(settings::Value::Int(*n)),
        _ => None,
    }
}

fn boolean(value: &Value) -> Option<settings::Value> {
    match value {
        Value::Bool(on) => Some(settings::Value::Bool(*on)),
        _ => None,
    }
}

fn text(value: &Value) -> Option<settings::Value> {
    match value {
        Value::Text(text) => Some(settings::Value::Text(text.clone())),
        _ => None,
    }
}

/// A part of a segment as one of the key's words, in part order.
fn word(words: &'static [&'static str]) -> impl Fn(&Value) -> Option<settings::Value> {
    move |value| match value {
        Value::Choice(index) => words
            .get(*index)
            .map(|w| settings::Value::Text((*w).into())),
        _ => None,
    }
}

/// A dropdown over `choices`, with the value in effect added when it is
/// none of them, so the page never shows a value that is not the real one.
/// It writes `to(choice)` to `name`.
fn choose<T: PartialEq + Copy + 'static>(
    name: &'static str,
    choices: &[T],
    current: T,
    label: impl Fn(T) -> String,
    to: fn(T) -> settings::Value,
    now: Instant,
) -> (Box<dyn Widget>, Bind) {
    let mut values = choices.to_vec();
    if !values.contains(&current) {
        values.push(current);
    }
    let chosen = values.iter().position(|v| *v == current).unwrap_or(0);
    let dropdown = Dropdown::new(values.iter().copied().map(label).collect(), chosen, now);
    let bind = key(name, move |value| match value {
        Value::Choice(index) => values.get(*index).copied().map(to),
        _ => None,
    });
    (Box::new(dropdown), bind)
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

fn switch(name: &'static str, on: bool, now: Instant) -> (Box<dyn Widget>, Bind) {
    (Box::new(Switch::new(on, now)), key(name, boolean))
}

/// A segment over `words`, one part each, writing the chosen word.
fn segment(
    name: &'static str,
    parts: Vec<Part>,
    words: &'static [&'static str],
    chosen: usize,
    now: Instant,
) -> (Box<dyn Widget>, Bind) {
    (
        Box::new(Segment::new(parts, chosen, now)),
        key(name, word(words)),
    )
}

fn folder(name: &'static str, path: &str) -> (Box<dyn Widget>, Bind) {
    (Box::new(TextField::new(path)), key(name, text))
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
            Block::Heading("Cards".into()),
            row(
                Icon::Scaling,
                "Card size",
                None,
                (
                    Box::new(Slider::new(prefs.card_width.into(), (140, 380, 10), "px")),
                    key("shelf.card_width", int),
                ),
            ),
            row(
                Icon::GalleryVerticalEnd,
                "Cards shown",
                None,
                (
                    Box::new(Stepper::new(prefs.visible_cards.into(), 1, 8, now)),
                    key("shelf.visible_cards", int),
                ),
            ),
            row(
                Icon::Scan,
                "Corner",
                None,
                segment(
                    "shelf.corner",
                    corners.iter().map(|(_, icon)| Part::icon(*icon)).collect(),
                    &["top-left", "top-right", "bottom-left", "bottom-right"],
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
            Block::Heading("Saving".into()),
            row(
                Icon::Folder,
                "Save folder",
                None,
                folder("save_dir", &prefs.save_dir),
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
    let mut blocks = Vec::new();
    if prefs.recording {
        // One note while it matters, instead of a hint under every row.
        blocks.push(Block::Banner {
            icon: Icon::Info,
            text: "A recording is running, changes here apply to the next one".into(),
            color: theme::SUB,
        });
    }
    blocks.extend([
        Block::Heading("Video".into()),
        row(
            Icon::Gauge,
            "Frame rate",
            None,
            choose("record_fps", &FRAME_RATES, prefs.fps, fps, fps_value, now),
        ),
        row(
            Icon::Monitor,
            "Both displays",
            None,
            segment(
                "record_both_mode",
                vec![Part::text("Separate"), Part::text("Combined")],
                &["separate", "combined"],
                usize::from(prefs.both_combined),
                now,
            ),
        ),
        row(
            Icon::MousePointer2,
            "Cursor",
            None,
            segment(
                "record_cursor",
                vec![
                    Part::text("System"),
                    Part::text("Mellow"),
                    Part::text("Quick"),
                ],
                &["system", "mellow", "quick"],
                cursor,
                now,
            ),
        ),
        row(
            Icon::SquareDashed,
            "Show frame",
            Some("Outline around a recorded region"),
            switch("record_show_frame", prefs.show_frame, now),
        ),
        Block::Heading("Audio".into()),
        row(
            Icon::Volume2,
            "Audio",
            None,
            switch("record_audio_enabled", prefs.audio, now),
        ),
        row(
            Icon::Mic,
            "Source",
            None,
            segment(
                "record_audio_source",
                vec![
                    Part::text("System and mic"),
                    Part::text("Mic"),
                    Part::text("System"),
                ],
                &["system-and-mic", "mic", "system"],
                source,
                now,
            ),
        ),
        Block::Heading("Saving".into()),
        row(
            Icon::Folder,
            "Video folder",
            None,
            folder("record_dir", &prefs.record_dir),
        ),
        row(
            Icon::HardDriveDownload,
            "Show saved videos on the shelf",
            None,
            switch("record_disk_add_to_shelf", prefs.disk_to_shelf, now),
        ),
        row(
            Icon::Clock,
            "Keep long shelf saves",
            Some("From this length they go to the video folder"),
            choose(
                "record_shelf_to_disk_after_seconds",
                &SHELF_TO_DISK,
                prefs.shelf_to_disk_after,
                duration,
                // Zero turns it off (schema).
                |after| settings::Value::Int(after.map_or(0, |s| s as i64)),
                now,
            ),
        ),
    ]);
    Page {
        name: "Recording".into(),
        icon: Icon::Video,
        blocks,
    }
}

fn fps_value(fps: u32) -> settings::Value {
    settings::Value::Int(fps.into())
}

fn replay(prefs: &Prefs, now: Instant) -> Page {
    let mut blocks = vec![
        Block::Heading("Buffer".into()),
        row(
            Icon::RotateCcwClock,
            "Replay buffer",
            prefs
                .replay_stalled
                .then_some("No video is arriving, turn it off and on"),
            (
                Box::new(Switch::new(prefs.replay_running, now)),
                Bind::ReplayBuffer,
            ),
        ),
        row(
            Icon::Power,
            "Start with Boltsnap",
            None,
            switch("replay.autostart", prefs.replay_autostart, now),
        ),
        row(
            Icon::Timer,
            "Length",
            None,
            (
                Box::new(Slider::new(prefs.replay_seconds as i64, (5, 600, 5), "s")),
                key("replay.duration_seconds", int),
            ),
        ),
        row(
            Icon::MemoryStick,
            "Memory",
            None,
            choose(
                "replay.memory_mib",
                &REPLAY_MEMORY_MIB,
                prefs.replay_memory_mib,
                mebibytes,
                |mib| settings::Value::Int(mib as i64),
                now,
            ),
        ),
        row(
            Icon::Gauge,
            "Frame rate",
            None,
            choose(
                "replay.fps",
                &FRAME_RATES,
                prefs.replay_fps,
                fps,
                fps_value,
                now,
            ),
        ),
    ];
    if prefs.replay_running && prefs.replay_stale {
        // Right under the buffer it would empty; never on its own.
        blocks.insert(
            2,
            row(
                Icon::RotateCcw,
                "Restart replay",
                Some("Applies your changes, the buffer starts empty"),
                (
                    Box::new(IconButton::new(Icon::RotateCcw)),
                    Bind::ReplayRestart,
                ),
            ),
        );
    }
    Page {
        name: "Replay".into(),
        icon: Icon::RotateCcwClock,
        blocks,
    }
}

/// The pages, each opening with `notice` when there is one; a locked notice
/// turns every row off.
pub fn pages(prefs: &Prefs, notice: Option<&Notice>, now: Instant) -> Vec<Page> {
    let mut pages = vec![
        shelf(prefs, now),
        screenshots(prefs),
        recording(prefs, now),
        replay(prefs, now),
    ];
    for block in pages.iter_mut().flat_map(|page| &mut page.blocks) {
        if let Block::Row(row) = block
            && let Bind::Key(key, _) = &row.bind
        {
            row.reset = prefs.set_keys.iter().any(|set| set == key);
        }
    }
    if let Some(notice) = notice {
        for page in &mut pages {
            if notice.locked {
                for block in &mut page.blocks {
                    if let Block::Row(row) = block {
                        row.enabled = false;
                    }
                }
            }
            page.blocks.insert(
                0,
                Block::Banner {
                    icon: Icon::TriangleAlert,
                    text: notice.text.clone(),
                    color: theme::ERROR,
                },
            );
        }
    }
    pages
}
