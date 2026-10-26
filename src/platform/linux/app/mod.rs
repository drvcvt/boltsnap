//! The Boltsnap window, `boltsnap settings`: its own process with one
//! window, so a crash here never touches the daemon or a recording.

use std::path::Path;
use std::time::Instant;

use boltsnap::app::{App, gallery};
use boltsnap::ui::Canvas;
use boltsnap::ui::text::Fonts;

use crate::DynResult;

/// Logical size the window opens at.
const SIZE: (u32, u32) = (760, 520);
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
        [] | [_] => Err("boltsnap settings: the window is not built yet; use --render".into()),
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
