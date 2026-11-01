//! `xdg_popup` against literal wire traces (R6): shared fixtures and helpers
//! for creation, the grab rules, the close cascade and placement.
use super::*;
use crate::window::tests::peer::{
    InputWireEvent as Input, LayerWireEvent as LayerWire, PopupWireEvent as Wire,
};

mod cascade;
mod creation;
mod grab;
mod placement;

const BUTTON_LEFT: u32 = 272;

/// A fixture with `options.seat_count` seats, `wl_shm`, and one shown
/// 100x100 toplevel (native 0). Ledger and events are empty.
pub(super) fn popup_fixture(options: PeerOptions) -> (Fixture, WindowId, Vec<InputSeatId>) {
    let (mut fixture, toplevel, seats) = super::manage::fixture_with(PeerOptions {
        shm: true,
        ..options
    });
    fixture.host.enable_cpu(toplevel).unwrap();
    fixture.configure(toplevel, 0, 100, 100);
    show(&mut fixture, toplevel);
    (fixture, toplevel, seats)
}

/// Present one frame at the window's current configuration and return the
/// peer index of the buffer it used. The buffer stays busy.
pub(super) fn present(fixture: &mut Fixture, id: WindowId) -> usize {
    let configuration = fixture.host.configuration(id).unwrap().unwrap();
    let length = configuration.pixel_width as usize * configuration.pixel_height as usize * 4;
    let outcome = fixture
        .host
        .present_cpu(id, configuration.revision, &vec![0; length])
        .unwrap();
    assert_eq!(outcome, PresentOutcome::Presented);
    fixture.sync();
    fixture.peer.buffers().len() - 1
}

/// Present one frame and release its buffer: the window counts as shown and
/// holds no busy slot, so its destruction never waits for a release. Ledger
/// and events are empty afterwards.
pub(super) fn show(fixture: &mut Fixture, id: WindowId) {
    let buffer = present(fixture, id);
    fixture.peer.emit(PeerEvent::Release(buffer));
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
}

/// A 200x160 menu at (40, 30) of the parent: anchor top left, gravity bottom
/// right, every flip and slide.
pub(super) fn menu(parent: WindowId) -> PopupOptions {
    PopupOptions {
        size: (200, 160),
        anchor_rect: LogicalRect {
            x: 40,
            y: 30,
            width: 1,
            height: 1,
        },
        anchor: PopupAnchor::TopLeft,
        gravity: PopupGravity::BottomRight,
        constraint_adjustment: ConstraintAdjustment::SLIDE_X
            | ConstraintAdjustment::SLIDE_Y
            | ConstraintAdjustment::FLIP_X
            | ConstraintAdjustment::FLIP_Y,
        ..PopupOptions::new(parent)
    }
}

/// The literal positioner requests for [`menu`] with this offset.
pub(super) fn rules(positioner: usize, offset: (i32, i32)) -> Vec<Request> {
    vec![
        Request::Positioner(positioner),
        Request::PositionerSize(positioner, 200, 160),
        Request::PositionerAnchorRect(positioner, 40, 30, 1, 1),
        Request::PositionerAnchor(positioner, 5),
        Request::PositionerGravity(positioner, 8),
        Request::PositionerConstraint(positioner, 15),
        Request::PositionerOffset(positioner, offset.0, offset.1),
    ]
}

/// Create a popup; the ledger is empty afterwards.
pub(super) fn open(fixture: &mut Fixture, options: PopupOptions) -> WindowId {
    let id = fixture.host.create_popup(options).unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    id
}

pub(super) fn emit(fixture: &mut Fixture, events: impl IntoIterator<Item = Wire>) {
    for event in events {
        fixture.peer.emit(PeerEvent::Popup(event));
    }
    fixture.sync();
}

/// The single `Configured` of `id` among the pending events.
pub(super) fn single_configured(fixture: &mut Fixture, id: WindowId) -> Configuration {
    let events = fixture.host.events();
    let [
        WindowEvent::Configured {
            window,
            configuration,
        },
    ] = events.as_slice()
    else {
        panic!("expected exactly one configure: {events:?}");
    };
    assert_eq!(*window, id);
    assert_eq!(
        fixture.host.configuration(id).unwrap(),
        Some(*configuration)
    );
    *configuration
}

/// Deliver `xdg_popup.configure(rect)` and `xdg_surface.configure(serial)`
/// and return the single configuration. The ack is on the wire before the
/// consumer observes it.
pub(super) fn configure_popup(
    fixture: &mut Fixture,
    id: WindowId,
    native: usize,
    rect: (i32, i32, i32, i32),
    serial: u32,
) -> Configuration {
    let (x, y, width, height) = rect;
    emit(
        fixture,
        [
            Wire::Configure(native, x, y, width, height),
            Wire::SurfaceConfigure(native, serial),
        ],
    );
    assert!(
        fixture
            .peer
            .requests()
            .contains(&Request::Ack(native, serial)),
        "acked before the consumer observes the configuration"
    );
    single_configured(fixture, id)
}

/// Create, configure at `rect` and show a popup with CPU output.
pub(super) fn shown(
    fixture: &mut Fixture,
    options: PopupOptions,
    native: usize,
    rect: (i32, i32, i32, i32),
) -> WindowId {
    let id = open(fixture, options);
    fixture.host.enable_cpu(id).unwrap();
    configure_popup(fixture, id, native, rect, 11);
    show(fixture, id);
    id
}

/// Pointer 0 enters `window`, presses the left button with `serial` and
/// leaves again. The ledger is empty afterwards.
pub(super) fn click(fixture: &mut Fixture, window: usize, serial: u32) {
    for event in [
        Input::PointerEnter(0, window, 1, 1.0, 1.0),
        Input::Button(0, serial, 1, BUTTON_LEFT, true),
        Input::Frame(0),
        Input::PointerLeave(0, window, 2),
        Input::Frame(0),
    ] {
        fixture.peer.emit(PeerEvent::Input(event));
    }
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
}

/// Keyboard 0 enters `window`, presses a key with `serial` and leaves again.
pub(super) fn key(fixture: &mut Fixture, window: usize, serial: u32) {
    for event in [
        Input::KeyboardEnter(0, window, 3, Vec::new()),
        Input::Key(0, serial, 1, 30, true),
        Input::KeyboardLeave(0, window, 4),
    ] {
        fixture.peer.emit(PeerEvent::Input(event));
    }
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
}

/// Only refused grabs and closes, in order.
pub(super) fn lifecycle(events: Vec<WindowEvent>) -> Vec<WindowEvent> {
    events
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                WindowEvent::Closed { .. } | WindowEvent::PopupGrabRefused { .. }
            )
        })
        .collect()
}

pub(super) fn closed(windows: &[WindowId]) -> Vec<WindowEvent> {
    windows
        .iter()
        .map(|window| WindowEvent::Closed { window: *window })
        .collect()
}

/// Only role and surface destruction, in order.
pub(super) fn destroys(requests: Vec<Request>) -> Vec<Request> {
    requests
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                Request::DestroyPopup(_)
                    | Request::DestroyRole(_)
                    | Request::DestroyToplevel(_)
                    | Request::DestroyLayerSurface(_)
                    | Request::DestroySurface(_)
                    | Request::DestroyShell
            )
        })
        .collect()
}

/// The destruction of popup `native`, child first.
pub(super) fn popup_gone(native: usize) -> [Request; 3] {
    [
        Request::DestroyPopup(native),
        Request::DestroyRole(native),
        Request::DestroySurface(native),
    ]
}

/// The placement every shown [`menu`] gets in these tests.
const PLACED: (i32, i32, i32, i32) = (40, 30, 200, 160);

fn seated() -> (Fixture, WindowId, InputSeatId) {
    let (fixture, toplevel, seats) = popup_fixture(PeerOptions {
        seat_count: 1,
        ..Default::default()
    });
    (fixture, toplevel, seats[0])
}

fn grab(parent: WindowId, seat: InputSeatId) -> PopupOptions {
    PopupOptions {
        grab: Some(seat),
        ..menu(parent)
    }
}

/// Create a popup and return it with the requests it sent.
fn create(fixture: &mut Fixture, options: PopupOptions) -> (WindowId, Vec<Request>) {
    let id = fixture.host.create_popup(options).unwrap();
    fixture.sync();
    (id, fixture.peer.take_requests())
}

/// `create_popup` fails as `expected` and sends nothing.
fn refused(fixture: &mut Fixture, options: PopupOptions, expected: fn(&Error) -> bool) {
    let result = fixture.host.create_popup(options);
    assert!(result.as_ref().is_err_and(expected), "{result:?}");
    assert!(fixture.sync().is_empty());
}

fn not_ready(error: &Error) -> bool {
    matches!(error, Error::NotReady)
}

fn invalid(error: &Error) -> bool {
    matches!(error, Error::InvalidInput(_))
}

fn close(fixture: &mut Fixture, id: WindowId) {
    fixture.host.close(id).unwrap();
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
}

/// A second shown 100x100 toplevel with peer index `native`.
fn second_toplevel(fixture: &mut Fixture, native: usize) -> WindowId {
    let id = fixture.create(100, 100);
    fixture.host.enable_cpu(id).unwrap();
    fixture.configure(id, native, 100, 100);
    show(fixture, id);
    id
}

fn toplevel_gone() -> [Request; 3] {
    [
        Request::DestroyToplevel(0),
        Request::DestroyRole(0),
        Request::DestroySurface(0),
    ]
}
