//! Caller-armed wp_presentation feedback for commits this host issues.
//! Every armed or outstanding request holds one slot of the host event cap.
use crate::{
    Error, Result,
    window::{Host, WindowEvent, WindowId, state::WindowState},
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_registry::WlRegistry, wl_surface::WlSurface},
};
use wayland_protocols::wp::presentation_time::client::{
    wp_presentation::{self, WpPresentation},
    wp_presentation_feedback::{self, WpPresentationFeedback},
};

/// Armed plus outstanding requests per window. With at most 16 windows per
/// host, presentation never holds more than 128 event slots.
const MAX_WINDOW: usize = 8;

/// What became of one consumed presentation request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresentationOutcome {
    /// The content update turned into light at the reported time.
    Presented(PresentationTiming),
    /// The content update was never shown: superseded, or its surface destroyed.
    Discarded,
    /// No feedback can be given: `wp_presentation` was gone at commit time, or the
    /// compositor sent malformed timing. Never replaced with a guessed timestamp.
    Unavailable,
}

/// Compositor-reported timing of one presented content update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PresentationTiming {
    /// Raw `clock_id` (a POSIX `clockid_t`, usually `CLOCK_MONOTONIC`) for `timestamp`.
    pub clock: u32,
    /// Time since the epoch of `clock` when the update first turned into light.
    pub timestamp: Duration,
    /// Predicted time from `timestamp` to the next refresh; `None` when the
    /// compositor cannot predict it.
    pub refresh: Option<Duration>,
    /// The output's vertical retrace counter (MSC); zero when it has none.
    pub sequence: u64,
    /// How the presentation was done.
    pub flags: PresentationFlags,
}

/// `wp_presentation_feedback.kind` bits, including bits unknown to this version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentationFlags(u32);

impl PresentationFlags {
    /// Synchronized to the vertical retrace, without tearing.
    pub fn vsync(self) -> bool {
        self.0 & 0x1 != 0
    }
    /// The display hardware provided the timestamp.
    pub fn hw_clock(self) -> bool {
        self.0 & 0x2 != 0
    }
    /// The display hardware signalled that it started using the content.
    pub fn hw_completion(self) -> bool {
        self.0 & 0x4 != 0
    }
    /// The buffer reached the display hardware without a copy.
    pub fn zero_copy(self) -> bool {
        self.0 & 0x8 != 0
    }
    /// The raw bitfield as sent by the compositor.
    pub fn bits(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::window) struct FeedbackToken {
    window: WindowId,
    tag: u64,
}

#[derive(Default)]
struct Requests {
    armed: Option<u64>,
    outstanding: usize,
}

impl Requests {
    fn held(&self) -> usize {
        usize::from(self.armed.is_some()) + self.outstanding
    }
}

#[derive(Default)]
pub(in crate::window) struct Presentation {
    manager: Option<(u32, WpPresentation)>,
    // The presentation clock does not change during the connection's lifetime.
    clock: Option<u32>,
    windows: BTreeMap<WindowId, Requests>,
    reserved: usize,
}

impl Presentation {
    /// Bind when advertised and none is bound, also after discovery.
    pub(in crate::window) fn bind(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if interface == "wp_presentation" && version != 0 && self.manager.is_none() {
            self.manager = Some((name, registry.bind(name, version.min(2), qh, ())));
        }
    }

    /// Destroy a removed manager. Feedback objects it created keep delivering.
    pub(in crate::window) fn remove(&mut self, name: u32) {
        if self
            .manager
            .as_ref()
            .is_some_and(|(bound, _)| *bound == name)
        {
            self.destroy();
        }
    }

    /// Event slots held by armed and outstanding requests.
    pub(in crate::window) fn reserved(&self) -> usize {
        self.reserved
    }

    /// Arm or replace the request for this window's next commit. `room` says
    /// whether the host event cap has a slot for a new reservation.
    fn arm(&mut self, window: WindowId, tag: u64, room: bool) -> Result<()> {
        if self.manager.is_none() {
            return Err(Error::Unsupported("wp_presentation"));
        }
        match self.windows.get_mut(&window) {
            Some(requests) if requests.armed.is_some() => {
                requests.armed = Some(tag);
                return Ok(());
            }
            Some(requests) if requests.held() >= MAX_WINDOW => {
                return Err(Error::LimitExceeded);
            }
            _ => {}
        }
        if !room {
            return Err(Error::LimitExceeded);
        }
        self.windows.entry(window).or_default().armed = Some(tag);
        self.reserved += 1;
        Ok(())
    }

    /// Attach the armed request to the commit that follows immediately. Without a
    /// manager its reserved slot carries `Unavailable` instead.
    pub(in crate::window) fn consume(
        &mut self,
        window: WindowId,
        surface: &WlSurface,
        qh: &QueueHandle<WindowState>,
        events: &mut VecDeque<WindowEvent>,
    ) {
        let Some(requests) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tag) = requests.armed.take() else {
            return;
        };
        if let Some((_, manager)) = &self.manager {
            manager.feedback(surface, qh, FeedbackToken { window, tag });
            requests.outstanding += 1;
            return;
        }
        self.reserved -= 1;
        self.forget_empty(window);
        events.push_back(WindowEvent::Presentation {
            window,
            tag,
            outcome: PresentationOutcome::Unavailable,
        });
    }

    /// Turn an answer into its event, releasing its slot. Answers for closed
    /// windows have no slot and produce nothing.
    fn answered(
        &mut self,
        token: FeedbackToken,
        outcome: PresentationOutcome,
    ) -> Option<WindowEvent> {
        let requests = self.windows.get_mut(&token.window)?;
        if requests.outstanding == 0 {
            return None;
        }
        requests.outstanding -= 1;
        self.reserved -= 1;
        self.forget_empty(token.window);
        Some(WindowEvent::Presentation {
            window: token.window,
            tag: token.tag,
            outcome,
        })
    }

    /// Release a closed window's slots. Its feedback objects stay until the
    /// compositor's destructor event; the interface has no destroy request.
    pub(in crate::window) fn close_window(&mut self, window: WindowId) {
        if let Some(requests) = self.windows.remove(&window) {
            self.reserved -= requests.held();
        }
    }

    pub(in crate::window) fn clear(&mut self) {
        self.windows.clear();
        self.reserved = 0;
        self.destroy();
    }

    fn forget_empty(&mut self, window: WindowId) {
        if self.windows.get(&window).is_some_and(|r| r.held() == 0) {
            self.windows.remove(&window);
        }
    }

    fn destroy(&mut self) {
        if let Some((_, manager)) = self.manager.take() {
            manager.destroy();
        }
    }

    fn timing(&self, raw: Presented) -> PresentationOutcome {
        let Some(clock) = self.clock else {
            return PresentationOutcome::Unavailable;
        };
        if raw.nanoseconds > 999_999_999 {
            return PresentationOutcome::Unavailable;
        }
        let seconds = (u64::from(raw.seconds.0) << 32) | u64::from(raw.seconds.1);
        PresentationOutcome::Presented(PresentationTiming {
            clock,
            // Nanoseconds below one second cannot carry into the seconds.
            timestamp: Duration::new(seconds, raw.nanoseconds),
            refresh: (raw.refresh != 0).then(|| Duration::from_nanos(raw.refresh.into())),
            sequence: (u64::from(raw.sequence.0) << 32) | u64::from(raw.sequence.1),
            flags: PresentationFlags(raw.flags),
        })
    }
}

struct Presented {
    seconds: (u32, u32),
    nanoseconds: u32,
    refresh: u32,
    sequence: (u32, u32),
    flags: u32,
}

impl Host {
    /// Ask for presentation feedback on this window's next content commit.
    ///
    /// The tag is consumed by the commit of a [`Host::present_cpu`] or
    /// [`Host::present_cpu_damaged`] call that returns `Presented`, or by
    /// `NativeFrame::stage` on the native path (feature `owned-native-system`).
    /// Results that commit nothing keep it armed. Arming again before that commit replaces the
    /// tag; a replaced tag produces no event. Every consumed tag produces exactly
    /// one [`WindowEvent::Presentation`] unless the window closes or the host stops
    /// first. Without arming, commits carry no feedback.
    ///
    /// At most eight armed and unanswered requests per window, each holding one
    /// slot of the host event cap. More, or a full event cap, return
    /// `LimitExceeded`; replacing an armed tag never does. Without `wp_presentation` (absent, or
    /// removed by the compositor) this returns `Unsupported`. A retired or foreign
    /// window returns `InvalidInput`. Nothing changes on error.
    pub fn request_presentation(&mut self, window: WindowId, tag: u64) -> Result<()> {
        self.coordinator
            .mutate(|state| state.request_presentation(window, tag))
    }
}

impl WindowState {
    pub(crate) fn request_presentation(&mut self, window: WindowId, tag: u64) -> Result<()> {
        self.ensure_active()?;
        self.record(window)?;
        let room = self.event_room(1);
        self.presentation.arm(window, tag, room)
    }
}

impl Dispatch<WpPresentation, ()> for WindowState {
    fn event(
        state: &mut Self,
        _: &WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            state.presentation.clock = Some(clk_id);
        }
    }
}

impl Dispatch<WpPresentationFeedback, FeedbackToken> for WindowState {
    fn event(
        state: &mut Self,
        _: &WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        token: &FeedbackToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let outcome = match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                seq_hi,
                seq_lo,
                flags,
            } => state.presentation.timing(Presented {
                seconds: (tv_sec_hi, tv_sec_lo),
                nanoseconds: tv_nsec,
                refresh,
                sequence: (seq_hi, seq_lo),
                flags: match flags {
                    WEnum::Value(kind) => kind.bits(),
                    WEnum::Unknown(bits) => bits,
                },
            }),
            wp_presentation_feedback::Event::Discarded => PresentationOutcome::Discarded,
            // No public output identity exists to report the sync output with.
            _ => return,
        };
        // The answer used its reserved slot, so this push cannot exceed the cap.
        if let Some(event) = state.presentation.answered(*token, outcome)
            && state.active()
        {
            state.events.push_back(event);
        }
    }
}
