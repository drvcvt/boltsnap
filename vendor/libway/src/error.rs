use std::{fmt, io};

/// Capture errors are explicit; an unsupported operation never returns empty success.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io(io::Error),
    Wayland(String),
    Unsupported(&'static str),
    NoOutputs,
    OutputGone,
    LayoutChanged,
    UnsupportedFormat,
    InvalidDimensions,
    LimitExceeded,
    Timeout,
    Cancelled,
    CaptureFailed(String),
    SessionStopped,
}
pub type Result<T> = std::result::Result<T, Error>;
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
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
