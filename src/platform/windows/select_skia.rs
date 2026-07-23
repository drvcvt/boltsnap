use std::sync::Arc;

use image::RgbaImage;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{COLORREF, POINT, RECT, SIZE};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CombineRgn,
    CreateCompatibleDC, CreateDIBSection, CreateRectRgn, CreateRoundRectRgn, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, RGN_OR, ReleaseDC, SelectObject,
    SetWindowRgn,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowDisplayAffinity, SetWindowLongPtrW,
    SetWindowPos, SetWindowTextW, ULW_ALPHA, UpdateLayeredWindow, WDA_EXCLUDEFROMCAPTURE,
    WS_CAPTION, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId, WindowLevel};

use crate::DynResult;
use crate::image_model::Rect;
use crate::selector::{edit, render};

const HANDLE_RADIUS: f64 = 9.0;
const MIN_SELECTION: f64 = 4.0;
const DRAG_SLOP: f64 = 3.0;
const SCREENSHOT_BACKGROUND_PERCENT: u16 = 80;

fn selection_cursor(
    position: PhysicalPosition<f64>,
    surface_width: u32,
    surface_height: u32,
) -> (f64, f64) {
    let map_axis = |coordinate: f64, extent: u32| {
        let extent = extent as f64;
        let coordinate = coordinate.clamp(0.0, extent);
        if extent > 1.0 && coordinate >= extent - 1.0 {
            extent
        } else {
            coordinate
        }
    };
    (
        map_axis(position.x, surface_width),
        map_axis(position.y, surface_height),
    )
}

fn screenshot_overlay(
    base: &tiny_skia::Pixmap,
    selection: Option<(f32, f32, f32, f32)>,
) -> tiny_skia::Pixmap {
    let width = base.width();
    let height = base.height();
    let mut frame = tiny_skia::Pixmap::new(width.max(1), height.max(1)).expect("overlay pixmap");
    let selection = selection.and_then(|(x, y, w, h)| {
        if w < 1.0 || h < 1.0 {
            return None;
        }
        let x0 = (x.max(0.0).floor() as u32).min(width);
        let y0 = (y.max(0.0).floor() as u32).min(height);
        let x1 = ((x + w).max(0.0).ceil() as u32).min(width);
        let y1 = ((y + h).max(0.0).ceil() as u32).min(height);
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
    });
    for (index, (pixel, source)) in frame
        .data_mut()
        .chunks_exact_mut(4)
        .zip(base.data().chunks_exact(4))
        .enumerate()
    {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let selected =
            selection.is_some_and(|(x0, y0, x1, y1)| x >= x0 && x < x1 && y >= y0 && y < y1);
        if selected {
            pixel.copy_from_slice(source);
        } else {
            pixel.copy_from_slice(&[
                (source[0] as u16 * SCREENSHOT_BACKGROUND_PERCENT / 100) as u8,
                (source[1] as u16 * SCREENSHOT_BACKGROUND_PERCENT / 100) as u8,
                (source[2] as u16 * SCREENSHOT_BACKGROUND_PERCENT / 100) as u8,
                255,
            ]);
        }
    }
    frame
}

struct LayeredSurface {
    memory_dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
}

impl LayeredSurface {
    fn new(width: u32, height: u32) -> DynResult<Self> {
        let memory_dc = unsafe { CreateCompatibleDC(None) };
        if memory_dc.0.is_null() {
            return Err("CreateCompatibleDC failed for selector overlay".into());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap =
            match unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) } {
                Ok(bitmap) => bitmap,
                Err(error) => {
                    unsafe {
                        let _ = DeleteDC(memory_dc);
                    }
                    return Err(
                        format!("CreateDIBSection failed for selector overlay: {error}").into(),
                    );
                }
            };
        let previous = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
        if previous.0.is_null() || bits.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(memory_dc);
            }
            return Err("SelectObject failed for selector overlay".into());
        }
        Ok(Self {
            memory_dc,
            bitmap,
            previous,
            bits: bits.cast(),
            width,
            height,
        })
    }

    fn present(
        &mut self,
        window: &Window,
        monitor: RECT,
        frame: &tiny_skia::Pixmap,
    ) -> DynResult<()> {
        if frame.width() != self.width || frame.height() != self.height {
            return Err("selector overlay dimensions changed".into());
        }
        let target = unsafe {
            std::slice::from_raw_parts_mut(
                self.bits,
                self.width as usize * self.height as usize * 4,
            )
        };
        for (source, destination) in frame.data().chunks_exact(4).zip(target.chunks_exact_mut(4)) {
            destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
        }

        let screen = unsafe { GetDC(None) };
        if screen.0.is_null() {
            return Err("GetDC failed for selector overlay".into());
        }
        let destination = POINT {
            x: monitor.left,
            y: monitor.top,
        };
        let size = SIZE {
            cx: self.width as i32,
            cy: self.height as i32,
        };
        let source = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let hwnd = window_hwnd(window)?;
        let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if extended_style & WS_EX_LAYERED.0 == 0 {
            unsafe {
                ReleaseDC(None, screen);
            }
            return Err(format!(
                "selector overlay window is not layered (extended style {extended_style:#x})"
            )
            .into());
        }
        let result = unsafe {
            UpdateLayeredWindow(
                hwnd,
                Some(screen),
                Some(std::ptr::from_ref(&destination)),
                Some(std::ptr::from_ref(&size)),
                Some(self.memory_dc),
                Some(std::ptr::from_ref(&source)),
                COLORREF(0),
                Some(std::ptr::from_ref(&blend)),
                ULW_ALPHA,
            )
        };
        unsafe {
            ReleaseDC(None, screen);
        }
        result.map_err(|error| {
            format!(
                "UpdateLayeredWindow failed for {}x{} overlay at {},{} with style {extended_style:#x}: {error}",
                self.width, self.height, monitor.left, monitor.top
            )
        })?;
        Ok(())
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memory_dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.memory_dc);
        }
    }
}

pub struct RecordSelectionResult {
    pub rect: Option<Rect>,
    pub show_frame: bool,
    pub audio_enabled: bool,
}

fn focused_monitor_info() -> DynResult<MONITORINFO> {
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor)? };
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Err("GetMonitorInfoW failed".into());
    }
    Ok(info)
}

pub(crate) fn focused_monitor_rect() -> DynResult<RECT> {
    Ok(focused_monitor_info()?.rcMonitor)
}

pub(crate) fn focused_monitor_work_rect() -> DynResult<RECT> {
    Ok(focused_monitor_info()?.rcWork)
}

pub(crate) fn configure_utility_window(window: &Window) -> DynResult<()> {
    configure_utility_window_style(window, false)
}

pub(crate) fn configure_nonactivating_utility_window(window: &Window) -> DynResult<()> {
    configure_utility_window_style(window, true)
}

fn configure_utility_window_style(window: &Window, no_activate: bool) -> DynResult<()> {
    let hwnd = window_hwnd(window)?;
    let chrome_mask =
        WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
    let current_window_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let window_style = (current_window_style & !chrome_mask) | WS_POPUP.0;
    let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let utility_style = (extended_style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
    let utility_style = if no_activate {
        utility_style | WS_EX_NOACTIVATE.0
    } else {
        utility_style & !WS_EX_NOACTIVATE.0
    };
    unsafe {
        if window_style != current_window_style || utility_style != extended_style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, window_style as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, utility_style as isize);
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
        }
        SetWindowTextW(hwnd, windows::core::w!(""))?;
        if std::env::var_os("BOLTSNAP_ALLOW_SELECTOR_CAPTURE").is_none() {
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }
    let applied_window_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let applied_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if applied_window_style & chrome_mask != 0
        || applied_window_style & WS_POPUP.0 == 0
        || applied_style & WS_EX_TOOLWINDOW.0 == 0
        || applied_style & WS_EX_APPWINDOW.0 != 0
        || no_activate != (applied_style & WS_EX_NOACTIVATE.0 != 0)
    {
        return Err("failed to apply borderless Boltsnap utility window style".into());
    }
    window.set_taskbar_icon(None);
    window.set_skip_taskbar(true);
    Ok(())
}

pub(crate) fn set_rounded_window_regions(
    window: &Window,
    rectangles: &[(u32, u32, u32, u32)],
    radius: u32,
) -> DynResult<()> {
    let hwnd = window_hwnd(window)?;
    let radius = i32::try_from(radius)?;
    let rectangles = rectangles
        .iter()
        .map(|&(x, y, width, height)| {
            Ok::<_, std::num::TryFromIntError>((
                i32::try_from(x)?,
                i32::try_from(y)?,
                i32::try_from(width)?,
                i32::try_from(height)?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let preference = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&preference).cast(),
            std::mem::size_of_val(&preference) as u32,
        );
    }
    let combined = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if combined.is_invalid() {
        return Err("failed to create rounded Windows shelf region".into());
    }
    for (x, y, width, height) in rectangles {
        let radius = radius.min(width / 2).min(height / 2).max(1);
        let card = unsafe {
            CreateRoundRectRgn(
                x,
                y,
                x.saturating_add(width).saturating_add(1),
                y.saturating_add(height).saturating_add(1),
                radius * 2,
                radius * 2,
            )
        };
        if card.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(combined.0));
            }
            return Err("failed to create rounded Windows shelf card region".into());
        }
        let result = unsafe { CombineRgn(Some(combined), Some(combined), Some(card), RGN_OR) };
        unsafe {
            let _ = DeleteObject(HGDIOBJ(card.0));
        }
        if result.0 == 0 {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(combined.0));
            }
            return Err("failed to combine rounded Windows shelf regions".into());
        }
    }
    if unsafe { SetWindowRgn(hwnd, Some(combined), true) } == 0 {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(combined.0));
        }
        return Err("failed to apply rounded Windows shelf region".into());
    }
    Ok(())
}

pub(crate) fn window_hwnd(window: &Window) -> DynResult<windows::Win32::Foundation::HWND> {
    let handle = window.window_handle()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err("Boltsnap utility window is not a Win32 window".into());
    };
    Ok(windows::Win32::Foundation::HWND(
        handle.hwnd.get() as *mut std::ffi::c_void
    ))
}

pub(crate) fn run_select_image(
    image: RgbaImage,
    monitor: RECT,
    instant: bool,
) -> DynResult<Option<RgbaImage>> {
    let result = run_selector(image.clone(), monitor, instant, false, true, true)?;
    let Some(rect) = result.rect else {
        return Ok(None);
    };
    let Some((x, y, width, height)) = render::rect_to_image(
        (rect.x, rect.y),
        (rect.right(), rect.bottom()),
        image.width(),
        image.height(),
        image.width(),
        image.height(),
    ) else {
        return Ok(None);
    };
    Ok(Some(
        image::imageops::crop_imm(&image, x, y, width, height).to_image(),
    ))
}

pub fn run_select_record(
    initial_show_frame: bool,
    initial_audio_enabled: bool,
) -> DynResult<RecordSelectionResult> {
    let monitor = focused_monitor_rect()?;
    let image = crate::platform::windows::capture::capture_rect(monitor)?;
    let result = run_selector(
        image,
        monitor,
        false,
        true,
        initial_show_frame,
        initial_audio_enabled,
    )?;
    Ok(RecordSelectionResult {
        rect: result.rect,
        show_frame: result.show_frame,
        audio_enabled: result.audio_enabled,
    })
}

struct SelectorResult {
    rect: Option<Rect>,
    show_frame: bool,
    audio_enabled: bool,
}

fn run_selector(
    image: RgbaImage,
    monitor: RECT,
    instant: bool,
    record_mode: bool,
    show_frame: bool,
    audio_enabled: bool,
) -> DynResult<SelectorResult> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut application = SelectorApplication::new(
        image,
        monitor,
        instant,
        record_mode,
        show_frame,
        audio_enabled,
    );
    event_loop.run_app(&mut application)?;
    if let Some(error) = application.error {
        return Err(error.into());
    }
    Ok(SelectorResult {
        rect: application.result,
        show_frame: application.show_frame,
        audio_enabled: application.audio_enabled,
    })
}

#[derive(Clone, Copy)]
enum Mode {
    Idle,
    Drawing { anchor: (f64, f64), now: (f64, f64) },
    Editing { rect: Rect },
}

#[derive(Clone, Copy)]
enum Interaction {
    Resize { handle: edit::Handle },
    Move { grab: (f64, f64) },
    ClickInside { press: (f64, f64) },
}

struct SelectorApplication {
    image: RgbaImage,
    base: tiny_skia::Pixmap,
    monitor: RECT,
    instant: bool,
    record_mode: bool,
    show_frame: bool,
    audio_enabled: bool,
    window: Option<Arc<Window>>,
    layered_surface: Option<LayeredSurface>,
    mode: Mode,
    interaction: Option<Interaction>,
    cursor: (f64, f64),
    modifiers: ModifiersState,
    result: Option<Rect>,
    error: Option<String>,
}

impl SelectorApplication {
    fn new(
        image: RgbaImage,
        monitor: RECT,
        instant: bool,
        record_mode: bool,
        show_frame: bool,
        audio_enabled: bool,
    ) -> Self {
        let base = render::base_pixmap_from_image(&image, image.width(), image.height());
        Self {
            image,
            base,
            monitor,
            instant,
            record_mode,
            show_frame,
            audio_enabled,
            window: None,
            layered_surface: None,
            mode: Mode::Idle,
            interaction: None,
            cursor: (0.0, 0.0),
            modifiers: ModifiersState::default(),
            result: None,
            error: None,
        }
    }

    fn attributes(&self) -> WindowAttributes {
        Window::default_attributes()
            .with_title("Boltsnap Selector")
            .with_window_icon(Some(crate::platform::windows::app_window_icon()))
            .with_decorations(false)
            .with_resizable(false)
            .with_skip_taskbar(true)
            .with_taskbar_icon(None)
            .with_undecorated_shadow(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(self.monitor.left, self.monitor.top))
            .with_inner_size(PhysicalSize::new(self.image.width(), self.image.height()))
            .with_visible(false)
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn selection(&self) -> Option<Rect> {
        match self.mode {
            Mode::Idle => None,
            Mode::Drawing { anchor, now } => Some(Rect::from_corners(anchor, now)),
            Mode::Editing { rect } => Some(rect),
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }

    fn confirm(&mut self, event_loop: &ActiveEventLoop, rect: Rect) {
        if rect.w >= MIN_SELECTION && rect.h >= MIN_SELECTION {
            self.result = Some(rect);
            event_loop.exit();
        } else {
            self.mode = Mode::Idle;
            self.request_redraw();
        }
    }

    fn press(&mut self, event_loop: &ActiveEventLoop) {
        match self.mode {
            Mode::Idle | Mode::Drawing { .. } => {
                self.mode = Mode::Drawing {
                    anchor: self.cursor,
                    now: self.cursor,
                };
            }
            Mode::Editing { rect } => {
                if self.record_mode {
                    let selection = (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
                    if render::record_audio_button_rect(
                        selection,
                        self.image.width(),
                        self.image.height(),
                    )
                    .is_some_and(|control| contains(control, self.cursor))
                    {
                        self.audio_enabled = !self.audio_enabled;
                        self.request_redraw();
                        return;
                    }
                    if render::rec_pill_rect(selection, self.image.width(), self.image.height())
                        .is_some_and(|control| contains(control, self.cursor))
                    {
                        self.confirm(event_loop, rect);
                        return;
                    }
                }
                self.interaction = match edit::hit_region(rect, self.cursor, HANDLE_RADIUS) {
                    edit::Region::Handle(handle) => Some(Interaction::Resize { handle }),
                    edit::Region::Inside => Some(Interaction::ClickInside { press: self.cursor }),
                    edit::Region::Outside => {
                        self.mode = Mode::Drawing {
                            anchor: self.cursor,
                            now: self.cursor,
                        };
                        None
                    }
                };
            }
        }
        self.request_redraw();
    }

    fn release(&mut self, event_loop: &ActiveEventLoop) {
        match self.mode {
            Mode::Drawing { anchor, now } => {
                let rect = Rect::from_corners(anchor, now);
                if self.instant {
                    self.confirm(event_loop, rect);
                } else if rect.w >= MIN_SELECTION && rect.h >= MIN_SELECTION {
                    self.mode = Mode::Editing { rect };
                } else {
                    self.mode = Mode::Idle;
                }
            }
            Mode::Editing { rect } => {
                if matches!(self.interaction, Some(Interaction::ClickInside { .. })) {
                    self.confirm(event_loop, rect);
                }
                self.interaction = None;
            }
            Mode::Idle => {}
        }
        self.request_redraw();
    }

    fn moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = selection_cursor(position, self.image.width(), self.image.height());
        match self.mode {
            Mode::Drawing { anchor, .. } => {
                self.mode = Mode::Drawing {
                    anchor,
                    now: self.cursor,
                };
            }
            Mode::Editing { rect } => match self.interaction {
                Some(Interaction::Resize { handle }) => {
                    self.mode = Mode::Editing {
                        rect: edit::resize_rect(
                            rect,
                            handle,
                            self.cursor,
                            MIN_SELECTION,
                            self.image.width() as f64,
                            self.image.height() as f64,
                        ),
                    };
                }
                Some(Interaction::ClickInside { press })
                    if (self.cursor.0 - press.0).abs() > DRAG_SLOP
                        || (self.cursor.1 - press.1).abs() > DRAG_SLOP =>
                {
                    self.interaction = Some(Interaction::Move {
                        grab: (press.0 - rect.x, press.1 - rect.y),
                    });
                }
                Some(Interaction::Move { grab }) => {
                    self.mode = Mode::Editing {
                        rect: edit::move_rect(
                            rect,
                            self.cursor.0 - grab.0 - rect.x,
                            self.cursor.1 - grab.1 - rect.y,
                            self.image.width() as f64,
                            self.image.height() as f64,
                        ),
                    };
                }
                _ => {}
            },
            Mode::Idle => {}
        }
        self.request_redraw();
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let width = self.image.width().max(1);
        let height = self.image.height().max(1);
        let selection = self
            .selection()
            .map(|rect| (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32));
        let overlay_style = if self.record_mode {
            render::WINDOWS_RECORD_OVERLAY_STYLE
        } else {
            render::SCREENSHOT_OVERLAY_STYLE
        };
        let mut frame = if self.record_mode {
            let mut frame = self.base.clone();
            render::dim_and_restore_with_style(&mut frame, selection, overlay_style);
            frame
        } else {
            screenshot_overlay(&self.base, selection)
        };
        if let Some(selection) = selection {
            render::draw_border_with_style(&mut frame, selection, overlay_style);
            if matches!(self.mode, Mode::Editing { .. }) {
                render::draw_handles_with_style(&mut frame, selection, overlay_style);
            }
            if self.record_mode {
                render::draw_rec_pill(&mut frame, selection, width, height);
                // No frame checkbox on Windows: there is no recording border
                // overlay, so the selector only offers the audio toggle and
                // `show_frame` passes through unchanged.
                render::draw_record_audio_button(
                    &mut frame,
                    selection,
                    width,
                    height,
                    self.audio_enabled,
                );
            } else {
                render::draw_badge(&mut frame, selection, width, height);
            }
        }
        if self.modifiers.alt_key() && !self.record_mode {
            render::draw_magnifier(&mut frame, &self.base, self.cursor, width, height);
        }

        let (Some(window), Some(surface)) = (self.window.as_ref(), self.layered_surface.as_mut())
        else {
            return;
        };
        if let Err(error) = surface.present(window, self.monitor, &frame) {
            self.fail(event_loop, error);
        }
    }
}

impl ApplicationHandler for SelectorApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(self.attributes()) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        if let Err(error) = configure_utility_window(&window) {
            self.fail(event_loop, error);
            return;
        }
        self.layered_surface = match LayeredSurface::new(self.image.width(), self.image.height()) {
            Ok(surface) => Some(surface),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        window.set_cursor(CursorIcon::Crosshair);
        window.set_visible(true);
        if let Err(error) = configure_utility_window(&window) {
            self.fail(event_loop, error);
            return;
        }
        let hwnd = match window_hwnd(&window) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (extended_style | WS_EX_LAYERED.0) as isize,
            );
        }
        window.focus_window();
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != window_id)
        {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { position, .. } => self.moved(position),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                self.request_redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => self.press(event_loop),
                ElementState::Released => self.release(event_loop),
            },
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => {
                        if let Some(rect) = self.selection() {
                            self.confirm(event_loop, rect);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }
}

fn contains(rect: (f64, f64, f64, f64), point: (f64, f64)) -> bool {
    point.0 >= rect.0
        && point.0 <= rect.0 + rect.2
        && point.1 >= rect.1
        && point.1 <= rect.1 + rect.3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_drag_includes_last_windows_pixel() {
        let start = selection_cursor(PhysicalPosition::new(0.0, 0.0), 1920, 1080);
        let end = selection_cursor(PhysicalPosition::new(1919.0, 1079.0), 1920, 1080);
        let selection = Rect::from_corners(start, end);

        assert_eq!(
            selection,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
            }
        );
    }

    #[test]
    fn selection_cursor_keeps_interior_coordinates() {
        assert_eq!(
            selection_cursor(PhysicalPosition::new(640.5, 360.5), 1920, 1080),
            (640.5, 360.5)
        );
    }

    #[test]
    fn screenshot_overlay_restores_original_inside_and_dims_outside() {
        let mut base = tiny_skia::Pixmap::new(4, 3).unwrap();
        for pixel in base.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[180, 140, 100, 255]);
        }
        let frame = screenshot_overlay(&base, Some((1.0, 1.0, 2.0, 1.0)));

        for y in 0..frame.height() {
            for x in 0..frame.width() {
                let offset = ((y * frame.width() + x) * 4) as usize;
                let pixel = &frame.data()[offset..offset + 4];
                if y == 1 && (1..3).contains(&x) {
                    assert_eq!(pixel, &[180, 140, 100, 255]);
                } else {
                    assert_eq!(pixel, &[144, 112, 80, 255]);
                }
            }
        }
    }

    #[test]
    fn screenshot_overlay_dims_entire_monitor_before_selection() {
        let mut base = tiny_skia::Pixmap::new(1, 1).unwrap();
        base.data_mut().copy_from_slice(&[180, 140, 100, 255]);
        let frame = screenshot_overlay(&base, None);

        assert_eq!(frame.data(), &[144, 112, 80, 255]);
    }
}
