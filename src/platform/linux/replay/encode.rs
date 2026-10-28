//! What replay's gsr gets beyond its own encoder flags (replay memory plan,
//! parts 2 and 3). Only NVENC: Vulkan and VAAPI name these options
//! differently and keep gsr's CBR.

/// gsr's `-ffmpeg-video-opts`. They overwrite gsr's own encoder options, so
/// on NVENC `rc=vbr` with a target quality replaces CBR while maxrate and
/// bufsize stay at the rate: a calm picture takes a few Mbit/s, full motion
/// the same as CBR. Two surfaces keep one frame encoding and one in flight;
/// FFmpeg's default four cost ~25 MB more driver memory at 3840 × 1080.
pub(super) fn video_opts(encoder: &str) -> String {
    let mut opts = "bf=0;flags=+cgop".to_owned();
    if nvenc(encoder) {
        opts.push_str(";rc=vbr;cq=19;b=0;surfaces=2");
    }
    opts
}

/// gsr's environment. CUDA sets up eight hardware queues, each with command
/// buffers in host memory; gsr encodes through one stream, so a single queue
/// saves ~40 MB.
pub(super) fn env(encoder: &str) -> &'static [(&'static str, &'static str)] {
    if nvenc(encoder) {
        &[("CUDA_DEVICE_MAX_CONNECTIONS", "1")]
    } else {
        &[]
    }
}

fn nvenc(encoder: &str) -> bool {
    encoder.ends_with("_nvenc")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_nvenc_trades_constant_bitrate_for_quality_and_a_lean_session() {
        let nvenc = "bf=0;flags=+cgop;rc=vbr;cq=19;b=0;surfaces=2";
        assert_eq!(video_opts("h264_nvenc"), nvenc);
        assert_eq!(video_opts("av1_nvenc"), nvenc);
        assert_eq!(env("hevc_nvenc"), [("CUDA_DEVICE_MAX_CONNECTIONS", "1")]);
        for other in ["h264_vulkan", "hevc_vaapi"] {
            assert_eq!(video_opts(other), "bf=0;flags=+cgop");
            assert!(env(other).is_empty());
        }
    }
}
