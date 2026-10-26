//! The values the pages show, in plain types, filled by the platform.

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
    /// `ui_font`; unset, the desktop's font.
    pub ui_font: Option<String>,
    /// The installed font families, once known.
    pub fonts: Option<Vec<String>>,
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
