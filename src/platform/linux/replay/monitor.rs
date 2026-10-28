//! Which monitor a replay session captures, or both side by side on one
//! canvas (replay plan, `docs/plans/replay/2026-10-28-replay-both-displays.md`).

use serde_json::{Value, json};

use crate::config::RecordDefaultTarget;
use crate::record::{Monitor, resolve_record_outputs};

/// `[replay] output` for every display on one canvas.
pub(crate) const BOTH: &str = "both";

/// What a replay session captures.
pub(super) enum Capture {
    One(Monitor),
    /// The monitors on one canvas, gsr's source for them, and where each
    /// lies on it.
    Both {
        monitors: Vec<Monitor>,
        source: String,
        layout: Vec<Placed>,
    },
}

impl Capture {
    /// gsr's `-w`.
    pub(super) fn source(&self) -> String {
        match self {
            Capture::One(monitor) => monitor.name.clone(),
            Capture::Both { source, .. } => source.clone(),
        }
    }

    /// The session's output as the status names it: the monitor, or `both`.
    pub(super) fn output(&self) -> String {
        match self {
            Capture::One(monitor) => monitor.name.clone(),
            Capture::Both { .. } => BOTH.into(),
        }
    }

    pub(super) fn layout(&self) -> Vec<Placed> {
        match self {
            Capture::One(_) => Vec::new(),
            Capture::Both { layout, .. } => layout.clone(),
        }
    }
}

/// Where a monitor lies on the canvas, in video pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Placed {
    pub(crate) fn to_value(&self) -> Value {
        json!({"name": self.name, "x": self.x, "y": self.y, "width": self.width, "height": self.height})
    }

    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let number = |key: &str| value[key].as_u64().and_then(|n| u32::try_from(n).ok());
        Some(Self {
            name: value["name"].as_str()?.to_owned(),
            x: number("x")?,
            y: number("y")?,
            width: number("width")?,
            height: number("height")?,
        })
    }
}

impl Placed {
    /// The crop of the canvas that is this monitor, as a save request takes it.
    pub(crate) fn crop_value(&self) -> Value {
        json!({"x": self.x, "y": self.y, "width": self.width, "height": self.height})
    }
}

/// The monitor `name` on the canvas, else the first.
pub(crate) fn chosen<'a>(layout: &'a [Placed], name: Option<&str>) -> Option<&'a Placed> {
    name.and_then(|name| layout.iter().find(|p| p.name == name))
        .or_else(|| layout.first())
}

/// The canvas positions `gsr::combined_source` gives the monitors (same
/// formula: their layout offset times their shared scale), with their sizes.
fn layout(monitors: &[Monitor]) -> Vec<Placed> {
    let scale = monitors.first().map_or(1.0, |m| m.scale);
    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    let place = |v: i32| (f64::from(v) * scale).round() as u32;
    monitors
        .iter()
        .map(|m| Placed {
            name: m.name.clone(),
            x: place(m.x - min_x),
            y: place(m.y - min_y),
            width: m.width,
            height: m.height,
        })
        .collect()
}

/// `replay_monitor`, and `"both"`: every monitor on one canvas when there
/// are two or more at one scale (gsr places them by their layout); else the
/// focused one, with a notice why.
pub(super) fn replay_capture(
    explicit: Option<&str>,
    inherited: &RecordDefaultTarget,
    monitors: &[Monitor],
) -> Result<(Capture, Option<String>), String> {
    if explicit != Some(BOTH) {
        return replay_monitor(explicit, inherited, monitors)
            .map(|(monitor, notice)| (Capture::One(monitor), notice));
    }
    if let Some(source) = super::super::gsr::combined_source(monitors) {
        return Ok((
            Capture::Both {
                monitors: monitors.to_vec(),
                source,
                layout: layout(monitors),
            },
            None,
        ));
    }
    let (monitor, _) = replay_monitor(None, &RecordDefaultTarget::Focused, monitors)?;
    let why = if monitors.len() < 2 {
        "only one display is connected"
    } else {
        "the displays have different scales"
    };
    let notice = format!(
        "Replay on both displays: {why}, so it captures {}",
        monitor.name
    );
    Ok((Capture::One(monitor), Some(notice)))
}

/// An explicit `[replay] output` is strict. Without one, replay follows the
/// recording target: a named output falls back to the focused one with a
/// notice, as recordings do, and any other target captures the focused one.
pub(super) fn replay_monitor(
    explicit: Option<&str>,
    inherited: &RecordDefaultTarget,
    monitors: &[Monitor],
) -> Result<(Monitor, Option<String>), String> {
    if let Some(name) = explicit {
        return monitors
            .iter()
            .find(|monitor| monitor.name == name)
            .map(|monitor| (monitor.clone(), None))
            .ok_or_else(|| format!("configured replay monitor {name} is unavailable"));
    }
    let target = match inherited {
        RecordDefaultTarget::Output(name) => RecordDefaultTarget::Output(name.clone()),
        _ => RecordDefaultTarget::Focused,
    };
    let (outputs, notice) = resolve_record_outputs(&target, monitors)?;
    let monitor = outputs.into_iter().next().ok_or("no connected outputs")?;
    Ok((monitor, notice))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, focused: bool) -> Monitor {
        Monitor {
            name: name.into(),
            description: String::new(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused,
        }
    }

    fn pick(
        explicit: Option<&str>,
        inherited: RecordDefaultTarget,
        monitors: &[Monitor],
    ) -> Result<(String, Option<String>), String> {
        replay_monitor(explicit, &inherited, monitors).map(|(m, notice)| (m.name, notice))
    }

    #[test]
    fn an_explicit_monitor_is_strict() {
        let monitors = [monitor("DP-1", false), monitor("DP-3", true)];
        let focused = RecordDefaultTarget::Focused;
        assert_eq!(
            pick(Some("DP-1"), focused.clone(), &monitors),
            Ok(("DP-1".into(), None))
        );
        assert_eq!(
            pick(Some("HDMI-A-1"), focused, &monitors),
            Err("configured replay monitor HDMI-A-1 is unavailable".into())
        );
    }

    #[test]
    fn an_inherited_monitor_falls_back_to_the_focused_one() {
        let monitors = [monitor("DP-1", false), monitor("DP-3", true)];
        let output = |name: &str| RecordDefaultTarget::Output(name.into());
        assert_eq!(
            pick(None, output("DP-1"), &monitors),
            Ok(("DP-1".into(), None))
        );
        let (name, notice) = pick(None, output("HDMI-A-1"), &monitors).unwrap();
        assert_eq!(name, "DP-3");
        assert!(notice.unwrap().contains("HDMI-A-1 is disconnected"));
    }

    #[test]
    fn other_recording_targets_capture_the_focused_monitor() {
        let monitors = [monitor("DP-1", false), monitor("DP-3", true)];
        for target in [RecordDefaultTarget::Focused, RecordDefaultTarget::Both] {
            assert_eq!(pick(None, target, &monitors), Ok(("DP-3".into(), None)));
        }
    }

    #[test]
    fn both_puts_the_monitors_on_one_canvas_where_gsr_does() {
        let mut left = monitor("DP-3", false);
        let mut right = monitor("DP-1", true);
        right.x = 1920;
        let (capture, notice) = replay_capture(
            Some(BOTH),
            &RecordDefaultTarget::Focused,
            &[right.clone(), left.clone()],
        )
        .unwrap();
        assert_eq!(notice, None);
        let Capture::Both { source, layout, .. } = capture else {
            panic!("one canvas");
        };
        assert_eq!(source, "DP-1;x=1920;y=0|DP-3;x=0;y=0");
        let placed = |name: &str, x| Placed {
            name: name.into(),
            x,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(layout, [placed("DP-1", 1920), placed("DP-3", 0)]);
        // The layout and gsr's source agree on every position.
        for p in &layout {
            assert!(source.contains(&format!("{};x={};y={}", p.name, p.x, p.y)));
        }
        assert_eq!(
            Placed::from_value(&layout[0].to_value()),
            Some(layout[0].clone())
        );
        // Different scales cannot share a canvas; one display is just that one.
        left.scale = 2.0;
        let (capture, notice) = replay_capture(
            Some(BOTH),
            &RecordDefaultTarget::Focused,
            &[right.clone(), left],
        )
        .unwrap();
        assert!(matches!(capture, Capture::One(m) if m.name == "DP-1"));
        assert!(notice.unwrap().contains("different scales"));
        let (capture, notice) =
            replay_capture(Some(BOTH), &RecordDefaultTarget::Focused, &[right]).unwrap();
        assert!(matches!(capture, Capture::One(m) if m.name == "DP-1"));
        assert!(notice.unwrap().contains("only one display"));
    }

    #[test]
    fn no_focused_monitor_is_an_error() {
        let monitors = [monitor("DP-1", false)];
        assert!(pick(None, RecordDefaultTarget::Focused, &monitors).is_err());
        assert!(pick(None, RecordDefaultTarget::Output("DP-9".into()), &monitors).is_err());
        assert!(pick(None, RecordDefaultTarget::Focused, &[]).is_err());
    }
}
