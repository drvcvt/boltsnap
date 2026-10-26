//! The Boltsnap window, `boltsnap settings`: its own process with one
//! window, so a crash here never touches the daemon or a recording.

mod input;
mod single;
mod window;

use std::path::Path;
use std::time::Instant;

use boltsnap::app::{App, gallery};
use boltsnap::ui::text::Fonts;
use boltsnap::ui::{Canvas, Input};
use calloop::generic::Generic;
use calloop::{EventLoop, Interest, Mode, PostAction};
use calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::CompositorState, output::OutputState, registry::RegistryState, seat::SeatState,
    shell::xdg::XdgShell, shm::Shm,
};
use wayland_client::{Connection, globals::registry_queue_init};

use crate::DynResult;
use window::{OPEN_SIZE as SIZE, Parts, Win};
const USAGE: &str =
    "usage: boltsnap settings [PAGE]\n       boltsnap settings --render PAGE OUT.png [--scale N]";

pub fn cli(args: &[String]) -> DynResult<()> {
    match args {
        [flag, page, out, rest @ ..] if flag == "--render" => {
            let scale = match rest {
                [] => 1,
                [flag, scale] if flag == "--scale" => scale
                    .parse()
                    .ok()
                    .filter(|scale| (1..=4).contains(scale))
                    .ok_or("--scale takes 1 to 4")?,
                _ => return Err(USAGE.into()),
            };
            render(page, Path::new(out), scale)
        }
        [] => run(None),
        [page] if !page.starts_with('-') => run(Some(page)),
        _ => Err(USAGE.into()),
    }
}

/// The UI face the selector and the recording controls use.
fn fonts() -> Fonts {
    let family = crate::config::Config::load().ui_font;
    let (medium, demibold) =
        super::shelf::font::load_ui_font_weights(family.as_deref(), (500.0, 600.0));
    Fonts::new(medium, demibold)
}

/// Draw `page` without a window, as the window would show it at first.
pub fn render(page: &str, out: &Path, scale: u32) -> DynResult<()> {
    let now = Instant::now();
    let size = (SIZE.0 as f32, SIZE.1 as f32);
    let mut app = App::new(fonts(), gallery::pages(now), size, now);
    if !app.show_named(page, now) {
        return Err(format!("no page named '{page}'").into());
    }
    let mut canvas = Canvas::new(SIZE.0, SIZE.1, scale).ok_or("empty canvas")?;
    app.draw(&mut canvas, now);
    canvas
        .pixmap
        .save_png(out)
        .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    Ok(())
}

/// Open the window, or bring the open one forward.
fn run(page: Option<&str>) -> DynResult<()> {
    let now = Instant::now();
    let size = (SIZE.0 as f32, SIZE.1 as f32);
    let mut app = App::new(fonts(), gallery::pages(now), size, now);
    if let Some(page) = page
        && !app.show_named(page, now)
    {
        return Err(format!("no page named '{page}'").into());
    }
    let single::Instance::First { _lock, listener } = single::claim(page)? else {
        return Ok(());
    };

    let conn = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init(&conn)?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<'static, Win> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|error| format!("wayland source: {error}"))?;
    let parts = Parts {
        registry: RegistryState::new(&globals),
        seats: SeatState::new(&globals, &qh),
        outputs: OutputState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh)?,
        xdg: XdgShell::bind(&globals, &qh)?,
        shm: Shm::bind(&globals, &qh)?,
    };
    let (paste_tx, paste_rx) = calloop::channel::channel();
    let mut win = Win::new(parts, app, &qh, handle.clone(), paste_tx)?;
    handle
        .insert_source(paste_rx, |event, _, win: &mut Win| {
            if let calloop::channel::Event::Msg(text) = event {
                win.input(Input::Pasted(text));
            }
        })
        .map_err(|error| format!("paste source: {error}"))?;
    let raises = Generic::new(listener, Interest::READ, Mode::Level);
    handle
        .insert_source(raises, |_, listener, win: &mut Win| {
            while let Ok((stream, _)) = listener.accept() {
                if let Some(page) = single::read_raise(stream) {
                    win.raise(&page);
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|error| format!("raise source: {error}"))?;
    while !win.exit {
        event_loop.dispatch(None, &mut win)?;
    }
    Ok(())
}
