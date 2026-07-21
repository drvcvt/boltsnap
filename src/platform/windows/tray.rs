use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayAction {
    CaptureArea,
    CaptureFull,
    RecordArea,
    RecordFull,
    ShowRecordingControls,
    Quit,
}

pub struct TrayState {
    _icon: TrayIcon,
    capture_area: MenuId,
    capture_full: MenuId,
    record_area: MenuId,
    record_full: MenuId,
    show_recording_controls: MenuId,
    quit: MenuId,
}

impl TrayState {
    pub fn action(&self, event: &MenuEvent) -> Option<TrayAction> {
        match &event.id {
            id if id == &self.capture_area => Some(TrayAction::CaptureArea),
            id if id == &self.capture_full => Some(TrayAction::CaptureFull),
            id if id == &self.record_area => Some(TrayAction::RecordArea),
            id if id == &self.record_full => Some(TrayAction::RecordFull),
            id if id == &self.show_recording_controls => Some(TrayAction::ShowRecordingControls),
            id if id == &self.quit => Some(TrayAction::Quit),
            _ => None,
        }
    }
}

pub fn create() -> Result<TrayState, String> {
    let capture_area = MenuItem::new("Screenshot: Bereich", true, None);
    let capture_full = MenuItem::new("Screenshot: Vollbild", true, None);
    let record_area = MenuItem::new("Aufnahme: Bereich", true, None);
    let record_full = MenuItem::new("Aufnahme: Vollbild", true, None);
    let show_recording_controls = MenuItem::new("Aufnahmesteuerung anzeigen", true, None);
    let quit = MenuItem::new("Boltsnap beenden", true, None);
    let separator_one = PredefinedMenuItem::separator();
    let separator_two = PredefinedMenuItem::separator();
    let menu = Menu::new();
    menu.append_items(&[
        &capture_area,
        &capture_full,
        &separator_one,
        &record_area,
        &record_full,
        &show_recording_controls,
        &separator_two,
        &quit,
    ])
    .map_err(|error| format!("create Windows tray menu: {error}"))?;

    let image =
        image::load_from_memory(include_bytes!("../../../assets/icons/boltsnap-tray-32.png"))
            .map_err(|error| format!("decode Windows tray icon: {error}"))?
            .into_rgba8();
    let (width, height) = image.dimensions();
    let icon = Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|error| format!("create Windows tray icon: {error}"))?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Boltsnap")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon)
        .build()
        .map_err(|error| format!("create Windows tray: {error}"))?;

    Ok(TrayState {
        _icon: tray,
        capture_area: capture_area.id().clone(),
        capture_full: capture_full.id().clone(),
        record_area: record_area.id().clone(),
        record_full: record_full.id().clone(),
        show_recording_controls: show_recording_controls.id().clone(),
        quit: quit.id().clone(),
    })
}
