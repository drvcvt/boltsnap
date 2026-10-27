//! Why Clip cannot be pressed yet, from the replay buffer's status. Platform
//! neutral: the replay backend maps its status here, the toolbar says it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipState {
    /// A frozen frame of the buffer is there to clip from.
    Ready,
    /// No replay buffer runs (or nothing answers).
    Off,
    Starting,
    /// The buffer runs; its frozen frame is on the way.
    Loading,
    /// The buffer is freezing or saving for someone else.
    Busy,
    /// The buffer runs but gets no video.
    Stalled,
    /// Freezing the buffer failed.
    Failed,
}

impl ClipState {
    #[cfg(test)]
    pub const ALL: [ClipState; 7] = [
        ClipState::Ready,
        ClipState::Off,
        ClipState::Starting,
        ClipState::Loading,
        ClipState::Busy,
        ClipState::Stalled,
        ClipState::Failed,
    ];

    /// The short line under the toolbar while the pointer is on Clip.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            ClipState::Ready => None,
            ClipState::Off => Some("Replay is off"),
            ClipState::Starting => Some("Replay is starting"),
            ClipState::Loading => Some("Replay is loading"),
            ClipState::Busy => Some("Replay is busy"),
            ClipState::Stalled => Some("Replay has no video"),
            ClipState::Failed => Some("Replay could not load"),
        }
    }
}
