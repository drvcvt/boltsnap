//! Scale/output callbacks update bounded owned state after the read is resolved.
use super::*;
use wayland_client::{
    Dispatch, delegate_noop,
    protocol::{wl_output, wl_surface},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };
        let Some(record) = state.windows.get_mut(id) else {
            return;
        };
        if scale == 0 {
            state.remember(&Error::InvalidDimensions);
            return;
        }
        record.scale.fractional = Some(scale);
        super::super::protocol::refresh_scale(state, *id);
    }
}

impl Dispatch<wl_surface::WlSurface, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        _: &WlSurface,
        event: wl_surface::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        let Some(record) = state.windows.get_mut(id) else {
            return;
        };
        match event {
            wl_surface::Event::PreferredBufferScale { factor } => {
                let Ok(factor) = u32::try_from(factor) else {
                    state.remember(&Error::InvalidDimensions);
                    return;
                };
                if factor == 0 {
                    state.remember(&Error::InvalidDimensions);
                    return;
                }
                record.scale.preferred = factor;
            }
            wl_surface::Event::Enter { output } => {
                if let Some(name) = state.outputs.name(&output) {
                    record.scale.entered.insert(name);
                }
            }
            wl_surface::Event::Leave { output } => {
                if let Some(name) = state.outputs.name(&output) {
                    record.scale.entered.remove(&name);
                }
            }
            // Preferred transforms are hints. Buffers remain normal orientation.
            _ => return,
        }
        super::super::protocol::refresh_scale(state, *id);
    }
}

impl Dispatch<wl_output::WlOutput, u32> for WindowState {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        let Some(record) = state.outputs.records.get_mut(name) else {
            return;
        };
        // Late callbacks from a removed output must not modify a reused global name.
        if &record.proxy != output {
            return;
        }
        match event {
            wl_output::Event::Scale { factor } => {
                let Ok(factor) = u32::try_from(factor) else {
                    state.remember(&Error::InvalidDimensions);
                    return;
                };
                if factor == 0 {
                    state.remember(&Error::InvalidDimensions);
                    return;
                }
                record.staged = factor;
            }
            wl_output::Event::Name { name } => record.identity.name = super::identity::text(name),
            wl_output::Event::Description { description } => {
                record.identity.description = super::identity::text(description);
            }
            wl_output::Event::Done => {
                if !record.identity.announced {
                    record.identity.announced = true;
                    let info = record.identity.info();
                    if !state.push(WindowEvent::OutputAdded(info)) {
                        return;
                    }
                }
                let Some(record) = state.outputs.records.get_mut(name) else {
                    return;
                };
                if record.current == record.staged {
                    return;
                }
                record.current = record.staged;
                let affected: Vec<_> = state
                    .windows
                    .iter()
                    .filter_map(|(id, record)| record.scale.entered.contains(name).then_some(*id))
                    .collect();
                for id in affected {
                    super::super::protocol::refresh_scale(state, id);
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(WindowState: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(WindowState: ignore wp_viewporter::WpViewporter);

impl Dispatch<wp_viewport::WpViewport, WindowId> for WindowState {
    fn event(
        _: &mut Self,
        _: &WpViewport,
        _: wp_viewport::Event,
        _: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
