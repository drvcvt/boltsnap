//! The settings pages with the values in effect (settings plan, table under
//! "Aufbau"). The platform fills [`Prefs`] from the config; every row is
//! bound to the key it writes (Phase 5b).

use std::time::Instant;

mod prefs;
mod rows;

pub use prefs::{AudioSource, Corner, Cursor, Display, Notice, Prefs};

use rows::{
    choose, choose_text, codec_entries, display_entries, duration, folder, fps, fps_value, int,
    key, mebibytes, row, segment, switch,
};

use super::{Bind, Block, Page};
use crate::settings;
use crate::ui::icons::Icon;
use crate::ui::theme;
use crate::ui::widgets::{Dropdown, IconButton, Part, Slider, Stepper, Switch};

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

fn shelf(prefs: &Prefs, now: Instant) -> Page {
    let corners = [
        (Corner::TopLeft, Icon::ShelfTopLeft),
        (Corner::TopRight, Icon::ShelfTopRight),
        (Corner::BottomLeft, Icon::ShelfBottomLeft),
        (Corner::BottomRight, Icon::ShelfBottomRight),
    ];
    let corner = corners
        .iter()
        .position(|(c, _)| *c == prefs.corner)
        .unwrap_or(2);
    Page {
        name: "Shelf".into(),
        icon: Icon::Layers,
        blocks: vec![
            Block::Preview(super::Preview::Shelf),
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

fn screenshots(prefs: &Prefs, now: Instant) -> Page {
    Page {
        name: "Screenshots".into(),
        icon: Icon::Image,
        blocks: vec![
            Block::Heading("Saving".into()),
            row(
                Icon::Folder,
                "Save folder",
                None,
                folder("save_dir", &prefs.save_dir, prefs.can_browse, now),
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
            folder("record_dir", &prefs.record_dir, prefs.can_browse, now),
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

/// The face the shelf, the selector, the recording controls and this window
/// use: the desktop's, or an installed family.
fn general(prefs: &Prefs, now: Instant) -> Page {
    let mut entries: Vec<(Option<String>, String)> = vec![(None, "Desktop font".into())];
    entries.extend(
        prefs
            .fonts
            .iter()
            .flatten()
            .map(|family| (Some(family.clone()), family.clone())),
    );
    if let Some(font) = &prefs.ui_font
        && !entries
            .iter()
            .any(|(value, _)| value.as_ref() == Some(font))
    {
        entries.push((Some(font.clone()), font.clone()));
    }
    let chosen = entries
        .iter()
        .position(|(value, _)| *value == prefs.ui_font)
        .unwrap_or(0);
    let labels = entries.iter().map(|(_, label)| label.clone()).collect();
    let values = entries
        .into_iter()
        .map(|(value, _)| value.map(settings::Value::Text))
        .collect();
    Page {
        name: "General".into(),
        icon: Icon::Settings,
        blocks: vec![
            Block::Heading("Look".into()),
            row(
                Icon::Type,
                "Font",
                Some("For the shelf, the selector and this window"),
                (
                    Box::new(Dropdown::new(labels, chosen, now)),
                    Bind::Choices {
                        key: "ui_font",
                        values,
                    },
                ),
            ),
        ],
    }
}

/// The pages, each opening with `notice` when there is one; a locked notice
/// turns every row off.
pub fn pages(prefs: &Prefs, notice: Option<&Notice>, now: Instant) -> Vec<Page> {
    let mut pages = vec![
        shelf(prefs, now),
        screenshots(prefs, now),
        recording(prefs, now),
        replay(prefs, now),
        general(prefs, now),
    ];
    for block in pages.iter_mut().flat_map(|page| &mut page.blocks) {
        if let Block::Row(row) = block
            && let Bind::Key(key, _) | Bind::Choices { key, .. } = &row.bind
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
