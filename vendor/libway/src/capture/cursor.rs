//! A cursor session uses a dedicated Connection so image waits never block video.
use super::*;
use std::collections::VecDeque;
use wayland_client::protocol::{wl_pointer, wl_seat};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_cursor_session_v1 as cursor_session;

/// Ordered cursor observations. `received_at` is local monotonic receipt time,
/// not a hardware input timestamp. Positions use transformed output-buffer pixels.
/// Image pixels/hotspot remain in the cursor buffer's own coordinates.
pub enum CursorEvent {
    /// Cursor became visible within the captured output.
    Enter {
        /// Local monotonic event receipt time.
        received_at: Instant,
    },
    /// Cursor left the output or became hidden.
    Leave {
        /// Local monotonic event receipt time.
        received_at: Instant,
    },
    /// Cursor position changed, independently of its image.
    Position {
        /// Horizontal position in transformed output-buffer pixels.
        x: i32,
        /// Vertical position in transformed output-buffer pixels.
        y: i32,
        /// Local monotonic event receipt time.
        received_at: Instant,
    },
    /// A completed cursor image and its associated hotspot.
    Image {
        /// Owned SHM frame; dimensions describe the cursor buffer, not the output.
        frame: Box<Frame>,
        /// Pointer position in the cursor buffer's pixel coordinates.
        hotspot: (i32, i32),
        /// Increasing image generation within this stream.
        generation: u64,
        /// Local monotonic completion receipt time.
        received_at: Instant,
    },
}

enum Notice {
    Event(CursorEvent),
    Ready {
        hotspot: (i32, i32),
        generation: u64,
        received_at: Instant,
    },
}

pub(super) struct CursorState {
    pub seat_global: u32,
    token: u64,
    seat: wl_seat::WlSeat,
    pointer: Option<wl_pointer::WlPointer>,
    notices: VecDeque<Notice>,
    hotspot: (i32, i32),
    generation: u64,
    visible: bool,
    pub error: Option<Error>,
}
impl CursorState {
    fn push(&mut self, notice: Notice) {
        if self.notices.len() >= 256 {
            self.error = Some(Error::LimitExceeded);
        } else {
            self.notices.push_back(notice);
        }
    }
    pub fn image_ready(&mut self) {
        self.generation += 1;
        self.push(Notice::Ready {
            hotspot: self.hotspot,
            generation: self.generation,
            received_at: Instant::now(),
        });
    }
}
impl Drop for CursorState {
    fn drop(&mut self) {
        if let Some(pointer) = self.pointer.take()
            && pointer.version() >= 3
        {
            pointer.release();
        }
        if self.seat.version() >= 5 {
            self.seat.release();
        }
    }
}

struct ImageBuffer {
    cpu: CpuBuffer,
    width: u32,
    height: u32,
    stride: u32,
    format: PixelFormat,
}

/// Independently polling cursor metadata and cursor images. No worker is spawned.
/// Call from a dedicated capture thread/connection. A poll timeout is idle, not
/// a fatal session error. Other errors close the stream. Images use bounded SHM;
/// full-screen GPU capture can proceed independently on another connection.
pub struct CursorStream<'a> {
    connection: &'a mut Connection,
    output: Output,
    options: CaptureOptions,
    objects: Objects,
    session: Option<cursor_session::ExtImageCopyCaptureCursorSessionV1>,
    buffer: Option<ImageBuffer>,
    closed: bool,
}

impl Connection {
    /// Capture one output's pointer cursor separately. The initial API requires
    /// exactly one advertised seat; ambiguous multi-seat selection fails explicitly.
    /// It never falls back to WLR or overlays a cursor onto desktop pixels.
    pub fn cursor_stream(
        &mut self,
        id: OutputId,
        mut options: CaptureOptions,
    ) -> Result<CursorStream<'_>> {
        check_options(&options)?;
        if options.backend == Backend::Wlr {
            return Err(Error::Unsupported("WLR separate cursor capture"));
        }
        options.backend = Backend::Ext;
        options.limits.max_pixels = options.limits.max_pixels.min(1024 * 1024);
        options.limits.max_bytes = options.limits.max_bytes.min(16 * 1024 * 1024);
        let end = deadline(&options)?;
        self.barrier(&options, end)?;
        let output = self
            .output_snapshot()?
            .into_iter()
            .find(|o| o.id == id)
            .ok_or(Error::OutputGone)?;
        if !self.capabilities().ext_output_capture {
            return Err(Error::Unsupported("EXT cursor capture"));
        }
        if self.state.seats.len() != 1 {
            return Err(Error::Unsupported(
                "cursor capture requires exactly one seat",
            ));
        }
        let (&name, &version) = self
            .state
            .seats
            .first_key_value()
            .ok_or(Error::SessionStopped)?;
        let seat = self
            .registry
            .bind(name, version.min(5), &self.queue.handle(), ());
        self.serial += 1;
        let token = self.serial;
        self.state.cursor = Some(CursorState {
            token,
            seat_global: name,
            seat,
            pointer: None,
            notices: VecDeque::new(),
            hotspot: (0, 0),
            generation: 0,
            visible: false,
            error: None,
        });
        let mut stream = CursorStream {
            connection: self,
            output,
            options,
            objects: Objects::default(),
            session: None,
            buffer: None,
            closed: false,
        };
        stream.initialize(end)?;
        Ok(stream)
    }
}

impl CursorStream<'_> {
    fn initialize(&mut self, end: Instant) -> Result<()> {
        let c = &mut self.connection;
        c.pump(&self.options, end, |s| {
            s.cursor.as_ref().is_some_and(|c| c.pointer.is_some())
        })?;
        let pointer = c
            .state
            .cursor
            .as_ref()
            .and_then(|s| s.pointer.as_ref())
            .ok_or(Error::SessionStopped)?;
        let manager = c
            .state
            .ext
            .as_ref()
            .ok_or(Error::Unsupported("EXT cursor capture"))?;
        let source_manager = c
            .state
            .source
            .as_ref()
            .ok_or(Error::Unsupported("EXT output source"))?;
        let output = &c
            .state
            .outputs
            .values()
            .find(|o| o.info.id == self.output.id)
            .ok_or(Error::OutputGone)?
            .wl;
        let qh = c.queue.handle();
        let token = c.state.cursor.as_ref().ok_or(Error::SessionStopped)?.token;
        c.state.pending = Some(Pending::new(token));
        let source = source_manager.create_source(output, &qh, ());
        let session = manager.create_pointer_cursor_session(&source, pointer, &qh, token);
        self.objects.session = Some(session.get_capture_session(&qh, token));
        self.objects.source = Some(source);
        self.session = Some(session);
        c.conn.flush().map_err(|e| Error::Wayland(e.to_string()))?;
        Ok(())
    }

    /// Wait up to `timeout` for the next observation; idle timeout yields `Ok(None)` and
    /// preserves the session. Cancellation, layout changes and protocol errors close it.
    /// An unrepresentable timeout returns [`Error::InvalidDimensions`] without closing it.
    pub fn poll_event(&mut self, timeout: Duration) -> Result<Option<CursorEvent>> {
        if self.closed {
            return Err(Error::SessionStopped);
        }
        let end = Instant::now()
            .checked_add(timeout)
            .ok_or(Error::InvalidDimensions)?;
        let result = self.poll_inner(end);
        match result {
            Err(Error::Timeout) => Ok(None),
            Err(error) => {
                self.close();
                Err(error)
            }
            other => other,
        }
    }

    fn poll_inner(&mut self, end: Instant) -> Result<Option<CursorEvent>> {
        loop {
            let in_flight = self.buffer.is_some();
            let c = &mut self.connection;
            c.pump(&self.options, end, |s| {
                s.cursor.as_ref().is_some_and(|c| !c.notices.is_empty())
                    || (!in_flight
                        && s.pending
                            .as_ref()
                            .is_some_and(|p| p.formats_done && !p.batch_open))
            })?;
            if !c.output_snapshot()?.contains(&self.output) {
                return Err(Error::LayoutChanged);
            }
            let notice = c
                .state
                .cursor
                .as_mut()
                .ok_or(Error::SessionStopped)?
                .notices
                .pop_front();
            match notice {
                Some(Notice::Event(event)) => return Ok(Some(event)),
                Some(Notice::Ready {
                    hotspot,
                    generation,
                    received_at,
                }) => {
                    let image = self
                        .buffer
                        .take()
                        .ok_or(Error::CaptureFailed("cursor ready without a buffer".into()))?;
                    let p = c.state.pending.as_mut().ok_or(Error::SessionStopped)?;
                    let frame = Frame {
                        output: self.output.clone(),
                        backend: Backend::Ext,
                        width: image.width,
                        height: image.height,
                        stride: image.stride,
                        format: image.format,
                        transform: p.transform.ok_or(Error::CaptureFailed(
                            "cursor frame missing transform".into(),
                        ))?,
                        y_inverted: false,
                        presentation_time: p.timestamp,
                        damage: std::mem::take(&mut p.damage),
                        storage: FrameStorage::Cpu(image.cpu),
                    };
                    self.objects.frame_done();
                    return Ok(Some(CursorEvent::Image {
                        frame: Box::new(frame),
                        hotspot,
                        generation,
                        received_at,
                    }));
                }
                None => self.request_image()?,
            }
        }
    }

    fn request_image(&mut self) -> Result<()> {
        let c = &mut self.connection;
        let p = c.state.pending.as_mut().ok_or(Error::SessionStopped)?;
        let (width, height) = p.constraints.size;
        if width > 1024 || height > 1024 {
            return Err(Error::LimitExceeded);
        }
        let (wire, format) = p
            .constraints
            .shm
            .iter()
            .find_map(|f| PixelFormat::from_shm(*f as u32).map(|fmt| (*f, fmt)))
            .ok_or(Error::UnsupportedFormat)?;
        let stride = width
            .checked_mul(format.bytes_per_pixel())
            .ok_or(Error::InvalidDimensions)?;
        let size = self
            .options
            .limits
            .buffer(width, height, stride, format.bytes_per_pixel())?;
        let shm = ShmBuffer::new(size)?;
        p.reset_frame();
        p.frame_size = (width, height);
        let qh = c.queue.handle();
        let buffer = create_wl_shm(
            c.state.shm.as_ref().ok_or(Error::Unsupported("wl_shm"))?,
            &qh,
            &shm,
            width,
            height,
            stride,
            wire,
        );
        let frame = self
            .objects
            .session
            .as_ref()
            .ok_or(Error::SessionStopped)?
            .create_frame(&qh, p.token);
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        self.objects.buffer = Some(buffer);
        self.objects.ext_frame = Some(frame);
        self.buffer = Some(ImageBuffer {
            cpu: CpuBuffer { shm },
            width,
            height,
            stride,
            format,
        });
        Ok(())
    }

    fn close(&mut self) {
        self.closed = true;
        self.objects.frame_done();
        if let Some(s) = self.objects.session.take() {
            s.destroy();
        }
        if let Some(s) = self.session.take() {
            s.destroy();
        }
        self.objects = Objects::default();
        self.connection.state.pending = None;
        self.connection.state.cursor = None;
        // The caller owns returned frames; only an unpublished allocation drops here.
        self.buffer = None;
        let _ = self.connection.conn.flush();
    }
}
impl Drop for CursorStream<'_> {
    fn drop(&mut self) {
        self.close();
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &WlConnection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(cursor) = &mut s.cursor else { return };
        if seat.id() != cursor.seat.id() {
            return;
        }
        if let wl_seat::Event::Capabilities { capabilities } = event {
            if matches!(capabilities, WEnum::Value(c) if c.contains(wl_seat::Capability::Pointer)) {
                if cursor.pointer.is_none() {
                    cursor.pointer = Some(seat.get_pointer(qh, ()));
                }
            } else {
                cursor.error = Some(Error::Unsupported("seat has no pointer"));
            }
        }
    }
}
delegate_noop!(State: ignore wl_pointer::WlPointer);

impl Dispatch<cursor_session::ExtImageCopyCaptureCursorSessionV1, u64> for State {
    fn event(
        s: &mut Self,
        _: &cursor_session::ExtImageCopyCaptureCursorSessionV1,
        event: cursor_session::Event,
        token: &u64,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(c) = &mut s.cursor else { return };
        if c.token != *token {
            return;
        }
        let received_at = Instant::now();
        match event {
            cursor_session::Event::Enter => {
                c.visible = true;
                c.push(Notice::Event(CursorEvent::Enter { received_at }));
            }
            cursor_session::Event::Leave => {
                c.visible = false;
                c.push(Notice::Event(CursorEvent::Leave { received_at }));
            }
            cursor_session::Event::Position { x, y } if c.visible => {
                c.push(Notice::Event(CursorEvent::Position { x, y, received_at }))
            }
            cursor_session::Event::Hotspot { x, y } => c.hotspot = (x, y),
            _ => {}
        }
    }
}
