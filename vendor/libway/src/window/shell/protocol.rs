//! Internal dispatch handlers retain owned values and invoke no application code.
use super::{
    configure::{SuggestedConfiguration, rescale},
    state::WindowState,
    types::{ConfigurationRevision, WindowEvent, WindowId},
};
use crate::Error;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_callback, wl_compositor, wl_registry},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

impl Dispatch<wl_registry::WlRegistry, ()> for WindowState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => registry_global(state, registry, (name, interface.as_str(), version), qh),
            wl_registry::Event::GlobalRemove { name } => {
                let required = state.globals.as_ref().is_some_and(|globals| {
                    name == globals.compositor_name || name == globals.shell_name
                }) || state.bootstrap.as_ref().is_some_and(|bootstrap| {
                    bootstrap
                        .compositor
                        .as_ref()
                        .is_some_and(|(bound, _)| name == *bound)
                        || bootstrap
                            .shell
                            .as_ref()
                            .is_some_and(|(bound, _)| name == *bound)
                });
                if required {
                    state.create_allowed = false;
                }
                if let Some(globals) = &state.globals {
                    globals.scaling.remove(name);
                    globals.decoration.remove(name);
                    globals.layer_shell.remove(name);
                    globals.attention.remove(name);
                }
                if let Some(bootstrap) = &state.bootstrap {
                    bootstrap.scaling.remove(name);
                    bootstrap.decoration.remove(name);
                    bootstrap.layer_shell.remove(name);
                    bootstrap.attention.remove(name);
                }
                state.text_input.remove_manager(name, &mut state.events);
                state.clipboard.remove_manager(name);
                state.input.remove_cursor_manager(name);
                state.presentation.remove(name);
                super::input::remove_seat(state, name);
                super::scale::remove_output(state, name);
            }
            _ => {}
        }
    }
}

fn registry_global(
    state: &mut WindowState,
    registry: &wl_registry::WlRegistry,
    global: (u32, &str, u32),
    qh: &QueueHandle<WindowState>,
) {
    state.text_input.bind_manager(registry, global, qh);
    state.clipboard.bind_manager(registry, global, qh);
    super::input::bind_seat(state, registry, global, qh);
    if state.active() {
        state.input.bind_cursor_manager(registry, global, qh);
        state.presentation.bind(registry, global, qh);
    }
    if !state.active() {
        return;
    }
    super::scale::bind_output(state, registry, global, qh);
    if !state.active() {
        return;
    }
    let Some(bootstrap) = &mut state.bootstrap else {
        return;
    };
    let (name, interface, version) = global;
    if version == 0 {
        return;
    }
    bootstrap.scaling.bind(registry, global, qh);
    bootstrap.decoration.bind(registry, global, qh);
    bootstrap.layer_shell.bind(registry, global, qh);
    bootstrap.attention.bind(registry, global, qh);
    match interface {
        "wl_compositor" if bootstrap.compositor.is_none() => {
            bootstrap.compositor = Some((name, registry.bind(name, version.min(6), qh, ())));
        }
        "xdg_wm_base" if bootstrap.shell.is_none() => {
            bootstrap.shell = Some((name, registry.bind(name, version.min(6), qh, ())));
        }
        "wl_shm" if bootstrap.shm.is_none() => {
            bootstrap.shm = Some(registry.bind(name, 1, qh, ()));
        }
        _ => {}
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WindowState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.discovered();
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WindowState {
    fn event(
        _: &mut Self,
        shell: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            shell.pong(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() || !state.windows.contains_key(id) {
            return;
        }
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => match SuggestedConfiguration::new(width, height, &states) {
                Ok(mut configuration) => {
                    if let Some(record) = state.windows.get_mut(id) {
                        // The decoration mode comes from its own object and persists.
                        configuration.states.decorations = record.staged.states.decorations;
                        record.staged = configuration;
                    }
                }
                Err(error) => state.remember(&error),
            },
            xdg_toplevel::Event::Close => {
                state.push(WindowEvent::CloseRequested { window: *id });
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, WindowId> for WindowState {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state.active() {
            return;
        }
        let xdg_surface::Event::Configure { serial } = event else {
            return;
        };
        configure_surface(state, *id, serial, |serial| surface.ack_configure(serial));
    }
}

/// Accept the staged suggestion of a complete configure sequence once, with the
/// role's own acknowledgement.
pub(super) fn configure_surface(
    state: &mut WindowState,
    id: WindowId,
    serial: u32,
    ack: impl FnOnce(u32),
) {
    let event_room = state.event_room(1);
    let Some(record) = state.windows.get_mut(&id) else {
        return;
    };
    if !event_room {
        state.remember(&Error::LimitExceeded);
        return;
    }
    let Some(next) = record.next_revision.checked_add(1) else {
        state.remember(&Error::InvalidInput(
            "window configuration revision exhausted",
        ));
        return;
    };
    let selected = record.scale.selected(&state.outputs).and_then(|scale| {
        record.staged.select(
            record.preferred,
            record.accepted,
            ConfigurationRevision(record.next_revision),
            scale,
        )
    });
    let configuration = match selected {
        Ok(configuration) => configuration,
        Err(error) => {
            state.remember(&error);
            return;
        }
    };
    // Coordinator dispatch is a resolved-read phase. Each complete sequence is
    // accepted once here; no serial ordering or application callback is involved.
    ack(serial);
    record.accepted = Some(configuration);
    record.next_revision = next;
    state.events.push_back(WindowEvent::Configured {
        window: id,
        configuration,
    });
}

delegate_noop!(WindowState: ignore wl_compositor::WlCompositor);

/// A scale update uses the last complete logical/suspension snapshot. It neither
/// accepts a staged xdg suggestion nor acknowledges or commits a surface sequence.
pub(super) fn refresh_scale(state: &mut WindowState, id: WindowId) {
    if !state.active() {
        return;
    }
    let event_room = state.event_room(1);
    let Some(record) = state.windows.get_mut(&id) else {
        return;
    };
    let Some(accepted) = record.accepted else {
        return;
    };
    let scale = match record.scale.selected(&state.outputs) {
        Ok(scale) => scale,
        Err(error) => {
            state.remember(&error);
            return;
        }
    };
    if scale == accepted.scale_120 {
        return;
    }
    if !event_room {
        state.remember(&Error::LimitExceeded);
        return;
    }
    let Some(next) = record.next_revision.checked_add(1) else {
        state.remember(&Error::InvalidInput(
            "window configuration revision exhausted",
        ));
        return;
    };
    let configuration = match rescale(accepted, scale, ConfigurationRevision(record.next_revision))
    {
        Ok(configuration) => configuration,
        Err(error) => {
            state.remember(&error);
            return;
        }
    };
    record.accepted = Some(configuration);
    record.next_revision = next;
    state.events.push_back(WindowEvent::Configured {
        window: id,
        configuration,
    });
}
