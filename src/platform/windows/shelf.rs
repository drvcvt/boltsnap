use std::fs::File;
use std::io::Write;
use std::num::NonZeroU32;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

use image::{Rgba, RgbaImage};
use softbuffer::{Context, Surface};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::{PCWSTR, w};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use crate::shelf::layout::{Hit, Layout, LayoutConfig};
use crate::shelf::model::{CardKind, FileLifetime, ShelfModel};
use crate::shelf::{paint, thumbnail};
use crate::{Backend, DynResult};

enum ShelfEvent {
    Request {
        request: crate::ipc::Request,
        reply: Sender<crate::ipc::Response>,
    },
    Tray(tray_icon::menu::MenuEvent),
}

const SHELF_MARGIN: i32 = 12;
const SHELF_CORNER_RADIUS: u32 = 18;

fn prepare_shelf_video(path: &Path) -> DynResult<PathBuf> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("shelf video is empty".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mp4");
    let retained = crate::paths::rec_file("shelf-video", extension);
    if std::fs::hard_link(path, &retained).is_err() {
        std::fs::copy(path, &retained).map_err(|error| format!("copy shelf video: {error}"))?;
    }
    Ok(retained)
}

fn shelf_position(work_area: windows::Win32::Foundation::RECT, height: u32) -> (i32, i32) {
    let height = i32::try_from(height).unwrap_or(i32::MAX);
    let x = work_area.left.saturating_add(SHELF_MARGIN);
    let y = work_area
        .bottom
        .saturating_sub(height)
        .saturating_sub(SHELF_MARGIN)
        .max(work_area.top.saturating_add(SHELF_MARGIN));
    (x, y)
}

pub fn focused_monitor_origin() -> Option<(i32, i32)> {
    crate::platform::windows::select_skia::focused_monitor_rect()
        .ok()
        .map(|rect| (rect.left, rect.top))
}

pub fn run_daemon(save_dir_cli: Option<PathBuf>) -> DynResult<()> {
    if crate::ipc::daemon_alive() {
        return Ok(());
    }
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };
    // No daemon was running, so the shelf is empty: any leftover shelf tempfiles
    // or cached recordings are orphans from a previous run/crash.
    let cleaned = crate::paths::clean_orphan_shelf_temps();
    if cleaned > 0 {
        eprintln!("boltsnap daemon: cleaned {cleaned} orphaned shelf tempfile(s)");
    }
    let cleaned_rec = crate::paths::clean_orphan_rec_files();
    if cleaned_rec > 0 {
        eprintln!("boltsnap daemon: cleaned {cleaned_rec} orphaned recording file(s)");
    }
    if let Err(error) = crate::platform::windows::hotkey::register_snipping_shortcuts() {
        eprintln!("boltsnap daemon: {error}");
    }
    let event_loop = EventLoop::<ShelfEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    start_pipe_server(proxy.clone());
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(ShelfEvent::Tray(event));
    }));
    let save_dir = save_dir_cli
        .unwrap_or_else(|| crate::config::resolve_save_dir(None, &crate::config::Config::load()));
    let mut application = ShelfApplication::new(save_dir);
    event_loop.run_app(&mut application)?;
    if let Some(error) = application.error {
        Err(error.into())
    } else {
        Ok(())
    }
}

struct SingleInstance(HANDLE);

impl SingleInstance {
    fn acquire() -> DynResult<Option<Self>> {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let name = format!("Local\\BoltsnapShelf-{user}")
            .encode_utf16()
            .chain([0])
            .collect::<Vec<_>>();
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr()))? };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Ok(None)
        } else {
            Ok(Some(Self(handle)))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn start_pipe_server(proxy: EventLoopProxy<ShelfEvent>) {
    std::thread::spawn(move || {
        loop {
            let name = crate::ipc::socket_path()
                .to_string_lossy()
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>();
            let handle = match create_current_user_pipe(PCWSTR(name.as_ptr())) {
                Ok(handle) => handle,
                Err(error) => {
                    eprintln!("boltsnap daemon: create secured named pipe: {error}");
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            };
            if handle.is_invalid() {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            let connected = unsafe { ConnectNamedPipe(handle, None) }.is_ok()
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !connected {
                unsafe {
                    drop(File::from_raw_handle(handle.0));
                }
                continue;
            }
            let mut pipe = unsafe { File::from_raw_handle(handle.0) };
            let request = match crate::ipc::Request::read(&mut pipe) {
                Ok(request) => request,
                Err(_) => continue,
            };
            let (reply_tx, reply_rx) = mpsc::channel();
            if proxy
                .send_event(ShelfEvent::Request {
                    request,
                    reply: reply_tx,
                })
                .is_err()
            {
                return;
            }
            if let Ok(response) = reply_rx.recv() {
                let _ = pipe.write_all(&response.encode());
                let _ = pipe.flush();
            }
        }
    });
}

fn create_current_user_pipe(name: PCWSTR) -> windows::core::Result<HANDLE> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w!("D:P(A;;GA;;;OW)"),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let handle = unsafe {
        CreateNamedPipeW(
            name,
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            Some(&attributes),
        )
    };
    unsafe {
        let _ = windows::Win32::Foundation::LocalFree(Some(HLOCAL(descriptor.0)));
    }
    if handle.is_invalid() {
        Err(windows::core::Error::from_thread())
    } else {
        Ok(handle)
    }
}

struct ShelfApplication {
    save_dir: PathBuf,
    model: ShelfModel,
    config: LayoutConfig,
    layout: Layout,
    hovered: Option<u64>,
    cursor: (f64, f64),
    pressed_body: Option<(u64, f64, f64)>,
    drag_started: bool,
    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    tray: Option<crate::tray::TrayState>,
    recording: Option<crate::platform::windows::recording::WindowsRecording>,
    recording_controls_visible: bool,
    error: Option<String>,
}

impl ShelfApplication {
    fn new(save_dir: PathBuf) -> Self {
        let config = LayoutConfig {
            pad: 0,
            gap: 8,
            ..LayoutConfig::default()
        };
        Self {
            save_dir,
            model: ShelfModel::new(),
            layout: Layout::compute(&[], &config),
            config,
            hovered: None,
            cursor: (0.0, 0.0),
            pressed_body: None,
            drag_started: false,
            window: None,
            context: None,
            surface: None,
            tray: None,
            recording: None,
            recording_controls_visible: false,
            error: None,
        }
    }

    fn attributes(&self) -> WindowAttributes {
        Window::default_attributes()
            .with_title("")
            .with_decorations(false)
            .with_resizable(false)
            .with_skip_taskbar(true)
            .with_taskbar_icon(None)
            .with_undecorated_shadow(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new(1, 1))
            .with_visible(false)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }

    fn report_native_error(&mut self, error: impl ToString) {
        let error = error.to_string();
        eprintln!("boltsnap daemon: {error}");
        self.error = Some(error);
    }

    fn native_regions(&self) -> Vec<(u32, u32, u32, u32)> {
        if self.recording_controls_visible && self.recording.is_some() {
            vec![(
                0,
                0,
                crate::shelf::recording::POPUP_W,
                crate::shelf::recording::POPUP_H,
            )]
        } else {
            self.layout
                .thumbs
                .iter()
                .map(|card| (card.x, card.y, card.w, card.h))
                .collect()
        }
    }

    fn restore_native_window(&mut self) {
        let regions = self.native_regions();
        let Some(window) = self.window.clone() else {
            return;
        };
        let result =
            crate::platform::windows::select_skia::configure_nonactivating_utility_window(&window)
                .and_then(|()| {
                    if regions.is_empty() {
                        Ok(())
                    } else {
                        crate::platform::windows::select_skia::set_rounded_window_regions(
                            &window,
                            &regions,
                            SHELF_CORNER_RADIUS,
                        )
                    }
                });
        if let Err(error) = result {
            self.report_native_error(error);
        }
    }

    fn ensure_content_visible(&mut self) -> bool {
        if self.native_regions().is_empty() {
            return false;
        }
        let Some(window) = self.window.clone() else {
            return false;
        };
        if window.is_visible() != Some(true) {
            window.set_visible(true);
            self.restore_native_window();
            window.request_redraw();
        }
        true
    }

    fn rebuild_layout(&mut self) {
        if self.recording_controls_visible && self.recording.is_some() {
            self.show_recording_controls();
            return;
        }
        let sizes = self
            .model
            .newest_first()
            .map(|thumbnail| {
                (
                    thumbnail.id,
                    thumbnail.thumb.width(),
                    thumbnail.thumb.height(),
                )
            })
            .collect::<Vec<_>>();
        self.layout = Layout::compute(&sizes, &self.config);
        let Some(window) = self.window.clone() else {
            return;
        };
        if sizes.is_empty() {
            window.set_visible(false);
            return;
        }
        let _ = window.request_inner_size(PhysicalSize::new(self.layout.width, self.layout.height));
        if let Ok(work_area) = crate::platform::windows::select_skia::focused_monitor_work_rect() {
            let (x, y) = shelf_position(work_area, self.layout.height);
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        window.set_visible(true);
        self.restore_native_window();
        window.request_redraw();
    }

    fn show_recording_controls(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.recording.is_none() {
            self.recording_controls_visible = false;
            self.rebuild_layout();
            return;
        }
        self.recording_controls_visible = true;
        let width = crate::shelf::recording::POPUP_W;
        let height = crate::shelf::recording::POPUP_H;
        let _ = window.request_inner_size(PhysicalSize::new(width, height));
        if let Ok(monitor) = crate::platform::windows::select_skia::focused_monitor_rect() {
            window.set_outer_position(PhysicalPosition::new(
                monitor.left + ((monitor.right - monitor.left - width as i32) / 2),
                monitor.top + 24,
            ));
        }
        window.set_visible(true);
        self.restore_native_window();
        window.request_redraw();
    }

    fn add_image(&mut self, source: String, png: Vec<u8>) -> DynResult<()> {
        let image = image::load_from_memory(&png)?.to_rgba8();
        let path = crate::paths::temp_file("shelf", "png");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, png)?;
        let card = thumbnail::make_card_thumbnail(&image, thumbnail::CARD_W, thumbnail::CARD_H);
        self.model.add(path, card, source);
        self.rebuild_layout();
        Ok(())
    }

    fn add_video(
        &mut self,
        source: String,
        path: PathBuf,
        take_ownership: bool,
    ) -> DynResult<PathBuf> {
        let retained = if take_ownership {
            prepare_shelf_video(&path)?
        } else {
            let metadata = std::fs::metadata(&path)?;
            if !metadata.is_file() || metadata.len() == 0 {
                return Err("shelf video is empty".into());
            }
            path
        };
        let mut placeholder = RgbaImage::new(thumbnail::CARD_W, thumbnail::CARD_H);
        for (x, y, pixel) in placeholder.enumerate_pixels_mut() {
            let shade = 24 + ((x + y) % 24) as u8;
            *pixel = Rgba([shade, shade, shade + 8, 255]);
        }
        self.model.add_kind_with_lifetime(
            retained.clone(),
            placeholder,
            source,
            CardKind::Video,
            if take_ownership {
                FileLifetime::Temporary
            } else {
                FileLifetime::Permanent
            },
        );
        self.rebuild_layout();
        Ok(retained)
    }

    fn handle_request(&mut self, request: crate::ipc::Request) -> crate::ipc::Response {
        use crate::ipc::{Request, Response};
        let result = match request {
            Request::Ping => return Response::ok(None),
            Request::Add { source, png, .. } => self.add_image(source, png),
            Request::RecordingStatus => return Response::ok(Some(self.recording_snapshot())),
            Request::RecordingWatch => {
                return Response::error(
                    "recording watch streaming is not connected on Windows yet",
                );
            }
            Request::StartRecording {
                x,
                y,
                w,
                h,
                audio_enabled,
                ..
            } => {
                if self.recording.is_some() {
                    return Response::error("a recording is already in progress");
                }
                let audio_source = crate::config::Config::load().recording_prefs().audio_source;
                return match crate::platform::windows::recording::WindowsRecording::start_area(
                    x,
                    y,
                    w,
                    h,
                    audio_enabled,
                    audio_source,
                ) {
                    Ok(recording) => {
                        self.recording = Some(recording);
                        self.show_recording_controls();
                        Response::ok(Some(self.recording_snapshot()))
                    }
                    Err(error) => Response::error(error.to_string()),
                };
            }
            Request::StartDefaultRecording => {
                if self.recording.is_some() {
                    return Response::error("a recording is already in progress");
                }
                let prefs = crate::config::Config::load().recording_prefs();
                return match crate::platform::windows::recording::WindowsRecording::start_focused(
                    prefs.audio_enabled,
                    prefs.audio_source,
                ) {
                    Ok(recording) => {
                        self.recording = Some(recording);
                        self.show_recording_controls();
                        Response::ok(Some(self.recording_snapshot()))
                    }
                    Err(error) => Response::error(error.to_string()),
                };
            }
            Request::RecordingControl { action } => {
                return self.recording_control(action);
            }
            Request::StopRecording => {
                return self.recording_control(crate::protocol::RecordingAction::SaveShelf);
            }
            Request::ShowRecordingControls => {
                return if self.recording.is_some() {
                    self.show_recording_controls();
                    Response::ok(Some(self.recording_snapshot()))
                } else {
                    Response::error("no recording is active")
                };
            }
            Request::StartRecordingOutput { .. } | Request::StartRecordingOutputs { .. } => {
                return Response::error(
                    "named multi-monitor recording is not connected on Windows yet",
                );
            }
            Request::RecordingThumb { .. } => return Response::ok(None),
        };
        match result {
            Ok(()) => {
                self.rebuild_layout();
                Response::ok(None)
            }
            Err(error) => Response::error(error.to_string()),
        }
    }

    fn recording_snapshot(&self) -> crate::ipc::RecordingSnapshot {
        self.recording
            .as_ref()
            .map(crate::platform::windows::recording::WindowsRecording::snapshot)
            .unwrap_or_else(crate::ipc::RecordingSnapshot::idle)
    }

    fn recording_control(
        &mut self,
        action: crate::protocol::RecordingAction,
    ) -> crate::ipc::Response {
        use crate::ipc::Response;
        use crate::protocol::RecordingAction;

        match action {
            RecordingAction::Pause => {
                let Some(recording) = self.recording.as_mut() else {
                    return Response::error("no recording is active");
                };
                match recording.pause() {
                    Ok(()) => {
                        self.show_recording_controls();
                        Response::ok(Some(self.recording_snapshot()))
                    }
                    Err(error) => Response::error(error.to_string()),
                }
            }
            RecordingAction::Resume => {
                let Some(recording) = self.recording.as_mut() else {
                    return Response::error("no recording is active");
                };
                match recording.resume() {
                    Ok(()) => {
                        self.show_recording_controls();
                        Response::ok(Some(self.recording_snapshot()))
                    }
                    Err(error) => Response::error(error.to_string()),
                }
            }
            RecordingAction::SaveShelf | RecordingAction::SaveDisk | RecordingAction::Discard => {
                let Some(recording) = self.recording.take() else {
                    return Response::error("no recording is active");
                };
                let temporary = match recording.finish() {
                    Ok(path) => path,
                    Err(error) => return Response::error(error.to_string()),
                };
                self.recording_controls_visible = false;
                match action {
                    RecordingAction::SaveShelf => {
                        match self.add_video("record".into(), temporary, true) {
                            Ok(_) => Response::ok(Some(self.recording_snapshot())),
                            Err(error) => Response::error(error.to_string()),
                        }
                    }
                    RecordingAction::SaveDisk => {
                        match crate::platform::windows::recording::move_to_recording_dir(&temporary)
                        {
                            Ok(path) => {
                                if crate::config::Config::load()
                                    .recording_prefs()
                                    .disk_add_to_shelf
                                {
                                    let _ = self.add_video("record".into(), path.clone(), false);
                                }
                                Response::ok_path(path)
                            }
                            Err(error) => Response::error(error.to_string()),
                        }
                    }
                    RecordingAction::Discard => {
                        let _ = std::fs::remove_file(temporary);
                        Response::ok(Some(self.recording_snapshot()))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn click(&mut self) {
        if self.recording_controls_visible {
            let snapshot = self.recording_snapshot();
            let button = crate::shelf::recording::popup_hit(
                snapshot.state,
                snapshot.actions_enabled,
                self.cursor.0,
                self.cursor.1,
            );
            let action = match button {
                Some(crate::shelf::recording::PopupButton::PauseResume) => {
                    if snapshot.state == crate::protocol::PublicRecordingState::Paused {
                        Some(crate::protocol::RecordingAction::Resume)
                    } else {
                        Some(crate::protocol::RecordingAction::Pause)
                    }
                }
                Some(crate::shelf::recording::PopupButton::SaveShelf) => {
                    Some(crate::protocol::RecordingAction::SaveShelf)
                }
                Some(crate::shelf::recording::PopupButton::SaveDisk) => {
                    Some(crate::protocol::RecordingAction::SaveDisk)
                }
                Some(crate::shelf::recording::PopupButton::Discard) => {
                    Some(crate::protocol::RecordingAction::Discard)
                }
                None => None,
            };
            if let Some(action) = action {
                let response = self.recording_control(action);
                if !response.ok {
                    eprintln!(
                        "boltsnap daemon: {}",
                        response
                            .error
                            .unwrap_or_else(|| "recording action failed".into())
                    );
                }
                if self.recording.is_none() {
                    self.rebuild_layout();
                }
            }
            return;
        }
        let Some(hit) = self.layout.hit(self.cursor.0, self.cursor.1, &self.config) else {
            return;
        };
        match hit {
            Hit::Close(id) => {
                if let Some(card) = self.model.remove(id) {
                    let _ = card.delete_file_on_dismiss();
                }
            }
            Hit::Save(id) => {
                let _ = self.save(id);
            }
            Hit::Body(id) => {
                self.copy_card(id);
            }
        }
        self.rebuild_layout();
    }

    fn copy_card(&self, id: u64) {
        if let Some(card) = self.model.get(id) {
            if card.kind == CardKind::Image {
                let _ = crate::clipboard::copy_to_clipboard(&card.png_path, Backend::Windows);
            } else {
                let _ = crate::clipboard::copy_uri_to_clipboard(&card.png_path);
            }
        }
    }

    fn begin_drag_if_needed(&mut self) {
        let Some((id, start_x, start_y)) = self.pressed_body else {
            return;
        };
        if (self.cursor.0 - start_x).hypot(self.cursor.1 - start_y) < 6.0 {
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        let Some(card) = self.model.get(id) else {
            self.pressed_body = None;
            return;
        };
        // Not fs::canonicalize: that returns a verbatim `\\?\C:\...` path, and
        // Chromium-based drop targets (Discord, browsers) reject the prefix in
        // CF_HDROP. The card path is already absolute.
        let path = crate::paths::normalize_path(&card.png_path);
        if let Err(error) = crate::paths::ensure_file(&path) {
            eprintln!("boltsnap daemon: prepare drag path: {error}");
            self.pressed_body = None;
            return;
        }
        let mut preview = std::io::Cursor::new(Vec::new());
        if let Err(error) = image::DynamicImage::ImageRgba8(card.thumb.clone())
            .write_to(&mut preview, image::ImageFormat::Png)
        {
            eprintln!("boltsnap daemon: encode drag preview: {error}");
            self.pressed_body = None;
            return;
        }
        self.drag_started = true;
        self.pressed_body = None;
        if let Err(error) = drag::start_drag(
            window.as_ref(),
            drag::DragItem::Files(vec![path]),
            drag::Image::Raw(preview.into_inner()),
            |_, _| {},
            drag::Options::default(),
        ) {
            eprintln!("boltsnap daemon: start OLE drag: {error}");
        }
        self.restore_native_window();
        if let Some(window) = &self.window {
            window.set_visible(true);
            window.request_redraw();
        }
    }

    fn save(&mut self, id: u64) -> DynResult<()> {
        let card = self.model.get(id).ok_or("unknown shelf card")?;
        std::fs::create_dir_all(&self.save_dir)?;
        let extension = card
            .png_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        let destination = unique_path(&self.save_dir, extension);
        std::fs::copy(&card.png_path, &destination)?;
        self.model.promote(id, destination);
        Ok(())
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let (width, height) = if self.recording_controls_visible && self.recording.is_some() {
            (
                crate::shelf::recording::POPUP_W,
                crate::shelf::recording::POPUP_H,
            )
        } else {
            (self.layout.width.max(1), self.layout.height.max(1))
        };
        let mut canvas = vec![0_u8; width as usize * height as usize * 4];
        if self.recording_controls_visible && self.recording.is_some() {
            let snapshot = self.recording_snapshot();
            paint::draw_recording_popup(
                &mut canvas,
                width,
                height,
                snapshot.state,
                snapshot.actions_enabled,
                &crate::shelf::recording::fmt_elapsed(snapshot.elapsed_ms / 1_000),
                &crate::shelf::font::fallback_popup_font(),
            );
        } else {
            paint::draw_shelf_opaque(
                &mut canvas,
                width,
                height,
                &self.layout,
                &self.model,
                self.hovered,
                &self.config,
                &[],
                None,
            );
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
        for (target, pixel) in buffer.iter_mut().zip(canvas.chunks_exact(4)) {
            let alpha = pixel[3] as u32;
            let inverse = 255 - alpha;
            let background = 20_u32;
            let red = (pixel[2] as u32 + background * inverse / 255).min(255);
            let green = (pixel[1] as u32 + background * inverse / 255).min(255);
            let blue = (pixel[0] as u32 + background * inverse / 255).min(255);
            *target = red << 16 | green << 8 | blue;
        }
        if let Err(error) = buffer.present() {
            self.fail(event_loop, error);
            return;
        }
        self.restore_native_window();
    }
}

impl ApplicationHandler<ShelfEvent> for ShelfApplication {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let recording_visible = self.recording_controls_visible && self.recording.is_some();
        let has_visible_content = self.ensure_content_visible();
        if recording_visible {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        if has_visible_content {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now()
                    + std::time::Duration::from_millis(if recording_visible { 250 } else { 500 }),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

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
        if let Err(error) =
            crate::platform::windows::select_skia::configure_nonactivating_utility_window(&window)
        {
            self.fail(event_loop, error);
            return;
        }
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
        match crate::tray::create() {
            Ok(tray) => self.tray = Some(tray),
            Err(error) => eprintln!("boltsnap daemon: {error}"),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShelfEvent) {
        match event {
            ShelfEvent::Request { request, reply } => {
                let response = self.handle_request(request);
                let _ = reply.send(response);
            }
            ShelfEvent::Tray(event) => {
                let action = self.tray.as_ref().and_then(|tray| tray.action(&event));
                match action {
                    Some(crate::tray::TrayAction::CaptureArea) => spawn_capture("area"),
                    Some(crate::tray::TrayAction::CaptureFull) => spawn_capture("full"),
                    Some(crate::tray::TrayAction::RecordArea) => spawn_command(&["record"]),
                    Some(crate::tray::TrayAction::RecordFull) => spawn_command(&["record", "full"]),
                    Some(crate::tray::TrayAction::ShowRecordingControls) => {
                        self.show_recording_controls()
                    }
                    Some(crate::tray::TrayAction::Quit) => event_loop.exit(),
                    None => {}
                }
            }
        }
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
            WindowEvent::CloseRequested => {
                let has_content = !self.native_regions().is_empty();
                self.restore_native_window();
                if let Some(window) = &self.window
                    && has_content
                {
                    window.set_visible(true);
                }
            }
            WindowEvent::Moved(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Focused(true) => {
                self.restore_native_window();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                if !self.recording_controls_visible {
                    self.hovered = self.layout.hit(position.x, position.y, &self.config).map(
                        |hit| match hit {
                            Hit::Body(id) | Hit::Save(id) | Hit::Close(id) => id,
                        },
                    );
                }
                self.begin_drag_if_needed();
                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.hovered = None;
                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.drag_started = false;
                self.pressed_body = if self.recording_controls_visible {
                    None
                } else {
                    match self.layout.hit(self.cursor.0, self.cursor.1, &self.config) {
                        Some(Hit::Body(id)) => Some((id, self.cursor.0, self.cursor.1)),
                        _ => None,
                    }
                };
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.pressed_body = None;
                if self.drag_started {
                    self.drag_started = false;
                } else {
                    self.click();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Right,
                ..
            } => {
                if !self.recording_controls_visible
                    && let Some(Hit::Body(id)) =
                        self.layout.hit(self.cursor.0, self.cursor.1, &self.config)
                {
                    self.copy_card(id);
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            _ => {}
        }
    }
}

fn unique_path(directory: &Path, extension: &str) -> PathBuf {
    let base = crate::paths::local_timestamp();
    for suffix in 0_u32.. {
        let suffix = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let path = directory.join(format!("boltsnap-{base}{suffix}.{extension}"));
        if !path.exists() {
            return path;
        }
    }
    unreachable!()
}

fn spawn_capture(command: &str) {
    spawn_command(&[command]);
}

fn spawn_command(arguments: &[&str]) {
    let result = std::env::current_exe().and_then(|executable| {
        let mut command = std::process::Command::new(executable);
        command
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::paths::spawn_reaped(&mut command)
    });
    if let Err(error) = result {
        eprintln!("boltsnap daemon: start {}: {error}", arguments.join(" "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::RECT;

    fn video_file(label: &str, contents: &[u8]) -> PathBuf {
        let path = crate::paths::temp_file(label, "mp4");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn shelf_is_anchored_bottom_left_inside_work_area() {
        let work_area = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(shelf_position(work_area, 300), (12, 728));
    }

    #[test]
    fn shelf_position_supports_negative_monitor_coordinates() {
        let work_area = RECT {
            left: -1920,
            top: -40,
            right: 0,
            bottom: 1040,
        };
        assert_eq!(shelf_position(work_area, 300), (-1908, 728));
    }

    #[test]
    fn add_video_retains_and_returns_a_boltsnap_owned_path() {
        let source = video_file("windows-shelf-source", b"video bytes");
        let mut app = ShelfApplication::new(std::env::temp_dir());

        let retained = app
            .add_video("record".into(), source.clone(), true)
            .unwrap();

        assert_ne!(retained, source);
        assert_eq!(std::fs::read(&retained).unwrap(), b"video bytes");
        let card = app.model.newest_first().next().unwrap();
        assert_eq!(card.png_path, retained);
        assert_eq!(card.lifetime, FileLifetime::Temporary);
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(retained).unwrap();
    }
}
