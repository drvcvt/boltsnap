//! Test pages for Phase 4: every control in every state, and the type
//! scale. Values stay in the window; nothing is saved.

use std::time::Instant;

use super::{Block, Page, Row};
use crate::ui::icons::Icon;
use crate::ui::theme::{self, TextStyle};
use crate::ui::widgets::{
    Dropdown, IconButton, Part, Segment, Slider, Stepper, Switch, TextField, Widget,
};

fn row(icon: Icon, label: &str, hint: Option<&str>, control: Box<dyn Widget>) -> Block {
    Block::Row(Row {
        icon,
        label: label.to_owned(),
        hint: hint.map(str::to_owned),
        control,
        enabled: true,
    })
}

fn disabled(block: Block) -> Block {
    match block {
        Block::Row(row) => Block::Row(Row {
            enabled: false,
            ..row
        }),
        other => other,
    }
}

fn frame_rates(now: Instant) -> Box<Dropdown> {
    let items = ["30 fps", "60 fps", "120 fps", "144 fps", "240 fps"];
    Box::new(Dropdown::new(
        items.iter().map(|item| item.to_string()).collect(),
        2,
        now,
    ))
}

fn corners(now: Instant) -> Box<Segment> {
    Box::new(Segment::new(
        vec![
            Part::icon(Icon::ArrowUpLeft),
            Part::icon(Icon::ArrowUpRight),
            Part::icon(Icon::ArrowDownLeft),
            Part::icon(Icon::ArrowDownRight),
        ],
        2,
        now,
    ))
}

fn widgets_page(now: Instant) -> Page {
    let both = || {
        Box::new(Segment::new(
            vec![Part::text("Separate"), Part::text("Combined")],
            0,
            now,
        ))
    };
    let blocks = vec![
        Block::Banner {
            icon: Icon::TriangleAlert,
            text: "config.toml has an error on line 3; showing the last good values".into(),
            color: theme::ERROR,
        },
        Block::Heading("Toggles".into()),
        row(
            Icon::Volume2,
            "Audio",
            Some("Applies to the next recording"),
            Box::new(Switch::new(true, now)),
        ),
        row(Icon::Monitor, "Both displays", None, both()),
        row(Icon::LayoutGrid, "Position", None, corners(now)),
        Block::Heading("Values".into()),
        row(
            Icon::Scaling,
            "Card size",
            None,
            Box::new(Slider::new(190, (140, 380, 10), "px")),
        ),
        row(
            Icon::Layers,
            "Cards shown",
            None,
            Box::new(Stepper::new(5, 1, 8)),
        ),
        row(Icon::Gauge, "Frame rate", None, frame_rates(now)),
        Block::Heading("Text".into()),
        row(
            Icon::Folder,
            "Save folder",
            Some("Will be created"),
            Box::new(TextField::new("~/Pictures/boltsnap", 240.0)),
        ),
        row(
            Icon::FolderOpen,
            "Choose folder",
            None,
            Box::new(IconButton::new(Icon::FolderOpen)),
        ),
        row(
            Icon::RotateCcw,
            "Reset to default",
            None,
            Box::new(IconButton::new(Icon::RotateCcw)),
        ),
        Block::Heading("Disabled".into()),
        disabled(row(
            Icon::Volume2,
            "Audio",
            Some("Daemon not running"),
            Box::new(Switch::new(true, now)),
        )),
        disabled(row(Icon::Monitor, "Both displays", None, both())),
        disabled(row(
            Icon::Scaling,
            "Card size",
            None,
            Box::new(Slider::new(300, (140, 380, 10), "px")),
        )),
        disabled(row(
            Icon::Layers,
            "Cards shown",
            None,
            Box::new(Stepper::new(1, 1, 8)),
        )),
        disabled(row(Icon::Gauge, "Frame rate", None, frame_rates(now))),
        disabled(row(
            Icon::Folder,
            "Save folder",
            None,
            Box::new(TextField::new("~/Pictures/boltsnap", 240.0)),
        )),
        disabled(row(
            Icon::RotateCcw,
            "Reset to default",
            None,
            Box::new(IconButton::new(Icon::RotateCcw)),
        )),
    ];
    Page {
        name: "Widgets".into(),
        icon: Icon::LayoutGrid,
        blocks,
    }
}

fn text_page() -> Page {
    let specimen = |style, note: &str| Block::Specimen {
        style,
        text: "Shelf cards and recordings 0123456789".into(),
        note: note.into(),
    };
    Page {
        name: "Text".into(),
        icon: Icon::Type,
        blocks: vec![
            Block::Heading("Type scale".into()),
            specimen(TextStyle::Title, "Title 15/600"),
            specimen(TextStyle::Label, "Label 13/500"),
            specimen(TextStyle::Heading, "Heading 13/600"),
            specimen(TextStyle::Hint, "Hint 12/500"),
            specimen(TextStyle::Key, "Key 11/500"),
            Block::Heading("Cut to fit".into()),
            Block::Specimen {
                style: TextStyle::Label,
                text: "/home/someone/Videos/boltsnap/recordings/2026/october/a folder name long \
                       enough that it has to end in an ellipsis before the note"
                    .into(),
                note: "Label".into(),
            },
            Block::Banner {
                icon: Icon::Info,
                text: "Values on these pages stay in this window".into(),
                color: theme::SUB,
            },
        ],
    }
}

pub fn pages(now: Instant) -> Vec<Page> {
    vec![widgets_page(now), text_page()]
}
