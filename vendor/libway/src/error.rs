use std::{fmt, io};

/// Capture, connection and DnD request errors. Unsupported operations never return empty success.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Operating-system or device I/O failed; available through `source()`.
    Io(io::Error),
    /// Wayland connection, dispatch or protocol failure.
    Wayland(String),
    /// Required protocol or requested operation is unavailable.
    Unsupported(&'static str),
    /// No outputs intersect the requested capture area.
    NoOutputs,
    /// Output id belongs to another connection or has been removed.
    OutputGone,
    /// Layout changed during capture; refresh outputs and retry.
    LayoutChanged,
    /// No mutually supported buffer format/layout was found.
    UnsupportedFormat,
    /// Invalid image dimensions, stride, rectangle or unrepresentable capture timeout.
    InvalidDimensions,
    /// A configured resource bound or library input-size bound was exceeded.
    LimitExceeded,
    /// A blocking operation reached its deadline.
    Timeout,
    /// Capture cancellation was requested.
    Cancelled,
    /// Compositor rejected capture or returned inconsistent capture data.
    CaptureFailed(String),
    /// Capture session ended or was previously closed after a failure.
    SessionStopped,
    /// The supplied or tracked input serial is missing; compositors reject the drag.
    InvalidSerial,
    /// The DnD session has not received its initial registry sync yet.
    NotReady,
    /// Target id is unknown to this DnD session or has been unregistered.
    UnknownTarget,
    /// No drop or drag with this id is waiting for an answer.
    UnknownTransfer,
    /// A request argument violates the protocol rules libway would otherwise send on.
    InvalidInput(&'static str),
    /// Typed payload/transfer failure, available through `source()`.
    TransferFailed(TransferError),
}
/// Why a single DnD transfer ended without a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferError {
    /// The offer went away (leave/cancel, seat removed, source closed) before completion.
    Aborted,
    /// Payload exceeded `DndLimits`.
    TooLarge,
    /// No progress within `DndLimits::inactivity` or `DndLimits::total`.
    Timeout,
    /// The pipe reported an I/O error.
    Io,
    /// The target was unregistered while the transfer was running.
    TargetGone,
    /// Payload bytes did not decode as the requested convenience type.
    Malformed,
}
/// Result type for libway operations.
pub type Result<T> = std::result::Result<T, Error>;
impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Aborted => "transfer aborted",
            Self::TooLarge => "payload exceeds configured limits",
            Self::Timeout => "transfer deadline exceeded",
            Self::Io => "transfer pipe I/O error",
            Self::TargetGone => "drop target was unregistered",
            Self::Malformed => "payload is not valid for the requested type",
        })
    }
}
impl std::error::Error for TransferError {}
impl From<TransferError> for Error {
    fn from(e: TransferError) -> Self {
        Self::TransferFailed(e)
    }
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "capture I/O: {e}"),
            Self::Wayland(e) => write!(f, "Wayland connection: {e}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::NoOutputs => f.write_str("no usable outputs"),
            Self::OutputGone => f.write_str("output no longer belongs to this connection"),
            Self::LayoutChanged => f.write_str("output layout changed during capture; retry"),
            Self::UnsupportedFormat => f.write_str("no supported capture buffer format"),
            Self::InvalidDimensions => f.write_str("invalid capture dimensions or stride"),
            Self::LimitExceeded => f.write_str("capture exceeds configured resource limits"),
            Self::Timeout => f.write_str("capture deadline exceeded"),
            Self::Cancelled => f.write_str("capture cancelled"),
            Self::CaptureFailed(s) => write!(f, "capture failed: {s}"),
            Self::SessionStopped => f.write_str("capture session stopped"),
            Self::InvalidSerial => f.write_str("no valid input serial for this seat"),
            Self::NotReady => f.write_str("drag-and-drop session not ready"),
            Self::UnknownTarget => f.write_str("unknown drop target"),
            Self::UnknownTransfer => f.write_str("unknown or already answered transfer"),
            Self::InvalidInput(s) => write!(f, "invalid input: {s}"),
            Self::TransferFailed(e) => write!(f, "drag-and-drop transfer failed: {e}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::TransferFailed(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn transfer_errors_preserve_a_typed_source() {
        let error = Error::from(TransferError::Malformed);
        assert!(matches!(
            error,
            Error::TransferFailed(TransferError::Malformed)
        ));
        assert_eq!(
            error.source().unwrap().downcast_ref::<TransferError>(),
            Some(&TransferError::Malformed)
        );
        assert!(
            error
                .to_string()
                .contains(&TransferError::Malformed.to_string())
        );
    }
}
