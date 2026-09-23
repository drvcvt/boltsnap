#![cfg(feature = "dnd")]
mod support;
use libway::Display;
use support::*;
use wayland_client::{
    Dispatch, delegate_noop,
    protocol::{wl_registry, wl_shm},
};

#[derive(Default)]
struct Globals(Vec<String>);
impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        s: &mut Self,
        _: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { interface, .. } = e {
            s.0.push(interface);
        }
    }
}
delegate_noop!(Globals: ignore wl_shm::WlShm);

fn globals_of(display: &Display) -> Vec<String> {
    let conn = display.connection().clone();
    let mut queue = conn.new_event_queue::<Globals>();
    let _registry = conn.display().get_registry(&queue.handle(), ());
    let mut state = Globals::default();
    queue.roundtrip(&mut state).unwrap();
    state.0
}

#[test]
fn owned_and_shared_displays_see_the_same_globals() {
    let (_server, socket) = Server::start(Config::default());
    let owned = Display::from_socket(socket).unwrap();
    assert!(owned.is_owned());
    assert!(globals_of(&owned).iter().any(|g| g == "wl_shm"));
    let shared = Display::from_connection(owned.connection().clone());
    assert!(!shared.is_owned());
    assert!(globals_of(&shared).iter().any(|g| g == "wl_shm"));
}

#[cfg(feature = "foreign-display")]
#[test]
fn guest_display_reads_through_the_owner() {
    let (_server, socket) = Server::start(Config::default());
    let owner = wayland_client::Connection::from_socket(socket).unwrap();
    let ptr = std::ptr::NonNull::new(owner.backend().display_ptr().cast()).unwrap();
    let guest = unsafe { Display::from_raw(ptr) };
    assert!(!guest.is_owned());
    assert!(globals_of(&guest).iter().any(|g| g == "wl_shm"));
    // The owner's own queue must still work after the guest read from the socket.
    let owned = Display::from_connection(owner.clone());
    assert!(globals_of(&owned).iter().any(|g| g == "wl_shm"));
    drop(guest);
    drop(owner);
}
#[cfg(feature = "foreign-display")]
#[test]
fn raw_surface_handles_match_wayland_rs_handles() {
    use wayland_client::{Proxy, protocol::wl_surface};
    let (_server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let surface = client::surface(&display);
    let ptr = std::ptr::NonNull::new(surface.id().as_ptr().cast()).unwrap();
    let raw = unsafe { libway::SurfaceHandle::from_raw(ptr) }.unwrap();
    assert_eq!(raw, libway::SurfaceHandle::from_surface(&surface));
    let guest = unsafe {
        Display::from_raw(
            std::ptr::NonNull::new(display.connection().backend().display_ptr().cast()).unwrap(),
        )
    };
    let dnd = libway::dnd::Dnd::new(&guest, Default::default()).unwrap();
    let target = dnd
        .register_target(
            &raw,
            libway::dnd::TargetSpec {
                rect: libway::dnd::LocalRect {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.,
                },
                accepts: vec![],
                actions: libway::dnd::Actions {
                    copy: true,
                    move_: false,
                    ask: false,
                },
                preferred: libway::dnd::Action::None,
                priority: 0,
                enabled: true,
            },
        )
        .unwrap();
    dnd.unregister_target(target).unwrap();
    assert!(dnd.wait_ready(std::time::Duration::ZERO).is_err());
    drop(dnd);
    drop(guest);
    let _: &wl_surface::WlSurface = &surface;
}

#[test]
fn safe_handle_hash_and_equality_survive_connection_drop() {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };
    let (_server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let surface = client::surface(&display);
    let handle = libway::SurfaceHandle::from_surface(&surface);
    let clone = handle.clone();
    let hash = |handle: &libway::SurfaceHandle| {
        let mut h = DefaultHasher::new();
        handle.hash(&mut h);
        h.finish()
    };
    let before = hash(&handle);
    surface.destroy();
    drop(surface);
    drop(display);
    assert_eq!(handle, clone);
    assert_eq!(hash(&handle), before);
    assert_eq!(hash(&clone), before);
}
