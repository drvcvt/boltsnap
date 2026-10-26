//! The Boltsnap window, `boltsnap settings`: its own process with one
//! window, so a crash here never touches the daemon or a recording.

mod input;
mod prefs;
mod save;
mod single;
mod window;

use std::path::Path;
use std::time::Instant;

use boltsnap::app::{App, Page, gallery, pages};
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
const USAGE: &str = "usage: boltsnap settings [PAGE]\n       \
     boltsnap settings --render PAGE OUT.png [--scale N] [--size WxH] [--fixture]\n       \
     boltsnap settings --measure";

/// How `--render` draws.
struct Render {
    scale: u32,
    size: (u32, u32),
    fixture: bool,
}

fn render_options(mut rest: &[String]) -> DynResult<Render> {
    let mut render = Render {
        scale: 1,
        size: SIZE,
        fixture: false,
    };
    while let Some((flag, more)) = rest.split_first() {
        rest = more;
        match flag.as_str() {
            "--fixture" => render.fixture = true,
            "--scale" | "--size" => {
                let (value, more) = rest.split_first().ok_or(USAGE)?;
                rest = more;
                if flag == "--scale" {
                    render.scale = value
                        .parse()
                        .ok()
                        .filter(|scale| (1..=4).contains(scale))
                        .ok_or("--scale takes 1 to 4")?;
                } else {
                    render.size = value
                        .split_once('x')
                        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                        .filter(|&(w, h): &(u32, u32)| {
                            (200..=4000).contains(&w) && (200..=4000).contains(&h)
                        })
                        .ok_or("--size takes WxH, 200 to 4000 each")?;
                }
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok(render)
}

pub fn cli(args: &[String]) -> DynResult<()> {
    match args {
        [flag, page, out, rest @ ..] if flag == "--render" => {
            render(page, Path::new(out), render_options(rest)?)
        }
        [flag] if flag == "--measure" => {
            // The capitals gate with the real UI font (settings plan, 4b).
            for (what, offset) in boltsnap::app::measure::cap_offsets(fonts()) {
                println!("{what:<12} {offset:+.2} px");
            }
            // Gate 8, icons against their names, with the real font.
            let now = Instant::now();
            let rows = boltsnap::app::measure::icon_rows(
                fonts(),
                settings_pages(None, now),
                (1000.0, 900.0),
            );
            match boltsnap::app::measure::icon_gate(&rows) {
                Ok(worst) => println!("icons        {} rows, {worst}", rows.len()),
                Err(failure) => return Err(failure.into()),
            }
            Ok(())
        }
        [] => run(None),
        [page] if !page.starts_with('-') => run(Some(page)),
        _ => Err(USAGE.into()),
    }
}

fn settings_pages(notice: Option<&pages::Notice>, now: Instant) -> Vec<Page> {
    pages::pages(&prefs::load(), notice, now)
}

/// The UI face the selector and the recording controls use.
fn fonts() -> Fonts {
    let family = crate::config::Config::load().ui_font;
    let (medium, demibold) =
        super::shelf::font::load_ui_font_weights(family.as_deref(), (500.0, 600.0));
    Fonts::new(medium, demibold)
}

/// Draw `page` without a window, as the window would show it at first.
fn render(page: &str, out: &Path, render: Render) -> DynResult<()> {
    let now = Instant::now();
    let (w, h) = render.size;
    let pages = if render.fixture {
        gallery::pages(now)
    } else {
        settings_pages(None, now)
    };
    let mut app = App::new(fonts(), pages, (w as f32, h as f32), now);
    if !app.show_named(page, now) {
        return Err(format!("no page named '{page}'").into());
    }
    // The page has come in; nothing is moving.
    let settled = now + std::time::Duration::from_secs(2);
    let mut canvas = Canvas::new(w, h, render.scale).ok_or("empty canvas")?;
    app.draw(&mut canvas, settled);
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
    let single::Instance::First { _lock, listener } = single::claim(page)? else {
        return Ok(());
    };
    let notice = save::notice();
    let mut app = App::new(fonts(), settings_pages(notice.as_ref(), now), size, now);
    if let Some(page) = page
        && !app.show_named(page, now)
    {
        return Err(format!("no page named '{page}'").into());
    }

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
    let (done_tx, done_rx) = calloop::channel::channel();
    win.saver = Some(save::Saver::spawn(done_tx));
    win.notice = notice;
    handle
        .insert_source(done_rx, |event, _, win: &mut Win| {
            if let calloop::channel::Event::Msg((tag, result)) = event {
                win.saved(tag, result);
            }
        })
        .map_err(|error| format!("save source: {error}"))?;
    let (watch_tx, watch_rx) = calloop::channel::channel();
    save::watch(watch_tx);
    handle
        .insert_source(watch_rx, |event, _, win: &mut Win| {
            if let calloop::channel::Event::Msg(()) = event {
                win.refresh();
            }
        })
        .map_err(|error| format!("watch source: {error}"))?;
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
    // A text field still being edited counts, and no write is cut off.
    win.finish();
    Ok(())
}

impl Win {
    /// What came of a write. A refusal shows its reason on every page and
    /// puts the values back to what is saved; the next success clears it.
    fn saved(&mut self, tag: save::Tag, result: Result<(), String>) {
        if let Some(count) = self.pending.get_mut(&tag) {
            *count -= 1;
            if *count == 0 {
                self.pending.remove(&tag);
            }
        }
        let notice = match result {
            Err(text) => Some(pages::Notice {
                text,
                locked: false,
            }),
            Ok(_) if self.notice.as_ref().is_some_and(|n| !n.locked) => None,
            Ok(_) => return,
        };
        self.notice = notice;
        let now = Instant::now();
        self.app
            .reload(settings_pages(self.notice.as_ref(), now), now);
        self.changed();
    }

    /// The daemon's settings changed (the tray, the file, our own write):
    /// show the values in effect without rebuilding what did not change.
    fn refresh(&mut self) {
        let notice = save::notice();
        // A refused write keeps its message until the next success.
        if !(notice.is_none() && self.notice.as_ref().is_some_and(|n| !n.locked)) {
            self.notice = notice;
        }
        let now = Instant::now();
        let pending = &self.pending;
        self.app.update(
            settings_pages(self.notice.as_ref(), now),
            now,
            |page, block| pending.contains_key(&(page, block)),
        );
        self.changed();
    }
}
