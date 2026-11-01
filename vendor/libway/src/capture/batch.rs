//! Per-output capture work split into phases that take the connection as an
//! argument, so one connection can drive several outputs at once.
//!
//! A [`Job`] owns one output's protocol objects and its pending token. Its
//! phases send requests without waiting; the caller pumps until every job's
//! predicate holds. [`super::Stream`] runs one job phase by phase;
//! [`Connection::capture_batch`] runs many in lockstep, so every capture
//! request is out before the first wait for `ready`.
use super::*;

/// The storage of a submitted frame, kept until `ready`.
struct InFlight {
    storage: FrameStorage,
    format: PixelFormat,
    stride: u32,
    size: (u32, u32),
}

pub(super) struct Job {
    pub(super) output: Output,
    pub(super) options: CaptureOptions,
    pub(super) kind: BufferKind,
    pub(super) objects: Objects,
    pub(super) backend: Backend,
    pub(super) token: u64,
    in_flight: Option<InFlight>,
}

impl Job {
    /// Resolve the backend for `options`; nothing is sent yet.
    pub(super) fn new(
        c: &Connection,
        output: Output,
        options: CaptureOptions,
        kind: BufferKind,
    ) -> Result<Self> {
        let backend = match options.backend {
            Backend::Auto if c.capabilities().ext_output_capture => Backend::Ext,
            Backend::Auto if c.state.wlr.is_some() => Backend::Wlr,
            Backend::Auto => {
                return Err(Error::Unsupported("EXT output capture or WLR screencopy"));
            }
            b => b,
        };
        Ok(Self {
            output,
            options,
            kind,
            objects: Objects::default(),
            backend,
            token: 0,
            in_flight: None,
        })
    }

    /// A failed EXT session in Auto mode retries WLR within the same deadline.
    pub(super) fn falls_back(&self, c: &Connection, error: &Error) -> bool {
        matches!(
            error,
            Error::UnsupportedFormat
                | Error::Unsupported(_)
                | Error::CaptureFailed(_)
                | Error::SessionStopped
        ) && self.options.backend == Backend::Auto
            && self.backend == Backend::Ext
            && c.state.wlr.is_some()
    }

    /// Switch to WLR after [`Self::falls_back`]: drop every EXT object and the
    /// pending entry; [`Self::start`] must run again.
    pub(super) fn use_wlr(&mut self, c: &mut Connection) {
        self.release(c);
        self.backend = Backend::Wlr;
    }

    fn wl_output<'c>(&self, c: &'c Connection) -> Result<&'c wl_output::WlOutput> {
        Ok(&c
            .state
            .outputs
            .values()
            .find(|o| o.info.id == self.output.id)
            .ok_or(Error::OutputGone)?
            .wl)
    }

    /// Register a fresh pending entry and, for EXT, send the source and session.
    pub(super) fn start(&mut self, c: &mut Connection) -> Result<()> {
        c.serial += 1;
        self.token = c.serial;
        c.state.pending.insert(self.token, Pending::new(self.token));
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
            let qh = c.queue.handle();
            let source = sm.create_source(self.wl_output(c)?, &qh, ());
            let opts = if self.options.cursor {
                ext_manager::Options::PaintCursors
            } else {
                ext_manager::Options::empty()
            };
            self.objects.session = Some(mgr.create_session(&source, opts, &qh, self.token));
            self.objects.source = Some(source);
        } else if c.state.wlr.is_none() {
            return Err(Error::Unsupported("wlr-screencopy"));
        }
        Ok(())
    }

    fn pending<'s>(&self, s: &'s State) -> Option<&'s Pending> {
        s.pending.get(&self.token)
    }

    /// EXT: the session announced its constraints. WLR has nothing to wait for.
    pub(super) fn started(&self, s: &State) -> bool {
        self.backend == Backend::Wlr || self.pending(s).is_some_and(|p| p.formats_done)
    }

    /// No constraint batch is half delivered.
    pub(super) fn settled(&self, s: &State) -> bool {
        self.pending(s).is_some_and(|p| !p.batch_open)
    }

    /// Reset the per-frame state and ask for a frame: EXT creates it, WLR asks
    /// the compositor which buffer it wants.
    pub(super) fn request(&mut self, c: &mut Connection) -> Result<()> {
        let p = c
            .state
            .pending
            .get_mut(&self.token)
            .ok_or(Error::SessionStopped)?;
        p.reset_frame();
        let qh = c.queue.handle();
        if self.backend == Backend::Wlr {
            p.formats_done = false;
            p.constraints = Constraints::default();
            let frame = c
                .state
                .wlr
                .as_ref()
                .ok_or(Error::Unsupported("wlr-screencopy"))?
                .capture_output(
                    i32::from(self.options.cursor),
                    self.wl_output(c)?,
                    &qh,
                    self.token,
                );
            self.objects.wlr_frame = Some(frame);
        } else {
            // Hyprland sends the first damage when the frame is created, which
            // another job's dma-buf import pump can dispatch before this job's
            // submit: the frame size must be known from creation on.
            p.frame_size = p.constraints.size;
            self.objects.ext_frame = Some(
                self.objects
                    .session
                    .as_ref()
                    .ok_or(Error::SessionStopped)?
                    .create_frame(&qh, self.token),
            );
        }
        Ok(())
    }

    /// WLR: the buffer constraints for this frame arrived. EXT already has them.
    pub(super) fn requested(&self, s: &State) -> bool {
        self.backend == Backend::Ext || self.pending(s).is_some_and(|p| p.formats_done)
    }

    /// Allocate storage for the current constraints, attach it and capture.
    pub(super) fn submit(&mut self, c: &mut Connection, end: Instant) -> Result<()> {
        let constraints = c
            .state
            .pending
            .get(&self.token)
            .ok_or(Error::SessionStopped)?
            .constraints
            .clone();
        let (width, height) = constraints.size;
        // Session constraints may change while this frame is in flight.
        c.state
            .pending
            .get_mut(&self.token)
            .ok_or(Error::SessionStopped)?
            .frame_size = (width, height);
        let qh = c.queue.handle();
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
                (FrameStorage::Cpu(CpuBuffer::shm(shm)), format, stride)
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
                let params = dma.create_params(&qh, self.token);
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
                let token = self.token;
                c.pump(&self.options, end, |s| {
                    s.pending.get(&token).is_some_and(|p| p.import_done)
                })?;
                self.objects.buffer = c
                    .state
                    .pending
                    .get_mut(&self.token)
                    .and_then(|p| p.imported.take());
                if self.objects.buffer.is_none() {
                    return Err(Error::CaptureFailed("DMA-BUF import rejected".into()));
                }
                let format = buffer.format();
                let stride = buffer.planes()[0].stride();
                (FrameStorage::Gpu(buffer), format, stride)
            }
        };
        #[cfg(not(feature = "gpu"))]
        let _ = end;
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
        self.in_flight = Some(InFlight {
            storage,
            format,
            stride,
            size: (width, height),
        });
        Ok(())
    }

    pub(super) fn ready(&self, s: &State) -> bool {
        self.pending(s).is_some_and(|p| p.ready)
    }

    /// Turn the completed storage into a [`Frame`]. The caller checks the layout.
    pub(super) fn finish(&mut self, c: &mut Connection) -> Result<Frame> {
        let in_flight = self.in_flight.take().ok_or(Error::SessionStopped)?;
        let p = c
            .state
            .pending
            .get_mut(&self.token)
            .ok_or(Error::SessionStopped)?;
        Ok(Frame {
            output: self.output.clone(),
            backend: self.backend,
            width: in_flight.size.0,
            height: in_flight.size.1,
            stride: in_flight.stride,
            format: in_flight.format,
            transform: p.transform.unwrap_or(self.output.transform),
            y_inverted: p.inverted,
            presentation_time: p.timestamp,
            damage: std::mem::take(&mut p.damage),
            storage: in_flight.storage,
        })
    }

    /// Destroy this frame's objects; the session stays for the next frame.
    pub(super) fn frame_done(&mut self) {
        self.objects.frame_done();
        self.in_flight = None;
    }

    /// Destroy every protocol object and forget the pending entry.
    pub(super) fn release(&mut self, c: &mut Connection) {
        self.objects = Objects::default();
        self.in_flight = None;
        c.state.pending.remove(&self.token);
    }
}

impl Connection {
    /// Capture one frame of every output in `outputs` on this connection, with
    /// all capture requests in flight together. `options.limits` applies per
    /// output; `end` bounds the whole batch. Any error, timeout or cancellation
    /// of one output fails the batch and releases every output's objects. In
    /// `Auto` mode an EXT failure that permits it retries the whole batch with
    /// WLR within the same deadline. Frames are returned in `outputs` order.
    pub(crate) fn capture_batch(
        &mut self,
        outputs: &[Output],
        options: &CaptureOptions,
        end: Instant,
    ) -> Result<Vec<Frame>> {
        let mut frames: Vec<Option<Frame>> = outputs.iter().map(|_| None).collect();
        self.capture_batch_each(
            outputs,
            options,
            end,
            BufferKind::Cpu,
            &mut |index, frame| {
                frames[index] = Some(frame);
                Ok(())
            },
        )?;
        frames
            .into_iter()
            .map(|frame| frame.ok_or(Error::SessionStopped))
            .collect()
    }

    /// [`Self::capture_batch`], handing each frame to `on_frame` with its index
    /// in `outputs` as soon as its `ready` arrives, so the caller can process
    /// it while the compositor copies the others. The barrier and layout check
    /// run after the last frame; if they fail, or `on_frame` fails, the whole
    /// call fails and the caller discards what it received. A WLR retry hands
    /// every frame over again; an `on_frame` failure is the caller's and never
    /// retries.
    pub(crate) fn capture_batch_each(
        &mut self,
        outputs: &[Output],
        options: &CaptureOptions,
        end: Instant,
        kind: BufferKind,
        on_frame: &mut dyn FnMut(usize, Frame) -> Result<()>,
    ) -> Result<()> {
        check_options(options)?;
        self.barrier(options, end)?;
        let mut jobs = outputs
            .iter()
            .map(|output| Job::new(self, output.clone(), options.clone(), kind.clone()))
            .collect::<Result<Vec<_>>>()?;
        let consumer_failed = std::cell::Cell::new(false);
        let mut on_frame = |index, frame| {
            let result = on_frame(index, frame);
            consumer_failed.set(result.is_err());
            result
        };
        let mut result = self.run_batch(&mut jobs, end, &mut on_frame);
        if let Err(error) = &result
            && !consumer_failed.get()
            && jobs.iter().all(|job| job.falls_back(self, error))
        {
            for job in &mut jobs {
                job.use_wlr(self);
            }
            result = self.run_batch(&mut jobs, end, &mut on_frame);
        }
        for job in &mut jobs {
            job.release(self);
        }
        let _ = self.conn.flush();
        result
    }

    fn run_batch(
        &mut self,
        jobs: &mut [Job],
        end: Instant,
        on_frame: &mut dyn FnMut(usize, Frame) -> Result<()>,
    ) -> Result<()> {
        let options = jobs
            .first()
            .map(|job| job.options.clone())
            .ok_or(Error::NoOutputs)?;
        for job in jobs.iter_mut() {
            job.start(self)?;
        }
        self.pump(&options, end, |s| jobs.iter().all(|job| job.started(s)))?;
        self.barrier(&options, end)?;
        self.pump(&options, end, |s| jobs.iter().all(|job| job.settled(s)))?;
        self.check_layout(jobs)?;
        for job in jobs.iter_mut() {
            job.request(self)?;
        }
        self.pump(&options, end, |s| jobs.iter().all(|job| job.requested(s)))?;
        for job in jobs.iter_mut() {
            job.submit(self, end)?;
        }
        // Hand over each frame in ready order while the others are in flight.
        let mut delivered = vec![false; jobs.len()];
        while delivered.contains(&false) {
            self.pump(&options, end, |s| {
                jobs.iter()
                    .zip(&delivered)
                    .any(|(job, done)| !done && job.ready(s))
            })?;
            for (index, job) in jobs.iter_mut().enumerate() {
                if !delivered[index] && job.ready(&self.state) {
                    delivered[index] = true;
                    let frame = job.finish(self)?;
                    job.frame_done();
                    on_frame(index, frame)?;
                }
            }
        }
        self.barrier(&options, end)?;
        self.check_layout(jobs)
    }

    fn check_layout(&self, jobs: &[Job]) -> Result<()> {
        let snapshot = self.output_snapshot()?;
        if jobs.iter().any(|job| !snapshot.contains(&job.output)) {
            return Err(Error::LayoutChanged);
        }
        Ok(())
    }
}
