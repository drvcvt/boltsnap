//! Typed CPU buffer releases and frame callbacks retain no strong native owner.
use super::{FrameToken, slots::BufferToken};
use crate::window::{WindowEvent, state::WindowState};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_buffer, wl_callback, wl_shm, wl_shm_pool},
};

impl Dispatch<wl_buffer::WlBuffer, BufferToken> for WindowState {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        token: &BufferToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            let ready = state.output.release(*token);
            if state.active() {
                for window in ready {
                    if !state.push(WindowEvent::RedrawRequested { window }) {
                        break;
                    }
                }
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, FrameToken> for WindowState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        token: &FrameToken,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. })
            && let Some(window) = state.output.frame(*token)
            && state.active()
        {
            state.push(WindowEvent::RedrawRequested { window });
        }
    }
}

// wl_shm guarantees ARGB8888 and XRGB8888; additional advertised formats do
// not change this deliberately fixed presentation format. Pool has no events.
delegate_noop!(WindowState: ignore wl_shm::WlShm);
delegate_noop!(WindowState: ignore wl_shm_pool::WlShmPool);
