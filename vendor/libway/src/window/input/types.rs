//! Owned native input observations. Serial and time values may wrap independently.
use super::payload::Owned;
use crate::window::WindowId;

/// Connection-provenanced seat identity, never reused on this connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputSeatId {
    pub(crate) connection: u64,
    pub(crate) sequence: u64,
}

/// Immutable text keymap, including its one final NUL in `as_bytes`.
/// Each map is at most 1 MiB; all retained maps share an 8 MiB host budget.
/// Cloning shares the allocation and its charge, including after host shutdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap(pub(super) Owned<Box<[u8]>>);
impl Keymap {
    /// Validated UTF-8 text without the final NUL. This does not decode XKB.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.as_bytes()[..self.as_bytes().len() - 1])
            .expect("validated keymap text")
    }
    /// Validated bytes including exactly one final NUL.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0.value
    }
}

/// Held evdev codes observed on Enter, without synthesizing press actions.
/// At most 1024 distinct codes per payload and 8192 retained codes per host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldKeys(pub(super) Owned<Box<[u32]>>);
impl HeldKeys {
    /// Codes in their original native array order.
    pub fn as_slice(&self) -> &[u32] {
        &self.0.value
    }
}

/// Immutable UTF-8 seat name, at most 256 bytes with 2048 retained bytes per host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatName(pub(super) Owned<Box<str>>);
impl SeatName {
    /// Native name, without an interior NUL.
    pub fn as_str(&self) -> &str {
        &self.0.value
    }
}

/// One ordered pointer batch. Each record retains its own window identity.
/// At most 256 records per batch and 4096 staged/retained records per host.
/// Clones share both storage and its retained-record charge.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerFrame {
    pub(super) records: Owned<Vec<PointerObservation>>,
    pub(super) legacy: bool,
    pub(super) interrupted: bool,
}
impl PointerFrame {
    /// Original wire order without motion coalescing.
    pub fn records(&self) -> &[PointerObservation] {
        &self.records.value
    }
    /// True when retirement flushed a partial batch before its native Frame event.
    /// Its records precede cancellation/close and do not imply a native frame arrived.
    pub fn interrupted(&self) -> bool {
        self.interrupted
    }
    /// True for immediate single-record delivery from pointer versions below 5.
    pub fn legacy(&self) -> bool {
        self.legacy
    }
}

/// Seat provenance for one owned observation.
#[derive(Clone, Debug, PartialEq)]
pub struct InputEvent {
    /// Seat on the host's exclusive connection.
    pub seat: InputSeatId,
    /// Native facts, not application actions or committed text.
    pub observation: InputObservation,
}

/// Native seat, keyboard and pointer observations.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InputObservation {
    /// A fresh seat was bound, at protocol version 1 through 9.
    SeatAdded {
        /// Bound protocol version, capped at nine.
        version: u32,
    },
    /// Compositor-provided seat name.
    SeatName(SeatName),
    /// Original capability bits, including unknown bits. Touch is not bound.
    Capabilities {
        /// Original native capability bits, including unknown bits.
        bits: u32,
    },
    /// Native seat removed. Its identity cannot be reused.
    SeatRemoved,
    /// Raw keyboard fact, without layout decoding or local repetition.
    Keyboard(KeyboardObservation),
    /// Ordered pointer records, including explicit legacy delivery.
    PointerFrame(PointerFrame),
    /// Cancel active input before native/window ownership retires.
    Cancelled {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Device association to cancel.
        device: InputDevice,
        /// Cause of cancellation.
        reason: InputCancelReason,
    },
}

/// Device association being cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDevice {
    /// Keyboard focus and held/repeating keys.
    Keyboard,
    /// Pointer focus and active button/axis associations.
    Pointer,
}

/// Why a device association must be cancelled. Native Leave is reported separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputCancelReason {
    /// Native device capability disappeared.
    CapabilityLost,
    /// Seat global disappeared.
    SeatRemoved,
    /// Application explicitly retired the window.
    WindowClosed,
    /// Host stopped, including terminal failure.
    Shutdown,
}

/// Raw keyboard observations. `time_ms` is the native wrapping millisecond clock.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyboardObservation {
    /// None is the valid native no_keymap format and clears a downstream decoder.
    Keymap(Option<Keymap>),
    /// Held keys are a snapshot, never new key-down actions.
    Enter {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
        /// Already-held physical keys, without synthetic presses.
        held: HeldKeys,
    },
    /// Native keyboard focus left this window.
    Leave {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
    },
    /// Physical evdev transition; libway does not add the XKB keycode offset.
    Key {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
        /// Original wrapping native millisecond timestamp.
        time_ms: u32,
        /// Physical evdev code, without the XKB offset.
        physical: u32,
        /// True for a genuine native pressed transition.
        down: bool,
    },
    /// Native masks/group. Their semantic meanings depend on the current keymap.
    Modifiers {
        /// Native target identity, valid when this fact was received.
        window: Option<WindowId>,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
        /// Native depressed modifier mask.
        depressed: u32,
        /// Native latched modifier mask.
        latched: u32,
        /// Native locked modifier mask.
        locked: u32,
        /// Native keyboard layout group.
        group: u32,
    },
    /// Native repeat parameters. Zero rate disables repetition.
    RepeatInfo {
        /// Nonnegative repetitions per second; zero disables repeat.
        rate: u32,
        /// Nonnegative initial repeat delay in milliseconds.
        delay_ms: u32,
    },
}

/// Scroll axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerAxis {
    /// Vertical scroll.
    Vertical,
    /// Horizontal scroll.
    Horizontal,
}
/// Native scroll source. Unknown values are retained for future protocol extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisSource {
    /// Physical wheel.
    Wheel,
    /// Touch surface finger.
    Finger,
    /// Continuous source.
    Continuous,
    /// Wheel tilt.
    WheelTilt,
    /// Unrecognized numeric source.
    Unknown(u32),
}
/// Native relative axis direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisDirection {
    /// Same as physical motion.
    Identical,
    /// Inverted relative to physical motion.
    Inverted,
    /// Unrecognized numeric direction.
    Unknown(u32),
}

/// Ordered pointer facts. Positions and continuous axis values are f64 logical
/// surface coordinates, preserving Wayland's fixed-point values exactly.
/// `time_ms` is the native wrapping millisecond clock, not a local deadline.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PointerObservation {
    /// Pointer focus entered.
    Enter {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
        /// Logical surface-local horizontal coordinate.
        x: f64,
        /// Logical surface-local vertical coordinate.
        y: f64,
    },
    /// Pointer focus left.
    Leave {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
    },
    /// Motion within the focused surface.
    Motion {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native millisecond timestamp.
        time_ms: u32,
        /// Logical surface-local horizontal coordinate.
        x: f64,
        /// Logical surface-local vertical coordinate.
        y: f64,
    },
    /// Native button number and transition.
    Button {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native serial; no ordering is inferred.
        serial: u32,
        /// Original wrapping native millisecond timestamp.
        time_ms: u32,
        /// Original native button code.
        button: u32,
        /// True for a genuine native pressed transition.
        down: bool,
    },
    /// Continuous scroll distance in logical surface coordinates.
    Axis {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native millisecond timestamp.
        time_ms: u32,
        /// Native scroll axis.
        axis: PointerAxis,
        /// Native continuous distance or value120 wheel units, as named by the variant.
        value: f64,
    },
    /// Integer wheel steps from pointer versions 5 through 7.
    AxisDiscrete {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Native scroll axis.
        axis: PointerAxis,
        /// Native signed discrete wheel steps.
        steps: i32,
    },
    /// Wheel units where 120 is one logical wheel detent.
    AxisValue120 {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Native scroll axis.
        axis: PointerAxis,
        /// Native continuous distance or value120 wheel units, as named by the variant.
        value: i32,
    },
    /// End of a continuous axis sequence.
    AxisStop {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Original wrapping native millisecond timestamp.
        time_ms: u32,
        /// Native scroll axis.
        axis: PointerAxis,
    },
    /// Source in its native position within the frame.
    AxisSource {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Native source, retaining unknown numeric values.
        source: AxisSource,
    },
    /// Relative direction in its native position within the frame.
    AxisRelativeDirection {
        /// Native target identity, valid when this fact was received.
        window: WindowId,
        /// Native scroll axis.
        axis: PointerAxis,
        /// Native relative direction, retaining unknown numeric values.
        direction: AxisDirection,
    },
}
impl PointerObservation {
    pub(super) fn window(&self) -> WindowId {
        match self {
            Self::Enter { window, .. }
            | Self::Leave { window, .. }
            | Self::Motion { window, .. }
            | Self::Button { window, .. }
            | Self::Axis { window, .. }
            | Self::AxisDiscrete { window, .. }
            | Self::AxisValue120 { window, .. }
            | Self::AxisStop { window, .. }
            | Self::AxisSource { window, .. }
            | Self::AxisRelativeDirection { window, .. } => *window,
        }
    }
}
