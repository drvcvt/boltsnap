use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::DROPFILES;
use windows::core::BOOL;

use crate::{Backend, DynResult};

pub(crate) fn copy_to_clipboard(path: &Path, backend: Backend) -> DynResult<Backend> {
    let backend = backend.resolved()?;
    let image = image::open(path)?.to_rgba8();
    let (width, height) = (image.width() as usize, image.height() as usize);
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("open Windows clipboard: {error}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Owned(image.into_raw()),
        })
        .map_err(|error| format!("copy image to Windows clipboard: {error}"))?;
    Ok(backend)
}

/// Copy a file reference using a real owner window. Win32 rejects
/// `SetClipboardData` after `EmptyClipboard` when `OpenClipboard` received NULL.
pub fn copy_uri_to_clipboard(path: &Path, owner: HWND) -> DynResult<()> {
    let path = crate::paths::normalize_path(path);
    crate::paths::ensure_file(&path)?;
    let mut wide = path
        .as_os_str()
        .encode_wide()
        .chain([0, 0])
        .collect::<Vec<_>>();
    if wide.len() < 2 {
        wide.extend([0, 0]);
    }
    let header_size = std::mem::size_of::<DROPFILES>();
    let allocation_size = header_size + wide.len() * std::mem::size_of::<u16>();
    let memory = unsafe { GlobalAlloc(GHND, allocation_size)? };
    let data = unsafe { GlobalLock(memory) };
    if data.is_null() {
        unsafe {
            let _ = GlobalFree(Some(memory));
        }
        return Err("GlobalLock failed for file clipboard".into());
    }
    let header = DROPFILES {
        pFiles: header_size as u32,
        fWide: BOOL(1),
        ..Default::default()
    };
    unsafe {
        std::ptr::write_unaligned(data.cast::<DROPFILES>(), header);
        std::ptr::copy_nonoverlapping(
            wide.as_ptr().cast::<u8>(),
            data.cast::<u8>().add(header_size),
            wide.len() * std::mem::size_of::<u16>(),
        );
        let _ = GlobalUnlock(memory);
    }

    let mut opened = false;
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !opened {
        unsafe {
            let _ = GlobalFree(Some(memory));
        }
        return Err("open Windows clipboard for file reference failed".into());
    }
    // No `?` between OpenClipboard and CloseClipboard: an early return would
    // leave the clipboard open and block every other application's clipboard
    // for the daemon's lifetime.
    let result = (|| unsafe {
        EmptyClipboard().map_err(|error| format!("empty Windows clipboard: {error}"))?;
        SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(memory.0)))
            .map_err(|error| format!("set Windows file clipboard: {error}"))?;
        Ok::<(), String>(())
    })();
    unsafe {
        let _ = CloseClipboard();
    }
    if let Err(error) = result {
        unsafe {
            let _ = GlobalFree(Some(memory));
        }
        return Err(error.into());
    }
    Ok(())
}

pub fn serve_wayland_clipboard(_path: &Path) -> DynResult<()> {
    Err("the Wayland clipboard helper is unavailable on Windows".into())
}

pub fn serve_wayland_uri_list(_path: &Path) -> DynResult<()> {
    Err("the Wayland URI helper is unavailable on Windows".into())
}
