#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub cursor_smoothing: bool,
    pub seconds: u64,
    pub memory_mib: usize,
    pub fps: u32,
    pub encoder: String,
    pub output: Option<String>,
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cursor_smoothing: false,
            seconds: 60,
            memory_mib: 512,
            fps: 60,
            encoder: "auto".into(),
            output: None,
            autostart: false,
        }
    }
}
