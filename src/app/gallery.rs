//! A stress fixture, not a page anyone sees: every control enabled and
//! disabled, with the longest strings `/ui` asks for (a 200 character path,
//! a compound word over 60 characters, a long list entry). Tests run the
//! gates over it; `boltsnap settings --render PAGE OUT --fixture` draws it.

use std::time::Instant;

use super::{Bind, Block, Page, Row};
use crate::ui::icons::Icon;
use crate::ui::theme;
use crate::ui::widgets::{
    Dropdown, IconButton, Part, Segment, Slider, Stepper, Switch, TextField, Widget,
};

pub const LONG_PATH: &str = "/home/someone/Videos/boltsnap/recordings/2026/october/\
     a-folder-name-that-keeps-going-because-people-name-folders-like-this/\
     and-then-one-more-level-for-good-measure/final";
pub const COMPOUND: &str = "Bildschirmaufnahmeeinstellungsänderungsbenachrichtigungsfenster";

fn row(icon: Icon, label: &str, hint: Option<&str>, control: Box<dyn Widget>) -> Block {
    Block::Row(Row {
        icon,
        label: label.to_owned(),
        hint: hint.map(str::to_owned),
        control,
        enabled: true,
        bind: Bind::None,
        reset: false,
    })
}

fn controls(now: Instant) -> Vec<Block> {
    let list = || {
        Box::new(Dropdown::new(
            vec![
                "Focused display".into(),
                "DP-1 (disconnected)".into(),
                "Both displays, side by side".into(),
            ],
            0,
            now,
        ))
    };
    vec![
        row(
            Icon::Volume2,
            "Audio",
            Some("Applies to the next recording"),
            Box::new(Switch::new(true, now)),
        ),
        row(
            Icon::Mic,
            "Source",
            None,
            Box::new(Segment::new(
                vec![
                    Part::text("System and mic"),
                    Part::text("Mic"),
                    Part::text("System"),
                ],
                0,
                now,
            )),
        ),
        row(
            Icon::Scan,
            "Corner",
            None,
            Box::new(Segment::new(
                vec![
                    Part::icon(Icon::ArrowUpLeft),
                    Part::icon(Icon::ArrowUpRight),
                    Part::icon(Icon::ArrowDownLeft),
                    Part::icon(Icon::ArrowDownRight),
                ],
                2,
                now,
            )),
        ),
        row(
            Icon::Scaling,
            "Card size",
            None,
            Box::new(Slider::new(190, (140, 380, 10), "px")),
        ),
        row(
            Icon::GalleryVerticalEnd,
            "Cards shown",
            None,
            Box::new(Stepper::new(5, 1, 8, now)),
        ),
        row(Icon::Monitor, "Display", None, list()),
        row(
            Icon::Folder,
            COMPOUND,
            Some(LONG_PATH),
            Box::new(TextField::new(LONG_PATH)),
        ),
        row(
            Icon::RotateCcw,
            "Reset to default",
            None,
            Box::new(IconButton::new(Icon::RotateCcw)),
        ),
    ]
}

fn disabled(blocks: Vec<Block>) -> Vec<Block> {
    blocks
        .into_iter()
        .map(|block| match block {
            Block::Row(row) => Block::Row(Row {
                enabled: false,
                ..row
            }),
            other => other,
        })
        .collect()
}

pub fn pages(now: Instant) -> Vec<Page> {
    let mut first = vec![
        Block::Banner {
            icon: Icon::TriangleAlert,
            text: format!("config.toml cannot be read: {LONG_PATH}"),
            color: theme::ERROR,
        },
        Block::Heading(COMPOUND.into()),
    ];
    first.extend(controls(now));
    let mut second = vec![Block::Heading("Disabled".into())];
    second.extend(disabled(controls(now)));
    vec![
        Page {
            name: "Controls".into(),
            icon: Icon::Layers,
            blocks: first,
        },
        Page {
            name: COMPOUND.into(),
            icon: Icon::Power,
            blocks: second,
        },
    ]
}
