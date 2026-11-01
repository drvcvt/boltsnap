//! Opt-in wp_presentation peer. Feedback objects are indexed in request order.
use super::{Options, Request, State};
use wayland_protocols::wp::presentation_time::server::{
    wp_presentation::{self, WpPresentation},
    wp_presentation_feedback::{self, Kind, WpPresentationFeedback},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::GlobalId,
};

#[derive(Default)]
pub(in crate::window::tests) struct PresentationState {
    global: Option<GlobalId>,
    version: u32,
    clock: Option<u32>,
    feedbacks: Vec<Option<WpPresentationFeedback>>,
}

#[derive(Debug)]
pub(in crate::window::tests) enum PresentationWireEvent {
    /// Feedback index and the raw `presented` arguments in wire order:
    /// tv_sec_hi, tv_sec_lo, tv_nsec, refresh, seq_hi, seq_lo, flags.
    Presented(usize, [u32; 7]),
    Discarded(usize),
    Remove,
    Advertise,
}

pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut PresentationState) {
    state.version = options.presentation_version;
    state.clock = options.presentation_clock;
    if options.presentation {
        state.global = Some(dh.create_global::<State, WpPresentation, _>(state.version, ()));
    }
}

pub(super) fn emit(event: PresentationWireEvent, state: &mut State, dh: &DisplayHandle) {
    let presentation = &mut state.presentation;
    match event {
        PresentationWireEvent::Presented(index, raw) => {
            let [sec_hi, sec_lo, nsec, refresh, seq_hi, seq_lo, flags] = raw;
            presentation.feedbacks[index].take().unwrap().presented(
                sec_hi,
                sec_lo,
                nsec,
                refresh,
                seq_hi,
                seq_lo,
                Kind::from_bits_retain(flags),
            );
        }
        PresentationWireEvent::Discarded(index) => {
            presentation.feedbacks[index].take().unwrap().discarded();
        }
        PresentationWireEvent::Remove => {
            dh.remove_global::<State>(presentation.global.take().unwrap());
        }
        PresentationWireEvent::Advertise => {
            presentation.global =
                Some(dh.create_global::<State, WpPresentation, _>(presentation.version, ()));
        }
    }
}

impl GlobalDispatch<WpPresentation, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WpPresentation>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let presentation = init.init(resource, ());
        state
            .ledger
            .push(Request::BindPresentation(presentation.version()));
        if let Some(clock) = state.presentation.clock {
            presentation.clock_id(clock);
        }
    }
}

impl Dispatch<WpPresentation, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &WpPresentation,
        request: wp_presentation::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_presentation::Request::Feedback { surface, callback } => {
                let window = *surface.data::<usize>().unwrap();
                let index = state.presentation.feedbacks.len();
                state
                    .presentation
                    .feedbacks
                    .push(Some(init.init(callback, ())));
                state.ledger.push(Request::Feedback(window, index));
            }
            wp_presentation::Request::Destroy => state.ledger.push(Request::DestroyPresentation),
            request => panic!("unexpected presentation request: {request:?}"),
        }
    }
}

impl Dispatch<WpPresentationFeedback, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WpPresentationFeedback,
        request: wp_presentation_feedback::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        panic!("unexpected presentation feedback request: {request:?}");
    }
}
