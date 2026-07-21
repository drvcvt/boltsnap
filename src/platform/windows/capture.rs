use std::ffi::c_void;
use std::fs;
use std::mem::size_of;
use std::path::Path;

use image::{RgbaImage, imageops};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, POINT, RECT,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HGDIOBJ, MONITORINFO,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WindowFromPoint,
};

use crate::{Backend, CaptureMode, DynResult};

pub(crate) fn capture(
    mode: CaptureMode,
    output: &Path,
    backend: Backend,
    instant: bool,
) -> DynResult<(Backend, Option<String>)> {
    let backend = backend.resolved()?;
    if backend != Backend::Windows {
        return Err(format!("backend {} is unavailable on Windows", backend.as_str()).into());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let image = match mode {
        CaptureMode::Full => capture_rect(virtual_screen_rect()?)?,
        CaptureMode::ActiveWindow => capture_window(unsafe { GetForegroundWindow() })?,
        CaptureMode::Window => capture_window(window_under_pointer()?)?,
        CaptureMode::Area => {
            let _selection_session = SelectionSession::acquire()?;
            let monitor = crate::platform::windows::select_skia::focused_monitor_rect()?;
            let screenshot = capture_rect(monitor)?;
            crate::platform::windows::select_skia::run_select_image(screenshot, monitor, instant)?
                .ok_or("selection cancelled")?
        }
    };
    image::DynamicImage::ImageRgba8(image)
        .to_rgb8()
        .save(output)
        .map_err(|error| format!("png encode failed: {error}"))?;
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(format!("capture did not create {}", output.display()).into());
    }
    Ok((backend, None))
}

fn virtual_screen_rect() -> DynResult<RECT> {
    let rect = RECT {
        left: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        top: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        right: 0,
        bottom: 0,
    };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    valid_rect(RECT {
        right: rect.left + width,
        bottom: rect.top + height,
        ..rect
    })
}

fn window_under_pointer() -> DynResult<HWND> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point)? };
    let window = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
    if window.0.is_null() {
        Err("no window found under the pointer".into())
    } else {
        Ok(window)
    }
}

fn capture_window(window: HWND) -> DynResult<RgbaImage> {
    if window.0.is_null() {
        return Err("no active Windows window available".into());
    }
    match capture_window_wgc(window) {
        Ok(image) => Ok(image),
        Err(wgc_error) => {
            eprintln!(
                "boltsnap: Windows Graphics Capture unavailable, using GDI compatibility fallback: {wgc_error}"
            );
            capture_rect_gdi(window_rect(window)?).map_err(|gdi_error| {
                format!("WGC failed ({wgc_error}); GDI fallback failed ({gdi_error})").into()
            })
        }
    }
}

fn capture_window_wgc(window: HWND) -> DynResult<RgbaImage> {
    use std::sync::mpsc::{SyncSender, sync_channel};

    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    struct StillCapture {
        sender: Option<SyncSender<RgbaImage>>,
    }

    impl GraphicsCaptureApiHandler for StillCapture {
        type Flags = SyncSender<RgbaImage>;
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self {
                sender: Some(context.flags),
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let buffer = frame.buffer()?;
            let width = buffer.width();
            let height = buffer.height();
            let mut packed = Vec::new();
            let bytes = buffer.as_nopadding_buffer(&mut packed);
            let image = RgbaImage::from_raw(width, height, bytes.to_vec())
                .ok_or("could not construct WGC window image")?;
            if let Some(sender) = self.sender.take() {
                sender.send(image)?;
            }
            control.stop();
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    let (sender, receiver) = sync_channel(1);
    let item = Window::from_raw_hwnd(window.0);
    let settings = Settings::new(
        item,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        sender,
    );
    StillCapture::start(settings)?;
    Ok(receiver.recv()?)
}

fn window_rect(window: HWND) -> DynResult<RECT> {
    let mut rect = RECT::default();
    let dwm = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if dwm.is_err() {
        unsafe { GetWindowRect(window, &mut rect)? };
    }
    valid_rect(rect)
}

fn valid_rect(rect: RECT) -> DynResult<RECT> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        Err(format!(
            "invalid capture rectangle {},{} {}x{}",
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top
        )
        .into())
    } else {
        Ok(rect)
    }
}

pub(crate) fn capture_rect(rect: RECT) -> DynResult<RgbaImage> {
    match capture_rect_dxgi(rect) {
        Ok(image) if looks_like_empty_frame(&image) => {
            eprintln!("boltsnap: DXGI returned an empty frame, using GDI compatibility fallback");
            match capture_rect_gdi(rect) {
                Ok(fallback) => Ok(fallback),
                Err(error) => {
                    eprintln!(
                        "boltsnap: GDI fallback after empty DXGI frame failed, keeping DXGI frame: {error}"
                    );
                    Ok(image)
                }
            }
        }
        Ok(image) => Ok(image),
        Err(dxgi_error) => {
            eprintln!("boltsnap: DXGI unavailable, using GDI compatibility fallback: {dxgi_error}");
            capture_rect_gdi(rect).map_err(|gdi_error| {
                format!("DXGI capture failed ({dxgi_error}); GDI fallback failed ({gdi_error})")
                    .into()
            })
        }
    }
}

fn looks_like_empty_frame(image: &RgbaImage) -> bool {
    if image.width() == 0 || image.height() == 0 {
        return true;
    }
    let step_x = (image.width() / 64).max(1) as usize;
    let step_y = (image.height() / 64).max(1) as usize;
    let mut samples = 0_u32;
    let mut black_samples = 0_u32;
    for y in (0..image.height()).step_by(step_y) {
        for x in (0..image.width()).step_by(step_x) {
            let pixel = image.get_pixel(x, y);
            samples += 1;
            if pixel[0] <= 4 && pixel[1] <= 4 && pixel[2] <= 4 {
                black_samples += 1;
            }
        }
    }
    black_samples.saturating_mul(200) >= samples.saturating_mul(199)
}

struct SelectionSession(HANDLE);

impl SelectionSession {
    fn acquire() -> DynResult<Self> {
        let mutex = unsafe {
            CreateMutexW(
                None,
                true,
                windows::core::w!("Local\\BoltsnapCaptureSelection"),
            )?
        };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(mutex);
            }
            return Err("another Boltsnap selection is already open".into());
        }
        Ok(Self(mutex))
    }
}

impl Drop for SelectionSession {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn capture_rect_dxgi(rect: RECT) -> DynResult<RgbaImage> {
    use windows_capture::dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat};
    use windows_capture::monitor::Monitor;

    let rect = valid_rect(rect)?;
    let mut output = RgbaImage::new(
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
    );
    let mut covered_pixels = 0_u64;
    for monitor in Monitor::enumerate()? {
        let monitor_rect = monitor_rect(monitor.as_raw_hmonitor())?;
        let Some(intersection) = intersect(rect, monitor_rect) else {
            continue;
        };
        let mut duplication = DxgiDuplicationApi::new(monitor)?;
        let mut frame = duplication.acquire_next_frame(250)?;
        let buffer = frame.buffer()?;
        let width = buffer.width();
        let height = buffer.height();
        let format = buffer.format();
        let mut packed = Vec::new();
        let bytes = buffer.as_nopadding_buffer(&mut packed);
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        match format {
            DxgiDuplicationFormat::Bgra8 | DxgiDuplicationFormat::Bgra8Srgb => {
                for pixel in bytes.chunks_exact(4) {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                }
            }
            DxgiDuplicationFormat::Rgba8 | DxgiDuplicationFormat::Rgba8Srgb => {
                for pixel in bytes.chunks_exact(4) {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
            other => return Err(format!("unsupported DXGI desktop format {other:?}").into()),
        }
        let monitor_image = RgbaImage::from_raw(width, height, rgba)
            .ok_or("could not construct DXGI monitor image")?;
        let source_x = (intersection.left - monitor_rect.left) as u32;
        let source_y = (intersection.top - monitor_rect.top) as u32;
        let source_width = (intersection.right - intersection.left) as u32;
        let source_height = (intersection.bottom - intersection.top) as u32;
        let cropped = imageops::crop_imm(
            &monitor_image,
            source_x,
            source_y,
            source_width,
            source_height,
        )
        .to_image();
        imageops::replace(
            &mut output,
            &cropped,
            (intersection.left - rect.left) as i64,
            (intersection.top - rect.top) as i64,
        );
        covered_pixels += source_width as u64 * source_height as u64;
    }
    let expected = output.width() as u64 * output.height() as u64;
    if covered_pixels != expected {
        return Err(format!("DXGI covered {covered_pixels} of {expected} requested pixels").into());
    }
    Ok(output)
}

fn monitor_rect(handle: *mut c_void) -> DynResult<RECT> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe {
        !GetMonitorInfoW(windows::Win32::Graphics::Gdi::HMONITOR(handle), &mut info).as_bool()
    } {
        return Err("GetMonitorInfoW failed for DXGI output".into());
    }
    Ok(info.rcMonitor)
}

fn intersect(first: RECT, second: RECT) -> Option<RECT> {
    let rect = RECT {
        left: first.left.max(second.left),
        top: first.top.max(second.top),
        right: first.right.min(second.right),
        bottom: first.bottom.min(second.bottom),
    };
    (rect.right > rect.left && rect.bottom > rect.top).then_some(rect)
}

fn capture_rect_gdi(rect: RECT) -> DynResult<RgbaImage> {
    let rect = valid_rect(rect)?;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let screen = unsafe { GetDC(None) };
    if screen.0.is_null() {
        return Err("GetDC failed".into());
    }
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    if memory.0.is_null() {
        unsafe { ReleaseDC(None, screen) };
        return Err("CreateCompatibleDC failed".into());
    }

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) };
    let bitmap = match bitmap {
        Ok(bitmap) => bitmap,
        Err(error) => {
            unsafe {
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
            }
            return Err(format!("CreateDIBSection failed: {error}").into());
        }
    };
    let previous = unsafe { SelectObject(memory, HGDIOBJ(bitmap.0)) };
    let copied = unsafe {
        BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            rect.left,
            rect.top,
            SRCCOPY | CAPTUREBLT,
        )
    };

    let result = if copied.is_ok() && !bits.is_null() {
        let len = width as usize * height as usize * 4;
        let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), len) };
        let mut rgba = Vec::with_capacity(len);
        for pixel in bgra.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
        RgbaImage::from_raw(width as u32, height as u32, rgba)
            .ok_or_else(|| "could not build captured image".into())
    } else {
        Err(format!("BitBlt failed: {:?}", copied.err()).into())
    };

    unsafe {
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }
    result
}

pub fn strip_uniform_border(path: &Path) -> DynResult<()> {
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    if width < 32 || height < 32 {
        return Ok(());
    }
    let edge = *image.get_pixel(0, 0);
    let mut peel = 0;
    while peel < 4 {
        let row = |y| (peel..width - peel).all(|x| *image.get_pixel(x, y) == edge);
        let column = |x| (peel..height - peel).all(|y| *image.get_pixel(x, y) == edge);
        if !(row(peel) && row(height - 1 - peel) && column(peel) && column(width - 1 - peel)) {
            break;
        }
        peel += 1;
    }
    if peel > 0 {
        imageops::crop_imm(&image, peel, peel, width - peel * 2, height - peel * 2)
            .to_image()
            .save(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn empty_frame_detection_rejects_black_dxgi_frames() {
        let black = RgbaImage::from_pixel(128, 128, Rgba([0, 0, 0, 255]));
        assert!(looks_like_empty_frame(&black));

        let mut desktop = black;
        for pixel in desktop.pixels_mut().take(256) {
            *pixel = Rgba([32, 48, 64, 255]);
        }
        assert!(!looks_like_empty_frame(&desktop));
    }
}
