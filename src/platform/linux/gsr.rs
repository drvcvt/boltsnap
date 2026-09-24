//! GPU Screen Recorder adapter for ordinary recordings: encoder discovery and argv.

use crate::record::Geometry;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

pub const PROGRAM: &str = "gpu-screen-recorder";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Codec {
    /// gsr `-k` value.
    pub name: String,
    /// Encode on the CPU (`-encoder cpu`).
    pub cpu: bool,
    /// Matching FFmpeg encoder, used when finalize has to re-encode.
    pub encoder: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub vendor: String,
    pub codecs: Vec<String>,
}

pub enum Target<'a> {
    Output(&'a str),
    Region(&'a Geometry),
    /// Several outputs in one stream, as built by `combined_source`.
    Combined(&'a str),
}

/// gsr multi-source capture of `monitors` as laid out by the compositor
/// (`DP-3;x=0;y=0|DP-1;x=1920;y=0`, positions in video pixels). `None` for
/// fewer than two outputs, mixed scales, or a canvas beyond what hardware H.264
/// encodes (gsr refuses to start); those keep one stream per output and are
/// composed on save. `-s` does not help: gsr applies it to each source.
/// Earlier hangs with two outputs came from the Vulkan encoder, not from this.
pub fn combined_source(monitors: &[crate::record::Monitor]) -> Option<String> {
    let scale = monitors.first()?.scale;
    if monitors.len() < 2 || monitors.iter().any(|m| m.scale != scale || scale <= 0.0) {
        return None;
    }
    let min_x = monitors.iter().map(|m| m.x).min()?;
    let min_y = monitors.iter().map(|m| m.y).min()?;
    let place = |v: i32| (f64::from(v) * scale).round() as i64;
    let limit = i64::from(crate::record::finalize::HARDWARE_H264_MAX_SIDE);
    if monitors.iter().any(|m| {
        place(m.x - min_x) + i64::from(m.width) > limit
            || place(m.y - min_y) + i64::from(m.height) > limit
    }) {
        return None;
    }
    Some(
        monitors
            .iter()
            .map(|m| {
                format!(
                    "{};x={};y={}",
                    m.name,
                    place(m.x - min_x),
                    place(m.y - min_y)
                )
            })
            .collect::<Vec<_>>()
            .join("|"),
    )
}

/// Only successful discovery is cached; a failed probe is retried on the next start.
static INFO: Mutex<Option<Info>> = Mutex::new(None);

/// GPU vendor and video codecs gsr can use on this machine (`--info`, about 0.5 s).
pub fn info() -> Result<Info, String> {
    let mut cached = INFO.lock().unwrap();
    if let Some(info) = cached.as_ref() {
        return Ok(info.clone());
    }
    let output = super::replay::process::output(
        Command::new(PROGRAM).arg("--info"),
        Duration::from_secs(5),
    )?;
    if !output.status.success() {
        return Err(format!(
            "{PROGRAM} --info failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let info = parse_info(&String::from_utf8_lossy(&output.stdout));
    *cached = Some(info.clone());
    Ok(info)
}

fn parse_info(text: &str) -> Info {
    let mut info = Info::default();
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("section=") {
            section = name;
        } else if section == "video_codecs" && !line.is_empty() {
            info.codecs.push(line.to_owned());
        } else if let Some(vendor) = line.strip_prefix("vendor|") {
            info.vendor = vendor.to_owned();
        }
    }
    info
}

/// Map `record_codec` to a gsr codec. `auto` picks hardware H.264 only; CPU
/// encoding must be requested explicitly with `libx264`.
pub fn choose(requested: &str, info: &Info) -> Result<Codec, String> {
    let available = &info.codecs;
    let has = |name: &str| available.iter().any(|codec| codec == name);
    let (name, cpu) = match requested {
        "auto" => (
            ["h264", "h264_vulkan"]
                .into_iter()
                .find(|name| has(name))
                .ok_or(
                    "no hardware H.264 encoder is available to gpu-screen-recorder; \
                     set record_codec = \"libx264\" to encode on the CPU",
                )?,
            false,
        ),
        "libx264" => ("h264_software", true),
        "h264_nvenc" | "h264_vaapi" => ("h264", false),
        "hevc_nvenc" | "hevc_vaapi" => ("hevc", false),
        "av1_nvenc" | "av1_vaapi" => ("av1", false),
        "h264" | "hevc" | "av1" => (requested, false),
        name if name.ends_with("_vulkan") => (name, false),
        other => {
            return Err(format!(
                "record_codec {other} is not supported by gpu-screen-recorder"
            ));
        }
    };
    if !has(name) {
        return Err(format!(
            "gpu-screen-recorder cannot encode {requested} on this system (available: {})",
            available.join(", ")
        ));
    }
    let name = name.strip_suffix("_software").unwrap_or(name);
    let encoder = if cpu {
        "libx264".to_owned()
    } else if name.ends_with("_vulkan") {
        name.to_owned()
    } else if info.vendor == "nvidia" {
        format!("{name}_nvenc")
    } else {
        format!("{name}_vaapi")
    };
    Ok(Codec {
        name: name.into(),
        cpu,
        encoder,
    })
}

/// With `plugin` the system pointer is left out and the smooth-cursor plugin
/// draws instead; the first frame's monotonic timestamp goes to `<out>.ts` for
/// aligning the recorded cursor track.
pub fn args(
    target: &Target,
    codec: &Codec,
    fps: u32,
    audio: &[String],
    plugin: Option<&Path>,
    out: &Path,
) -> Vec<String> {
    let source = match target {
        Target::Output(name) => (*name).to_owned(),
        Target::Region(geo) => format!("{}x{}+{}+{}", geo.w, geo.h, geo.x, geo.y),
        Target::Combined(source) => (*source).to_owned(),
    };
    let mut args: Vec<String> = [
        "-w",
        &source,
        "-c",
        "mkv",
        "-f",
        &fps.to_string(),
        "-fm",
        "cfr",
        "-k",
        &codec.name,
        "-q",
        "very_high",
        "-bm",
        "qp",
        "-keyint",
        "2",
        "-cursor",
        if plugin.is_some() { "no" } else { "yes" },
        "-fallback-cpu-encoding",
        "no",
        // Write packets as they come in 100 ms clusters: a gsr that hangs on
        // stop (seen with two instances at 240 FPS) still leaves the recording.
        "-ffmpeg-opts",
        "flush_packets=1;cluster_time_limit=100",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if codec.cpu {
        args.extend(["-encoder".into(), "cpu".into()]);
    }
    if let Some(plugin) = plugin {
        args.extend(["-write-first-frame-ts".into(), "yes".into()]);
        args.extend(["-p".into(), plugin.to_string_lossy().into_owned()]);
    }
    if !audio.is_empty() {
        let sources = audio
            .iter()
            .map(|source| format!("device:{source}"))
            .collect::<Vec<_>>()
            .join("|");
        args.extend(["-a".into(), sources, "-ac".into(), "aac".into()]);
    }
    args.extend(["-o".into(), out.to_string_lossy().into_owned()]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "section=system_info\ndisplay_server|wayland\nsection=gpu_info\nvendor|nvidia\ncard_path|/dev/dri/card0\nsection=video_codecs\nh264_software\nh264_vulkan\nhevc_vulkan\nsection=image_formats\npng\n";

    fn info(vendor: &str, codecs: &[&str]) -> Info {
        Info {
            vendor: vendor.into(),
            codecs: codecs.iter().map(|codec| (*codec).to_owned()).collect(),
        }
    }

    #[test]
    fn info_reads_vendor_and_only_video_codecs() {
        assert_eq!(
            parse_info(INFO),
            info("nvidia", &["h264_software", "h264_vulkan", "hevc_vulkan"])
        );
        assert_eq!(parse_info("section=system_info\n"), Info::default());
    }

    #[test]
    fn auto_prefers_native_hardware_then_vulkan_and_never_the_cpu() {
        let vulkan = choose("auto", &parse_info(INFO)).unwrap();
        assert_eq!(
            (vulkan.name.as_str(), vulkan.encoder.as_str()),
            ("h264_vulkan", "h264_vulkan")
        );
        let nvidia = choose("auto", &info("nvidia", &["h264", "h264_vulkan"])).unwrap();
        assert_eq!(
            (nvidia.name.as_str(), nvidia.encoder.as_str()),
            ("h264", "h264_nvenc")
        );
        let amd = choose("auto", &info("amd", &["h264"])).unwrap();
        assert_eq!(amd.encoder, "h264_vaapi");
        assert!(choose("auto", &info("nvidia", &["h264_software"])).is_err());
    }

    #[test]
    fn explicit_codecs_map_to_gsr_names_without_fallback() {
        let local = parse_info(INFO);
        assert_eq!(
            choose("libx264", &local).unwrap(),
            Codec {
                name: "h264".into(),
                cpu: true,
                encoder: "libx264".into(),
            }
        );
        assert_eq!(choose("hevc_vulkan", &local).unwrap().name, "hevc_vulkan");
        assert!(choose("h264_nvenc", &local).is_err());
        assert!(choose("prores", &local).is_err());
        assert_eq!(
            choose("h264_nvenc", &info("nvidia", &["h264"]))
                .unwrap()
                .name,
            "h264"
        );
    }

    #[test]
    fn session_encoder_round_trips_through_choose() {
        for (vendor, codecs) in [
            ("nvidia", &["h264", "h264_software"][..]),
            ("amd", &["h264"][..]),
            ("nvidia", &["h264_vulkan"][..]),
        ] {
            let info = info(vendor, codecs);
            let first = choose("auto", &info).unwrap();
            assert_eq!(choose(&first.encoder, &info).unwrap(), first);
        }
    }

    fn monitor(name: &str, x: i32, y: i32, scale: f64) -> crate::record::Monitor {
        crate::record::Monitor {
            name: name.into(),
            description: String::new(),
            x,
            y,
            width: 1920,
            height: 1080,
            scale,
            focused: false,
        }
    }

    #[test]
    fn combined_source_places_outputs_in_video_pixels() {
        assert_eq!(
            combined_source(&[monitor("DP-1", 1920, 0, 1.0), monitor("DP-3", 0, 0, 1.0)])
                .as_deref(),
            Some("DP-1;x=1920;y=0|DP-3;x=0;y=0")
        );
        // Logical layout at scale 1.5 becomes physical offsets.
        assert_eq!(
            combined_source(&[monitor("A", -1280, 100, 1.5), monitor("B", 0, 0, 1.5)]).as_deref(),
            Some("A;x=0;y=150|B;x=1920;y=0")
        );
        assert_eq!(combined_source(&[monitor("A", 0, 0, 1.0)]), None);
        assert_eq!(
            combined_source(&[monitor("A", 0, 0, 1.0), monitor("B", 1920, 0, 2.0)]),
            None,
            "mixed scales keep one stream per output"
        );
        let wide = crate::record::Monitor {
            width: 2560,
            height: 1440,
            ..monitor("A", 0, 0, 1.0)
        };
        assert_eq!(
            combined_source(&[wide, monitor("B", 2560, 0, 1.0)]),
            None,
            "a 4480 px canvas is beyond hardware H.264"
        );
    }

    #[test]
    fn args_cover_codec_audio_and_region() {
        let geo = Geometry {
            x: -10,
            y: 20,
            w: 800,
            h: 600,
        };
        let cpu = Codec {
            name: "h264".into(),
            cpu: true,
            encoder: "libx264".into(),
        };
        let region = args(
            &Target::Region(&geo),
            &cpu,
            60,
            &["sink.monitor".into(), "mic".into()],
            Some(Path::new("/opt/libboltsnap_gsr_cursor.so")),
            Path::new("/tmp/seg.mkv"),
        );
        let joined = region.join(" ");
        assert!(joined.starts_with("-w 800x600+-10+20 -c mkv -f 60 -fm cfr -k h264 "));
        assert!(joined.contains("-encoder cpu"));
        assert!(joined.contains("-cursor no"));
        assert!(joined.contains("-write-first-frame-ts yes -p /opt/libboltsnap_gsr_cursor.so"));
        assert!(joined.contains("-a device:sink.monitor|device:mic -ac aac"));
        assert!(joined.ends_with("-o /tmp/seg.mkv"));

        let gpu = Codec {
            name: "h264_vulkan".into(),
            cpu: false,
            encoder: "h264_vulkan".into(),
        };
        let output = args(
            &Target::Output("DP-3"),
            &gpu,
            240,
            &[],
            None,
            Path::new("/tmp/o.mkv"),
        );
        assert_eq!(output[..2], ["-w", "DP-3"]);
        assert!(!output.iter().any(|a| a == "-a" || a == "-encoder"));
        assert!(output.windows(2).any(|w| w == ["-cursor", "yes"]));
        assert!(
            !output
                .iter()
                .any(|a| a == "-write-first-frame-ts" || a == "-p")
        );
    }
}
