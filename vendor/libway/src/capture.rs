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
mod batch;
mod cursor;
mod events;
#[cfg(feature = "vulkan")]
mod fast;
mod pending;
mod readback;
mod stream;
#[cfg(test)]
mod tests;
mod tiles;
pub use cursor::{CursorEvent, CursorStream};
pub(crate) use pending::Constraints;
use pending::Pending;
pub use readback::{ReadbackFallback, TileReadback};
pub use stream::Stream;
pub use tiles::{DesktopTile, DesktopTiles};
#[cfg(feature = "image")]
pub(crate) use tiles::{near, scaled_size};

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
    pending: BTreeMap<u64, Pending>,
    /// The latest `dmabuf_device` a capture session announced.
    dmabuf_device: Option<u64>,
    #[cfg(feature = "vulkan")]
    readback: fast::ReadbackState,
    global_error: Option<Error>,
    limits: Limits,
}
impl State {
    fn pending(&mut self, token: u64) -> Option<&mut Pending> {
        self.pending.get_mut(&token)
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
                pending: BTreeMap::new(),
                dmabuf_device: None,
                #[cfg(feature = "vulkan")]
                readback: Default::default(),
                global_error: None,
                limits: options.limits,
            },
            serial: 0,
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
        let job = batch::Job::new(self, output, options, kind)?;
        let mut stream = Stream {
            connection: self,
            job,
            closed: false,
            first_deadline: Some(end),
        };
        if let Err(error) = stream.prepare(end) {
            if !stream.falls_back(&error) {
                return Err(error);
            }
            stream.retry_wlr(end)?;
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
            if let Some(e) = self.state.pending.values_mut().find_map(|p| p.error.take()) {
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
