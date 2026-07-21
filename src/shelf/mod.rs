pub mod font;
pub mod layout;
pub mod model;
pub mod paint;
pub mod recording;
pub mod thumbnail;

use crate::DynResult;

pub use crate::platform::shelf::{focused_monitor_origin, run_daemon};

pub fn debug_render(output: &std::path::Path) -> DynResult<()> {
    use image::{Rgba, RgbaImage};

    let (thumbnail_width, thumbnail_height) = (thumbnail::CARD_W, thumbnail::CARD_H);
    let mut sample = RgbaImage::new(thumbnail_width, thumbnail_height);
    for (x, y, pixel) in sample.enumerate_pixels_mut() {
        *pixel = Rgba([
            (x * 255 / thumbnail_width) as u8,
            (y * 200 / thumbnail_height) as u8,
            170,
            255,
        ]);
    }

    let mut model = model::ShelfModel::new();
    let id = model.add(
        std::path::PathBuf::from("sample.png"),
        sample,
        "area".into(),
    );
    let config = layout::LayoutConfig::default();
    let sizes = model
        .newest_first()
        .map(|thumbnail| {
            (
                thumbnail.id,
                thumbnail.thumb.width(),
                thumbnail.thumb.height(),
            )
        })
        .collect::<Vec<_>>();
    let layout = layout::Layout::compute(&sizes, &config);
    let (width, height) = (layout.width, layout.height);
    let mut canvas = vec![0_u8; (width * height * 4) as usize];
    paint::draw_shelf(
        &mut canvas,
        width,
        height,
        &layout,
        &model,
        Some(id),
        &config,
        &[],
        None,
    );

    let background = 64_u32;
    let mut image = RgbaImage::new(width, height);
    for (index, pixel) in canvas.chunks_exact(4).enumerate() {
        let (blue, green, red, alpha) = (
            pixel[0] as u32,
            pixel[1] as u32,
            pixel[2] as u32,
            pixel[3] as u32,
        );
        let inverse = 255 - alpha;
        let red = (red + background * inverse / 255).min(255) as u8;
        let green = (green + background * inverse / 255).min(255) as u8;
        let blue = (blue + background * inverse / 255).min(255) as u8;
        image.put_pixel(
            (index as u32) % width,
            (index as u32) / width,
            Rgba([red, green, blue, 255]),
        );
    }
    image.save(output)?;
    Ok(())
}
