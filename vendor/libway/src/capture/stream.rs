//! One output captured frame by frame on an exclusively borrowed connection.
use super::{batch::Job, *};

/// Sequential output capture that exclusively borrows its connection. Drop to release the
/// session; previously returned frames remain owned by the caller and valid independently.
pub struct Stream<'a> {
    pub(super) connection: &'a mut Connection,
    pub(super) job: Job,
    pub(super) closed: bool,
    pub(super) first_deadline: Option<Instant>,
}
impl Stream<'_> {
    /// A failed EXT session in Auto mode retries WLR within the same deadline.
    pub(super) fn falls_back(&self, error: &Error) -> bool {
        self.job.falls_back(self.connection, error)
    }
    /// Switch to WLR after [`Self::falls_back`] and prepare again.
    pub(super) fn retry_wlr(&mut self, end: Instant) -> Result<()> {
        self.job.use_wlr(self.connection);
        self.prepare(end)
    }
    pub(super) fn prepare(&mut self, end: Instant) -> Result<()> {
        let c = &mut *self.connection;
        self.job.start(c)?;
        let job = &self.job;
        c.pump(&job.options, end, |s| job.started(s))
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
            .unwrap_or_else(|| deadline(&self.job.options))?;
        let mut result = self.next_inner(end);
        if result.as_ref().is_err_and(|error| self.falls_back(error)) {
            result = self.retry_wlr(end).and_then(|()| self.next_inner(end));
        }
        self.job.frame_done();
        if result.is_err() {
            self.closed = true;
            self.job.release(self.connection);
        }
        let _ = self.connection.conn.flush();
        result
    }
    fn next_inner(&mut self, end: Instant) -> Result<Frame> {
        let c = &mut *self.connection;
        let job = &mut self.job;
        let options = job.options.clone();
        c.barrier(&options, end)?;
        c.pump(&options, end, |s| job.settled(s))?;
        if !c.output_snapshot()?.contains(&job.output) {
            return Err(Error::LayoutChanged);
        }
        job.request(c)?;
        c.pump(&options, end, |s| job.requested(s))?;
        job.submit(c, end)?;
        c.pump(&options, end, |s| job.ready(s))?;
        c.barrier(&options, end)?;
        if !c.output_snapshot()?.contains(&job.output) {
            return Err(Error::LayoutChanged);
        }
        job.finish(c)
    }
}
impl Drop for Stream<'_> {
    fn drop(&mut self) {
        self.job.release(self.connection);
        let _ = self.connection.conn.flush();
    }
}
