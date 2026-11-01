//! Native text-input identities and owned bounded transactions.
use super::budget::Payload;
use crate::window::{ConfigurationRevision, InputSeatId, WindowId};

/// Exact connection, seat and activation identity, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImeSession {
    pub(super) window: WindowId,
    pub(super) seat: InputSeatId,
    pub(super) sequence: u64,
}
/// Protocol state-change reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImeCause {
    /// State resulted from an input-method transaction.
    InputMethod,
    /// State resulted from other application work.
    Other,
}
/// Surface-local logical integer candidate rectangle, without output scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImeRectangle {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Nonnegative width with representable right edge.
    pub width: i32,
    /// Nonnegative height with representable bottom edge.
    pub height: i32,
}
/// Known surrounding text, at most 4000 UTF-8 bytes. Empty is distinct from unknown.
#[derive(Clone, Copy, Debug)]
pub struct ImeSurrounding<'a> {
    /// Exact committed bytes without an interior NUL.
    pub text: &'a str,
    /// Scalar-boundary byte offset.
    pub cursor: usize,
    /// Directed selection anchor at a scalar boundary.
    pub anchor: usize,
}
/// Complete state admission; all fields validate before any protocol effect.
#[derive(Clone, Copy, Debug)]
pub struct ImeState<'a> {
    /// Latest native window configuration.
    pub configuration: ConfigurationRevision,
    /// Surrounding capability is fixed for an activation.
    pub surrounding: Option<ImeSurrounding<'a>>,
    /// Candidate rectangle in logical surface units.
    pub caret: ImeRectangle,
    /// State-change cause.
    pub cause: ImeCause,
}
/// Immutable native text. Each value caps at 4096 bytes, with a shared 128 KiB
/// host ledger retained through the final clone, including after shutdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImeText(pub(super) Payload);
impl ImeText {
    /// Exact validated UTF-8 bytes.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Share exact bytes without a copy; escaped clones remain in the native ledger.
    pub fn shared(&self) -> std::sync::Arc<str> {
        self.0.clone()
    }
}
/// Directed scalar-boundary byte offsets within the preedit string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImePreeditCursor {
    /// First directed endpoint.
    pub begin: usize,
    /// Second directed endpoint.
    pub end: usize,
}
/// Provisional display, independent of any application's document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImePreedit {
    /// Exact provisional UTF-8 bytes.
    pub text: ImeText,
    /// None is the protocol's explicitly hidden cursor.
    pub cursor: Option<ImePreeditCursor>,
}
/// One complete Done block, never independently delivered component edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImeTransaction {
    /// Byte count before the advertised cursor.
    pub delete_before: u32,
    /// Byte count after the advertised cursor.
    pub delete_after: u32,
    /// None means no replacement; Some(empty) is an explicit replacement.
    pub commit: Option<ImeText>,
    /// None clears any preceding provisional display.
    pub preedit: Option<ImePreedit>,
}
/// Ordered text-input facts on the host's existing window event queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// Readiness enters/leaves are reported even without an active producer.
    Focus {
        /// Owned window.
        window: WindowId,
        /// Native seat.
        seat: InputSeatId,
        /// Actual text-input enter versus leave.
        entered: bool,
    },
    /// A complete transaction for the exact active association.
    Done {
        /// Owned window.
        window: WindowId,
        /// Native seat.
        seat: InputSeatId,
        /// Exact activation.
        session: ImeSession,
        /// Checked native commit count echoed by the compositor.
        serial: u32,
        /// True only when this Done matches the latest sent state.
        synchronized: bool,
        /// Entire buffered edit block.
        transaction: ImeTransaction,
    },
    /// Retirement clears pending work and cancels this exact association once.
    Ended {
        /// Owned window.
        window: WindowId,
        /// Native seat.
        seat: InputSeatId,
        /// Retired activation.
        session: ImeSession,
    },
}
