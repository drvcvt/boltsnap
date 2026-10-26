#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub seconds: u64,
    pub memory_mib: usize,
    pub fps: u32,
    pub encoder: String,
    pub output: Option<String>,
    pub autostart: bool,
    /// Constant video bitrate; unset, `video_kbps` sizes it to the buffer.
    pub bitrate_kbps: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            seconds: 60,
            memory_mib: 512,
            fps: 60,
            encoder: "auto".into(),
            output: None,
            autostart: false,
            bitrate_kbps: None,
        }
    }
}

/// gsr's AAC track, `-ab 128`.
pub const AUDIO_KBPS: u64 = 128;
const MIN_VIDEO_KBPS: u64 = 2_000;
const MAX_VIDEO_KBPS: u64 = 80_000;

impl Settings {
    /// Video bitrate for gsr's constant bitrate mode. Unset, it fills the
    /// worker's live third of `memory_mib` with `seconds` of video and audio,
    /// one spare second because eviction drops whole one-second GOPs, and a
    /// tenth for rate-control bursts.
    /// How many seconds the live buffer holds at `video_kbps`, one spare
    /// second taken off (eviction drops whole one-second GOPs).
    pub fn seconds_held(&self) -> u64 {
        let live_bits = self.memory_mib as u64 * 1024 * 1024 / 3 * 8;
        let kbps = u64::from(self.video_kbps()) + AUDIO_KBPS;
        (live_bits / (kbps * 1000)).saturating_sub(1)
    }

    pub fn video_kbps(&self) -> u32 {
        if let Some(kbps) = self.bitrate_kbps {
            return kbps;
        }
        let live_bits = self.memory_mib as u64 * 1024 * 1024 / 3 * 8;
        let per_second_kbps = live_bits / (self.seconds + 1) / 1000;
        (per_second_kbps * 9 / 10)
            .saturating_sub(AUDIO_KBPS)
            .clamp(MIN_VIDEO_KBPS, MAX_VIDEO_KBPS) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bitrate_fills_the_live_buffer_without_overrunning_it() {
        let settings = Settings::default();
        let kbps = u64::from(settings.video_kbps());
        assert_eq!(kbps, 20_994);
        let live = settings.memory_mib as u64 * 1024 * 1024 / 3;
        let needed = (kbps + AUDIO_KBPS) * 1000 / 8 * (settings.seconds + 1);
        assert!(needed < live, "{needed} >= {live}");
    }

    #[test]
    fn explicit_and_extreme_bitrates() {
        let explicit = Settings {
            bitrate_kbps: Some(12_000),
            ..Settings::default()
        };
        assert_eq!(explicit.video_kbps(), 12_000);
        let long = Settings {
            seconds: 600,
            memory_mib: 64,
            ..Settings::default()
        };
        assert_eq!(long.video_kbps(), 2_000);
        let short = Settings {
            seconds: 5,
            memory_mib: 4096,
            ..Settings::default()
        };
        assert_eq!(short.video_kbps(), 80_000);
    }
}
