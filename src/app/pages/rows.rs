//! Building blocks for rows: a control with the key it writes and how its
//! value becomes the key's, and how values read.

use std::time::Instant;

use super::{Display, Prefs};
use crate::app::{Bind, Block, Encode, Row};
use crate::settings;
use crate::ui::icons::Icon;
use crate::ui::widgets::{Dropdown, Part, Segment, Switch, TextField, Value, Widget};

pub(super) fn row(
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

pub(super) fn key(
    key: &'static str,
    encode: impl Fn(&Value) -> Option<settings::Value> + 'static,
) -> Bind {
    Bind::Key(key, Box::new(encode) as Encode)
}

pub(super) fn int(value: &Value) -> Option<settings::Value> {
    match value {
        Value::Int(n) => Some(settings::Value::Int(*n)),
        _ => None,
    }
}

pub(super) fn boolean(value: &Value) -> Option<settings::Value> {
    match value {
        Value::Bool(on) => Some(settings::Value::Bool(*on)),
        _ => None,
    }
}

pub(super) fn text(value: &Value) -> Option<settings::Value> {
    match value {
        Value::Text(text) => Some(settings::Value::Text(text.clone())),
        _ => None,
    }
}

/// A part of a segment as one of the key's words, in part order.
pub(super) fn word(words: &'static [&'static str]) -> impl Fn(&Value) -> Option<settings::Value> {
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
pub(super) fn choose<T: PartialEq + Copy + 'static>(
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
pub(super) fn choose_text(
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
pub(super) fn display_entries(prefs: &Prefs) -> (Vec<(String, String)>, String) {
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
pub(super) fn codec_entries(prefs: &Prefs) -> Vec<(String, String)> {
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

pub(super) fn fps(fps: u32) -> String {
    format!("{fps}\u{a0}fps")
}

pub(super) fn duration(seconds: Option<u64>) -> String {
    match seconds {
        None => "Off".into(),
        Some(s) if s.is_multiple_of(60) => format!("{}\u{a0}min", s / 60),
        Some(s) => format!("{s}\u{a0}s"),
    }
}

pub(super) fn mebibytes(mib: usize) -> String {
    if mib.is_multiple_of(1024) {
        format!("{}\u{a0}GiB", mib / 1024)
    } else {
        format!("{mib}\u{a0}MiB")
    }
}

pub(super) fn switch(name: &'static str, on: bool, now: Instant) -> (Box<dyn Widget>, Bind) {
    (Box::new(Switch::new(on, now)), key(name, boolean))
}

/// A segment over `words`, one part each, writing the chosen word.
pub(super) fn segment(
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

pub(super) fn folder(name: &'static str, path: &str) -> (Box<dyn Widget>, Bind) {
    (Box::new(TextField::new(path)), key(name, text))
}

pub(super) fn fps_value(fps: u32) -> settings::Value {
    settings::Value::Int(fps.into())
}
