//! An isolated compositor with acknowledged control and a literal request ledger.
use crate::runtime::mailbox::{Limits, Mailbox, Sender as CommandSender};
use std::{
    os::{
        fd::{AsFd, AsRawFd},
        unix::net::UnixStream,
    },
    sync::{
        Arc,
        mpsc::{Sender as Reply, channel},
    },
    thread::{self, JoinHandle},
};
use wayland_protocols::wp::{
    fractional_scale::v1::server::wp_fractional_scale_v1, viewporter::server::wp_viewport,
};
use wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1;
use wayland_protocols::xdg::shell::server::{xdg_popup, xdg_surface, xdg_toplevel};
use wayland_server::protocol::{wl_buffer, wl_callback, wl_output, wl_surface};
use wayland_server::{Display, backend::ClientData};

#[derive(Clone, Copy)]
pub(super) struct Options {
    pub compositor: bool,
    pub shell: bool,
    pub shm: bool,
    pub fractional: bool,
    pub viewport: bool,
    pub outputs: bool,
    pub output_count: usize,
    /// False holds back the initial `wl_output.done` until a test emits one.
    pub output_done: bool,
    /// Also advertise `ABGR8888` from `wl_shm` at bind.
    pub shm_abgr: bool,
    pub surface_version: u32,
    pub seat_count: usize,
    pub seat_version: u32,
    pub ime: bool,
    pub clipboard: bool,
    pub clipboard_version: u32,
    pub decoration: bool,
    pub cursor_shape: bool,
    pub presentation: bool,
    pub presentation_version: u32,
    pub presentation_clock: Option<u32>,
    pub primary: bool,
    pub layer_shell: bool,
    pub layer_shell_version: u32,
    pub activation: bool,
    pub idle_inhibit: bool,
    /// Advertised `xdg_wm_base` version.
    pub shell_version: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            compositor: true,
            shell: true,
            shm: false,
            fractional: false,
            viewport: false,
            outputs: false,
            output_count: 2,
            output_done: true,
            shm_abgr: false,
            surface_version: 6,
            seat_count: 0,
            seat_version: 9,
            ime: false,
            clipboard: false,
            clipboard_version: 3,
            decoration: false,
            cursor_shape: false,
            presentation: false,
            presentation_version: 2,
            presentation_clock: Some(1),
            primary: false,
            layer_shell: false,
            layer_shell_version: 5,
            activation: false,
            idle_inhibit: false,
            shell_version: 6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Request {
    BindIme,
    ImeObject(usize, usize),
    ImeEnable(usize),
    ImeDisable(usize),
    ImeCommit(usize),
    ImeSurrounding(usize, String, i32, i32),
    ImeCause(usize, u32),
    ImeContent(usize, u32, u32),
    ImeCaret(usize, i32, i32, i32, i32),
    DestroyIme(usize),
    DestroyImeManager,
    BindClipboard,
    ClipboardDevice(usize),
    ClipboardSource(usize),
    ClipboardMime(usize, String),
    ClipboardSet(Option<usize>, u32),
    ClipboardReceive(usize, String),
    ClipboardDestroyOffer(usize),
    ClipboardDestroySource(usize),
    ClipboardReleaseDevice(usize),
    ClipboardBytes(Vec<u8>),
    BindCompositor,
    BindSeat(usize, u32),
    Keyboard(usize, usize),
    Pointer(usize, usize),
    ReleaseSeat(usize),
    ReleaseKeyboard(usize),
    ReleasePointer(usize),
    BindShell,
    Surface(usize),
    Role(usize),
    Toplevel(usize),
    Title(usize, String),
    AppId(usize, String),
    Attach(usize, bool),
    Commit(usize, bool),
    Ack(usize, u32),
    DestroyToplevel(usize),
    DestroyRole(usize),
    DestroySurface(usize),
    DestroyShell,
    Pong(u32),
    BindShm,
    BindFractional,
    BindViewport,
    BindOutput(usize),
    ReleaseOutput(usize),
    Fractional(usize),
    Viewport(usize),
    DestroyFractional(usize),
    DestroyViewport(usize),
    ViewportDestination(usize, i32, i32),
    BufferScale(usize, i32),
    Pool(i32),
    Buffer(usize, i32, i32, i32),
    DestroyPool,
    DestroyBuffer(usize),
    AttachedBuffer(usize, Option<usize>),
    Frame(usize, usize),
    Damage(usize, i32, i32, i32, i32),
    BindDecoration,
    Decoration(usize),
    DecorationMode(usize, u32),
    DestroyDecoration(usize),
    /// Window, seat and serial.
    Move(usize, usize, u32),
    /// Window, seat, serial and the raw edge value.
    Resize(usize, usize, u32, u32),
    /// Window, seat, serial, x and y.
    ShowWindowMenu(usize, usize, u32, i32, i32),
    SetMaximized(usize),
    UnsetMaximized(usize),
    SetMinimized(usize),
    SetFullscreen(usize),
    UnsetFullscreen(usize),
    BindCursor,
    /// Pointer the shape device was created for.
    CursorDevice(usize),
    /// Pointer, enter serial and raw shape value.
    SetShape(usize, u32, u32),
    DestroyCursorDevice(usize),
    DestroyCursorManager,
    /// Bound version.
    BindPresentation(u32),
    /// Window and feedback index.
    Feedback(usize, usize),
    DestroyPresentation,
    BindPrimary,
    PrimaryDevice(usize),
    PrimarySource(usize),
    PrimaryMime(usize, String),
    PrimarySet(Option<usize>, u32),
    PrimaryReceive(usize, String),
    PrimaryBytes(Vec<u8>),
    PrimaryDestroyOffer(usize),
    PrimaryDestroySource(usize),
    PrimaryDestroyDevice(usize),
    PrimaryDestroyManager,
    /// Bound version.
    BindLayerShell(u32),
    /// Window, output index, raw layer and namespace.
    LayerSurface(usize, Option<usize>, u32, String),
    LayerSize(usize, u32, u32),
    LayerAnchor(usize, u32),
    LayerZone(usize, i32),
    /// Window and margins in wire order: top, right, bottom, left.
    LayerMargin(usize, [i32; 4]),
    LayerKeyboard(usize, u32),
    LayerSetLayer(usize, u32),
    LayerEdge(usize, u32),
    LayerAck(usize, u32),
    DestroyLayerSurface(usize),
    DestroyLayerShell,
    Region(usize),
    /// Region, x, y, width, height.
    RegionAdd(usize, i32, i32, i32, i32),
    DestroyRegion(usize),
    /// Window and region, `None` for a null region.
    InputRegion(usize, Option<usize>),
    OpaqueRegion(usize, Option<usize>),
    /// Window, min width and min height.
    MinSize(usize, i32, i32),
    /// Window, max width and max height.
    MaxSize(usize, i32, i32),
    BindActivation,
    /// Token index.
    ActivationToken(usize),
    /// Token and window.
    TokenSurface(usize, usize),
    TokenAppId(usize, String),
    TokenSerial(usize, u32),
    TokenCommit(usize),
    DestroyToken(usize),
    /// Token string and window.
    Activate(String, usize),
    DestroyActivation,
    BindIdleInhibit,
    /// Inhibitor index and window.
    Inhibitor(usize, usize),
    DestroyInhibitor(usize),
    DestroyIdleInhibitManager,
    /// Positioner index.
    Positioner(usize),
    /// Positioner, width and height.
    PositionerSize(usize, i32, i32),
    /// Positioner, x, y, width and height.
    PositionerAnchorRect(usize, i32, i32, i32, i32),
    /// Positioner and raw anchor value.
    PositionerAnchor(usize, u32),
    /// Positioner and raw gravity value.
    PositionerGravity(usize, u32),
    /// Positioner and raw constraint adjustment bits.
    PositionerConstraint(usize, u32),
    /// Positioner, x and y.
    PositionerOffset(usize, i32, i32),
    PositionerReactive(usize),
    DestroyPositioner(usize),
    /// Window, parent window (`None` for a null parent) and positioner.
    Popup(usize, Option<usize>, usize),
    /// Window, seat and serial.
    Grab(usize, usize, u32),
    /// Window, positioner and token.
    Reposition(usize, usize, u32),
    DestroyPopup(usize),
    /// Layer window and popup window.
    LayerGetPopup(usize, usize),
}

#[derive(Debug)]
pub(super) enum Event {
    Ime(ImeWireEvent),
    Input(InputWireEvent),
    Clipboard(ClipboardWireEvent),
    Release(usize),
    Frame(usize),
    Fractional(usize, u32),
    Preferred(usize, i32),
    /// Send one more `wl_shm.format` with this raw value.
    ShmFormat(u32),
    Output(usize, i32, bool),
    Enter(usize, usize),
    Leave(usize, usize),
    /// Window and raw decoration mode, sent before the next configure.
    Decoration(usize, u32),
    RemoveDecorationManager,
    RemoveCursorManager,
    Presentation(PresentationWireEvent),
    Primary(PrimaryWireEvent),
    /// Remove the output global with this peer index.
    RemoveOutput(usize),
    Layer(LayerWireEvent),
    Activation(ActivationWireEvent),
    IdleInhibit(IdleInhibitWireEvent),
    Popup(PopupWireEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BufferSnapshot {
    pub index: usize,
    /// Raw `wl_shm` format value.
    pub format: u32,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub bytes: Vec<u8>,
}

pub(super) struct Buffer {
    pub proxy: wl_buffer::WlBuffer,
    /// Raw `wl_shm` format value.
    pub format: u32,
    pub map: Arc<memmap2::Mmap>,
    pub offset: usize,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

#[derive(Debug)]
enum Command {
    Configure {
        window: usize,
        width: i32,
        height: i32,
        serial: u32,
        states: Vec<u8>,
        done: Reply<()>,
    },
    Close {
        window: usize,
        done: Reply<()>,
    },
    Snapshot {
        clear: bool,
        done: Reply<Vec<Request>>,
    },
    Stop,
    Emit {
        event: Event,
        done: Reply<()>,
    },
    Buffers {
        done: Reply<Vec<BufferSnapshot>>,
    },
}

pub(super) struct Native {
    pub surface: wl_surface::WlSurface,
    pub role: Option<xdg_surface::XdgSurface>,
    pub toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub attached: bool,
    pub fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    pub viewport: Option<wp_viewport::WpViewport>,
    pub decoration: Option<ZxdgToplevelDecorationV1>,
    pub layer: Option<
        wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    >,
    pub popup: Option<xdg_popup::XdgPopup>,
}

#[derive(Default)]
pub(super) struct State {
    pub windows: Vec<Native>,
    pub ledger: Vec<Request>,
    pub buffers: Vec<Buffer>,
    pub frames: Vec<(usize, wl_callback::WlCallback)>,
    pub outputs: Vec<Option<wl_output::WlOutput>>,
    pub output_globals: Vec<Option<wayland_server::backend::GlobalId>>,
    pub hold_output_done: bool,
    pub shm_abgr: bool,
    pub shm: Option<wayland_server::protocol::wl_shm::WlShm>,
    pub input: input::InputState,
    pub clipboard: clipboard::ClipboardState,
    pub ime: text_input::TextInputState,
    pub decoration: decoration::DecorationState,
    pub cursor: cursor::CursorState,
    pub presentation: presentation::PresentationState,
    pub primary: primary::PrimaryState,
    pub layer: layer::LayerPeerState,
    pub attention: toplevel::AttentionPeerState,
    pub popup: roles::popup::PopupPeerState,
}

pub(super) struct Peer {
    sender: CommandSender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl Peer {
    pub fn start(options: Options) -> (Self, UnixStream) {
        let (client, socket) = UnixStream::pair().unwrap();
        let mailbox = Mailbox::new(Limits {
            max_queued: 16,
            max_bytes: 4096,
        })
        .unwrap();
        let sender = mailbox.sender();
        let thread = thread::spawn(move || run(socket, options, mailbox));
        (
            Self {
                sender,
                thread: Some(thread),
            },
            client,
        )
    }

    pub fn configure(&self, window: usize, width: i32, height: i32, serial: u32, states: Vec<u8>) {
        let (done, ack) = channel();
        self.sender
            .send(
                Command::Configure {
                    window,
                    width,
                    height,
                    serial,
                    states,
                    done,
                },
                0,
            )
            .unwrap();
        ack.recv_timeout(super::WATCHDOG).unwrap();
    }

    pub fn close_requested(&self, window: usize) {
        let (done, ack) = channel();
        self.sender
            .send(Command::Close { window, done }, 0)
            .unwrap();
        ack.recv_timeout(super::WATCHDOG).unwrap();
    }

    pub fn requests(&self) -> Vec<Request> {
        self.snapshot(false)
    }

    pub fn emit(&self, event: Event) {
        let (done, ack) = channel();
        self.sender.send(Command::Emit { event, done }, 0).unwrap();
        ack.recv_timeout(super::WATCHDOG).unwrap();
    }

    pub fn buffers(&self) -> Vec<BufferSnapshot> {
        let (done, ack) = channel();
        self.sender.send(Command::Buffers { done }, 0).unwrap();
        ack.recv_timeout(super::WATCHDOG).unwrap()
    }
    pub fn take_requests(&self) -> Vec<Request> {
        self.snapshot(true)
    }

    fn snapshot(&self, clear: bool) -> Vec<Request> {
        let (done, ack) = channel();
        self.sender
            .send(Command::Snapshot { clear, done }, 0)
            .unwrap();
        ack.recv_timeout(super::WATCHDOG).unwrap()
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Stop, 0);
        if let Some(worker) = self.thread.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

mod cursor;
mod decoration;
mod presentation;
pub(super) use presentation::PresentationWireEvent;
mod primary;
pub(super) use primary::PrimaryWireEvent;
mod layer;
pub(super) use layer::LayerWireEvent;
mod toplevel;
pub(super) use toplevel::{ActivationWireEvent, IdleInhibitWireEvent};
mod roles;
pub(super) use roles::popup::PopupWireEvent;
mod text_input;
pub(super) use text_input::ImeWireEvent;
mod clipboard;
pub(super) use clipboard::ClipboardWireEvent;
mod input;
pub(super) use input::InputWireEvent;
mod runtime;
use runtime::run;
