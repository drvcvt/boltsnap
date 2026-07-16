//! Pure helpers for area recording (geometry mapping + wf-recorder argv). The
//! live lifecycle (spawn, overlays, stop) lives in the shelf daemon.

use crate::config::RecordDefaultTarget;
use std::path::Path;

pub mod audio;
pub mod finalize;
pub mod session;

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub focused: bool,
}

pub fn parse_hyprland_monitors(json: &[u8]) -> Result<Vec<Monitor>, String> {
    let values = serde_json::from_slice::<serde_json::Value>(json)
        .map_err(|error| format!("invalid monitor JSON: {error}"))?;
    let values = values
        .as_array()
        .ok_or_else(|| "monitor JSON is not an array".to_string())?;

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let field = |name| {
                value
                    .get(name)
                    .ok_or_else(|| format!("monitor {index} is missing {name}"))
            };
            Ok(Monitor {
                name: field("name")?
                    .as_str()
                    .ok_or_else(|| format!("monitor {index} has invalid name"))?
                    .to_string(),
                description: field("description")?
                    .as_str()
                    .ok_or_else(|| format!("monitor {index} has invalid description"))?
                    .to_string(),
                x: i32::try_from(
                    field("x")?
                        .as_i64()
                        .ok_or_else(|| format!("monitor {index} has invalid x"))?,
                )
                .map_err(|_| format!("monitor {index} x is out of range"))?,
                y: i32::try_from(
                    field("y")?
                        .as_i64()
                        .ok_or_else(|| format!("monitor {index} has invalid y"))?,
                )
                .map_err(|_| format!("monitor {index} y is out of range"))?,
                width: u32::try_from(
                    field("width")?
                        .as_u64()
                        .ok_or_else(|| format!("monitor {index} has invalid width"))?,
                )
                .map_err(|_| format!("monitor {index} width is out of range"))?,
                height: u32::try_from(
                    field("height")?
                        .as_u64()
                        .ok_or_else(|| format!("monitor {index} has invalid height"))?,
                )
                .map_err(|_| format!("monitor {index} height is out of range"))?,
                scale: field("scale")?
                    .as_f64()
                    .ok_or_else(|| format!("monitor {index} has invalid scale"))?,
                focused: field("focused")?
                    .as_bool()
                    .ok_or_else(|| format!("monitor {index} has invalid focused"))?,
            })
        })
        .collect()
}

pub fn resolve_record_outputs(
    target: &RecordDefaultTarget,
    monitors: &[Monitor],
) -> Result<(Vec<Monitor>, Option<String>), String> {
    if monitors.is_empty() {
        return Err("no connected outputs".into());
    }
    let focused = || {
        monitors
            .iter()
            .find(|monitor| monitor.focused)
            .cloned()
            .ok_or_else(|| "no focused output".to_string())
    };

    match target {
        RecordDefaultTarget::Focused => Ok((vec![focused()?], None)),
        RecordDefaultTarget::Output(name) => match monitors
            .iter()
            .find(|monitor| monitor.name == *name)
            .cloned()
        {
            Some(monitor) => Ok((vec![monitor], None)),
            None => Ok((
                vec![focused()?],
                Some(format!(
                    "configured output {name} is disconnected; using focused output"
                )),
            )),
        },
        RecordDefaultTarget::Both if monitors.len() == 1 => Ok((
            vec![monitors[0].clone()],
            Some("only one output is connected; recording that output".into()),
        )),
        RecordDefaultTarget::Both => Ok((monitors.to_vec(), None)),
    }
}

/// A recording region in compositor-global (logical) coordinates, as wf-recorder's
/// `-g` expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Geometry {
    /// wf-recorder `-g` string: "X,Y WxH".
    pub fn to_arg(&self) -> String {
        format!("{},{} {}x{}", self.x, self.y, self.w, self.h)
    }
}

/// Map a selection rect (logical surface px, origin = the overlay's output) to
/// global coords by adding the focused output's layout origin. Both are logical
/// (Hyprland monitor `x`/`y` are logical), so no scale division is needed.
pub fn to_global_geometry(
    rect_x: f64,
    rect_y: f64,
    rect_w: f64,
    rect_h: f64,
    output_x: i32,
    output_y: i32,
) -> Geometry {
    Geometry {
        x: output_x + rect_x.round() as i32,
        y: output_y + rect_y.round() as i32,
        w: rect_w.round().max(1.0) as u32,
        h: rect_h.round().max(1.0) as u32,
    }
}

/// Build the wf-recorder argv (excluding the program name) for a region recording.
pub fn wf_recorder_args(
    geo: &Geometry,
    codec: &str,
    audio_source: Option<&str>,
    out: &Path,
) -> Vec<String> {
    let mut args = vec!["-g".into(), geo.to_arg(), "-c".into(), codec.into()];
    args.extend(capture_profile_args(codec));
    if let Some(source) = audio_source {
        args.push(format!("--audio={source}"));
    }
    args.extend(["-f".into(), out.to_string_lossy().into_owned()]);
    args
}

/// Build the wf-recorder argv (excluding the program name) to record an entire
/// output (monitor) by name — used for fullscreen recording.
pub fn wf_recorder_output_args(
    output: &str,
    codec: &str,
    audio_source: Option<&str>,
    out: &Path,
) -> Vec<String> {
    let mut args = vec!["-o".into(), output.into(), "-c".into(), codec.into()];
    args.extend(capture_profile_args(codec));
    if let Some(source) = audio_source {
        args.push(format!("--audio={source}"));
    }
    args.extend(["-f".into(), out.to_string_lossy().into_owned()]);
    args
}

fn capture_profile_args(codec: &str) -> Vec<String> {
    let mut args = vec!["-r".into(), "240".into()];
    if codec.ends_with("_nvenc") {
        for option in ["preset=p5", "tune=hq", "rc=vbr", "cq=16"] {
            args.extend(["-p".into(), option.into()]);
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const MONITORS: &[u8] = br#"[
      {"name":"DP-3","description":"BenQ","x":0,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true},
      {"name":"DP-1","description":"AOC","x":2560,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false}
    ]"#;

    #[test]
    fn focused_default_resolves_focused_output() {
        let monitors = parse_hyprland_monitors(MONITORS).unwrap();
        let (resolved, notice) =
            resolve_record_outputs(&RecordDefaultTarget::Focused, &monitors).unwrap();
        assert_eq!(resolved[0].name, "DP-3");
        assert_eq!(notice, None);
    }

    #[test]
    fn named_default_resolves_connected_output() {
        let monitors = parse_hyprland_monitors(MONITORS).unwrap();
        let (resolved, notice) =
            resolve_record_outputs(&RecordDefaultTarget::Output("DP-1".into()), &monitors).unwrap();
        assert_eq!(resolved[0].name, "DP-1");
        assert_eq!(notice, None);
    }

    #[test]
    fn disconnected_default_falls_back_to_focused_output() {
        let monitors = parse_hyprland_monitors(MONITORS).unwrap();
        let (resolved, notice) =
            resolve_record_outputs(&RecordDefaultTarget::Output("HDMI-A-9".into()), &monitors)
                .unwrap();
        assert_eq!(
            resolved
                .iter()
                .map(|monitor| monitor.name.as_str())
                .collect::<Vec<_>>(),
            vec!["DP-3"]
        );
        assert!(notice.unwrap().contains("HDMI-A-9"));
    }

    #[test]
    fn both_keeps_connected_output_order() {
        let monitors = parse_hyprland_monitors(MONITORS).unwrap();
        let (resolved, notice) =
            resolve_record_outputs(&RecordDefaultTarget::Both, &monitors).unwrap();
        assert_eq!(
            resolved
                .iter()
                .map(|monitor| monitor.name.as_str())
                .collect::<Vec<_>>(),
            vec!["DP-3", "DP-1"]
        );
        assert_eq!(notice, None);
    }

    #[test]
    fn both_with_one_monitor_reports_fallback() {
        let monitors = parse_hyprland_monitors(MONITORS).unwrap();
        let (resolved, notice) =
            resolve_record_outputs(&RecordDefaultTarget::Both, &monitors[..1]).unwrap();
        assert_eq!(resolved[0].name, "DP-3");
        assert!(notice.unwrap().contains("one"));
    }

    #[test]
    fn global_geometry_adds_output_origin() {
        let g = to_global_geometry(10.0, 20.0, 800.4, 600.6, 2560, 0);
        assert_eq!(
            g,
            Geometry {
                x: 2570,
                y: 20,
                w: 800,
                h: 601
            }
        );
        assert_eq!(g.to_arg(), "2570,20 800x601");
    }

    #[test]
    fn wf_args_shape() {
        let g = Geometry {
            x: 0,
            y: 0,
            w: 1280,
            h: 720,
        };
        let args = wf_recorder_args(&g, "h264_nvenc", None, &PathBuf::from("/tmp/r.mp4"));
        assert_eq!(
            args,
            vec![
                "-g",
                "0,0 1280x720",
                "-c",
                "h264_nvenc",
                "-r",
                "240",
                "-p",
                "preset=p5",
                "-p",
                "tune=hq",
                "-p",
                "rc=vbr",
                "-p",
                "cq=16",
                "-f",
                "/tmp/r.mp4"
            ]
        );
    }

    #[test]
    fn wf_output_args_shape() {
        let args =
            wf_recorder_output_args("DP-1", "h264_nvenc", None, &PathBuf::from("/tmp/r.mp4"));
        assert_eq!(
            args,
            vec![
                "-o",
                "DP-1",
                "-c",
                "h264_nvenc",
                "-r",
                "240",
                "-p",
                "preset=p5",
                "-p",
                "tune=hq",
                "-p",
                "rc=vbr",
                "-p",
                "cq=16",
                "-f",
                "/tmp/r.mp4"
            ]
        );
    }

    #[test]
    fn wf_recorder_area_adds_selected_audio_source() {
        let args = wf_recorder_args(
            &Geometry {
                x: 1,
                y: 2,
                w: 3,
                h: 4,
            },
            "libx264",
            Some("desk.monitor"),
            Path::new("/tmp/out.mp4"),
        );
        assert!(args.iter().any(|arg| arg == "--audio=desk.monitor"));
    }

    #[test]
    fn wf_recorder_output_without_audio_keeps_previous_arguments() {
        let args = wf_recorder_output_args("DP-3", "libx264", None, Path::new("/tmp/out.mp4"));
        assert!(!args.iter().any(|arg| arg.starts_with("--audio")));
    }
}
