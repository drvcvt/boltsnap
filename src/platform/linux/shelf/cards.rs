//! What a shelf card is made of: its thumbnail, from a client's PNG or pixels,
//! and where it sits in the stack while cards come and go.

use std::sync::atomic::{AtomicU32, Ordering};

use super::thumbnail::Source;
use super::{Anchor, Daemon, MAX_CACHED_IMAGE_BYTES, PreparedAdd, ShelfModel, Thumb};
use crate::config::ShelfPrefs;
use crate::shelf::layout::Corner;
use crate::shelf::model::CardKind;
use crate::shelf::thumbnail::CardSize;
pub(super) use boltsnap::shelf_card::layout::{
    animated_card_layout, animated_item_limit, hold_size, overflow_card_opacity,
};

/// `[shelf] card_width` as the daemon applies it. Client threads and the
/// thumbnail worker read it; only `apply_shelf_prefs` writes it.
static CARD_WIDTH: AtomicU32 = AtomicU32::new(CardSize::DEFAULT.w);

pub(super) fn use_card_size(size: CardSize) {
    CARD_WIDTH.store(size.w, Ordering::Relaxed);
}

pub(super) fn card_size() -> CardSize {
    CardSize::from_width(CARD_WIDTH.load(Ordering::Relaxed).into()).unwrap_or(CardSize::DEFAULT)
}

/// Gap between the shelf and both screen edges of its corner.
const EDGE_MARGIN: i32 = 24;

/// Layer anchor and margins `[top, right, bottom, left]` for `corner`.
pub(super) fn anchor(corner: Corner) -> (Anchor, [i32; 4]) {
    let (top, right) = (corner.is_top(), corner.is_right());
    let margin = |on: bool| if on { EDGE_MARGIN } else { 0 };
    (
        (if top { Anchor::TOP } else { Anchor::BOTTOM })
            | (if right { Anchor::RIGHT } else { Anchor::LEFT }),
        [margin(top), margin(right), margin(!top), margin(!right)],
    )
}

impl Daemon {
    /// Takes `[shelf]` as it is now. A new corner needs a new layer surface;
    /// the cards stay.
    pub(super) fn apply_shelf_prefs(&mut self, prefs: ShelfPrefs) {
        let old = std::mem::replace(&mut self.shelf_prefs, prefs);
        if old == prefs {
            return;
        }
        if old.card != prefs.card {
            use_card_size(prefs.card);
            self.resize_cards(prefs.card);
        }
        let Some(qh) = self.qh.clone() else {
            return;
        };
        self.relayout();
        if old.corner != prefs.corner && self.layer.is_some() {
            // Destroying the drag's origin surface would end it without a
            // terminal event.
            self.forget_drag();
            self.layer = None;
            self.place_on_output(None, &qh);
        }
        self.draw(&qh);
    }

    /// Scales every card at once from the image it has, then has each
    /// rendered sharp from its file.
    fn resize_cards(&mut self, size: CardSize) {
        let started = std::time::Instant::now();
        let cards: Vec<_> = self
            .model
            .newest_first()
            .map(|card| {
                (
                    card.id,
                    crate::shelf::thumbnail::rescale(&card.thumb, size),
                    source(card),
                )
            })
            .collect();
        let count = cards.len();
        let mut sources = Vec::with_capacity(count);
        for (id, thumb, source) in cards {
            self.model.replace_thumb(id, thumb);
            sources.push((id, source));
        }
        let took = started.elapsed();
        if took > std::time::Duration::from_millis(8) {
            eprintln!(
                "boltsnap daemon: scaling {count} cards to {}x{} took {took:?}",
                size.w, size.h
            );
        }
        self.thumbnails.rerender(sources);
    }

    /// A card made at an older size, because the size changed while it was
    /// on its way: scaled now, rendered sharp after.
    pub(super) fn fit_card(&mut self, id: u64) {
        let size = card_size();
        let Some(card) = self.model.get(id) else {
            return;
        };
        if card.thumb.dimensions() == size.dims() {
            return;
        }
        let (thumb, source) = (
            crate::shelf::thumbnail::rescale(&card.thumb, size),
            source(card),
        );
        self.model.replace_thumb(id, thumb);
        self.thumbnails.rerender([(id, source)]);
    }
}

fn source(card: &Thumb) -> Source {
    match card.kind {
        CardKind::Image => Source::Image(card.png_path.clone()),
        CardKind::Video => Source::Video(card.png_path.clone()),
    }
}

pub(super) fn visible_cards(model: &ShelfModel, visible: usize) -> impl Iterator<Item = &Thumb> {
    model.newest_first().take(visible)
}

/// Store a screenshot PNG for a card. With the client's card thumbnail in
/// `size` only the PNG header is checked; otherwise the PNG is decoded for the
/// thumbnail.
pub(super) fn prepare_add(
    png: Vec<u8>,
    source: String,
    output: Option<String>,
    thumb: Option<crate::protocol::CardThumb>,
    size: CardSize,
) -> Result<PreparedAdd, String> {
    if png.len() as u64 > MAX_CACHED_IMAGE_BYTES {
        return Err("PNG exceeds the 256 MiB shelf cache limit".into());
    }
    let card = thumb
        .filter(|thumb| (thumb.width, thumb.height) == size.dims())
        .and_then(|thumb| image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba));
    let thumb = match card {
        Some(card) => {
            shelf_png_decoder(&png)?;
            card
        }
        None => crate::shelf::thumbnail::make_image_card_thumbnail(
            &decode_shelf_image(&png)?,
            size.w,
            size.h,
        ),
    };
    let path = write_temp_png("shelf", &png)?;
    Ok(PreparedAdd {
        path,
        thumb,
        trace: None,
        source,
        output,
    })
}

pub(super) fn write_temp_png(prefix: &str, png: &[u8]) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let path = crate::paths::temp_png(prefix);
    // Never truncate an existing file or follow a symlink in the temporary dir.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("create {prefix} PNG: {e}"))?;
    if let Err(e) = file.write_all(png) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("write {prefix} PNG: {e}"));
    }
    Ok(path)
}

pub(super) fn prepare_pixels(
    metadata: super::super::image_transfer::Metadata,
    pixels: &super::super::image_transfer::Pixels,
) -> Result<PreparedAdd, String> {
    prepare_pixels_with(
        metadata,
        pixels,
        card_size(),
        |png| {
            crate::paths::publish_last_png(png)
                .map(|_| ())
                .map_err(|e| format!("publish last PNG: {e}"))
        },
        |path| {
            super::super::clipboard::copy_temporary_to_clipboard(path, crate::Backend::Wayland)
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
    )
}

pub(super) fn prepare_pixels_with(
    metadata: super::super::image_transfer::Metadata,
    pixels: &super::super::image_transfer::Pixels,
    size: CardSize,
    publish: impl FnOnce(&[u8]) -> Result<(), String>,
    copy: impl FnOnce(&std::path::Path) -> Result<(), String>,
) -> Result<PreparedAdd, String> {
    use image::ImageEncoder;
    let _timing = super::super::timing::Span::for_request("shelf_pixels_prepare", metadata.trace);
    let image = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(
        metadata.width,
        metadata.height,
        pixels.bytes(),
    )
    .ok_or("invalid RGB image")?;
    let thumb = crate::shelf::thumbnail::make_rgb_card_thumbnail(&image, size.w, size.h);
    super::super::timing::mark_request("thumbnail_ready", metadata.trace);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            pixels.bytes(),
            metadata.width,
            metadata.height,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("encode shelf PNG: {e}"))?;
    let add = PreparedAdd {
        path: write_temp_png("shelf", &png)?,
        thumb,
        trace: metadata.trace,
        source: metadata.source,
        output: metadata.output,
    };
    super::super::timing::mark_request("png_written", metadata.trace);
    publish(&png)?;
    if metadata.copy {
        let clipboard = write_temp_png("clipboard", &png)
            .map_err(|e| format!("last PNG saved, shelf item not added: {e}"))?;
        if let Err(e) = copy(&clipboard) {
            let _ = std::fs::remove_file(&clipboard);
            return Err(format!(
                "last PNG saved, clipboard helper failed, shelf item not added: {e}"
            ));
        }
    }
    Ok(add)
}

pub(super) fn decode_shelf_image(png: &[u8]) -> Result<image::DynamicImage, String> {
    image::DynamicImage::from_decoder(shelf_png_decoder(png)?)
        .map_err(|e| format!("decode PNG: {e}"))
}

/// A PNG decoder after checking the header against the shelf's pixel limit.
pub(super) fn shelf_png_decoder(png: &[u8]) -> Result<impl image::ImageDecoder + '_, String> {
    use image::ImageDecoder;
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_CACHED_IMAGE_BYTES);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|e| format!("decode PNG: {e}"))?;
    let (width, height) = decoder.dimensions();
    // Covers RGBA conversion too, including small grayscale PNGs expanding to
    // huge images. Accepts 8K and wide multi-monitor captures up to 64M pixels.
    if u64::from(width) * u64::from(height) > MAX_CACHED_IMAGE_BYTES / 4 {
        return Err("decode PNG: image exceeds the 64 megapixel limit".into());
    }
    Ok(decoder)
}
