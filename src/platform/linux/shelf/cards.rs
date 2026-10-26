//! What a shelf card is made of: its thumbnail, from a client's PNG or pixels,
//! and where it sits in the stack while cards come and go.

use super::{
    Layout, LayoutConfig, MAX_CACHED_IMAGE_BYTES, MAX_VISIBLE_CARDS, PreparedAdd, ShelfModel,
    Thumb, ThumbRect,
};

pub(super) fn visible_cards(model: &ShelfModel) -> impl Iterator<Item = &Thumb> {
    model.newest_first().take(MAX_VISIBLE_CARDS)
}

pub(super) fn overflow_card_opacity(
    position: usize,
    appear_progress: Option<f32>,
    disappear_progresses: &[f32],
) -> Option<f32> {
    let mut overflow = position.checked_sub(MAX_VISIBLE_CARDS)?;
    if overflow == 0
        && let Some(progress) = appear_progress
    {
        return Some(1.0 - progress.clamp(0.0, 1.0));
    }
    overflow = overflow.saturating_sub(usize::from(appear_progress.is_some()));
    disappear_progresses
        .get(overflow)
        .map(|progress| progress.clamp(0.0, 1.0))
}

pub(super) fn animated_item_limit(model_len: usize, appearing: bool, disappearing: usize) -> usize {
    model_len.min(MAX_VISIBLE_CARDS + usize::from(appearing) + disappearing)
}

pub(super) fn animated_card_layout(
    items: &[(u64, u32, u32, f32)],
    cfg: &LayoutConfig,
    appear_progress: Option<f32>,
) -> Layout {
    if items.is_empty() {
        return Layout::compute(&[], cfg);
    }
    let scroll = appear_progress
        .filter(|_| items.len() > MAX_VISIBLE_CARDS)
        .map(|progress| {
            ((items[0].2 + cfg.gap) as f32 * (1.0 - progress.clamp(0.0, 1.0))).round() as u32
        });
    let widest = items
        .iter()
        .take(MAX_VISIBLE_CARDS)
        .map(|(_, w, ..)| *w)
        .max()
        .unwrap_or(0);
    let mut thumbs = Vec::with_capacity(items.len());
    let mut y = cfg.pad;
    for (i, (id, w, h, collapse)) in items.iter().enumerate() {
        if i > 0 {
            let gap_collapse = collapse.max(if i == 1 { items[0].3 } else { 0.0 });
            y += (cfg.gap as f32 * (1.0 - gap_collapse)).round() as u32;
        }
        let height = (*h as f32 * (1.0 - collapse)).round() as u32;
        thumbs.push(ThumbRect {
            id: *id,
            x: cfg.pad,
            y: if i > 0 {
                y.saturating_sub(scroll.unwrap_or(0))
            } else {
                y
            },
            w: *w,
            h: height,
        });
        y += height;
    }
    let height = if items.len() > MAX_VISIBLE_CARDS {
        cfg.pad * 2
            + items
                .iter()
                .take(MAX_VISIBLE_CARDS)
                .map(|(_, _, h, _)| *h)
                .sum::<u32>()
            + cfg.gap * (MAX_VISIBLE_CARDS as u32 - 1)
    } else {
        y + cfg.pad
    };
    Layout {
        width: cfg.pad * 2 + widest,
        height,
        thumbs,
    }
}

/// Store a screenshot PNG for a card. With the client's card thumbnail only the
/// PNG header is checked; otherwise the PNG is decoded for the thumbnail.
pub(super) fn prepare_add(
    png: Vec<u8>,
    source: String,
    output: Option<String>,
    thumb: Option<crate::protocol::CardThumb>,
) -> Result<PreparedAdd, String> {
    use crate::shelf::thumbnail::{CARD_H, CARD_W};
    if png.len() as u64 > MAX_CACHED_IMAGE_BYTES {
        return Err("PNG exceeds the 256 MiB shelf cache limit".into());
    }
    let card = thumb
        .filter(|thumb| (thumb.width, thumb.height) == (CARD_W, CARD_H))
        .and_then(|thumb| image::RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba));
    let thumb = match card {
        Some(card) => {
            shelf_png_decoder(&png)?;
            card
        }
        None => crate::shelf::thumbnail::make_image_card_thumbnail(
            &decode_shelf_image(&png)?,
            CARD_W,
            CARD_H,
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
    let thumb = crate::shelf::thumbnail::make_rgb_card_thumbnail(
        &image,
        crate::shelf::thumbnail::CARD_W,
        crate::shelf::thumbnail::CARD_H,
    );
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
