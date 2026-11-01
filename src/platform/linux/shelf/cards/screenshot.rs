//! Full-desktop screenshots the daemon takes itself, over a Wayland
//! connection it keeps open: a shot then skips the client's start, its
//! connection with globals and outputs, and handing the PNG over
//! (capture-speed plan, D1). Each output's frame goes into the PNG as it came
//! from the compositor, without composing or converting the desktop first.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::image_model::png::{self, Format, Tile};
use crate::protocol::CAPTURE_UNAVAILABLE;

use super::PreparedAdd;
use super::cards::{card_size, prepare_png};
use super::placement::focused_monitor_name;

/// The open connection: made by the first shot, dropped by a failed one.
static CONNECTION: Mutex<Option<libway::Connection>> = Mutex::new(None);

/// What a `capture` request came to.
pub(super) enum Taken {
    /// The PNG is at the requested path.
    File,
    /// A card for the shelf, with the PNG, `last.png` and any clipboard copy
    /// already written.
    Card(PreparedAdd),
}

/// The desktop as captured: the outputs' own frames where they share one
/// scale and orientation, else one composed RGB image.
enum Desktop {
    Tiles(libway::DesktopTiles),
    Composed(image::RgbImage),
}

/// A full-desktop screenshot into `output`, or onto the shelf. An error
/// before anything was captured starts with [`CAPTURE_UNAVAILABLE`].
pub(super) fn take(output: Option<PathBuf>, copy: bool) -> Result<Taken, String> {
    let capture = super::super::timing::Span::new("daemon_capture");
    let desktop = desktop().map_err(|error| format!("{CAPTURE_UNAVAILABLE}: {error}"))?;
    drop(capture);
    let _timing = super::super::timing::Span::new("daemon_capture_output");
    let (width, height, png) = match &desktop {
        Desktop::Tiles(tiles) => (tiles.width, tiles.height, encode_tiles(tiles)?),
        Desktop::Composed(image) => (
            image.width(),
            image.height(),
            crate::capture::encode_png(image)?,
        ),
    };
    let Some(path) = output else {
        let size = card_size();
        let thumb = match &desktop {
            Desktop::Tiles(desktop) => preview(
                &tiles(desktop),
                desktop.width,
                desktop.height,
                size.w,
                size.h,
            ),
            Desktop::Composed(image) => thumbnail(image, size.w, size.h),
        };
        let metadata = super::super::image_transfer::Metadata::new(
            width,
            height,
            "full".into(),
            focused_monitor_name(),
            copy,
        )
        .map_err(|error| error.to_string())?;
        return prepare_png(metadata, png, thumb).map(Taken::Card);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, png).map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(Taken::File)
}

/// The desktop over the open connection; a connection gone stale since the
/// last shot is replaced once.
fn desktop() -> Result<Desktop, String> {
    let mut connection = CONNECTION.lock().unwrap_or_else(PoisonError::into_inner);
    let start = std::time::Instant::now();
    let mut retried = false;
    loop {
        let options = crate::capture::remaining(start);
        if connection.is_none() {
            *connection = Some(libway::Connection::connect(&options).map_err(|e| e.to_string())?);
        }
        let open = connection.as_mut().expect("connected above");
        let shot = match open.capture_desktop_tiles(&options) {
            Ok(Some(tiles)) if tiles.tiles.iter().all(|tile| format(&tile.frame).is_some()) => {
                Ok(Desktop::Tiles(tiles))
            }
            // Mixed scales, a rotation or an unknown layout: composed instead.
            Ok(_) => open
                .capture_desktop(&options)
                .map(|desktop| Desktop::Composed(crate::capture::rgba_to_rgb(desktop.image))),
            Err(error) => Err(error),
        };
        match shot {
            Ok(desktop) => return Ok(desktop),
            Err(error) => {
                *connection = None;
                if retried {
                    return Err(error.to_string());
                }
                retried = true;
            }
        }
    }
}

/// The encoder's name for a frame's layout; `None` for one it cannot read.
fn format(frame: &libway::Frame) -> Option<Format> {
    match frame.format {
        libway::PixelFormat::Argb8888 | libway::PixelFormat::Xrgb8888 => Some(Format::Bgrx),
        libway::PixelFormat::Abgr8888 | libway::PixelFormat::Xbgr8888 => Some(Format::Rgba),
        _ => None,
    }
}

fn tiles(desktop: &libway::DesktopTiles) -> Vec<Tile<'_>> {
    desktop
        .tiles
        .iter()
        .map(|tile| {
            let libway::FrameStorage::Cpu(buffer) = &tile.frame.storage;
            Tile {
                pixels: buffer.bytes(),
                format: format(&tile.frame).expect("checked when captured"),
                x: tile.x,
                y: tile.y,
                width: tile.frame.width,
                height: tile.frame.height,
                stride: tile.frame.stride as usize,
            }
        })
        .collect()
}

fn encode_tiles(desktop: &libway::DesktopTiles) -> Result<Vec<u8>, String> {
    png::encode(desktop.width, desktop.height, &tiles(desktop))
}

/// The card thumbnail straight from the tiles of a `width` × `height`
/// desktop: every `step`-th pixel into an image twice the card's size or
/// more, which the usual box filter then shrinks, instead of reading all of
/// the desktop's pixels.
fn preview(tiles: &[Tile], width: u32, height: u32, card_w: u32, card_h: u32) -> image::RgbaImage {
    let step = (width / (card_w * 2)).min(height / (card_h * 2)).max(1);
    let mut small = image::RgbImage::new(width / step, height / step);
    let (small_w, small_h) = small.dimensions();
    for tile in tiles {
        let bytes = if tile.format == Format::Rgb { 3 } else { 4 };
        let ys = tile.y.div_ceil(step)..(tile.y + tile.height).div_ceil(step).min(small_h);
        let xs = tile.x.div_ceil(step)..(tile.x + tile.width).div_ceil(step).min(small_w);
        for y in ys {
            let row = &tile.pixels[(y * step - tile.y) as usize * tile.stride..];
            for x in xs.clone() {
                let p = &row[(x * step - tile.x) as usize * bytes..][..3];
                let rgb = match tile.format {
                    Format::Bgrx => [p[2], p[1], p[0]],
                    Format::Rgb | Format::Rgba => [p[0], p[1], p[2]],
                };
                small.put_pixel(x, y, image::Rgb(rgb));
            }
        }
    }
    thumbnail(&small, card_w, card_h)
}

fn thumbnail(image: &image::RgbImage, card_w: u32, card_h: u32) -> image::RgbaImage {
    let view =
        image::ImageBuffer::from_raw(image.width(), image.height(), image.as_raw().as_slice())
            .expect("an image views itself");
    crate::shelf::thumbnail::make_rgb_card_thumbnail(&view, card_w, card_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_puts_each_tile_where_it_lies_and_leaves_gaps_black() {
        // A 400 × 100 desktop: red on the left, blue on the right but only
        // its top half, black below it.
        let red: Vec<u8> = [0, 0, 255, 0].repeat(200 * 100);
        let blue: Vec<u8> = [255, 0, 0, 0].repeat(200 * 50);
        let tiles = [
            Tile::whole(&red, Format::Bgrx, 200, 100),
            Tile {
                x: 200,
                ..Tile::whole(&blue, Format::Bgrx, 200, 50)
            },
        ];
        let thumb = preview(&tiles, 400, 100, 40, 10);
        assert_eq!(thumb.dimensions(), (40, 10));
        assert_eq!(thumb.get_pixel(2, 2).0, [255, 0, 0, 255]);
        assert_eq!(thumb.get_pixel(37, 2).0, [0, 0, 255, 255]);
        assert_eq!(thumb.get_pixel(37, 8).0, [0, 0, 0, 255]);
    }
}
