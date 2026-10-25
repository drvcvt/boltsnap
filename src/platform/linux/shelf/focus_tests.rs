//! Monitor focus, recording targets and preference writes.
use super::*;

#[test]
fn area_uses_cached_monitor_layout() {
    let monitors = vec![
        crate::record::Monitor {
            name: "DP-1".into(),
            description: String::new(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: false,
        },
        crate::record::Monitor {
            name: "HDMI-A-1".into(),
            description: String::new(),
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440,
            scale: 1.0,
            focused: true,
        },
    ];
    let geo = crate::record::Geometry {
        x: 2100,
        y: 120,
        w: 640,
        h: 480,
    };

    assert_eq!(
        monitor_for_geometry(&monitors, &geo).map(|monitor| monitor.name.as_str()),
        Some("HDMI-A-1")
    );
}

#[test]
fn video_cards_follow_their_output_or_the_focused_monitor() {
    let clip = |output: Option<&str>| FinalizedClip {
        output: output.map(str::to_owned),
        path: "/tmp/clip.mp4".into(),
        permanent: false,
    };
    let focused = || Some("DP-3".to_owned());
    assert_eq!(card_output(&[clip(None)], focused).as_deref(), Some("DP-3"));
    assert_eq!(
        card_output(&[clip(Some("DP-1"))], focused).as_deref(),
        Some("DP-1")
    );
    assert_eq!(
        card_output(&[clip(Some("DP-3")), clip(Some("DP-1"))], || None).as_deref(),
        Some("DP-1")
    );
    assert_eq!(card_output(&[clip(None)], || None), None);
}

#[test]
fn focused_output_and_popup_target_use_current_hyprland_focus() {
    let json = br#"[
        {"name":"DP-1","description":"Main","x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false},
        {"name":"HDMI-A-1","description":"AOC","x":1920,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true}
    ]"#;
    let available = vec!["DP-1".to_string(), "HDMI-A-1".to_string()];

    let focused = focused_output_from_hyprland_json(json);
    assert_eq!(focused.as_deref(), Some("HDMI-A-1"));
    assert_eq!(
        resolve_output_name(focused.as_deref(), &available),
        Some("HDMI-A-1")
    );
    assert_eq!(
        resolve_output_name(Some("disconnected"), &available),
        Some("DP-1")
    );
}

fn monitor(name: &str, focused: bool) -> crate::record::Monitor {
    crate::record::Monitor {
        name: name.into(),
        description: name.into(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
        focused,
    }
}

#[test]
fn focus_query_snapshot_keeps_current_order_and_focus() {
    let json = br#"[
        {"name":"DP-1","description":"AOC","x":1920,"y":0,"width":1920,"height":1080,"scale":1.0,"focused":false},
        {"name":"DP-3","description":"Main","x":0,"y":0,"width":2560,"height":1440,"scale":1.0,"focused":true}
    ]"#;
    let snapshot = parse_focus_snapshot(json).unwrap();
    assert_eq!(
        snapshot
            .iter()
            .map(|monitor| (monitor.name.as_str(), monitor.focused))
            .collect::<Vec<_>>(),
        vec![("DP-1", false), ("DP-3", true)]
    );
}

#[test]
fn failed_focus_query_never_reuses_stale_focus_for_default_start() {
    let prefs = crate::config::RecordingPrefs::default();
    let stale_wayland = vec![monitor("DP-3", false), monitor("DP-1", true)];
    let plan = default_start_plan(&prefs, &Err("query timed out".into()), &stale_wayland).unwrap();
    assert_eq!(plan.outputs[0].name, "DP-3");

    let disconnected = crate::config::RecordingPrefs {
        default_target: crate::config::RecordDefaultTarget::Output("DP-OLD".into()),
        ..prefs
    };
    let plan = default_start_plan(
        &disconnected,
        &Err("query timed out".into()),
        &stale_wayland,
    )
    .unwrap();
    assert_eq!(plan.outputs[0].name, "DP-3");
}

#[test]
fn default_start_uses_fresh_snapshot_order_instead_of_wayland_order() {
    let prefs = crate::config::RecordingPrefs {
        default_target: crate::config::RecordDefaultTarget::Both,
        ..crate::config::RecordingPrefs::default()
    };
    let wayland = vec![monitor("DP-3", true), monitor("DP-1", false)];
    let fresh = Ok(vec![monitor("DP-1", false), monitor("DP-3", true)]);
    let plan = default_start_plan(&prefs, &fresh, &wayland).unwrap();
    assert_eq!(
        plan.outputs
            .iter()
            .map(|monitor| monitor.name.as_str())
            .collect::<Vec<_>>(),
        vec!["DP-1", "DP-3"]
    );
}

#[test]
fn hotplug_mismatch_cannot_focus_a_disconnected_output() {
    let prefs = crate::config::RecordingPrefs::default();
    let wayland = vec![monitor("DP-1", true)];
    let fresh = Ok(vec![monitor("DP-OLD", true), monitor("DP-1", false)]);
    assert!(default_start_plan(&prefs, &fresh, &wayland).is_err());

    let named = crate::config::RecordingPrefs {
        default_target: crate::config::RecordDefaultTarget::Output("DP-1".into()),
        ..prefs
    };
    let plan = default_start_plan(&named, &fresh, &wayland).unwrap();
    assert_eq!(plan.outputs[0].name, "DP-1");
    assert!(!plan.outputs[0].focused);
}

#[test]
fn failed_focus_query_can_still_start_both_connected_wayland_outputs() {
    let prefs = crate::config::RecordingPrefs {
        default_target: crate::config::RecordDefaultTarget::Both,
        ..crate::config::RecordingPrefs::default()
    };
    let stale_wayland = vec![monitor("DP-3", true), monitor("DP-1", false)];
    let plan = default_start_plan(&prefs, &Err("query failed".into()), &stale_wayland).unwrap();
    assert_eq!(plan.outputs.len(), 2);
    assert!(plan.outputs.iter().all(|monitor| !monitor.focused));
}

#[test]
fn rotated_output_bounds_select_the_marker_monitor() {
    let (width, height) = transformed_mode_size((2560, 1440), wl_output::Transform::_90);
    assert_eq!((width, height), (1440, 2560));
    let monitors = vec![crate::record::Monitor {
        name: "DP-ROTATED".into(),
        description: String::new(),
        x: 1920,
        y: 0,
        width: width as u32,
        height: height as u32,
        scale: 1.0,
        focused: true,
    }];
    let geo = crate::record::Geometry {
        x: 2000,
        y: 2200,
        w: 100,
        h: 100,
    };

    assert_eq!(
        monitor_for_geometry(&monitors, &geo).map(|monitor| monitor.name.as_str()),
        Some("DP-ROTATED")
    );
}

#[test]
fn delayed_focus_cannot_reopen_discarding_controls_but_finalizing_stays_visible() {
    assert!(!recording_controls_visible(SessionPhase::Discarding));
    assert!(recording_controls_visible(SessionPhase::Finalizing));
}

#[test]
fn daemon_rejects_zero_sized_recording_geometry() {
    assert!(validate_recording_dimensions(0, 1080).is_err());
    assert!(validate_recording_dimensions(1920, 0).is_err());
    assert!(validate_recording_dimensions(1920, 1080).is_ok());
}

#[test]
fn latest_value_mailbox_keeps_only_the_newest_value() {
    let latest = crate::tray::LatestValue::new();
    latest.replace(1);
    latest.replace(2);
    assert_eq!(latest.take(), Some(2));
    assert!(latest.take().is_none());
}
