//! Typed CPU buffer releases and frame callbacks retain no strong native owner.
use super::{FrameToken, slots::BufferToken};
use crate::window::{WindowEvent, state::WindowState};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
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

// wl_shm guarantees ARGB8888 and XRGB8888. An advertised ABGR8888 lets CPU
// presentation copy RGBA straight into new buffers. Pool has no events.
impl Dispatch<wl_shm::WlShm, ()> for WindowState {
    fn event(
        state: &mut Self,
        _: &wl_shm::WlShm,
        event: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format {
            format: WEnum::Value(format),
        } = event
        {
            state.output.shm_format(format);
        }
    }
}
delegate_noop!(WindowState: ignore wl_shm_pool::WlShmPool);
