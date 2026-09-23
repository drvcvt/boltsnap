use crate::{
    buffer::{ShmBuffer, create_wl_shm},
    *,
};
use std::{
    collections::BTreeMap,
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection as WlConnection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
    protocol::{wl_buffer, wl_callback, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols::{
    ext::{
        image_capture_source::v1::client::{
            ext_image_capture_source_v1 as source,
            ext_output_image_capture_source_manager_v1 as source_manager,
        },
        image_copy_capture::v1::client::{
            ext_image_copy_capture_frame_v1 as ext_frame,
            ext_image_copy_capture_manager_v1 as ext_manager,
            ext_image_copy_capture_session_v1 as ext_session,
        },
    },
    wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf,
    },
    xdg::xdg_output::zv1::client::{
        zxdg_output_manager_v1 as output_manager, zxdg_output_v1 as xdg_output,
    },
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1 as wlr_frame, zwlr_screencopy_manager_v1 as wlr_manager,
};
mod cursor;
mod events;
pub use cursor::{CursorEvent, CursorStream};

static CONNECTION_IDS: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
/// Storage requested for a capture. GPU requires the `gpu` feature and an explicit allocator.
pub enum BufferKind {
    /// Bounded shared memory, readable on the CPU after completion.
    Cpu,
    /// Allocate DMA-BUFs on this device; no implicit CPU readback or CPU fallback.
    #[cfg(feature = "gpu")]
    Gpu(crate::gpu::GpuAllocator),
}

struct OutputRecord {
    wl: wl_output::WlOutput,
    xdg: Option<xdg_output::ZxdgOutputV1>,
    info: Output,
    position: Option<(i32, i32)>,
    logical_size: Option<(u32, u32)>,
    scale: u32,
}
#[derive(Default, Clone)]
pub(crate) struct Constraints {
    pub size: (u32, u32),
    pub shm: Vec<wl_shm::Format>,
    pub stride: Option<u32>,
    pub dma: Vec<(u32, Vec<u64>)>,
    pub dma_any_modifier: bool,
    pub device: Option<u64>,
}
struct Pending {
    token: u64,
    constraints: Constraints,
    frame_size: (u32, u32),
    batch: Constraints,
    batch_open: bool,
    formats_done: bool,
    ready: bool,
    error: Option<Error>,
    transform: Option<Transform>,
    inverted: bool,
    timestamp: Option<Duration>,
    damage: Vec<(u32, u32, u32, u32)>,
    imported: Option<wl_buffer::WlBuffer>,
    import_done: bool,
}
impl Pending {
    fn new(token: u64) -> Self {
        Self {
            token,
            constraints: Constraints::default(),
            frame_size: (0, 0),
            batch: Constraints::default(),
            batch_open: false,
            formats_done: false,
            ready: false,
            error: None,
            transform: None,
            inverted: false,
            timestamp: None,
            damage: Vec::new(),
            imported: None,
            import_done: false,
        }
    }
    fn batch(&mut self) -> &mut Constraints {
        if !self.batch_open {
            self.batch = Constraints::default();
            self.batch_open = true;
        }
        &mut self.batch
    }
    fn reset_frame(&mut self) {
        self.ready = false;
        self.frame_size = (0, 0);
        self.error = None;
        self.transform = None;
        self.inverted = false;
        self.timestamp = None;
        self.damage.clear();
        self.import_done = false;
        if let Some(buffer) = self.imported.take() {
            buffer.destroy();
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        // An import can finish in the same event batch as a failure/cancellation.
        // Keep ownership here until it has explicitly moved into Objects.
        if let Some(buffer) = self.imported.take() {
            buffer.destroy();
        }
    }
}
pub(crate) struct State {
    connection_id: u64,
    next_id: u64,
    sync: u64,
    outputs: BTreeMap<u32, OutputRecord>,
    globals: BTreeMap<u32, String>,
    seats: BTreeMap<u32, u32>,
    cursor: Option<cursor::CursorState>,
    output_manager: Option<output_manager::ZxdgOutputManagerV1>,
    shm: Option<wl_shm::WlShm>,
    ext: Option<ext_manager::ExtImageCopyCaptureManagerV1>,
    source: Option<source_manager::ExtOutputImageCaptureSourceManagerV1>,
    wlr: Option<wlr_manager::ZwlrScreencopyManagerV1>,
    dma: Option<dmabuf::ZwpLinuxDmabufV1>,
    dma_formats: Vec<(u32, u64)>,
    pending: Option<Pending>,
    global_error: Option<Error>,
    limits: Limits,
}
impl State {
    fn pending(&mut self, token: u64) -> Option<&mut Pending> {
        self.pending.as_mut().filter(|p| p.token == token)
    }
    fn attach_xdg(&mut self, qh: &QueueHandle<Self>) {
        if let Some(manager) = &self.output_manager {
            for (name, o) in &mut self.outputs {
                if o.xdg.is_none() {
                    o.xdg = Some(manager.get_xdg_output(&o.wl, qh, *name));
                }
            }
        }
    }
}

/// Owns a dedicated Wayland connection. Never maps surfaces or opens portals.
/// Use a separate connection per independent worker; methods serialize capture.
pub struct Connection {
    conn: WlConnection,
    registry: wl_registry::WlRegistry,
    queue: EventQueue<State>,
    state: State,
    serial: u64,
    /// Filesystem path of the server socket, used to open sibling connections.
    /// `None` for inherited or unnamed sockets.
    #[cfg(feature = "image")]
    pub(crate) peer: Option<std::path::PathBuf>,
}
impl Connection {
    /// Open the display selected by `WAYLAND_DISPLAY`/`WAYLAND_SOCKET` and discover globals.
    /// Setup observes the timeout, cancellation and limits in `options`; connection and
    /// protocol failures are returned as [`Error::Wayland`]. Requires `capture`.
    pub fn connect(options: &CaptureOptions) -> Result<Self> {
        check_options(options)?;
        let deadline = deadline(options)?;
        let conn = WlConnection::connect_to_env().map_err(|e| Error::Wayland(e.to_string()))?;
        Self::initialize(conn, options, deadline)
    }
    /// Connect to an explicitly supplied server socket, useful for isolated tests.
    pub fn from_socket(socket: UnixStream, options: &CaptureOptions) -> Result<Self> {
        check_options(options)?;
        let deadline = deadline(options)?;
        let conn = WlConnection::from_socket(socket).map_err(|e| Error::Wayland(e.to_string()))?;
        Self::initialize(conn, options, deadline)
    }
    fn initialize(conn: WlConnection, options: &CaptureOptions, deadline: Instant) -> Result<Self> {
        #[cfg(feature = "image")]
        let peer = conn
            .backend()
            .poll_fd()
            .try_clone_to_owned()
            .ok()
            .and_then(|fd| UnixStream::from(fd).peer_addr().ok())
            .and_then(|addr| addr.as_pathname().map(Into::into));
        let queue = conn.new_event_queue();
        let registry = conn.display().get_registry(&queue.handle(), ());
        let mut this = Self {
            conn,
            registry,
            queue,
            state: State {
                connection_id: CONNECTION_IDS.fetch_add(1, Ordering::Relaxed),
                next_id: 0,
                sync: 0,
                outputs: BTreeMap::new(),
                globals: BTreeMap::new(),
                seats: BTreeMap::new(),
                cursor: None,
                output_manager: None,
                shm: None,
                ext: None,
                source: None,
                wlr: None,
                dma: None,
                dma_formats: Vec::new(),
                pending: None,
                global_error: None,
                limits: options.limits,
            },
            serial: 0,
            #[cfg(feature = "image")]
            peer,
        };
        this.barrier(options, deadline)?;
        this.state.attach_xdg(&this.queue.handle());
        this.barrier(options, deadline)?;
        // Globals can arrive in any order; objects created by registry callbacks
        // may issue requests after the previous sync was sent.
        this.barrier(options, deadline)?;
        Ok(this)
    }
    /// Last dispatched capability snapshot, without waiting for new registry events.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            ext_output_capture: self.state.ext.is_some() && self.state.source.is_some(),
            wlr_screencopy_version: self.state.wlr.as_ref().map_or(0, Proxy::version),
            linux_dmabuf_version: self.state.dma.as_ref().map_or(0, Proxy::version),
        }
    }
    /// Synchronize and return the current output layout; an empty desktop yields an empty Vec.
    /// May fail on timeout, cancellation, connection failure or invalid output dimensions.
    pub fn outputs(&mut self, options: &CaptureOptions) -> Result<Vec<Output>> {
        self.barrier(options, deadline(options)?)?;
        self.output_snapshot()
    }
    fn output_snapshot(&self) -> Result<Vec<Output>> {
        self.state
            .outputs
            .values()
            .map(|o| {
                let mut info = o.info.clone();
                let (w, h) = info.transform.size(info.mode_size.0, info.mode_size.1);
                let (width, height) = o
                    .logical_size
                    .unwrap_or((w / o.scale.max(1), h / o.scale.max(1)));
                let (x, y) = o.position.unwrap_or((info.logical.x, info.logical.y));
                info.logical = Rect {
                    x,
                    y,
                    width,
                    height,
                }
                .validate()?;
                Ok(info)
            })
            .collect()
    }
    /// Capture one completed CPU frame. The id must come from this connection's outputs.
    /// Uses the same deadline, layout checks and error behavior as [`Self::stream`].
    ///
    /// ```no_run
    /// use libway::{CaptureOptions, Connection, Error};
    /// # fn main() -> libway::Result<()> {
    /// let options = CaptureOptions::default();
    /// let mut connection = Connection::connect(&options)?;
    /// let output = connection.outputs(&options)?.into_iter().next().ok_or(Error::NoOutputs)?;
    /// let frame = connection.capture(output.id, &options)?;
    /// let pixels = frame.rgba8()?;
    /// assert_eq!(pixels.len(), frame.width as usize * frame.height as usize * 4);
    /// # Ok(())
    /// # }
    /// ```
    pub fn capture(&mut self, id: OutputId, options: &CaptureOptions) -> Result<Frame> {
        self.capture_with(id, options, BufferKind::Cpu)
    }
    /// Capture one frame using explicitly selected storage. GPU allocation/import failures
    /// are returned to the caller; there is no implicit GPU-to-CPU fallback.
    pub fn capture_with(
        &mut self,
        id: OutputId,
        options: &CaptureOptions,
        kind: BufferKind,
    ) -> Result<Frame> {
        self.stream(id, options.clone(), kind)?.next_frame()
    }
    /// EXT sessions persist across frames and wait for damage after the first frame.
    /// A timeout/error terminates the stream. Drop returned frames to release storage;
    /// libway does not retain a frame history or encoder queue.
    /// The stream exclusively borrows this connection until dropped. Unknown ids return
    /// [`Error::OutputGone`]; missing protocols return [`Error::Unsupported`]. Setup and
    /// the first frame share one deadline. Subsequent frames each receive a fresh timeout.
    ///
    /// ```no_run
    /// use libway::{BufferKind, CaptureOptions, Connection};
    /// # fn main() -> libway::Result<()> {
    /// let options = CaptureOptions::default();
    /// let mut connection = Connection::connect(&options)?;
    /// let output = connection.outputs(&options)?.into_iter().next().ok_or(libway::Error::NoOutputs)?;
    /// let mut stream = connection.stream(output.id, options, BufferKind::Cpu)?;
    /// for _ in 0..3 {
    ///     let frame = stream.next_frame()?;
    ///     println!("{:?}", frame.presentation_time);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn stream(
        &mut self,
        id: OutputId,
        options: CaptureOptions,
        kind: BufferKind,
    ) -> Result<Stream<'_>> {
        check_options(&options)?;
        let end = deadline(&options)?;
        self.barrier(&options, end)?;
        let output = self
            .output_snapshot()?
            .into_iter()
            .find(|o| o.id == id)
            .ok_or(Error::OutputGone)?;
        let backend = match options.backend {
            Backend::Auto if self.capabilities().ext_output_capture => Backend::Ext,
            Backend::Auto if self.state.wlr.is_some() => Backend::Wlr,
            Backend::Auto => {
                return Err(Error::Unsupported("EXT output capture or WLR screencopy"));
            }
            b => b,
        };
        let mut stream = Stream {
            connection: self,
            output,
            options,
            kind,
            objects: Objects::default(),
            backend,
            closed: false,
            first_deadline: Some(end),
        };
        if let Err(error) = stream.prepare(end) {
            if !stream.falls_back(&error) {
                return Err(error);
            }
            stream.objects = Objects::default();
            stream.backend = Backend::Wlr;
            stream.prepare(end)?;
        }
        Ok(stream)
    }
    fn barrier(&mut self, options: &CaptureOptions, end: Instant) -> Result<()> {
        self.serial += 1;
        let serial = self.serial;
        self.conn.display().sync(&self.queue.handle(), serial);
        self.pump(options, end, |s| s.sync >= serial)
    }
    fn pump(
        &mut self,
        options: &CaptureOptions,
        end: Instant,
        done: impl Fn(&State) -> bool,
    ) -> Result<()> {
        loop {
            check_wait(options, end)?;
            self.queue
                .dispatch_pending(&mut self.state)
                .map_err(|e| Error::Wayland(e.to_string()))?;
            if let Some(e) = self.state.global_error.take() {
                return Err(e);
            }
            if let Some(e) = self.state.pending.as_mut().and_then(|p| p.error.take()) {
                return Err(e);
            }
            if let Some(e) = self.state.cursor.as_mut().and_then(|c| c.error.take()) {
                return Err(e);
            }
            if done(&self.state) {
                return Ok(());
            }
            let mut events = libc::POLLIN;
            match self.conn.flush() {
                Ok(()) => {}
                Err(wayland_client::backend::WaylandError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    events |= libc::POLLOUT
                }
                Err(e) => return Err(Error::Wayland(e.to_string())),
            }
            let Some(guard) = self.conn.prepare_read() else {
                continue;
            };
            let remaining = end
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20));
            let mut fd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events,
                revents: 0,
            };
            // SAFETY: one initialized pollfd, bounded timeout, valid borrowed connection FD.
            let ret = unsafe { libc::poll(&mut fd, 1, remaining.as_millis().max(1) as i32) };
            if ret < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e.into());
            }
            if fd.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
                match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(e))
                        if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(Error::Wayland(e.to_string())),
                }
            }
        }
    }
}
fn check_options(o: &CaptureOptions) -> Result<()> {
    if o.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if o.timeout.is_zero() {
        return Err(Error::Timeout);
    }
    if o.limits.max_outputs == 0 || o.limits.max_pixels == 0 || o.limits.max_bytes == 0 {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}
fn deadline(o: &CaptureOptions) -> Result<Instant> {
    Instant::now()
        .checked_add(o.timeout)
        .ok_or(Error::InvalidDimensions)
}
fn check_wait(o: &CaptureOptions, end: Instant) -> Result<()> {
    if o.cancellation.is_cancelled() {
        Err(Error::Cancelled)
    } else if Instant::now() >= end {
        Err(Error::Timeout)
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct Objects {
    source: Option<source::ExtImageCaptureSourceV1>,
    session: Option<ext_session::ExtImageCopyCaptureSessionV1>,
    ext_frame: Option<ext_frame::ExtImageCopyCaptureFrameV1>,
    wlr_frame: Option<wlr_frame::ZwlrScreencopyFrameV1>,
    buffer: Option<wl_buffer::WlBuffer>,
    params: Option<params::ZwpLinuxBufferParamsV1>,
}
impl Objects {
    fn frame_done(&mut self) {
        if let Some(f) = self.ext_frame.take() {
            f.destroy();
        }
        if let Some(f) = self.wlr_frame.take() {
            f.destroy();
        }
        if let Some(b) = self.buffer.take() {
            b.destroy();
        }
        if let Some(p) = self.params.take() {
            p.destroy();
        }
    }
}
impl Drop for Objects {
    fn drop(&mut self) {
        self.frame_done();
        if let Some(s) = self.session.take() {
            s.destroy();
        }
        if let Some(s) = self.source.take() {
            s.destroy();
        }
    }
}

/// Sequential output capture that exclusively borrows its connection. Drop to release the
/// session; previously returned frames remain owned by the caller and valid independently.
pub struct Stream<'a> {
    connection: &'a mut Connection,
    output: Output,
    options: CaptureOptions,
    kind: BufferKind,
    objects: Objects,
    backend: Backend,
    closed: bool,
    first_deadline: Option<Instant>,
}
impl Stream<'_> {
    /// A failed EXT session in Auto mode retries WLR within the same deadline.
    fn falls_back(&self, error: &Error) -> bool {
        matches!(
            error,
            Error::UnsupportedFormat
                | Error::Unsupported(_)
                | Error::CaptureFailed(_)
                | Error::SessionStopped
        ) && self.options.backend == Backend::Auto
            && self.backend == Backend::Ext
            && self.connection.state.wlr.is_some()
    }
    fn prepare(&mut self, end: Instant) -> Result<()> {
        let c = &mut self.connection;
        c.serial += 1;
        let token = c.serial;
        c.state.pending = Some(Pending::new(token));
        if self.backend == Backend::Ext {
            let mgr = c
                .state
                .ext
                .as_ref()
                .ok_or(Error::Unsupported("ext-image-copy-capture"))?;
            let sm = c
                .state
                .source
                .as_ref()
                .ok_or(Error::Unsupported("EXT output capture source"))?;
            let output = &c
                .state
                .outputs
                .values()
                .find(|o| o.info.id == self.output.id)
                .ok_or(Error::OutputGone)?
                .wl;
            let qh = c.queue.handle();
            let source = sm.create_source(output, &qh, ());
            let opts = if self.options.cursor {
                ext_manager::Options::PaintCursors
            } else {
                ext_manager::Options::empty()
            };
            self.objects.session = Some(mgr.create_session(&source, opts, &qh, token));
            self.objects.source = Some(source);
            c.pump(&self.options, end, |s| {
                s.pending.as_ref().is_some_and(|p| p.formats_done)
            })?;
        } else if c.state.wlr.is_none() {
            return Err(Error::Unsupported("wlr-screencopy"));
        }
        Ok(())
    }
    /// Wait for a completed frame, bounded by the stream's capture options.
    /// A timeout, cancellation, layout change or capture failure closes the stream;
    /// later calls return [`Error::SessionStopped`]. EXT can wait for damage after frame one.
    pub fn next_frame(&mut self) -> Result<Frame> {
        if self.closed {
            return Err(Error::SessionStopped);
        }
        let end = self
            .first_deadline
            .take()
            .map(Ok)
            .unwrap_or_else(|| deadline(&self.options))?;
        let mut result = self.next_inner(end);
        if result.as_ref().is_err_and(|error| self.falls_back(error)) {
            self.objects = Objects::default();
            self.backend = Backend::Wlr;
            result = self.prepare(end).and_then(|()| self.next_inner(end));
        }
        self.objects.frame_done();
        if result.is_err() {
            self.closed = true;
            self.objects = Objects::default();
            self.connection.state.pending = None;
        }
        let _ = self.connection.conn.flush();
        result
    }
    fn next_inner(&mut self, end: Instant) -> Result<Frame> {
        let c = &mut self.connection;
        c.barrier(&self.options, end)?;
        c.pump(&self.options, end, |s| {
            s.pending.as_ref().is_some_and(|p| !p.batch_open)
        })?;
        if !c.output_snapshot()?.contains(&self.output) {
            return Err(Error::LayoutChanged);
        }
        let p = c.state.pending.as_mut().ok_or(Error::SessionStopped)?;
        p.reset_frame();
        let token = p.token;
        let qh = c.queue.handle();
        if self.backend == Backend::Wlr {
            p.formats_done = false;
            p.constraints = Constraints::default();
            let output = &c
                .state
                .outputs
                .values()
                .find(|o| o.info.id == self.output.id)
                .ok_or(Error::OutputGone)?
                .wl;
            self.objects.wlr_frame = Some(
                c.state
                    .wlr
                    .as_ref()
                    .ok_or(Error::Unsupported("wlr-screencopy"))?
                    .capture_output(i32::from(self.options.cursor), output, &qh, token),
            );
            c.pump(&self.options, end, |s| {
                s.pending.as_ref().is_some_and(|p| p.formats_done)
            })?;
        } else {
            self.objects.ext_frame = Some(
                self.objects
                    .session
                    .as_ref()
                    .ok_or(Error::SessionStopped)?
                    .create_frame(&qh, token),
            );
        }
        let constraints = c
            .state
            .pending
            .as_ref()
            .ok_or(Error::SessionStopped)?
            .constraints
            .clone();
        let (width, height) = constraints.size;
        // Session constraints may change while this frame is in flight.
        c.state
            .pending
            .as_mut()
            .ok_or(Error::SessionStopped)?
            .frame_size = (width, height);
        let (storage, format, stride) = match &self.kind {
            BufferKind::Cpu => {
                let (wire, format) = constraints
                    .shm
                    .iter()
                    .filter_map(|f| PixelFormat::from_shm(*f as u32).map(|fmt| (*f, fmt)))
                    .min_by_key(|(_, fmt)| fmt.cpu_rank())
                    .ok_or(Error::UnsupportedFormat)?;
                let stride = constraints.stride.unwrap_or(
                    width
                        .checked_mul(format.bytes_per_pixel())
                        .ok_or(Error::InvalidDimensions)?,
                );
                let bytes =
                    self.options
                        .limits
                        .buffer(width, height, stride, format.bytes_per_pixel())?;
                let shm = ShmBuffer::new(bytes)?;
                self.objects.buffer = Some(create_wl_shm(
                    c.state.shm.as_ref().ok_or(Error::Unsupported("wl_shm"))?,
                    &qh,
                    &shm,
                    width,
                    height,
                    stride,
                    wire,
                ));
                (FrameStorage::Cpu(CpuBuffer { shm }), format, stride)
            }
            #[cfg(feature = "gpu")]
            BufferKind::Gpu(allocator) => {
                let dma = c
                    .state
                    .dma
                    .as_ref()
                    .ok_or(Error::Unsupported("linux-dmabuf v3"))?;
                let buffer =
                    allocator.allocate(&constraints, &c.state.dma_formats, self.options.limits)?;
                let params = dma.create_params(&qh, token);
                for (index, plane) in buffer.planes().iter().enumerate() {
                    use std::os::fd::AsFd;
                    params.add(
                        plane.fd().as_fd(),
                        index as u32,
                        plane.offset(),
                        plane.stride(),
                        (buffer.modifier() >> 32) as u32,
                        buffer.modifier() as u32,
                    );
                }
                params.create(
                    width as i32,
                    height as i32,
                    buffer.format() as u32,
                    params::Flags::empty(),
                );
                self.objects.params = Some(params);
                c.pump(&self.options, end, |s| {
                    s.pending.as_ref().is_some_and(|p| p.import_done)
                })?;
                self.objects.buffer = c.state.pending.as_mut().and_then(|p| p.imported.take());
                if self.objects.buffer.is_none() {
                    return Err(Error::CaptureFailed("DMA-BUF import rejected".into()));
                }
                let format = buffer.format();
                let stride = buffer.planes()[0].stride();
                (FrameStorage::Gpu(buffer), format, stride)
            }
        };
        let buffer = self
            .objects
            .buffer
            .as_ref()
            .ok_or(Error::CaptureFailed("missing buffer".into()))?;
        if let Some(frame) = &self.objects.ext_frame {
            frame.attach_buffer(buffer);
            frame.damage_buffer(0, 0, width as i32, height as i32);
            frame.capture();
        }
        if let Some(frame) = &self.objects.wlr_frame {
            frame.copy(buffer);
        }
        c.pump(&self.options, end, |s| {
            s.pending.as_ref().is_some_and(|p| p.ready)
        })?;
        c.barrier(&self.options, end)?;
        if !c.output_snapshot()?.contains(&self.output) {
            return Err(Error::LayoutChanged);
        }
        let p = c.state.pending.as_mut().ok_or(Error::SessionStopped)?;
        Ok(Frame {
            output: self.output.clone(),
            backend: self.backend,
            width,
            height,
            stride,
            format,
            transform: p.transform.unwrap_or(self.output.transform),
            y_inverted: p.inverted,
            presentation_time: p.timestamp,
            damage: std::mem::take(&mut p.damage),
            storage,
        })
    }
}
impl Drop for Stream<'_> {
    fn drop(&mut self) {
        self.objects = Objects::default();
        self.connection.state.pending = None;
        let _ = self.connection.conn.flush();
    }
}
