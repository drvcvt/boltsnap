use std::num::NonZeroU32;
use std::sync::Arc;

use image::RgbaImage;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use softbuffer::{Context, Surface};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, RGN_OR, SetWindowRgn,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowDisplayAffinity, SetWindowLongPtrW,
    SetWindowPos, SetWindowTextW, WDA_EXCLUDEFROMCAPTURE, WS_CAPTION, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
    WS_THICKFRAME,
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
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
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
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
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
            context: None,
            surface: None,
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
                    if render::record_frame_checkbox_rect(
                        selection,
                        self.image.width(),
                        self.image.height(),
                    )
                    .is_some_and(|control| contains(control, self.cursor))
                    {
                        self.show_frame = !self.show_frame;
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
        self.cursor = (
            position.x.clamp(0.0, self.image.width() as f64),
            position.y.clamp(0.0, self.image.height() as f64),
        );
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
        let mut frame = self.base.clone();
        let selection = self
            .selection()
            .map(|rect| (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32));
        render::dim_and_restore(&mut frame, selection);
        if let Some(selection) = selection {
            render::draw_border(&mut frame, selection);
            if matches!(self.mode, Mode::Editing { .. }) {
                render::draw_handles(&mut frame, selection);
            }
            if self.record_mode {
                render::draw_rec_pill(&mut frame, selection, width, height);
                render::draw_record_frame_checkbox(
                    &mut frame,
                    selection,
                    width,
                    height,
                    self.show_frame,
                );
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

        let Some(surface) = self.surface.as_mut() else {
            return;
        };

        if let Err(error) = surface.resize(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        ) {
            self.fail(event_loop, error);
            return;
        }
        let mut buffer = match surface.buffer_mut() {
            Ok(buffer) => buffer,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        for (target, pixel) in buffer.iter_mut().zip(frame.data().chunks_exact(4)) {
            *target = (pixel[0] as u32) << 16 | (pixel[1] as u32) << 8 | pixel[2] as u32;
        }
        if let Err(error) = buffer.present() {
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
        let context = match Context::new(window.clone()) {
            Ok(context) => context,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        let surface = match Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        if let Err(error) = configure_utility_window(&window) {
            self.fail(event_loop, error);
            return;
        }
        window.set_cursor(CursorIcon::Crosshair);
        window.set_visible(true);
        if let Err(error) = configure_utility_window(&window) {
            self.fail(event_loop, error);
            return;
        }
        window.focus_window();
        window.request_redraw();
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
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
