//! The values the pages show, in plain types, filled by the platform.

/// The values the pages show, in plain types.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub card_width: u32,
    pub visible_cards: u32,
    pub corner: Corner,
    /// `shelf.card_opacity`, percent.
    pub card_opacity: u32,
    pub solid_on_hover: bool,
    pub card_shadow: bool,
    pub card_animations: bool,
    /// `[shelf] hover_fade`.
    pub card_hover_fade: bool,
    pub save_dir: String,
    /// `copy_to_clipboard`: screenshots also go to the clipboard.
    pub copy_to_clipboard: bool,
    /// `selector_magnifier = "always"`.
    pub magnifier_always: bool,
    /// `[shelf] open_with`, unset the default app.
    pub open_with: Option<String>,
    /// The editors installed here, as (name in the file, how people know it).
    pub editors: Vec<(String, String)>,
    pub both_combined: bool,
    pub display: Display,
    /// The connected monitors.
    pub displays: Vec<Screen>,
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
    /// The desktop has a folder dialog (the FileChooser portal).
    pub can_browse: bool,
    /// `ui_font`; unset, the desktop's font.
    pub ui_font: Option<String>,
    /// The installed font families, once known.
    pub fonts: Option<Vec<String>>,
    pub replay_seconds: u64,
    pub replay_memory_mib: usize,
    /// `replay.bitrate_kbps`; unset, the bitrate follows length and memory.
    pub replay_bitrate: Option<u32>,
    pub replay_fps: u32,
    /// `[replay] output`; unset, like recordings.
    pub replay_output: Option<String>,
    /// `[replay] encoder`, `auto` when unset.
    pub replay_encoder: String,
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

/// Makers as monitors report them (EDID's PNP codes) and as people call them.
const MAKERS: [(&str, &str); 11] = [
    ("BNQ", "BenQ"),
    ("GSM", "LG"),
    ("DEL", "Dell"),
    ("SAM", "Samsung"),
    ("ACR", "Acer"),
    ("AUS", "ASUS"),
    ("HWP", "HP"),
    ("LEN", "Lenovo"),
    ("PHL", "Philips"),
    ("GBT", "Gigabyte"),
    ("ENC", "EIZO"),
];

/// A monitor's name as people know it: maker and model ("BenQ ZOWIE XL
/// LCD"), the connector when it reports neither.
pub fn monitor_name(make: &str, model: &str, connector: &str) -> String {
    fn known(part: &str) -> Option<&str> {
        let part = part.trim();
        (!part.is_empty() && !part.eq_ignore_ascii_case("unknown")).then_some(part)
    }
    let make = known(make).map(|make| {
        MAKERS
            .iter()
            .find(|(code, _)| code.eq_ignore_ascii_case(make))
            .map_or(make, |(_, name)| name)
    });
    match (make, known(model)) {
        (Some(make), Some(model))
            if model
                .get(..make.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(make)) =>
        {
            model.to_owned()
        }
        (Some(make), Some(model)) => format!("{make} {model}"),
        (Some(only), None) | (None, Some(only)) => only.to_owned(),
        (None, None) => connector.to_owned(),
    }
}

/// A connected monitor: its connector, its name as people know it
/// ([`monitor_name`]), and where it sits in the compositor's layout (logical
/// pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Screen {
    /// In the Display list: the name, then the connector, so two monitors
    /// of one model stay apart.
    pub fn entry(&self) -> String {
        if self.label == self.name {
            self.label.clone()
        } else {
            format!("{} ({})", self.label, self.name)
        }
    }
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
