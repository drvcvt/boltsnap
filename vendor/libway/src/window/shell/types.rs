//! Owned application values without Wayland proxy or connection access.

/// A window identity with connection provenance and no reuse during that connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId {
    pub(crate) connection: u64,
    pub(crate) sequence: u64,
}

/// A checked local receive-order revision, independent of Wayland serial values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigurationRevision(pub(crate) u64);

/// Initial client preferences. Zero suggested native dimensions use these values
/// until the first configuration, then reuse the last accepted axis independently.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    /// Positive preferred logical width, representable by the native protocol.
    pub preferred_width: u32,
    /// Positive preferred logical height, representable by the native protocol.
    pub preferred_height: u32,
    /// Window title, at most 1024 UTF-8 bytes without an interior NUL.
    pub title: String,
    /// Desktop application identity, at most 256 UTF-8 bytes without an interior NUL.
    pub app_id: String,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            preferred_width: 640,
            preferred_height: 480,
            title: "libway".into(),
            app_id: "libway".into(),
        }
    }
}

/// Who draws the window frame, as negotiated through xdg-decoration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decorations {
    /// The compositor announced server-side decorations. The application draws none.
    Server,
    /// The application draws its own decorations. This is also the state without
    /// the xdg-decoration global, or when the compositor chose client-side mode.
    #[default]
    Client,
}

/// Edges the compositor currently tiles the window against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Tiled {
    /// Tiled against the left edge.
    pub left: bool,
    /// Tiled against the right edge.
    pub right: bool,
    /// Tiled against the top edge.
    pub top: bool,
    /// Tiled against the bottom edge.
    pub bottom: bool,
}

/// One validated logical/pixel configuration. Fractional toplevel extents round
/// to nearest pixels, with exact half values upward. This grants no presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Configuration {
    /// Positive selected width in logical window coordinates.
    pub width: u32,
    /// Positive selected height in logical window coordinates.
    pub height: u32,
    /// Positive physical buffer width, representable by the native protocol.
    pub pixel_width: u32,
    /// Positive physical buffer height, representable by the native protocol.
    pub pixel_height: u32,
    /// Positive selected scale numerator with a fixed denominator of 120.
    pub scale_120: u32,
    /// The last complete xdg configuration forbids presentation while suspended.
    pub suspended: bool,
    /// Decoration mode in effect for this configuration. [`Decorations::Server`]
    /// only when the compositor's last decoration configure said server-side.
    pub decorations: Decorations,
    /// The compositor reports the window as maximized.
    pub maximized: bool,
    /// The compositor reports the window as fullscreen.
    pub fullscreen: bool,
    /// The compositor reports the window as activated (focused or active).
    pub activated: bool,
    /// Edges the window is tiled against. All false when floating.
    pub tiled: Tiled,
    /// Connection-local receive-order revision for this window.
    pub revision: ConfigurationRevision,
}

/// Retained owned events. Compositor close requests do not close the window.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WindowEvent {
    /// Initial discovery completed with the required window protocols.
    Ready,
    /// Coalesced worker notification. Service published results after every pump;
    /// collecting this token permits another notification to be admitted.
    Wake,
    /// Owned native input with seat provenance and raw wrapping serial/time values.
    Input(super::InputEvent),
    /// Owned ordinary clipboard metadata, source retirement or terminal bytes.
    Clipboard(super::ClipboardEvent),
    /// Ordered text-input readiness, complete transaction or exact retirement.
    Ime(super::ImeEvent),
    /// Native pacing or released buffer credits permit another dirty-frame attempt.
    RedrawRequested {
        /// The live window whose pending content may be submitted again.
        window: WindowId,
    },
    /// A complete configure sequence was acknowledged once.
    Configured {
        /// The affected window.
        window: WindowId,
        /// Selected logical/pixel extent, scale, suspension and revision.
        configuration: Configuration,
    },
    /// The compositor asks the application to decide its close policy.
    CloseRequested {
        /// The affected window remains usable until explicitly closed.
        window: WindowId,
    },
    /// Feedback for one request consumed by a commit, see
    /// [`Host::request_presentation`](super::Host::request_presentation).
    Presentation {
        /// The window whose commit carried the request.
        window: WindowId,
        /// The caller's tag from the request.
        tag: u64,
        /// Presented with timing, discarded, or unavailable.
        outcome: super::PresentationOutcome,
    },
    /// An output finished its first description. See [`Host::outputs`](super::Host::outputs).
    OutputAdded(super::OutputInfo),
    /// The output's global was removed. Layer surfaces on it normally receive
    /// `closed` from the compositor, which libway reports as [`Self::Closed`];
    /// libway fabricates no close of its own.
    OutputRemoved(super::OutputId),
    /// Logical identity retired; retained leases may still own native resources.
    Closed {
        /// The retired identity, which will never identify another window.
        window: WindowId,
    },
}

/// Concrete cross-thread window requests admitted through the host's bounded proxy.
#[derive(Debug)]
pub enum WindowCommand {
    /// Retire a window's logical identity through the owner thread.
    Close(WindowId),
    /// Change a live window's title. Validation occurs before issuing its request.
    SetTitle(WindowId, String),
}

impl WindowCommand {
    /// The identity targeted by this command.
    pub fn window(&self) -> WindowId {
        match self {
            Self::Close(id) | Self::SetTitle(id, _) => *id,
        }
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        match self {
            Self::Close(_) => 0,
            Self::SetTitle(_, title) => title.capacity(),
        }
    }
}
