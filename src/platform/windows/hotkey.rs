use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_LSHIFT, VK_LWIN,
    VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SNAPSHOT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::DynResult;

const VK_S: u32 = 0x53;
const RECORD_HOTKEY_ID: i32 = 0xB018;

#[derive(Clone, Copy)]
enum ShortcutAction {
    Area,
    Record,
}

static LAUNCH_SENDER: OnceLock<SyncSender<ShortcutAction>> = OnceLock::new();
static LEFT_WIN_DOWN: AtomicBool = AtomicBool::new(false);
static RIGHT_WIN_DOWN: AtomicBool = AtomicBool::new(false);
static LEFT_SHIFT_DOWN: AtomicBool = AtomicBool::new(false);
static RIGHT_SHIFT_DOWN: AtomicBool = AtomicBool::new(false);
static GENERIC_SHIFT_DOWN: AtomicBool = AtomicBool::new(false);
static PRINT_SCREEN_BLOCKED: AtomicBool = AtomicBool::new(false);
static WIN_SHIFT_S_BLOCKED: AtomicBool = AtomicBool::new(false);

pub fn register_snipping_shortcuts() -> DynResult<()> {
    let (launch_tx, launch_rx) = mpsc::sync_channel(1);
    LAUNCH_SENDER
        .set(launch_tx)
        .map_err(|_| "Windows snipping shortcut hook is already initialized")?;
    std::thread::Builder::new()
        .name("boltsnap-hotkey-launcher".into())
        .spawn(move || {
            while let Ok(action) = launch_rx.recv() {
                launch_shortcut(action);
            }
        })?;

    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("boltsnap-snipping-shortcut-hook".into())
        .spawn(move || {
            let module = unsafe { GetModuleHandleW(None) }
                .map(|module| HINSTANCE(module.0))
                .map_err(|error| error.to_string());
            let hook = module.and_then(|module| unsafe {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module), 0)
                    .map_err(|error| error.to_string())
            });
            let hook = match hook {
                Ok(hook) => hook,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            if let Err(error) = unsafe {
                RegisterHotKey(
                    None,
                    RECORD_HOTKEY_ID,
                    MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                    VK_S,
                )
            } {
                unsafe {
                    let _ = UnhookWindowsHookEx(hook);
                }
                let _ = ready_tx.send(Err(error.to_string()));
                return;
            }
            let _ = ready_tx.send(Ok(()));

            let mut message = MSG::default();
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
                if message.message == WM_HOTKEY
                    && message.wParam.0 as i32 == RECORD_HOTKEY_ID
                    && let Some(sender) = LAUNCH_SENDER.get()
                {
                    let _ = sender.try_send(ShortcutAction::Record);
                }
            }
            unsafe {
                let _ = UnregisterHotKey(None, RECORD_HOTKEY_ID);
                let _ = UnhookWindowsHookEx(hook);
            }
        })?;

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("register Windows snipping shortcut hook: {error}").into()),
        Err(error) => Err(format!("Windows shortcut hook did not initialize: {error}").into()),
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let is_down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    let is_up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
    if !is_down && !is_up {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let keyboard = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    match keyboard.vkCode {
        value if value == VK_LWIN.0 as u32 => LEFT_WIN_DOWN.store(is_down, Ordering::Release),
        value if value == VK_RWIN.0 as u32 => RIGHT_WIN_DOWN.store(is_down, Ordering::Release),
        value if value == VK_LSHIFT.0 as u32 => LEFT_SHIFT_DOWN.store(is_down, Ordering::Release),
        value if value == VK_RSHIFT.0 as u32 => RIGHT_SHIFT_DOWN.store(is_down, Ordering::Release),
        value if value == VK_SHIFT.0 as u32 => GENERIC_SHIFT_DOWN.store(is_down, Ordering::Release),
        value if value == VK_SNAPSHOT.0 as u32 => {
            if is_down {
                notify_once(&PRINT_SCREEN_BLOCKED, ShortcutAction::Area);
                return LRESULT(1);
            }
            if PRINT_SCREEN_BLOCKED.swap(false, Ordering::AcqRel) {
                return LRESULT(1);
            }
        }
        VK_S => {
            if is_down && win_is_down() && shift_is_down() {
                notify_once(&WIN_SHIFT_S_BLOCKED, ShortcutAction::Area);
                return LRESULT(1);
            }
            if is_up && WIN_SHIFT_S_BLOCKED.swap(false, Ordering::AcqRel) {
                return LRESULT(1);
            }
        }
        _ => {}
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn notify_once(blocked: &AtomicBool, action: ShortcutAction) {
    if !blocked.swap(true, Ordering::AcqRel)
        && let Some(sender) = LAUNCH_SENDER.get()
    {
        let _ = sender.try_send(action);
    }
}

fn win_is_down() -> bool {
    LEFT_WIN_DOWN.load(Ordering::Acquire) || RIGHT_WIN_DOWN.load(Ordering::Acquire)
}

fn shift_is_down() -> bool {
    LEFT_SHIFT_DOWN.load(Ordering::Acquire)
        || RIGHT_SHIFT_DOWN.load(Ordering::Acquire)
        || GENERIC_SHIFT_DOWN.load(Ordering::Acquire)
}

fn launch_shortcut(action: ShortcutAction) {
    let result = std::env::current_exe().and_then(|executable| {
        let mut command = Command::new(executable);
        command
            .args(shortcut_arguments(action))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::paths::spawn_reaped(&mut command)
    });
    if let Err(error) = result {
        eprintln!("boltsnap daemon: launch shortcut: {error}");
    }
}

fn shortcut_arguments(action: ShortcutAction) -> &'static [&'static str] {
    match action {
        ShortcutAction::Area => &["area", "--instant"],
        ShortcutAction::Record => &["record"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_shortcut_captures_on_release() {
        assert_eq!(
            shortcut_arguments(ShortcutAction::Area),
            ["area", "--instant"]
        );
        assert_eq!(shortcut_arguments(ShortcutAction::Record), ["record"]);
    }
}
