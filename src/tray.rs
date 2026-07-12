use crate::config::{RecordBothMode, RecordDefaultTarget, RecordingPrefs};
use crate::record::Monitor;
use crate::record::session::PublicRecordingState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

static OFFLINE_LOGGED: AtomicBool = AtomicBool::new(false);

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
    SetShowFrame(bool),
    SetDiskAddToShelf(bool),
}

struct TrayMenuModel {
    start_region_enabled: bool,
    start_default_enabled: bool,
    default_labels: Vec<String>,
    default_selected: usize,
    both_mode_selected: usize,
    show_frame: bool,
    disk_add_to_shelf: bool,
    settings_enabled: bool,
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
            .map(|monitor| format!("{} ({})", monitor.description, monitor.name))
            .chain(std::iter::once("Both displays".into()))
            .collect(),
        default_selected,
        both_mode_selected: usize::from(snapshot.prefs.both_mode == RecordBothMode::Combined),
        show_frame: snapshot.prefs.show_frame,
        disk_add_to_shelf: snapshot.prefs.disk_add_to_shelf,
        settings_enabled: true,
    }
}

pub struct BoltsnapTray {
    snapshot: TraySnapshot,
    sender: calloop::channel::Sender<crate::shelf::DaemonEvent>,
}

pub struct TrayPublisher {
    latest: Arc<LatestValue<TraySnapshot>>,
    wake: std::sync::mpsc::SyncSender<()>,
}

impl TrayPublisher {
    pub fn spawn(
        snapshot: TraySnapshot,
        sender: calloop::channel::Sender<crate::shelf::DaemonEvent>,
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
        sender: calloop::channel::Sender<crate::shelf::DaemonEvent>,
    ) -> Self {
        Self { snapshot, sender }
    }

    pub fn set_snapshot(&mut self, snapshot: TraySnapshot) {
        self.snapshot = snapshot;
    }

    fn send(&self, action: TrayAction) {
        let _ = self.sender.send(crate::shelf::DaemonEvent::Tray(action));
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

    fn icon_name(&self) -> String {
        "camera-video".into()
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
