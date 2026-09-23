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

#[derive(Clone)]
pub struct DndConfig {
    pub version: u32,
    pub writer_delay: Duration,
    pub hold_writer_open: bool,
    /// Read outgoing drag data 64 KiB at a time with a pause, like a slow drop target.
    pub slow_reader: bool,
}
impl Default for DndConfig {
    fn default() -> Self {
        Self {
            version: 3,
            writer_delay: Duration::ZERO,
            hold_writer_open: false,
            slow_reader: false,
        }
    }
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
pub enum DndCommand {
    Pause {
        entered: Sender<()>,
        resume: Receiver<()>,
    },
    Offer {
        mimes: Vec<String>,
        actions: u32,
        data: Vec<(String, Vec<u8>)>,
    },
    Enter {
        surface: usize,
        x: f64,
        y: f64,
    },
    Motion {
        x: f64,
        y: f64,
    },
    Leave,
    Drop,
    Selection,
    Button {
        serial: u32,
    },
    Release {
        serial: u32,
    },
    RequestSend {
        mime: String,
    },
    DropPerformed,
    Finished,
    CancelSource,
    Target {
        mime: Option<String>,
        action: u32,
    },
}
#[derive(Clone)]
pub struct DndControl {
    pub(super) tx: Sender<DndCommand>,
}
/// Dropping the guard resumes the server even if a test panics.
pub struct PausedServer(Sender<()>);
impl Drop for PausedServer {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
impl DndControl {
    pub fn pause(&self) -> PausedServer {
        let (entered, ack) = std::sync::mpsc::channel();
        let (tx, resume) = std::sync::mpsc::channel();
        let guard = PausedServer(tx);
        self.send(DndCommand::Pause { entered, resume });
        ack.recv_timeout(Duration::from_secs(2)).unwrap();
        guard
    }
    pub fn send(&self, c: DndCommand) {
        self.tx.send(c).unwrap();
    }
    pub fn offer(&self, mimes: &[&str], actions: u32, data: &[(&str, &[u8])]) {
        self.send(DndCommand::Offer {
            mimes: mimes.iter().map(|m| m.to_string()).collect(),
            actions,
            data: data
                .iter()
                .map(|(m, d)| (m.to_string(), d.to_vec()))
                .collect(),
        });
    }
    pub fn enter(&self, surface: usize, x: f64, y: f64) {
        self.send(DndCommand::Enter { surface, x, y });
    }
    pub fn motion(&self, x: f64, y: f64) {
        self.send(DndCommand::Motion { x, y });
    }
    pub fn leave(&self) {
        self.send(DndCommand::Leave);
    }
    pub fn drop(&self) {
        self.send(DndCommand::Drop);
    }
    /// Announce the current offer as clipboard selection instead of a drag.
    pub fn selection(&self) {
        self.send(DndCommand::Selection);
    }
    pub fn button(&self, serial: u32) {
        self.send(DndCommand::Button { serial });
    }
    pub fn release(&self, serial: u32) {
        self.send(DndCommand::Release { serial });
    }
    pub fn request_send(&self, mime: &str) {
        self.send(DndCommand::RequestSend { mime: mime.into() });
    }
    pub fn drop_performed(&self) {
        self.send(DndCommand::DropPerformed);
    }
    pub fn finished(&self) {
        self.send(DndCommand::Finished);
    }
    pub fn cancel_source(&self) {
        self.send(DndCommand::CancelSource);
    }
    pub fn target(&self, mime: Option<&str>, action: u32) {
        self.send(DndCommand::Target {
            mime: mime.map(String::from),
            action,
        });
    }
}

pub struct OfferData {
    pub data: Vec<(String, Vec<u8>)>,
    pub source_actions: u32,
    /// The action last sent to the client, as a compositor tracks it for `finish`.
    pub current: std::sync::atomic::AtomicU32,
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

pub fn run_commands(s: &mut State, dh: &DisplayHandle) {
    let Some(d) = s.dnd.as_mut() else { return };
    while let Ok(cmd) = d.rx.try_recv() {
        match cmd {
            DndCommand::Pause { entered, resume } => {
                let _ = entered.send(());
                let _ = resume.recv_timeout(Duration::from_secs(5));
            }
            DndCommand::Offer {
                mimes,
                actions,
                data,
            } => {
                let Some(device) = d.devices.first().cloned() else {
                    continue;
                };
                let client = device.client().unwrap();
                let offer = client
                    .create_resource::<wl_data_offer::WlDataOffer, OfferData, State>(
                        dh,
                        device.version(),
                        OfferData {
                            data,
                            source_actions: actions,
                            current: Default::default(),
                        },
                    )
                    .unwrap();
                device.data_offer(&offer);
                for m in mimes {
                    offer.offer(m);
                }
                if offer.version() >= 3 {
                    offer.source_actions(DndAction::from_bits_truncate(actions));
                }
                s.metrics.dnd.offers_alive.fetch_add(1, Ordering::SeqCst);
                d.offer = Some(offer);
            }
            DndCommand::Enter { surface, x, y } => {
                let serial = d.next_serial();
                let (Some(device), Some(offer)) = (d.devices.first(), d.offer.as_ref()) else {
                    continue;
                };
                device.enter(serial, &d.surfaces[surface], x, y, Some(offer));
            }
            DndCommand::Motion { x, y } => {
                if let Some(device) = d.devices.first() {
                    device.motion(d.serial, x, y);
                }
            }
            DndCommand::Leave => {
                if let Some(device) = d.devices.first() {
                    device.leave();
                }
            }
            DndCommand::Drop => {
                if let Some(device) = d.devices.first() {
                    device.drop();
                }
            }
            DndCommand::Selection => {
                if let Some(device) = d.devices.first() {
                    device.selection(d.offer.as_ref());
                }
            }
            DndCommand::Button { serial } => {
                d.serial = serial;
                for p in &d.pointers {
                    if let Some(surface) = d.surfaces.first() {
                        p.enter(serial, surface, 1.0, 1.0);
                    }
                    p.button(serial, 0, 0x110, wl_pointer::ButtonState::Pressed);
                    if p.version() >= 5 {
                        p.frame();
                    }
                }
            }
            DndCommand::Release { serial } => {
                for p in &d.pointers {
                    p.button(serial, 0, 0x110, wl_pointer::ButtonState::Released);
                    if p.version() >= 5 {
                        p.frame();
                    }
                }
            }
            DndCommand::RequestSend { mime } => {
                let Some(source) = d.source.clone() else {
                    continue;
                };
                let (read, write) = pipe();
                source.send(mime.clone(), write.as_fd());
                drop(write);
                let metrics = s.metrics.clone();
                let slow = d.config.slow_reader;
                thread::spawn(move || {
                    let mut file = std::fs::File::from(read);
                    let mut bytes = Vec::new();
                    let mut chunk = vec![0; 64 * 1024];
                    loop {
                        let n = file.read(&mut chunk).unwrap();
                        if n == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        if slow {
                            thread::sleep(Duration::from_millis(3));
                        }
                    }
                    metrics.dnd.received.lock().unwrap().push((mime, bytes));
                });
            }
            DndCommand::DropPerformed => {
                if let Some(source) = d.source.as_ref().filter(|s| s.version() >= 3) {
                    source.dnd_drop_performed();
                }
            }
            DndCommand::Finished => {
                if let Some(source) = d.source.as_ref().filter(|s| s.version() >= 3) {
                    source.dnd_finished();
                }
            }
            DndCommand::CancelSource => {
                if let Some(source) = &d.source {
                    source.cancelled();
                }
            }
            DndCommand::Target { mime, action } => {
                if let Some(source) = &d.source {
                    source.target(mime);
                    if source.version() >= 3 {
                        source.action(DndAction::from_bits_truncate(action));
                    }
                }
            }
        }
    }
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

impl GlobalDispatch<wl_compositor::WlCompositor, DndGlobal> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_compositor::WlCompositor>,
        _: &DndGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(r, ());
    }
}
impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        r: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_compositor::Request::CreateSurface { id } => {
                let surface = init.init(id, ());
                s.metrics.dnd.surfaces.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().surfaces.push(surface);
            }
            wl_compositor::Request::CreateRegion { id } => {
                init.init(id, ());
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_surface::WlSurface,
        _: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, surface: &wl_surface::WlSurface, _: &()) {
        s.metrics.dnd.surfaces.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut() {
            d.surfaces.retain(|x| x != surface);
        }
    }
}
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_region::WlRegion,
        _: wl_region::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<wl_seat::WlSeat, DndGlobal> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_seat::WlSeat>,
        _: &DndGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        let seat = init.init(r, DndGlobal);
        seat.capabilities(wl_seat::Capability::Pointer | wl_seat::Capability::Touch);
        if seat.version() >= 2 {
            seat.name("dnd-seat".into());
        }
    }
}
impl Dispatch<wl_seat::WlSeat, DndGlobal> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        r: wl_seat::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_seat::Request::GetPointer { id } => {
                let p = init.init(id, DndGlobal);
                s.metrics.dnd.pointers.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().pointers.push(p);
            }
            wl_seat::Request::GetTouch { id } => {
                init.init(id, DndGlobal);
            }
            wl_seat::Request::GetKeyboard { id } => {
                init.init(id, DndGlobal);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, DndGlobal> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_pointer::WlPointer,
        _: wl_pointer::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, p: &wl_pointer::WlPointer, _: &DndGlobal) {
        s.metrics.dnd.pointers.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut() {
            d.pointers.retain(|x| x != p);
        }
    }
}
impl Dispatch<wl_touch::WlTouch, DndGlobal> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_touch::WlTouch,
        _: wl_touch::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, DndGlobal> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_keyboard::WlKeyboard,
        _: wl_keyboard::Request,
        _: &DndGlobal,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
impl GlobalDispatch<wl_data_device_manager::WlDataDeviceManager, DndGlobal> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_data_device_manager::WlDataDeviceManager>,
        _: &DndGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(r, ());
    }
}
impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_device_manager::WlDataDeviceManager,
        r: wl_data_device_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_device_manager::Request::GetDataDevice { id, .. } => {
                let device = init.init(id, ());
                s.metrics.dnd.devices.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().devices.push(device);
            }
            wl_data_device_manager::Request::CreateDataSource { id } => {
                let source = init.init(id, ());
                s.metrics.dnd.sources_alive.fetch_add(1, Ordering::SeqCst);
                s.dnd.as_mut().unwrap().source = Some(source);
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_device::WlDataDevice,
        r: wl_data_device::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_data_device::Request::StartDrag { serial, icon, .. } = r {
            s.metrics
                .dnd
                .start_drags
                .lock()
                .unwrap()
                .push((serial, icon.is_some()));
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, device: &wl_data_device::WlDataDevice, _: &()) {
        s.metrics.dnd.devices.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut() {
            d.devices.retain(|x| x != device);
        }
    }
}
impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_data_source::WlDataSource,
        r: wl_data_source::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_source::Request::Offer { mime_type } => {
                s.metrics.dnd.source_mimes.lock().unwrap().push(mime_type);
            }
            wl_data_source::Request::SetActions {
                dnd_actions: WEnum::Value(a),
            } => {
                s.metrics.dnd.actions.lock().unwrap().push((a.bits(), 0));
            }
            _ => {}
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, source: &wl_data_source::WlDataSource, _: &()) {
        s.metrics.dnd.sources_alive.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut().filter(|d| d.source.as_ref() == Some(source)) {
            d.source = None;
        }
    }
}
impl Dispatch<wl_data_offer::WlDataOffer, OfferData> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        offer: &wl_data_offer::WlDataOffer,
        r: wl_data_offer::Request,
        data: &OfferData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match r {
            wl_data_offer::Request::Accept { mime_type, .. } => {
                s.metrics.dnd.accepts.lock().unwrap().push(mime_type);
            }
            wl_data_offer::Request::SetActions {
                dnd_actions: WEnum::Value(a),
                preferred_action: WEnum::Value(p),
            } => {
                s.metrics
                    .dnd
                    .actions
                    .lock()
                    .unwrap()
                    .push((a.bits(), p.bits()));
                // Weston: the preferred action must be a single one out of the set.
                if p.bits().count_ones() > 1 || (p.bits() != 0 && a.bits() & p.bits() == 0) {
                    s.metrics.dnd.protocol_errors.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                let chosen = DndServer::action_for(a.bits(), p.bits(), data.source_actions);
                data.current.store(chosen, Ordering::SeqCst);
                offer.action(DndAction::from_bits_truncate(chosen));
            }
            wl_data_offer::Request::Receive { mime_type, fd } => {
                s.metrics
                    .dnd
                    .receives
                    .lock()
                    .unwrap()
                    .push(mime_type.clone());
                s.metrics.dnd.pipes.lock().unwrap().push(inode(&fd));
                let bytes = data
                    .data
                    .iter()
                    .find(|(m, _)| *m == mime_type)
                    .map(|(_, b)| b.clone());
                let config = s.dnd.as_ref().unwrap().config.clone();
                thread::spawn(move || {
                    let mut file = std::fs::File::from(fd);
                    for chunk in bytes.as_deref().unwrap_or_default().chunks(4096) {
                        if !config.writer_delay.is_zero() {
                            thread::sleep(config.writer_delay);
                        }
                        if file.write_all(chunk).is_err() {
                            return;
                        }
                    }
                    if config.hold_writer_open {
                        thread::sleep(Duration::from_secs(30));
                    }
                });
            }
            wl_data_offer::Request::Finish => {
                // Weston: finishing with none or ask is INVALID_OFFER.
                if !matches!(data.current.load(Ordering::SeqCst), 1 | 2) {
                    s.metrics.dnd.protocol_errors.fetch_add(1, Ordering::SeqCst);
                }
                s.metrics.dnd.finishes.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, offer: &wl_data_offer::WlDataOffer, _: &OfferData) {
        s.metrics.dnd.offers_alive.fetch_sub(1, Ordering::SeqCst);
        if let Some(d) = s.dnd.as_mut().filter(|d| d.offer.as_ref() == Some(offer)) {
            d.offer = None;
        }
    }
}
