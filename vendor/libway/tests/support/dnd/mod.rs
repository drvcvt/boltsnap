//! Compositor-side data device for DnD tests. Acts as compositor and as the remote peer.
use super::State;
use std::{
    io::{Read, Write},
    os::fd::{AsFd, FromRawFd, OwnedFd},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, Sender},
    },
    thread,
    time::Duration,
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
        wl_keyboard, wl_pointer, wl_region, wl_seat, wl_surface, wl_touch,
    },
};
use wl_data_device_manager::DndAction;

mod commands;
mod protocol;
pub use commands::run_commands;
pub use commands::{DndCommand, DndControl};

#[derive(Clone)]
pub struct DndConfig {
    pub version: u32,
    pub writer_delay: Duration,
    pub hold_writer_open: bool,
    /// Read outgoing drag data 64 KiB at a time with a pause, like a slow drop target.
    pub slow_reader: bool,
    /// Hyprland 0.56: the offer's action is fixed at creation (move if the source offers it,
    /// else copy) and `set_actions` never changes it.
    pub hyprland_actions: bool,
    /// Hand peer pipe ends to the test instead of starting background I/O.
    pub controlled_pipes: Option<Sender<PipePeer>>,
}
impl Default for DndConfig {
    fn default() -> Self {
        Self {
            version: 3,
            writer_delay: Duration::ZERO,
            hold_writer_open: false,
            slow_reader: false,
            hyprland_actions: false,
            controlled_pipes: None,
        }
    }
}
pub enum PipePeer {
    Incoming { mime: String, write: OwnedFd },
    Outgoing { mime: String, read: OwnedFd },
}
#[derive(Default)]
pub struct DndMetrics {
    pub offers_alive: AtomicUsize,
    pub finishes: AtomicUsize,
    pub sources_alive: AtomicUsize,
    pub devices: AtomicUsize,
    pub pointers: AtomicUsize,
    pub surfaces: AtomicUsize,
    pub accepts: Mutex<Vec<Option<String>>>,
    pub actions: Mutex<Vec<(u32, u32)>>,
    pub receives: Mutex<Vec<String>>,
    pub start_drags: Mutex<Vec<(u32, bool)>>,
    pub source_mimes: Mutex<Vec<String>>,
    pub received: Mutex<Vec<(String, Vec<u8>)>>,
    /// Inodes of every pipe the server was handed, to prove both ends get closed.
    pub pipes: Mutex<Vec<u64>>,
    /// Requests a real compositor would answer with a protocol error.
    pub protocol_errors: AtomicUsize,
    /// Finishes Hyprland turns into `abortDrag` because no type was accepted.
    pub aborts: AtomicUsize,
}
/// Whether any fd of this process still refers to the pipe inode.
pub fn pipe_open(inode: u64) -> bool {
    pipe_fds(inode) > 0
}
/// How many fds of this process refer to the pipe inode (both ends count).
pub fn pipe_fds(inode: u64) -> usize {
    let name = format!("pipe:[{inode}]");
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
        .filter(|link| link.as_os_str() == name.as_str())
        .count()
}
fn inode(fd: &OwnedFd) -> u64 {
    use std::os::fd::AsRawFd;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::fstat(fd.as_raw_fd(), &mut st) }, 0);
    st.st_ino
}
pub struct OfferData {
    pub data: Vec<(String, Vec<u8>)>,
    pub source_actions: u32,
    /// The action last sent to the client, as a compositor tracks it for `finish`.
    pub current: std::sync::atomic::AtomicU32,
    /// Whether the client's last `accept` named a type, as Hyprland tracks it.
    pub accepted: std::sync::atomic::AtomicBool,
}
pub struct DndServer {
    config: DndConfig,
    rx: Receiver<DndCommand>,
    pub devices: Vec<wl_data_device::WlDataDevice>,
    pub pointers: Vec<wl_pointer::WlPointer>,
    pub surfaces: Vec<wl_surface::WlSurface>,
    pub offer: Option<wl_data_offer::WlDataOffer>,
    pub source: Option<wl_data_source::WlDataSource>,
    pub serial: u32,
}
impl DndServer {
    pub fn advertise(dh: &DisplayHandle, config: DndConfig, rx: Receiver<DndCommand>) -> Self {
        dh.create_global::<State, wl_compositor::WlCompositor, DndGlobal>(6, DndGlobal);
        dh.create_global::<State, wl_seat::WlSeat, DndGlobal>(7, DndGlobal);
        dh.create_global::<State, wl_data_device_manager::WlDataDeviceManager, DndGlobal>(
            config.version,
            DndGlobal,
        );
        Self {
            config,
            rx,
            devices: Vec::new(),
            pointers: Vec::new(),
            surfaces: Vec::new(),
            offer: None,
            source: None,
            serial: 100,
        }
    }
    fn next_serial(&mut self) -> u32 {
        self.serial += 1;
        self.serial
    }
    /// Compositor action choice: preferred if possible, else copy, move, ask.
    fn action_for(dnd_actions: u32, preferred: u32, source_actions: u32) -> u32 {
        let both = dnd_actions & source_actions;
        if both & preferred != 0 {
            preferred
        } else {
            [1, 2, 4].into_iter().find(|a| both & a != 0).unwrap_or(0)
        }
    }
}
/// Marker so seat/compositor dispatch does not collide with the cursor fixture's `()` data.
pub struct DndGlobal;
