//! Only the role protocol under test is implemented; requests are recorded literally.
use super::{Native, Options, Request, State};
use wayland_protocols::wp::{
    fractional_scale::v1::server::wp_fractional_scale_manager_v1, viewporter::server::wp_viewporter,
};
use wayland_protocols::xdg::shell::server::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::GlobalId,
    protocol::{wl_compositor, wl_output, wl_shm, wl_surface},
};

pub(super) mod popup;

/// Returns the output globals, indexed like the peer's outputs.
pub(super) fn advertise(dh: &DisplayHandle, options: Options) -> Vec<Option<GlobalId>> {
    if options.compositor {
        dh.create_global::<State, wl_compositor::WlCompositor, _>(options.surface_version, ());
    }
    if options.shell {
        dh.create_global::<State, xdg_wm_base::XdgWmBase, _>(options.shell_version, ());
    }
    if options.shm {
        dh.create_global::<State, wl_shm::WlShm, _>(1, ());
    }
    if options.fractional {
        dh.create_global::<State, wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _>(
            1,
            (),
        );
    }
    if options.viewport {
        dh.create_global::<State, wp_viewporter::WpViewporter, _>(1, ());
    }
    let mut outputs = Vec::new();
    if options.outputs {
        for index in 0..options.output_count {
            outputs.push(Some(
                dh.create_global::<State, wl_output::WlOutput, _>(4, index),
            ));
        }
    }
    outputs
}

impl GlobalDispatch<wl_compositor::WlCompositor, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_compositor::WlCompositor>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
        state.ledger.push(Request::BindCompositor);
    }
}

impl GlobalDispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<xdg_wm_base::XdgWmBase>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
        state.ledger.push(Request::BindShell);
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                let index = state.windows.len();
                let surface = init.init(id, index);
                state.windows.push(Native {
                    surface,
                    role: None,
                    toplevel: None,
                    attached: false,
                    fractional: None,
                    viewport: None,
                    decoration: None,
                    layer: None,
                    popup: None,
                });
                state.ledger.push(Request::Surface(index));
            }
            wl_compositor::Request::CreateRegion { id } => {
                let index = state.layer.regions;
                state.layer.regions += 1;
                init.init(id, index);
                state.ledger.push(Request::Region(index));
            }
            request => panic!("unexpected compositor request: {request:?}"),
        }
    }
}

impl Dispatch<wl_surface::WlSurface, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        surface: &wl_surface::WlSurface,
        request: wl_surface::Request,
        index: &usize,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        assert_eq!(state.windows[*index].surface.id(), surface.id());
        match request {
            wl_surface::Request::Commit => {
                state
                    .ledger
                    .push(Request::Commit(*index, state.windows[*index].attached));
            }
            wl_surface::Request::Attach { buffer, .. } => {
                let attached = buffer.is_some();
                state.windows[*index].attached = attached;
                state.ledger.push(Request::Attach(*index, attached));
                state.ledger.push(Request::AttachedBuffer(
                    *index,
                    buffer.as_ref().map(|b| *b.data::<usize>().unwrap()),
                ));
            }
            wl_surface::Request::Frame { callback } => {
                let callback = init.init(callback, ());
                state
                    .ledger
                    .push(Request::Frame(*index, state.frames.len()));
                state.frames.push((*index, callback));
            }
            wl_surface::Request::DamageBuffer {
                x,
                y,
                width,
                height,
            }
            | wl_surface::Request::Damage {
                x,
                y,
                width,
                height,
            } => state
                .ledger
                .push(Request::Damage(*index, x, y, width, height)),
            wl_surface::Request::SetBufferScale { scale } => {
                state.ledger.push(Request::BufferScale(*index, scale))
            }
            wl_surface::Request::SetInputRegion { region } => {
                state.ledger.push(Request::InputRegion(
                    *index,
                    region.map(|region| *region.data::<usize>().unwrap()),
                ))
            }
            wl_surface::Request::SetOpaqueRegion { region } => {
                state.ledger.push(Request::OpaqueRegion(
                    *index,
                    region.map(|region| *region.data::<usize>().unwrap()),
                ))
            }
            wl_surface::Request::Destroy => state.ledger.push(Request::DestroySurface(*index)),
            request => panic!("unexpected surface request: {request:?}"),
        }
    }
}

macro_rules! unit_global {
    ($proxy:ty, $request:expr) => {
        impl GlobalDispatch<$proxy, ()> for State {
            fn bind(
                state: &mut Self,
                _: &DisplayHandle,
                _: &Client,
                resource: New<$proxy>,
                _: &(),
                init: &mut DataInit<'_, Self>,
            ) {
                init.init(resource, ());
                state.ledger.push($request);
            }
        }
    };
}
unit_global!(
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    Request::BindFractional
);
unit_global!(wp_viewporter::WpViewporter, Request::BindViewport);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &xdg_wm_base::XdgWmBase,
        request: xdg_wm_base::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_wm_base::Request::GetXdgSurface { id, surface } => {
                let index = *surface.data::<usize>().unwrap();
                state.windows[index].role = Some(init.init(id, index));
                state.ledger.push(Request::Role(index));
            }
            xdg_wm_base::Request::CreatePositioner { id } => {
                popup::create_positioner(state, id, init)
            }
            xdg_wm_base::Request::Destroy => state.ledger.push(Request::DestroyShell),
            xdg_wm_base::Request::Pong { serial } => state.ledger.push(Request::Pong(serial)),
            request => panic!("unexpected shell request: {request:?}"),
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &xdg_surface::XdgSurface,
        request: xdg_surface::Request,
        index: &usize,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_surface::Request::GetToplevel { id } => {
                state.windows[*index].toplevel = Some(init.init(id, *index));
                state.ledger.push(Request::Toplevel(*index));
            }
            xdg_surface::Request::GetPopup {
                id,
                parent,
                positioner,
            } => popup::get_popup(state, *index, (id, parent, positioner), init),
            xdg_surface::Request::AckConfigure { serial } => {
                state.ledger.push(Request::Ack(*index, serial));
            }
            xdg_surface::Request::Destroy => state.ledger.push(Request::DestroyRole(*index)),
            request => panic!("unexpected role request: {request:?}"),
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &xdg_toplevel::XdgToplevel,
        request: xdg_toplevel::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_toplevel::Request::SetTitle { title } => {
                state.ledger.push(Request::Title(*index, title))
            }
            xdg_toplevel::Request::SetAppId { app_id } => {
                state.ledger.push(Request::AppId(*index, app_id))
            }
            xdg_toplevel::Request::Destroy => state.ledger.push(Request::DestroyToplevel(*index)),
            xdg_toplevel::Request::Move { seat, serial } => {
                let seat = *seat.data::<usize>().unwrap();
                state.ledger.push(Request::Move(*index, seat, serial));
            }
            xdg_toplevel::Request::Resize {
                seat,
                serial,
                edges,
            } => {
                let seat = *seat.data::<usize>().unwrap();
                let edges: u32 = edges.into();
                state
                    .ledger
                    .push(Request::Resize(*index, seat, serial, edges));
            }
            xdg_toplevel::Request::ShowWindowMenu { seat, serial, x, y } => {
                let seat = *seat.data::<usize>().unwrap();
                state
                    .ledger
                    .push(Request::ShowWindowMenu(*index, seat, serial, x, y));
            }
            xdg_toplevel::Request::SetMaximized => state.ledger.push(Request::SetMaximized(*index)),
            xdg_toplevel::Request::UnsetMaximized => {
                state.ledger.push(Request::UnsetMaximized(*index))
            }
            xdg_toplevel::Request::SetMinimized => state.ledger.push(Request::SetMinimized(*index)),
            xdg_toplevel::Request::SetFullscreen { output } => {
                assert!(output.is_none(), "no output preference is expressed");
                state.ledger.push(Request::SetFullscreen(*index));
            }
            xdg_toplevel::Request::UnsetFullscreen => {
                state.ledger.push(Request::UnsetFullscreen(*index))
            }
            xdg_toplevel::Request::SetMinSize { width, height } => {
                state.ledger.push(Request::MinSize(*index, width, height))
            }
            xdg_toplevel::Request::SetMaxSize { width, height } => {
                state.ledger.push(Request::MaxSize(*index, width, height))
            }
            request => panic!("unexpected toplevel request: {request:?}"),
        }
    }
}
