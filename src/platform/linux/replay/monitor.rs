//! Which monitor a replay session captures.

use crate::config::RecordDefaultTarget;
use crate::record::{Monitor, resolve_record_outputs};

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
    fn no_focused_monitor_is_an_error() {
        let monitors = [monitor("DP-1", false)];
        assert!(pick(None, RecordDefaultTarget::Focused, &monitors).is_err());
        assert!(pick(None, RecordDefaultTarget::Output("DP-9".into()), &monitors).is_err());
        assert!(pick(None, RecordDefaultTarget::Focused, &[]).is_err());
    }
}
