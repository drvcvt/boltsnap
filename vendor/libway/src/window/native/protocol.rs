//! Native driver pacing has a distinct token and no strong native ownership cycle.
use super::state::NativeFrameToken;
use crate::window::{WindowEvent, state::WindowState};
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_callback};

impl Dispatch<wl_callback::WlCallback, NativeFrameToken> for WindowState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        token: &NativeFrameToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. })
            && let Some(window) = state.native_frame_done(*token)
            && state.active()
        {
            state.push(WindowEvent::RedrawRequested { window });
        }
    }
}
