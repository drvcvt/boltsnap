//! Live primary selection check: opens one static dark window. A left click sets
//! the primary selection to a fixed text, a middle click reads the current
//! primary selection and prints it. Run inside a Wayland session; it replaces
//! the session's real primary selection. Closes after two minutes.
use libway::window::{
    ClipboardEvent, ClipboardRead, ClipboardReceiveOutcome, ClipboardRepresentation, Configuration,
    Host, InputEvent, InputObservation, InputSeatId, PointerObservation, PresentOutcome,
    WindowEvent, WindowId, WindowOptions,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const TEXT: &str = "libway primary check";
const BACKGROUND: [u8; 4] = [0x20, 0x20, 0x20, 0xff];
// Linux evdev button codes.
const LEFT: u32 = 0x110;
const MIDDLE: u32 = 0x112;
const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

fn main() -> libway::Result<()> {
    let mut host = Host::connect()?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut window = None;
    let mut configuration: Option<Configuration> = None;
    let mut dirty = true;
    println!("left click: set primary to {TEXT:?}; middle click: read primary");
    while Instant::now() < deadline {
        host.pump(Some(Instant::now() + Duration::from_millis(100)))
            .map_err(|failure| failure.error)?;
        for event in host.events() {
            match event {
                WindowEvent::Ready => {
                    let id = host.create_window(WindowOptions {
                        preferred_width: 360,
                        preferred_height: 200,
                        title: "libway primary".into(),
                        app_id: "libway.primary".into(),
                    })?;
                    host.enable_cpu(id)?;
                    window = Some(id);
                }
                WindowEvent::Configured {
                    configuration: next,
                    ..
                } => (configuration, dirty) = (Some(next), true),
                WindowEvent::Input(InputEvent {
                    seat,
                    observation: InputObservation::PointerFrame(frame),
                }) => {
                    for record in frame.records() {
                        if let PointerObservation::Button {
                            window: target,
                            button,
                            down: true,
                            ..
                        } = record
                            && Some(*target) == window
                        {
                            click(&mut host, *target, seat, *button);
                        }
                    }
                }
                WindowEvent::Clipboard(event) => report(event),
                WindowEvent::CloseRequested { .. } => return Ok(()),
                _ => {}
            }
        }
        if let (true, Some(id), Some(config)) = (dirty, window, configuration) {
            let pixels =
                BACKGROUND.repeat(config.pixel_width as usize * config.pixel_height as usize);
            dirty = host.present_cpu(id, config.revision, &pixels)? != PresentOutcome::Presented;
        }
    }
    if let Some(id) = window {
        host.close(id)?;
    }
    host.shutdown();
    Ok(())
}

fn click(host: &mut Host, window: WindowId, seat: InputSeatId, button: u32) {
    match button {
        LEFT => {
            let bytes: Arc<[u8]> = TEXT.as_bytes().into();
            let representations: Vec<ClipboardRepresentation> = TEXT_MIMES
                .iter()
                .map(|mime| ClipboardRepresentation {
                    mime: (*mime).into(),
                    bytes: bytes.clone(),
                })
                .collect();
            match host.set_primary_selection(window, seat, &representations) {
                Ok(_) => println!("primary set"),
                Err(error) => println!("primary not set: {error}"),
            }
        }
        MIDDLE => match host.primary_selection_offer(window, seat) {
            Ok(Some(offer)) => {
                let Some(mime) = TEXT_MIMES
                    .iter()
                    .find(|mime| offer.mimes.iter().any(|offered| offered == *mime))
                else {
                    println!("primary has no text MIME: {:?}", offer.mimes);
                    return;
                };
                let read = ClipboardRead {
                    window,
                    seat,
                    offer: offer.id,
                    mime: (*mime).into(),
                    max_bytes: 64 * 1024,
                    deadline: Instant::now() + Duration::from_secs(5),
                };
                if let Err(error) = host.receive_clipboard(read) {
                    println!("primary read refused: {error}");
                }
            }
            Ok(None) => println!("no primary selection"),
            Err(error) => println!("primary unavailable: {error}"),
        },
        _ => {}
    }
}

fn report(event: ClipboardEvent) {
    match event {
        ClipboardEvent::PrimarySelection { offer, .. } => match offer {
            Some(offer) => println!("primary offer: {:?}", offer.mimes),
            None => println!("primary offer: none"),
        },
        ClipboardEvent::SourceCancelled { .. } => println!("own primary source retired"),
        ClipboardEvent::Received {
            outcome: ClipboardReceiveOutcome::Bytes(bytes),
            ..
        } => {
            let text = String::from_utf8_lossy(&bytes);
            let preview: String = text.chars().take(40).collect();
            println!("pasted {} bytes: {preview:?}", bytes.len());
        }
        ClipboardEvent::Received { outcome, .. } => println!("read ended: {outcome:?}"),
        _ => {}
    }
}
