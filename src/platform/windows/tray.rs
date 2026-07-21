use std::sync::atomic::{AtomicBool, Ordering};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_TIP, NIIF_ERROR, NIM_ADD, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{IDI_APPLICATION, LoadIconW};

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

const NOTIFY_ICON_ID: u32 = 2;
static NOTIFY_ICON_ADDED: AtomicBool = AtomicBool::new(false);

/// Show an error balloon on a Boltsnap notification-area icon. The daemon has
/// no console, so this is its only user-visible error channel; call it for
/// errors only, success paths stay silent.
pub fn notify_error(window: &winit::window::Window, body: &str) {
    if let Err(error) = show_error_balloon(window, body) {
        eprintln!("boltsnap daemon: error balloon failed: {error}");
    }
}

fn show_error_balloon(window: &winit::window::Window, body: &str) -> Result<(), String> {
    let hwnd = crate::platform::windows::select_skia::window_hwnd(window)
        .map_err(|error| error.to_string())?;
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: NOTIFY_ICON_ID,
        uFlags: NIF_ICON | NIF_TIP | NIF_INFO,
        dwInfoFlags: NIIF_ERROR,
        ..Default::default()
    };
    data.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION) }.map_err(|error| error.to_string())?;
    copy_to_wide(&mut data.szTip, "Boltsnap");
    copy_to_wide(&mut data.szInfoTitle, "Boltsnap");
    copy_to_wide(&mut data.szInfo, body);
    let message = if NOTIFY_ICON_ADDED.load(Ordering::Acquire) {
        NIM_MODIFY
    } else {
        NIM_ADD
    };
    if !unsafe { Shell_NotifyIconW(message, &data) }.as_bool() {
        // The icon is lost when Explorer restarts; retry with the other verb.
        let retry = if message == NIM_ADD {
            NIM_MODIFY
        } else {
            NIM_ADD
        };
        if !unsafe { Shell_NotifyIconW(retry, &data) }.as_bool() {
            return Err("Shell_NotifyIconW rejected the error balloon".into());
        }
    }
    NOTIFY_ICON_ADDED.store(true, Ordering::Release);
    Ok(())
}

fn copy_to_wide(target: &mut [u16], text: &str) {
    let mut length = 0;
    for unit in text.encode_utf16() {
        if length + 1 >= target.len() {
            break;
        }
        target[length] = unit;
        length += 1;
    }
    target[length] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_copy_truncates_and_terminates() {
        let mut small = [0xFFFF_u16; 4];
        copy_to_wide(&mut small, "abcdef");
        assert_eq!(small, [0x61, 0x62, 0x63, 0]);

        let mut roomy = [0xFFFF_u16; 8];
        copy_to_wide(&mut roomy, "ok");
        assert_eq!(&roomy[..3], [0x6F, 0x6B, 0]);
    }
}
