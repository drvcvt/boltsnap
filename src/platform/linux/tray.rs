use crate::config::{RecordAudioSource, RecordBothMode, RecordDefaultTarget, RecordingPrefs};
use crate::record::Monitor;
use crate::record::session::PublicRecordingState;
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
}

#[derive(Clone, Debug)]
pub enum TrayAction {
    StartRegion,
    StartDefault,
    SetDefaultTarget(RecordDefaultTarget),
    SetBothMode(RecordBothMode),
    SetAudioSource(RecordAudioSource),
    SetShowFrame(bool),
    SetDiskAddToShelf(bool),
}

struct TrayMenuModel {
    start_region_enabled: bool,
    start_default_enabled: bool,
    default_labels: Vec<String>,
    default_selected: usize,
    both_mode_selected: usize,
    audio_source_selected: usize,
    show_frame: bool,
    disk_add_to_shelf: bool,
    settings_enabled: bool,
}

const AUDIO_SOURCE_LABELS: [&str; 3] = ["System + microphone", "Microphone only", "System only"];

fn audio_source_at(index: usize) -> RecordAudioSource {
    match index {
        1 => RecordAudioSource::Mic,
        2 => RecordAudioSource::System,
        _ => RecordAudioSource::SystemAndMic,
    }
}

fn monitor_label(monitor: &Monitor) -> String {
    let description = monitor.description.trim();
    if description.is_empty() {
        monitor.name.clone()
    } else if description == monitor.name || description.contains(&format!("({})", monitor.name)) {
        description.to_owned()
    } else {
        format!("{description} ({})", monitor.name)
    }
}

fn menu_model(snapshot: &TraySnapshot) -> TrayMenuModel {
    let focused_selected = snapshot
        .monitors
        .iter()
        .position(|monitor| monitor.focused)
        .unwrap_or(0);
    let default_selected = match &snapshot.prefs.default_target {
        RecordDefaultTarget::Focused => focused_selected,
        RecordDefaultTarget::Output(name) => snapshot
            .monitors
            .iter()
            .position(|monitor| monitor.name == *name)
            .unwrap_or(focused_selected),
        RecordDefaultTarget::Both => snapshot.monitors.len(),
    };
    TrayMenuModel {
        start_region_enabled: snapshot.state == PublicRecordingState::Idle,
        start_default_enabled: snapshot.state == PublicRecordingState::Idle,
        default_labels: snapshot
            .monitors
            .iter()
            .map(monitor_label)
            .chain(std::iter::once("Both displays".into()))
            .collect(),
        default_selected,
        both_mode_selected: usize::from(snapshot.prefs.both_mode == RecordBothMode::Combined),
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
        tray_icons().to_vec()
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
        vec![
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
                            let target = tray
                                .snapshot
                                .monitors
                                .get(index)
                                .map(|monitor| RecordDefaultTarget::Output(monitor.name.clone()))
                                .unwrap_or(RecordDefaultTarget::Both);
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
        ]
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
        }
    }

    #[test]
    fn tray_menu_exposes_outputs_modes_and_checkmarks() {
        let model = menu_model(&snapshot(PublicRecordingState::Idle));
        assert!(model.start_region_enabled);
        assert!(model.start_default_enabled);
        assert_eq!(
            model.default_labels,
            vec!["BenQ (DP-3)", "AOC (DP-1)", "Both displays"]
        );
        assert_eq!(model.default_selected, 0);
        assert_eq!(model.both_mode_selected, 1);
        assert!(!model.show_frame);
        assert!(model.disk_add_to_shelf);
        assert!(model.settings_enabled);
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
        assert_eq!(model.default_selected, 0);
        assert_eq!(model.both_mode_selected, 1);
        assert!(!model.show_frame);
        assert!(model.disk_add_to_shelf);
        assert!(model.settings_enabled);
    }

    #[test]
    fn disconnected_default_marks_the_effective_focused_fallback() {
        let mut snapshot = snapshot(PublicRecordingState::Idle);
        snapshot.prefs.default_target = RecordDefaultTarget::Output("DP-GONE".into());
        snapshot.monitors[0].focused = false;
        snapshot.monitors[1].focused = true;
        assert_eq!(menu_model(&snapshot).default_selected, 1);
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
