use crate::config::{
    RecordAudioSource, RecordBothMode, RecordCursor, RecordDefaultTarget, RecordingPrefs,
};
use crate::record::Monitor;
use crate::record::session::{PublicRecordingState, RecordingAction};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

static OFFLINE_LOGGED: AtomicBool = AtomicBool::new(false);

fn tray_icons() -> &'static [ksni::Icon] {
    static ICONS: LazyLock<Vec<ksni::Icon>> = LazyLock::new(|| {
        [
            include_bytes!("../../../assets/icons/boltsnap-tray-32.png").as_slice(),
            include_bytes!("../../../assets/icons/boltsnap-tray-64.png").as_slice(),
        ]
        .into_iter()
        .map(|png| {
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
                .expect("embedded Boltsnap tray icon must be valid PNG");
            let (width, height) = (image.width(), image.height());
            let mut data = image.into_rgba8().into_raw();
            for pixel in data.chunks_exact_mut(4) {
                pixel.rotate_right(1);
            }
            ksni::Icon {
                width: width as i32,
                height: height as i32,
                data,
            }
        })
        .collect()
    });
    ICONS.as_slice()
}

/// The mark's snap dot doubles as the recording light: red while recording or
/// saving, amber while paused. Position and radius are fractions of the icon
/// size and fit both embedded sizes.
fn tray_icons_for(state: PublicRecordingState) -> Vec<ksni::Icon> {
    let (red, green, blue) = match state {
        PublicRecordingState::Idle => return tray_icons().to_vec(),
        PublicRecordingState::Paused => (0xff, 0xb0, 0x20),
        PublicRecordingState::Recording | PublicRecordingState::Finalizing => (0xff, 0x45, 0x3a),
    };
    tray_icons()
        .iter()
        .map(|icon| {
            let mut icon = icon.clone();
            let size = icon.width as f32;
            let (cx, cy, radius) = (0.71 * size, 0.40 * size, 0.085 * size);
            for (index, pixel) in icon.data.chunks_exact_mut(4).enumerate() {
                let x = (index as i32 % icon.width) as f32 + 0.5;
                let y = (index as i32 / icon.width) as f32 + 0.5;
                if (x - cx).hypot(y - cy) <= radius {
                    pixel[1..].copy_from_slice(&[red, green, blue]);
                }
            }
            icon
        })
        .collect()
}

pub(crate) struct LatestValue<T>(Mutex<Option<T>>);

impl<T> LatestValue<T> {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub(crate) fn replace(&self, value: T) {
        *self.0.lock().unwrap() = Some(value);
    }

    pub(crate) fn take(&self) -> Option<T> {
        self.0.lock().unwrap().take()
    }
}

#[derive(Clone, Debug)]
pub struct TraySnapshot {
    pub prefs: RecordingPrefs,
    pub monitors: Vec<Monitor>,
    pub state: PublicRecordingState,
    /// Whether the replay buffer is currently capturing.
    pub replay_running: bool,
}

#[derive(Clone, Debug)]
pub enum TrayAction {
    /// Turn the replay buffer on or off. One entry rather than a Start/Stop
    /// pair: the buffer is a thing that is either running or not, and its
    /// checkmark says which.
    ReplayToggle,
    ReplaySave,
    Recording(RecordingAction),
    StartRegion,
    StartDefault,
    SetDefaultTarget(RecordDefaultTarget),
    SetBothMode(RecordBothMode),
    SetCursor(RecordCursor),
    SetAudioSource(RecordAudioSource),
    SetShowFrame(bool),
    SetDiskAddToShelf(bool),
}

struct TrayMenuModel {
    replay_running: bool,
    /// A fullscreen clip needs a running buffer.
    clip_enabled: bool,
    start_region_enabled: bool,
    start_default_enabled: bool,
    default_labels: Vec<String>,
    default_selected: usize,
    both_mode_selected: usize,
    cursor_selected: usize,
    audio_source_selected: usize,
    show_frame: bool,
    disk_add_to_shelf: bool,
    settings_enabled: bool,
}

const CURSOR_MODES: [RecordCursor; 3] = [
    RecordCursor::System,
    RecordCursor::Mellow,
    RecordCursor::Quick,
];
const AUDIO_SOURCE_LABELS: [&str; 3] = ["System + microphone", "Microphone only", "System only"];

fn audio_source_at(index: usize) -> RecordAudioSource {
    match index {
        1 => RecordAudioSource::Mic,
        2 => RecordAudioSource::System,
        _ => RecordAudioSource::SystemAndMic,
    }
}

/// Recording controls at the top of the menu. Discard stays in the popup, where
/// it is one deliberate click away from its neighbours, not in a quick menu.
fn recording_entries(state: PublicRecordingState) -> Vec<(&'static str, Option<RecordingAction>)> {
    let pause = match state {
        PublicRecordingState::Idle => return Vec::new(),
        PublicRecordingState::Finalizing => return vec![("Saving recording…", None)],
        PublicRecordingState::Recording => ("Pause recording", RecordingAction::Pause),
        PublicRecordingState::Paused => ("Resume recording", RecordingAction::Resume),
    };
    vec![
        (pause.0, Some(pause.1)),
        ("Save recording to shelf", Some(RecordingAction::SaveShelf)),
        ("Save recording to disk", Some(RecordingAction::SaveDisk)),
    ]
}

fn monitor_label(monitor: &Monitor) -> String {
    output_label(&monitor.name, &monitor.description)
}

/// A monitor as people know it: its description with the connector, once.
pub(crate) fn output_label(name: &str, description: &str) -> String {
    let description = description.trim();
    if description.is_empty() {
        name.to_owned()
    } else if description == name || description.contains(&format!("({name})")) {
        description.to_owned()
    } else {
        format!("{description} ({name})")
    }
}

/// The default monitor's choices with their labels, in menu order. A
/// configured monitor that is unplugged stays listed, so the menu shows what
/// the file says.
fn default_targets(snapshot: &TraySnapshot) -> Vec<(String, RecordDefaultTarget)> {
    let mut targets = vec![("Focused display".to_owned(), RecordDefaultTarget::Focused)];
    targets.extend(snapshot.monitors.iter().map(|monitor| {
        (
            monitor_label(monitor),
            RecordDefaultTarget::Output(monitor.name.clone()),
        )
    }));
    if let RecordDefaultTarget::Output(name) = &snapshot.prefs.default_target
        && !snapshot
            .monitors
            .iter()
            .any(|monitor| monitor.name == *name)
    {
        targets.push((
            format!("{name} (disconnected)"),
            snapshot.prefs.default_target.clone(),
        ));
    }
    targets.push(("Both displays".to_owned(), RecordDefaultTarget::Both));
    targets
}

fn menu_model(snapshot: &TraySnapshot) -> TrayMenuModel {
    let targets = default_targets(snapshot);
    let default_selected = targets
        .iter()
        .position(|(_, target)| *target == snapshot.prefs.default_target)
        .unwrap_or(0);
    TrayMenuModel {
        replay_running: snapshot.replay_running,
        clip_enabled: snapshot.replay_running,
        start_region_enabled: snapshot.state == PublicRecordingState::Idle,
        start_default_enabled: snapshot.state == PublicRecordingState::Idle,
        default_labels: targets.into_iter().map(|(label, _)| label).collect(),
        default_selected,
        both_mode_selected: usize::from(snapshot.prefs.both_mode == RecordBothMode::Combined),
        cursor_selected: CURSOR_MODES
            .iter()
            .position(|mode| *mode == snapshot.prefs.cursor)
            .unwrap_or(0),
        audio_source_selected: match snapshot.prefs.audio_source {
            RecordAudioSource::SystemAndMic => 0,
            RecordAudioSource::Mic => 1,
            RecordAudioSource::System => 2,
        },
        show_frame: snapshot.prefs.show_frame,
        disk_add_to_shelf: snapshot.prefs.disk_add_to_shelf,
        settings_enabled: true,
    }
}

const SETTINGS_LABEL: &str = "Settings…";

/// Starts the settings window as its own process, right from the tray thread;
/// an open window raises itself instead (single instance).
fn open_settings() {
    let result = crate::paths::own_exe()
        .map_err(|error| format!("find boltsnap executable: {error}"))
        .and_then(|exe| {
            crate::paths::spawn_reaped(
                std::process::Command::new(exe)
                    .arg("settings")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null()),
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        super::shelf::notify(&format!("Settings: {error}"));
    }
}

pub struct BoltsnapTray {
    snapshot: TraySnapshot,
    sender: calloop::channel::Sender<super::shelf::DaemonEvent>,
}

pub struct TrayPublisher {
    latest: Arc<LatestValue<TraySnapshot>>,
    wake: std::sync::mpsc::SyncSender<()>,
}

impl TrayPublisher {
    pub fn spawn(
        snapshot: TraySnapshot,
        sender: calloop::channel::Sender<super::shelf::DaemonEvent>,
    ) -> Self {
        let latest = Arc::new(LatestValue::new());
        let worker_latest = Arc::clone(&latest);
        let (wake, wake_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            use ksni::blocking::TrayMethods;

            let tray = BoltsnapTray::new(snapshot, sender);
            let handle = match tray.assume_sni_available(true).spawn() {
                Ok(handle) => handle,
                Err(error) => {
                    eprintln!("boltsnap daemon: tray unavailable: {error}");
                    return;
                }
            };
            while wake_rx.recv().is_ok() {
                let snapshot = worker_latest.take();
                if let Some(snapshot) = snapshot {
                    let _ = handle.update(move |tray| tray.set_snapshot(snapshot));
                }
            }
        });
        Self { latest, wake }
    }

    pub fn publish(&self, snapshot: TraySnapshot) {
        self.latest.replace(snapshot);
        let _ = self.wake.try_send(());
    }
}

impl BoltsnapTray {
    pub fn new(
        snapshot: TraySnapshot,
        sender: calloop::channel::Sender<super::shelf::DaemonEvent>,
    ) -> Self {
        Self { snapshot, sender }
    }

    pub fn set_snapshot(&mut self, snapshot: TraySnapshot) {
        self.snapshot = snapshot;
    }

    fn send(&self, action: TrayAction) {
        let _ = self.sender.send(super::shelf::DaemonEvent::Tray(action));
    }
}

impl ksni::Tray for BoltsnapTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "boltsnap".into()
    }

    fn title(&self) -> String {
        "boltsnap".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_icons_for(self.snapshot.state)
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        if !OFFLINE_LOGGED.swap(true, Ordering::Relaxed) {
            eprintln!("boltsnap daemon: tray host unavailable: {reason:?}");
        }
        true
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};

        let model = menu_model(&self.snapshot);
        let default_options = model
            .default_labels
            .into_iter()
            .map(|label| RadioItem {
                label,
                ..Default::default()
            })
            .collect();
        let mut items: Vec<ksni::MenuItem<Self>> = recording_entries(self.snapshot.state)
            .into_iter()
            .map(|(label, action)| {
                StandardItem {
                    label: label.into(),
                    enabled: action.is_some(),
                    activate: Box::new(move |tray: &mut Self| {
                        if let Some(action) = action {
                            tray.send(TrayAction::Recording(action));
                        }
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        if !items.is_empty() {
            items.push(ksni::MenuItem::Separator);
        }
        items.extend([
            StandardItem {
                label: "Clip fullscreen to shelf".into(),
                enabled: model.clip_enabled,
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::ReplaySave)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Replay buffer".into(),
                checked: model.replay_running,
                activate: Box::new(|tray: &mut Self| {
                    // Reflect the new state at once; the daemon publishes the
                    // real one when the capture has actually come up or gone.
                    tray.snapshot.replay_running = !tray.snapshot.replay_running;
                    tray.send(TrayAction::ReplayToggle);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start region recording".into(),
                enabled: model.start_region_enabled,
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::StartRegion)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start fullscreen recording".into(),
                enabled: model.start_default_enabled,
                activate: Box::new(|tray: &mut Self| tray.send(TrayAction::StartDefault)),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Default monitor".into(),
                enabled: model.settings_enabled,
                submenu: vec![
                    RadioGroup {
                        selected: model.default_selected,
                        options: default_options,
                        select: Box::new(|tray: &mut Self, index| {
                            let Some((_, target)) =
                                default_targets(&tray.snapshot).into_iter().nth(index)
                            else {
                                return;
                            };
                            tray.snapshot.prefs.default_target = target.clone();
                            tray.send(TrayAction::SetDefaultTarget(target));
                        }),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Both displays mode".into(),
                enabled: model.settings_enabled,
                submenu: vec![
                    RadioGroup {
                        selected: model.both_mode_selected,
                        options: vec![
                            RadioItem {
                                label: "Separate clips".into(),
                                ..Default::default()
                            },
                            RadioItem {
                                label: "Combined clip".into(),
                                ..Default::default()
                            },
                        ],
                        select: Box::new(|tray: &mut Self, index| {
                            let mode = if index == 0 {
                                RecordBothMode::Separate
                            } else {
                                RecordBothMode::Combined
                            };
                            tray.snapshot.prefs.both_mode = mode;
                            tray.send(TrayAction::SetBothMode(mode));
                        }),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Cursor".into(),
                enabled: model.settings_enabled,
                submenu: vec![
                    RadioGroup {
                        selected: model.cursor_selected,
                        options: ["System cursor", "Smooth (mellow)", "Smooth (quick)"]
                            .into_iter()
                            .map(|label| RadioItem {
                                label: label.into(),
                                ..Default::default()
                            })
                            .collect(),
                        select: Box::new(|tray: &mut Self, index| {
                            let mode = CURSOR_MODES[index.min(CURSOR_MODES.len() - 1)];
                            tray.snapshot.prefs.cursor = mode;
                            tray.send(TrayAction::SetCursor(mode));
                        }),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Audio source".into(),
                enabled: model.settings_enabled,
                submenu: vec![
                    RadioGroup {
                        selected: model.audio_source_selected,
                        options: AUDIO_SOURCE_LABELS
                            .into_iter()
                            .map(|label| RadioItem {
                                label: label.into(),
                                ..Default::default()
                            })
                            .collect(),
                        select: Box::new(|tray: &mut Self, index| {
                            let source = audio_source_at(index);
                            tray.snapshot.prefs.audio_source = source;
                            tray.send(TrayAction::SetAudioSource(source));
                        }),
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Show recording frame".into(),
                enabled: model.settings_enabled,
                checked: model.show_frame,
                activate: Box::new(|tray: &mut Self| {
                    tray.snapshot.prefs.show_frame = !tray.snapshot.prefs.show_frame;
                    tray.send(TrayAction::SetShowFrame(tray.snapshot.prefs.show_frame));
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Video: Move to shelf after Disk Save".into(),
                enabled: model.settings_enabled,
                checked: model.disk_add_to_shelf,
                activate: Box::new(|tray: &mut Self| {
                    tray.snapshot.prefs.disk_add_to_shelf = !tray.snapshot.prefs.disk_add_to_shelf;
                    tray.send(TrayAction::SetDiskAddToShelf(
                        tray.snapshot.prefs.disk_add_to_shelf,
                    ));
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: SETTINGS_LABEL.into(),
                activate: Box::new(|_: &mut Self| open_settings()),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(state: PublicRecordingState) -> TraySnapshot {
        TraySnapshot {
            prefs: RecordingPrefs {
                default_target: RecordDefaultTarget::Output("DP-3".into()),
                both_mode: RecordBothMode::Combined,
                show_frame: false,
                disk_add_to_shelf: true,
                ..RecordingPrefs::default()
            },
            monitors: vec![
                Monitor {
                    name: "DP-3".into(),
                    description: "BenQ".into(),
                    x: 0,
                    y: 0,
                    width: 2560,
                    height: 1440,
                    scale: 1.0,
                    focused: true,
                },
                Monitor {
                    name: "DP-1".into(),
                    description: "AOC".into(),
                    x: 2560,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    scale: 1.0,
                    focused: false,
                },
            ],
            state,
            replay_running: false,
        }
    }

    #[test]
    fn replay_entry_follows_the_buffer_state() {
        assert!(!menu_model(&snapshot(PublicRecordingState::Idle)).replay_running);
        let mut running = snapshot(PublicRecordingState::Idle);
        running.replay_running = true;
        assert!(menu_model(&running).replay_running);
    }

    #[test]
    fn tray_menu_exposes_outputs_modes_and_checkmarks() {
        let model = menu_model(&snapshot(PublicRecordingState::Idle));
        assert!(model.start_region_enabled);
        assert!(model.start_default_enabled);
        assert_eq!(
            model.default_labels,
            vec![
                "Focused display",
                "BenQ (DP-3)",
                "AOC (DP-1)",
                "Both displays"
            ]
        );
        assert_eq!(model.default_selected, 1);
        assert_eq!(model.both_mode_selected, 1);
        assert!(!model.show_frame);
        assert!(model.disk_add_to_shelf);
        assert!(model.settings_enabled);
    }

    #[test]
    fn default_monitor_menu_offers_focused_and_keeps_an_unplugged_monitor() {
        let mut snapshot = snapshot(PublicRecordingState::Idle);
        snapshot.prefs.default_target = RecordDefaultTarget::Focused;
        assert_eq!(menu_model(&snapshot).default_selected, 0);
        snapshot.prefs.default_target = RecordDefaultTarget::Both;
        assert_eq!(menu_model(&snapshot).default_selected, 3);

        snapshot.prefs.default_target = RecordDefaultTarget::Output("HDMI-A-1".into());
        let model = menu_model(&snapshot);
        assert_eq!(model.default_labels[3], "HDMI-A-1 (disconnected)");
        assert_eq!(model.default_selected, 3);
        // A click picks the target at the same index the label has.
        let targets: Vec<_> = default_targets(&snapshot)
            .into_iter()
            .map(|(_, target)| target)
            .collect();
        assert_eq!(
            targets,
            vec![
                RecordDefaultTarget::Focused,
                RecordDefaultTarget::Output("DP-3".into()),
                RecordDefaultTarget::Output("DP-1".into()),
                RecordDefaultTarget::Output("HDMI-A-1".into()),
                RecordDefaultTarget::Both,
            ]
        );
    }

    #[test]
    fn menu_model_selects_microphone_audio_source() {
        let mut snapshot = snapshot(PublicRecordingState::Idle);
        snapshot.prefs.audio_source = RecordAudioSource::Mic;
        let model = menu_model(&snapshot);
        assert_eq!(model.audio_source_selected, 1);
        assert_eq!(
            AUDIO_SOURCE_LABELS,
            ["System + microphone", "Microphone only", "System only"]
        );
    }

    #[test]
    fn monitor_label_does_not_duplicate_an_embedded_connector() {
        let monitor = Monitor {
            name: "DP-1".into(),
            description: "AOC 27G4HRE (DP-1)".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: true,
        };

        assert_eq!(monitor_label(&monitor), "AOC 27G4HRE (DP-1)");
    }

    #[test]
    fn tray_start_entries_are_disabled_while_recording_but_settings_remain() {
        let model = menu_model(&snapshot(PublicRecordingState::Recording));
        assert!(!model.start_region_enabled);
        assert!(!model.start_default_enabled);
        assert_eq!(model.default_selected, 1);
        assert_eq!(model.both_mode_selected, 1);
        assert!(!model.show_frame);
        assert!(model.disk_add_to_shelf);
        assert!(model.settings_enabled);
    }

    #[test]
    fn settings_is_the_last_entry_and_stays_enabled_while_recording() {
        use ksni::Tray;

        for state in [PublicRecordingState::Idle, PublicRecordingState::Recording] {
            let (sender, _receiver) = calloop::channel::channel();
            let menu = BoltsnapTray::new(snapshot(state), sender).menu();
            let [
                ..,
                ksni::MenuItem::Separator,
                ksni::MenuItem::Standard(last),
            ] = menu.as_slice()
            else {
                panic!("the menu ends with a separator and a plain entry");
            };
            assert_eq!(last.label, "Settings…");
            assert!(last.enabled);
        }
    }

    /// The launcher entry and its icons, as packaged (settings plan, Phase 5a).
    #[test]
    fn launcher_entry_names_the_window_and_its_icons_have_their_sizes() {
        let entry = include_str!("../../../assets/linux/boltsnap.desktop");
        for line in [
            "Exec=boltsnap settings",
            "Icon=boltsnap",
            "StartupWMClass=boltsnap",
        ] {
            assert!(entry.lines().any(|l| l == line), "missing {line}");
        }
        for (png, size) in [
            (
                include_bytes!("../../../assets/linux/boltsnap-64.png").as_slice(),
                64,
            ),
            (
                include_bytes!("../../../assets/linux/boltsnap-128.png"),
                128,
            ),
            (
                include_bytes!("../../../assets/linux/boltsnap-256.png"),
                256,
            ),
        ] {
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).unwrap();
            assert_eq!((image.width(), image.height()), (size, size));
        }
    }

    #[test]
    fn disconnected_default_stays_selected_instead_of_the_focused_monitor() {
        let mut snapshot = snapshot(PublicRecordingState::Idle);
        snapshot.prefs.default_target = RecordDefaultTarget::Output("DP-GONE".into());
        snapshot.monitors[0].focused = false;
        snapshot.monitors[1].focused = true;
        let model = menu_model(&snapshot);
        assert_eq!(
            model.default_labels[model.default_selected],
            "DP-GONE (disconnected)"
        );
    }

    #[test]
    fn embedded_tray_icons_are_valid_argb_pixmaps() {
        let icons = tray_icons();
        assert_eq!(
            icons.iter().map(|icon| icon.width).collect::<Vec<_>>(),
            [32, 64]
        );
        assert!(icons.iter().all(|icon| {
            icon.width == icon.height
                && icon.data.len() == (icon.width * icon.height * 4) as usize
                && icon.data.chunks_exact(4).any(|pixel| pixel[0] == 0)
                && icon.data.chunks_exact(4).any(|pixel| pixel[0] > 0)
        }));

        let icon = &icons[0];
        let alpha_at = |x: i32, y: i32| icon.data[((y * icon.width + x) * 4) as usize];
        assert!(alpha_at(16, 20) > 200, "the chosen mark has a solid body");
        assert!(alpha_at(23, 15) > 0, "the snap dot must fill its cutout");

        let icon = &icons[1];
        let alpha_at = |x: i32, y: i32| icon.data[((y * icon.width + x) * 4) as usize];
        assert!(alpha_at(24, 56) > 200, "the base must remain solid");
        assert!(alpha_at(29, 56) > 200, "the base must remain continuous");
        assert!(alpha_at(39, 25) < 64, "the snap cutout must stay open");
        assert!(alpha_at(45, 25) > 200, "the snap dot must stay opaque");
    }

    #[test]
    fn recording_controls_follow_the_recording_state() {
        let labels = |state| recording_entries(state);
        assert!(labels(PublicRecordingState::Idle).is_empty());
        assert_eq!(
            labels(PublicRecordingState::Recording),
            [
                ("Pause recording", Some(RecordingAction::Pause)),
                ("Save recording to shelf", Some(RecordingAction::SaveShelf)),
                ("Save recording to disk", Some(RecordingAction::SaveDisk)),
            ]
        );
        assert_eq!(
            labels(PublicRecordingState::Paused)[0],
            ("Resume recording", Some(RecordingAction::Resume))
        );
        assert_eq!(
            labels(PublicRecordingState::Finalizing),
            [("Saving recording…", None)]
        );
        assert!(
            [
                PublicRecordingState::Recording,
                PublicRecordingState::Paused
            ]
            .into_iter()
            .flat_map(recording_entries)
            .all(|(_, action)| action != Some(RecordingAction::Discard))
        );
    }

    #[test]
    fn fullscreen_clip_needs_a_running_buffer() {
        assert!(!menu_model(&snapshot(PublicRecordingState::Idle)).clip_enabled);
        let mut running = snapshot(PublicRecordingState::Idle);
        running.replay_running = true;
        assert!(menu_model(&running).clip_enabled);
    }

    #[test]
    fn recording_tints_only_the_snap_dot() {
        let rgb = |icons: &[ksni::Icon], size: i32, x: i32, y: i32| {
            let icon = icons.iter().find(|icon| icon.width == size).unwrap();
            let at = ((y * icon.width + x) * 4) as usize;
            (icon.data[at + 1], icon.data[at + 2], icon.data[at + 3])
        };
        let idle = tray_icons_for(PublicRecordingState::Idle);
        let recording = tray_icons_for(PublicRecordingState::Recording);
        let paused = tray_icons_for(PublicRecordingState::Paused);
        assert!(idle.iter().zip(tray_icons()).all(|(a, b)| a.data == b.data));
        for (size, dot, body) in [(32, (22, 12), (10, 20)), (64, (45, 25), (24, 56))] {
            assert_eq!(rgb(&recording, size, dot.0, dot.1), (0xff, 0x45, 0x3a));
            assert_eq!(rgb(&paused, size, dot.0, dot.1), (0xff, 0xb0, 0x20));
            assert_eq!(rgb(&recording, size, body.0, body.1), (0xff, 0xff, 0xff));
        }
        // Alpha is untouched, so the mark keeps its shape.
        assert!(recording.iter().zip(tray_icons()).all(|(tinted, plain)| {
            tinted
                .data
                .chunks_exact(4)
                .zip(plain.data.chunks_exact(4))
                .all(|(a, b)| a[0] == b[0])
        }));
    }

    #[test]
    fn tray_publisher_mailbox_keeps_only_the_latest_snapshot() {
        let latest = LatestValue::new();
        latest.replace(TraySnapshot {
            state: PublicRecordingState::Recording,
            ..snapshot(PublicRecordingState::Idle)
        });
        latest.replace(TraySnapshot {
            state: PublicRecordingState::Paused,
            ..snapshot(PublicRecordingState::Idle)
        });
        assert_eq!(latest.take().unwrap().state, PublicRecordingState::Paused);
        assert!(latest.take().is_none());
    }
}
