#![allow(dead_code)]
mod cursor;
#[cfg(feature = "gpu")]
mod dma;
pub use cursor::Fault as CursorFault;
use memmap2::MmapOptions;
use std::{
    fs::File,
    os::unix::net::{UnixListener, UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use wayland_protocols::{
    ext::{
        image_capture_source::v1::server::{
            ext_image_capture_source_v1 as source,
            ext_output_image_capture_source_manager_v1 as source_manager,
        },
        image_copy_capture::v1::server::{
            ext_image_copy_capture_frame_v1 as ext_frame,
            ext_image_copy_capture_manager_v1 as ext_manager,
            ext_image_copy_capture_session_v1 as session,
        },
    },
    xdg::xdg_output::zv1::server::{
        zxdg_output_manager_v1 as output_manager, zxdg_output_v1 as xdg_output,
    },
};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1 as wlr_frame, zwlr_screencopy_manager_v1 as wlr_manager,
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientData, ClientId, DisconnectReason},
    protocol::{wl_buffer, wl_output, wl_shm, wl_shm_pool},
};

#[derive(Clone, Copy, Default)]
pub enum Fault {
    #[default]
    None,
    Stall,
    EarlyFail,
    Stopped,
    NoShm,
    InvalidSize,
    LayoutChange,
    ChangedConstraints,
    IdleSecond,
    RemoveOutput,
    RemoveExt,
    RemoveDuplicateExt,
}
#[derive(Clone)]
pub struct Config {
    pub ext: bool,
    pub cursor: Option<CursorFault>,
    pub wlr: u32,
    pub fault: Fault,
    pub transform: wl_output::Transform,
    pub inverted: bool,
    pub outputs: usize,
    pub dma: bool,
    pub reject_dma: bool,
    pub mode_sizes: Vec<(u32, u32)>,
    pub native_logical_size: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            ext: true,
            cursor: None,
            wlr: 3,
            fault: Fault::None,
            transform: wl_output::Transform::Normal,
            inverted: false,
            outputs: 1,
            dma: false,
            reject_dma: false,
            mode_sizes: Vec::new(),
            native_logical_size: false,
        }
    }
}
impl Config {
    fn size(&self, index: usize) -> (u32, u32) {
        self.mode_sizes.get(index).copied().unwrap_or((4, 2))
    }
}
#[derive(Default)]
pub struct Metrics {
    pub frames: AtomicUsize,
    pub cursor_sessions: AtomicUsize,
    pub seats: AtomicUsize,
    pub pointers: AtomicUsize,
    pub sessions: AtomicUsize,
    pub sources: AtomicUsize,
    pub buffers: AtomicUsize,
    pub captures: AtomicUsize,
    pub damage_requests: AtomicUsize,
    pub imports: AtomicUsize,
    pub params: AtomicUsize,
}
pub struct Server {
    pub metrics: Arc<Metrics>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(config: Config) -> (Self, UnixStream) {
        let (client, socket) = UnixStream::pair().unwrap();
        (Self::launch(config, Some(socket), None), client)
    }
    pub fn listen(config: Config, path: &std::path::Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        listener.set_nonblocking(true).unwrap();
        Self::launch(config, None, Some(listener))
    }
    fn launch(config: Config, socket: Option<UnixStream>, listener: Option<UnixListener>) -> Self {
        let metrics = Arc::new(Metrics::default());
        let stop = Arc::new(AtomicBool::new(false));
        let stats = metrics.clone();
        let quit = stop.clone();
        let thread = thread::spawn(move || {
            let mut display = Display::<State>::new().unwrap();
            let mut dh = display.handle();
            if let Some(socket) = socket {
                dh.insert_client(socket, Arc::new(ClientState)).unwrap();
            }
            dh.create_global::<State, wl_shm::WlShm, _>(1, ());
            dh.create_global::<State, output_manager::ZxdgOutputManagerV1, _>(3, ());
            if config.cursor.is_some() {
                cursor::advertise(&dh);
            }
            let mut output_globals = Vec::new();
            for index in 0..config.outputs {
                output_globals.push(dh.create_global::<State, wl_output::WlOutput, _>(4, index));
            }
            let mut ext_global = None;
            #[cfg(feature = "gpu")]
            if config.dma {
                dma::advertise(&dh);
            }
            if config.ext {
                ext_global = Some(
                    dh.create_global::<State, ext_manager::ExtImageCopyCaptureManagerV1, _>(1, ()),
                );
                dh.create_global::<State, source_manager::ExtOutputImageCaptureSourceManagerV1, _>(
                    1,
                    (),
                );
            }
            if matches!(config.fault, Fault::RemoveDuplicateExt) {
                ext_global = Some(
                    dh.create_global::<State, ext_manager::ExtImageCopyCaptureManagerV1, _>(1, ()),
                );
            }
            if config.wlr > 0 {
                dh.create_global::<State, wlr_manager::ZwlrScreencopyManagerV1, _>(config.wlr, ());
            }
            let mut state = State {
                config,
                metrics: stats,
                xdg: Vec::new(),
                output_globals,
                ext_global,
            };
            while !quit.load(Ordering::Acquire) {
                if let Some(listener) = &listener {
                    while let Ok((socket, _)) = listener.accept() {
                        dh.insert_client(socket, Arc::new(ClientState)).unwrap();
                    }
                }
                display.dispatch_clients(&mut state).unwrap();
                display.flush_clients().unwrap();
                use std::os::fd::{AsFd, AsRawFd};
                let mut fd = libc::pollfd {
                    fd: display.as_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // Wake on client requests; the timeout only bounds shutdown.
                unsafe {
                    libc::poll(&mut fd, 1, 10);
                }
            }
        });
        Self {
            metrics,
            stop,
            thread: Some(thread),
        }
    }
    pub fn settle(&self) {
        thread::sleep(Duration::from_millis(15));
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
struct ClientState;
impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}
struct State {
    config: Config,
    metrics: Arc<Metrics>,
    xdg: Vec<xdg_output::ZxdgOutputV1>,
    output_globals: Vec<wayland_server::backend::GlobalId>,
    ext_global: Option<wayland_server::backend::GlobalId>,
}
struct Pool {
    file: Arc<File>,
    size: usize,
}
struct Buffer {
    file: Arc<File>,
    offset: usize,
    width: u32,
    height: u32,
    stride: u32,
}
#[derive(Default)]
struct SessionData {
    captures: AtomicUsize,
    active: AtomicBool,
    index: usize,
    cursor: Option<cursor::CursorSession>,
}
struct FrameData {
    session: Arc<SessionData>,
    owner: session::ExtImageCopyCaptureSessionV1,
    buffer: Mutex<Option<wl_buffer::WlBuffer>>,
    damaged: AtomicBool,
}

macro_rules! global {
    ($ty:ty) => {
        impl GlobalDispatch<$ty, ()> for State {
            fn bind(
                _: &mut Self,
                _: &DisplayHandle,
                _: &Client,
                r: New<$ty>,
                _: &(),
                init: &mut DataInit<'_, Self>,
            ) {
                init.init(r, ());
            }
        }
    };
}
macro_rules! noop {
    ($ty:ty,$ud:ty) => {
        impl Dispatch<$ty, $ud> for State {
            fn request(
                _: &mut Self,
                _: &Client,
                _: &$ty,
                _: <$ty as Resource>::Request,
                _: &$ud,
                _: &DisplayHandle,
                _: &mut DataInit<'_, Self>,
            ) {
            }
        }
    };
}
global!(output_manager::ZxdgOutputManagerV1);
global!(source_manager::ExtOutputImageCaptureSourceManagerV1);
global!(ext_manager::ExtImageCopyCaptureManagerV1);
global!(wlr_manager::ZwlrScreencopyManagerV1);
noop!(wl_output::WlOutput, usize);
noop!(xdg_output::ZxdgOutputV1, ());
impl GlobalDispatch<wl_output::WlOutput, usize> for State {
    fn bind(
        s: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_output::WlOutput>,
        index: &usize,
        init: &mut DataInit<'_, Self>,
    ) {
        let o = init.init(r, *index);
        o.geometry(
            *index as i32 * 4 - 4,
            0,
            300,
            200,
            wl_output::Subpixel::Unknown,
            "test".into(),
            "headless".into(),
            s.config.transform,
        );
        let (w, h) = s.config.size(*index);
        o.mode(wl_output::Mode::Current, w as i32, h as i32, 60_000);
        o.scale(1);
        o.name(format!("TEST-{index}"));
        o.description("libway isolated output".into());
        o.done();
    }
}
impl GlobalDispatch<wl_shm::WlShm, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_shm::WlShm>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(r, ()).format(wl_shm::Format::Xrgb8888);
    }
}
impl Dispatch<output_manager::ZxdgOutputManagerV1, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &output_manager::ZxdgOutputManagerV1,
        r: output_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let output_manager::Request::GetXdgOutput { id, output } = r {
            let index = *output.data::<usize>().unwrap();
            let x = init.init(id, ());
            x.logical_position(index as i32 * 4 - 4, 0);
            let rotated = matches!(
                s.config.transform,
                wl_output::Transform::_90
                    | wl_output::Transform::_270
                    | wl_output::Transform::Flipped90
                    | wl_output::Transform::Flipped270
            );
            let (w, h) = if s.config.native_logical_size {
                s.config.size(index)
            } else {
                (4, 2)
            };
            x.logical_size(
                if rotated { h } else { w } as i32,
                if rotated { w } else { h } as i32,
            );
            x.name(format!("TEST-{index}"));
            x.description("libway isolated output".into());
            output.done();
            s.xdg.push(x);
        }
    }
}
impl Dispatch<wl_shm::WlShm, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        r: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, size } = r {
            assert!(size > 0);
            init.init(
                id,
                Pool {
                    file: Arc::new(File::from(fd)),
                    size: size as usize,
                },
            );
        }
    }
}
impl Dispatch<wl_shm_pool::WlShmPool, Pool> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_shm_pool::WlShmPool,
        r: wl_shm_pool::Request,
        p: &Pool,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer {
            id,
            offset,
            width,
            height,
            stride,
            format,
        } = r
        {
            assert!(matches!(
                format,
                wayland_server::WEnum::Value(wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888)
            ));
            assert!(offset >= 0 && width > 0 && height > 0 && stride >= width * 4);
            assert!(offset as usize + stride as usize * height as usize <= p.size);
            s.metrics.buffers.fetch_add(1, Ordering::SeqCst);
            init.init(
                id,
                Buffer {
                    file: p.file.clone(),
                    offset: offset as usize,
                    width: width as u32,
                    height: height as u32,
                    stride: stride as u32,
                },
            );
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, Buffer> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Request,
        _: &Buffer,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &wl_buffer::WlBuffer, _: &Buffer) {
        s.metrics.buffers.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatch<source_manager::ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &source_manager::ExtOutputImageCaptureSourceManagerV1,
        r: source_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let source_manager::Request::CreateSource { source, output } = r {
            s.metrics.sources.fetch_add(1, Ordering::SeqCst);
            init.init(source, *output.data::<usize>().unwrap());
        }
    }
}
impl Dispatch<source::ExtImageCaptureSourceV1, usize> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &source::ExtImageCaptureSourceV1,
        _: source::Request,
        _: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &source::ExtImageCaptureSourceV1, _: &usize) {
        s.metrics.sources.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatch<ext_manager::ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &ext_manager::ExtImageCopyCaptureManagerV1,
        r: ext_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_manager::Request::CreateSession {
            session, source, ..
        } = r
        {
            s.metrics.sessions.fetch_add(1, Ordering::SeqCst);
            let session = init.init(
                session,
                Arc::new(SessionData {
                    index: *source.data::<usize>().unwrap(),
                    ..Default::default()
                }),
            );
            if matches!(s.config.fault, Fault::Stopped) {
                session.stopped();
                return;
            }
            let (w, h) = s.config.size(*source.data::<usize>().unwrap());
            session.buffer_size(
                if matches!(s.config.fault, Fault::InvalidSize) {
                    u32::MAX
                } else {
                    w
                },
                h,
            );
            if !matches!(s.config.fault, Fault::NoShm) {
                session.shm_format(wl_shm::Format::Xrgb8888);
            }
            #[cfg(feature = "gpu")]
            if s.config.dma {
                dma::constraints(&session);
            }
            session.done();
        } else if let ext_manager::Request::CreatePointerCursorSession {
            session, source, ..
        } = r
        {
            cursor::create(s, init, session, *source.data::<usize>().unwrap());
        }
    }
}
impl Dispatch<session::ExtImageCopyCaptureSessionV1, Arc<SessionData>> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        owner: &session::ExtImageCopyCaptureSessionV1,
        r: session::Request,
        data: &Arc<SessionData>,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let session::Request::CreateFrame { frame } = r {
            assert!(
                !data.active.swap(true, Ordering::SeqCst),
                "at most one EXT frame per session"
            );
            s.metrics.frames.fetch_add(1, Ordering::SeqCst);
            let frame = init.init(
                frame,
                FrameData {
                    session: data.clone(),
                    owner: owner.clone(),
                    buffer: Mutex::new(None),
                    damaged: AtomicBool::new(false),
                },
            );
            if matches!(s.config.fault, Fault::EarlyFail) {
                frame.failed(ext_frame::FailureReason::Unknown);
            }
        }
    }
    fn destroyed(
        s: &mut Self,
        _: ClientId,
        _: &session::ExtImageCopyCaptureSessionV1,
        _: &Arc<SessionData>,
    ) {
        s.metrics.sessions.fetch_sub(1, Ordering::SeqCst);
    }
}
fn buffer_dimensions(buffer: &wl_buffer::WlBuffer) -> (u32, u32) {
    if let Some(b) = buffer.data::<Buffer>() {
        return (b.width, b.height);
    }
    #[cfg(feature = "gpu")]
    if let Some(b) = buffer.data::<dma::GpuBufferData>() {
        return (b.width, b.height);
    }
    panic!("unknown test buffer");
}
fn paint(buffer: &wl_buffer::WlBuffer, index: usize) {
    // DMA tests verify fd transport and lifecycle. They do not claim GPU rendering.
    let Some(b) = buffer.data::<Buffer>() else {
        return;
    };
    // Test server owns its FD and serializes access before sending ready.
    let mut map = unsafe { MmapOptions::new().map_mut(&*b.file).unwrap() };
    for y in 0..b.height {
        for x in 0..b.width {
            let at = b.offset + (y * b.stride + x * 4) as usize;
            let pixel = if index.is_multiple_of(2) {
                0x00102030_u32 + (x << 16) + (y << 8)
            } else {
                0x00a0b0c0
            };
            map[at..at + 4].copy_from_slice(&pixel.to_le_bytes());
        }
    }
}
impl Dispatch<ext_frame::ExtImageCopyCaptureFrameV1, FrameData> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        f: &ext_frame::ExtImageCopyCaptureFrameV1,
        r: ext_frame::Request,
        data: &FrameData,
        dh: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match r {
            ext_frame::Request::AttachBuffer { buffer } => {
                *data.buffer.lock().unwrap() = Some(buffer)
            }
            ext_frame::Request::DamageBuffer {
                x,
                y,
                width,
                height,
            } => {
                let guard = data.buffer.lock().unwrap();
                let (bw, bh) = buffer_dimensions(guard.as_ref().unwrap());
                assert_eq!((x, y, width, height), (0, 0, bw as i32, bh as i32));
                data.damaged.store(true, Ordering::SeqCst);
                s.metrics.damage_requests.fetch_add(1, Ordering::SeqCst);
            }
            ext_frame::Request::Capture => {
                assert!(
                    data.damaged.load(Ordering::SeqCst),
                    "new capture buffers require full damage"
                );
                let n = data.session.captures.fetch_add(1, Ordering::SeqCst);
                if data.session.cursor.is_some() {
                    cursor::capture(s, f, data, n);
                    return;
                }
                if matches!(s.config.fault, Fault::Stall | Fault::EarlyFail)
                    || (n > 0 && matches!(s.config.fault, Fault::IdleSecond))
                {
                    return;
                }
                let buffer = data.buffer.lock().unwrap();
                paint(buffer.as_ref().unwrap(), data.session.index);
                s.metrics.captures.fetch_add(1, Ordering::SeqCst);
                if n == 0 && matches!(s.config.fault, Fault::ChangedConstraints) {
                    data.owner.buffer_size(2, 3);
                    data.owner.shm_format(wl_shm::Format::Xrgb8888);
                    data.owner.done();
                }
                f.transform(s.config.transform);
                let (bw, bh) = buffer_dimensions(buffer.as_ref().unwrap());
                f.damage(0, 0, bw as i32, bh as i32);
                f.presentation_time(0, 123, 456);
                f.ready();
                if matches!(s.config.fault, Fault::RemoveOutput)
                    && let Some(id) = s.output_globals.pop()
                {
                    dh.disable_global::<State>(id);
                }
                if matches!(s.config.fault, Fault::RemoveExt | Fault::RemoveDuplicateExt)
                    && let Some(id) = s.ext_global.take()
                {
                    dh.disable_global::<State>(id);
                }
                if matches!(s.config.fault, Fault::LayoutChange) {
                    for x in &s.xdg {
                        x.logical_position(99, 100);
                    }
                }
            }
            _ => {}
        }
    }
    fn destroyed(
        s: &mut Self,
        _: ClientId,
        _: &ext_frame::ExtImageCopyCaptureFrameV1,
        d: &FrameData,
    ) {
        d.session.active.store(false, Ordering::SeqCst);
        s.metrics.frames.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatch<wlr_manager::ZwlrScreencopyManagerV1, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wlr_manager::ZwlrScreencopyManagerV1,
        r: wlr_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wlr_manager::Request::CaptureOutput { frame, output, .. } = r {
            s.metrics.frames.fetch_add(1, Ordering::SeqCst);
            let f = init.init(frame, *output.data::<usize>().unwrap());
            if matches!(s.config.fault, Fault::EarlyFail) {
                f.failed();
                return;
            }
            let (w, h) = s.config.size(*output.data::<usize>().unwrap());
            f.buffer(wl_shm::Format::Xrgb8888, w, h, w * 4 + 8);
            if f.version() >= 3 {
                if s.config.dma {
                    f.linux_dmabuf(0x34325258, w, h);
                }
                f.buffer_done();
            }
        }
    }
}
impl Dispatch<wlr_frame::ZwlrScreencopyFrameV1, usize> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        f: &wlr_frame::ZwlrScreencopyFrameV1,
        r: wlr_frame::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wlr_frame::Request::Copy { buffer } = r {
            if matches!(s.config.fault, Fault::Stall) {
                return;
            }
            paint(&buffer, *index);
            s.metrics.captures.fetch_add(1, Ordering::SeqCst);
            f.flags(if s.config.inverted {
                wlr_frame::Flags::YInvert
            } else {
                wlr_frame::Flags::empty()
            });
            f.ready(0, 123, 456);
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &wlr_frame::ZwlrScreencopyFrameV1, _: &usize) {
        s.metrics.frames.fetch_sub(1, Ordering::SeqCst);
    }
}
