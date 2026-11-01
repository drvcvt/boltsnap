//! Borrowed native configure/acquire boundary and irreversible pre-present staging.
use super::{NativePresenter, state::Admission};
use crate::{
    Result,
    window::{Configuration, ConfigurationRevision, Host},
};
use std::fmt;

/// Readiness before any driver configure/acquire or libway staging requests.
#[derive(Debug)]
#[non_exhaustive]
pub enum NativeFrameStart<'a> {
    /// The exclusive owner/claim borrow remains live through this attempt.
    Ready(NativeFrame<'a>),
    /// A real preceding native frame callback is pending. Wait for native events.
    NeedsFrame,
    /// Current configuration forbids configure/acquire/present until resume.
    Suspended,
}

/// One cooperating driver attempt, exclusively borrowing host and native claim.
/// Configure from [`Self::configuration`], then acquire, encode and submit before
/// [`Self::stage`]. Immediately call the driver's sole present/commit and consume
/// this guard with [`Self::scheduled`]. No callback or read preparation can run on
/// this host during that borrow. Driver calls outside the guard remain the caller's
/// responsibility, including foreign internal readers or concurrent surfaces.
///
/// Cancellation/Drop before staging issues no requests and retains no pacing token.
/// After staging it retires this window, because wl_callback has no cancel request;
/// keep driver resources alive until safely released. Retirement does not detach,
/// commit or fabricate GPU completion. A forgotten guard blocks pump until close.
///
/// The host cannot pump while the guard is used:
/// ```compile_fail
/// use libway::window::{Host, NativeFrame, NativeFrameStart, NativePresenter, ConfigurationRevision};
/// fn overlapping(host: &mut Host, claim: &NativePresenter, revision: ConfigurationRevision) {
///     if let NativeFrameStart::Ready(frame) = host.begin_native_frame(claim, revision).unwrap() {
///         host.pump(None);
///         frame.cancel();
///     }
/// }
/// ```
/// The claim cannot be destroyed while the guard is used:
/// ```compile_fail
/// use libway::window::{Host, NativeFrameStart, NativePresenter, ConfigurationRevision};
/// fn destruction(host: &mut Host, claim: NativePresenter, revision: ConfigurationRevision) {
///     if let NativeFrameStart::Ready(frame) = host.begin_native_frame(&claim, revision).unwrap() {
///         drop(claim);
///         frame.cancel();
///     }
/// }
/// ```
/// Close and consuming shutdown also require releasing the guard:
/// ```compile_fail
/// use libway::window::{Host, NativeFrameStart, NativePresenter, ConfigurationRevision};
/// fn closing(host: &mut Host, claim: &NativePresenter, revision: ConfigurationRevision) {
///     if let NativeFrameStart::Ready(frame) = host.begin_native_frame(claim, revision).unwrap() {
///         host.close(claim.id());
///         frame.cancel();
///     }
/// }
/// ```
/// ```compile_fail
/// use libway::window::{Host, NativeFrameStart, NativePresenter, ConfigurationRevision};
/// fn shutdown(mut host: Host, claim: &NativePresenter, revision: ConfigurationRevision) {
///     if let NativeFrameStart::Ready(frame) = host.begin_native_frame(claim, revision).unwrap() {
///         host.shutdown();
///         frame.cancel();
///     }
/// }
/// ```
#[must_use = "cancel before staging or schedule immediately after the driver present call"]
pub struct NativeFrame<'a> {
    host: &'a mut Host,
    presenter: &'a NativePresenter,
    configuration: Configuration,
    finished: bool,
}

impl fmt::Debug for NativeFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFrame")
            .field("window", &self.presenter.id())
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

pub(in crate::window) fn begin<'a>(
    host: &'a mut Host,
    presenter: &'a NativePresenter,
    revision: ConfigurationRevision,
) -> Result<NativeFrameStart<'a>> {
    match host
        .coordinator
        .mutate(|state| state.begin_native(presenter, revision))?
    {
        Admission::Ready(configuration) => Ok(NativeFrameStart::Ready(NativeFrame {
            host,
            presenter,
            configuration,
            finished: false,
        })),
        Admission::NeedsFrame => Ok(NativeFrameStart::NeedsFrame),
        Admission::Suspended => Ok(NativeFrameStart::Suspended),
    }
}

impl NativeFrame<'_> {
    /// Validated immutable snapshot. Configure the driver with its physical extent.
    pub fn configuration(&self) -> Configuration {
        self.configuration
    }

    /// Validate the acquired texture extent and stage scale/viewport plus one frame
    /// callback, after successful acquire/encode/submit and immediately before
    /// driver present. Never attaches, damages, commits, flushes or dispatches.
    /// Stale/foreign/duplicate staging and mismatched extents fail before requests.
    /// Any error after requests requires cancellation/Drop to retire the window.
    pub fn stage(&mut self, pixel_width: u32, pixel_height: u32) -> Result<()> {
        self.host.coordinator.mutate(|state| {
            state.stage_native(
                self.presenter,
                self.configuration,
                (pixel_width, pixel_height),
            )
        })
    }

    /// Record that the driver present call was made. This certifies neither display
    /// success nor GPU completion; the real matching callback remains the pace gate.
    /// Call only after the cooperating driver committed the staged state once.
    /// Later asynchronous driver failure requires closing the affected window.
    pub fn scheduled(mut self) -> Result<()> {
        let result = self
            .host
            .coordinator
            .mutate(|state| state.finish_native(self.presenter.id(), true));
        self.finished = result.is_ok();
        result
    }

    /// Cancel this attempt. Before staging it is reusable; after staging it closes
    /// the window logically without a second surface commit or fake callback.
    pub fn cancel(mut self) -> Result<()> {
        let result = self
            .host
            .coordinator
            .mutate(|state| state.finish_native(self.presenter.id(), false));
        self.finished = result.is_ok();
        result
    }
}

impl Drop for NativeFrame<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // Drop does not read, dispatch, wait for native work or invoke app code.
            // Coordinator errors retire the entire owner and close admission.
            let _ = self
                .host
                .coordinator
                .mutate(|state| state.finish_native(self.presenter.id(), false));
        }
    }
}
