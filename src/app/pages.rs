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
    pub display: Display,
    /// The connected monitors: (connector, label as people know it).
    pub displays: Vec<(String, String)>,
    /// `record_codec`, `auto` when unset.
    pub codec: String,
    /// What gpu-screen-recorder can encode here (`--info`), once known.
    pub codecs: Option<Vec<String>>,
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
    /// `replay.bitrate_kbps`; unset, the bitrate follows length and memory.
    pub replay_bitrate: Option<u32>,
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

/// Where a fullscreen recording goes: `record_default_target`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Display {
    Focused,
    Output(String),
    Both,
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
const REPLAY_SECONDS: [u64; 6] = [15, 30, 60, 120, 300, 600];

/// What the memory buys at this length, from the recorder's own formula
/// (`replay::settings::video_kbps`): the video's bitrate, or, where even the
/// lowest bitrate does not fit, how much does.
fn memory_hint(prefs: &Prefs) -> String {
    use crate::replay::settings::{AUDIO_KBPS, Settings};
    let settings = Settings {
        seconds: prefs.replay_seconds,
        memory_mib: prefs.replay_memory_mib,
        bitrate_kbps: prefs.replay_bitrate,
        ..Settings::default()
    };
    let kbps = u64::from(settings.video_kbps());
    let live_bits = prefs.replay_memory_mib as u64 * 1024 * 1024 / 3 * 8;
    // One second stays spare: eviction drops whole one-second GOPs.
    let fits = (live_bits / ((kbps + AUDIO_KBPS) * 1000)).saturating_sub(1);
    if fits < prefs.replay_seconds {
        let time = if fits < 120 {
            format!("{fits}\u{a0}s")
        } else {
            format!("{}\u{a0}min", fits / 60)
        };
        format!("Only about {time} fit, give it more memory")
    } else {
        let mbit = kbps as f64 / 1000.0;
        let rate = if mbit < 10.0 {
            format!("{mbit:.1}")
        } else {
            format!("{mbit:.0}")
        };
        format!("Video at about {rate}\u{a0}Mbit/s")
    }
}

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

/// A dropdown over (value, label) entries writing the chosen value as text;
/// `current` not among them is added under its own name, so the page never
/// shows a value that is not the real one.
fn choose_text(
    name: &'static str,
    mut entries: Vec<(String, String)>,
    current: &str,
    now: Instant,
) -> (Box<dyn Widget>, Bind) {
    if !entries.iter().any(|(value, _)| value == current) {
        entries.push((current.to_owned(), current.to_owned()));
    }
    let chosen = entries
        .iter()
        .position(|(value, _)| value == current)
        .unwrap_or(0);
    let labels = entries.iter().map(|(_, label)| label.clone()).collect();
    let values: Vec<String> = entries.into_iter().map(|(value, _)| value).collect();
    let bind = key(name, move |value| match value {
        Value::Choice(index) => values.get(*index).cloned().map(settings::Value::Text),
        _ => None,
    });
    (Box::new(Dropdown::new(labels, chosen, now)), bind)
}

/// Focused display, each connected monitor, a configured one that is not
/// connected, both: as the tray offers them.
fn display_entries(prefs: &Prefs) -> (Vec<(String, String)>, String) {
    let mut entries = vec![("focused".to_owned(), "Focused display".to_owned())];
    entries.extend(
        prefs
            .displays
            .iter()
            .map(|(name, label)| (format!("output:{name}"), label.clone())),
    );
    if let Display::Output(name) = &prefs.display
        && !prefs
            .displays
            .iter()
            .any(|(connected, _)| connected == name)
    {
        entries.push((format!("output:{name}"), format!("{name} (disconnected)")));
    }
    entries.push(("both".to_owned(), "Both displays".to_owned()));
    let current = match &prefs.display {
        Display::Focused => "focused".to_owned(),
        Display::Both => "both".to_owned(),
        Display::Output(name) => format!("output:{name}"),
    };
    (entries, current)
}

/// Auto, then what gpu-screen-recorder can encode here as `record_codec`
/// takes it (`gsr::choose`): HDR and 10-bit variants it refuses stay out.
fn codec_entries(prefs: &Prefs) -> Vec<(String, String)> {
    let mut entries = vec![("auto".to_owned(), "Auto".to_owned())];
    let label = |base: &str| match base {
        "h264" => Some("H.264"),
        "hevc" => Some("HEVC"),
        "av1" => Some("AV1"),
        _ => None,
    };
    for codec in prefs.codecs.iter().flatten() {
        let entry = if codec == "h264_software" {
            Some(("libx264".to_owned(), "H.264 on the CPU".to_owned()))
        } else if let Some(base) = codec.strip_suffix("_vulkan") {
            label(base).map(|label| (codec.clone(), format!("{label} via Vulkan")))
        } else {
            label(codec).map(|label| (codec.clone(), label.to_owned()))
        };
        entries.extend(entry);
    }
    entries
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
        row(Icon::Monitor, "Display", None, {
            let (entries, current) = display_entries(prefs);
            choose_text("record_default_target", entries, &current, now)
        }),
        row(
            Icon::Columns2,
            // The Display row offers "Both displays"; this one says how.
            "Both displays as",
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
            Icon::Gauge,
            "Frame rate",
            None,
            choose("record_fps", &FRAME_RATES, prefs.fps, fps, fps_value, now),
        ),
        row(
            Icon::Film,
            "Codec",
            None,
            choose_text("record_codec", codec_entries(prefs), &prefs.codec, now),
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
            choose(
                "replay.duration_seconds",
                &REPLAY_SECONDS,
                prefs.replay_seconds,
                |s| duration(Some(s)),
                |s| settings::Value::Int(s as i64),
                now,
            ),
        ),
        row(
            Icon::MemoryStick,
            "Memory",
            Some(&memory_hint(prefs)),
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
