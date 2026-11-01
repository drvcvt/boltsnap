//! Acknowledged compositor commands and controlled peer pipe ownership.
use super::*;

pub enum DndCommand {
    Barrier(Sender<()>),
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
    pub(crate) tx: Sender<DndCommand>,
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
    /// Acknowledge all earlier commands; a following wire sync orders their events.
    pub fn barrier(&self) {
        let (done, ack) = std::sync::mpsc::channel();
        self.send(DndCommand::Barrier(done));
        ack.recv_timeout(Duration::from_secs(2)).unwrap();
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

pub fn run_commands(s: &mut State, dh: &DisplayHandle) {
    while let Some(cmd) = s.dnd.as_mut().and_then(|d| d.rx.try_recv().ok()) {
        match cmd {
            DndCommand::Pause { entered, resume } => {
                let _ = entered.send(());
                let _ = resume.recv_timeout(Duration::from_secs(5));
            }
            DndCommand::Barrier(done) => {
                let _ = done.send(());
            }
            DndCommand::Offer {
                mimes,
                actions,
                data,
            } => offer(s, dh, mimes, actions, data),
            DndCommand::RequestSend { mime } => send_data(s, mime),
            other => send_event(s.dnd.as_mut().unwrap(), other),
        }
    }
}

fn offer(
    s: &mut State,
    dh: &DisplayHandle,
    mimes: Vec<String>,
    actions: u32,
    data: Vec<(String, Vec<u8>)>,
) {
    let d = s.dnd.as_mut().unwrap();
    let Some(device) = d.devices.first().cloned() else {
        return;
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
                accepted: std::sync::atomic::AtomicBool::new(false),
            },
        )
        .unwrap();
    device.data_offer(&offer);
    for m in mimes {
        offer.offer(m);
    }
    if offer.version() >= 3 {
        offer.source_actions(DndAction::from_bits_truncate(actions));
        if d.config.hyprland_actions {
            let fixed = if actions & 2 != 0 { 2 } else { 1 };
            offer
                .data::<OfferData>()
                .unwrap()
                .current
                .store(fixed, Ordering::SeqCst);
            offer.action(DndAction::from_bits_truncate(fixed));
        }
    }
    s.metrics.dnd.offers_alive.fetch_add(1, Ordering::SeqCst);
    d.offer = Some(offer);
}

fn send_data(s: &mut State, mime: String) {
    let d = s.dnd.as_ref().unwrap();
    let Some(source) = d.source.clone() else {
        return;
    };
    let (read, write) = pipe();
    source.send(mime.clone(), write.as_fd());
    drop(write);
    if let Some(tx) = &d.config.controlled_pipes {
        let _ = tx.send(PipePeer::Outgoing { mime, read });
        return;
    }
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

fn send_event(d: &mut DndServer, cmd: DndCommand) {
    match cmd {
        DndCommand::Enter { surface, x, y } => {
            let serial = d.next_serial();
            if let (Some(device), Some(offer)) = (d.devices.first(), d.offer.as_ref()) {
                device.enter(serial, &d.surfaces[surface], x, y, Some(offer));
            }
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
        DndCommand::Button { serial } => button(d, serial, true),
        DndCommand::Release { serial } => button(d, serial, false),
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
        _ => unreachable!("command handled by run_commands"),
    }
}

fn button(d: &mut DndServer, serial: u32, pressed: bool) {
    if pressed {
        d.serial = serial;
    }
    for p in &d.pointers {
        if pressed && let Some(surface) = d.surfaces.first() {
            p.enter(serial, surface, 1.0, 1.0);
        }
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        p.button(serial, 0, 0x110, state);
        if p.version() >= 5 {
            p.frame();
        }
    }
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}
