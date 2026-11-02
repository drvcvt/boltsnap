//! The README's pictures, drawn by Boltsnap's own painters without a desktop.
//! `tools/readme_shots.sh` runs every step:
//!
//! - `desktop BACK.png FRONT.png OUT.png`: two settings pages as floating
//!   windows on a plain wallpaper, the backdrop for the `selector_shots` test
//!   (`SEL_BASE`).
//! - `shelf OUT.png CARD...`: the shelf with these pictures as cards, the
//!   first hovered, at twice its size over the wallpaper. A card is
//!   `PATH` or `PATH@X,Y,W`: the part `W` wide at (`X`, `Y`).
use boltsnap::shelf_card::layout::{Layout, LayoutConfig};
use boltsnap::shelf_card::paint::{Frame, draw_shelf};
use boltsnap::shelf_card::{CardKind, CardLook};
use image::{RgbaImage, imageops};
use std::path::Path;

const SCREEN: (u32, u32) = (1920, 1080);
/// Where the two windows float, (x, y, w, h), the second in front; the
/// selector shots' region lies across both and the wallpaper.
const WINDOWS: [(u32, u32, u32, u32); 2] = [(140, 140, 900, 760), (1100, 240, 680, 600)];
/// Hyprland's default window rounding.
const ROUNDING: f32 = 10.0;
/// The wallpaper's colours, top and bottom: light enough that the selector's
/// dimming shows against the dark windows.
const TOP: [f32; 3] = [62.0, 84.0, 110.0];
const BOTTOM: [f32; 3] = [104.0, 126.0, 150.0];
/// The shelf picture: device pixels per shelf pixel, the card size and the
/// room around the stack.
const SCALE: f32 = 2.0;
const CARD: (u32, u32) = (190, 132);
const MARGIN: u32 = 64;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode, left, right, out] if mode == "desktop" => {
            let pages = [load(left), load(right)];
            save(&desktop(&pages), out);
        }
        [mode, out, cards @ ..] if mode == "shelf" && !cards.is_empty() => {
            let cards: Vec<RgbaImage> = cards.iter().map(|card| part(card)).collect();
            save(&shelf(&cards), out);
        }
        _ => {
            eprintln!("usage: readme_shots desktop LEFT.png RIGHT.png OUT.png");
            eprintln!("       readme_shots shelf OUT.png PATH[@X,Y,W]...");
            std::process::exit(2);
        }
    }
}

fn load(path: &str) -> RgbaImage {
    image::open(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgba8()
}

/// `PATH`, or with `@X,Y,W` its part `W` wide at (`X`, `Y`) in the card's
/// proportions.
fn part(card: &str) -> RgbaImage {
    let Some((path, at)) = card.split_once('@') else {
        return load(card);
    };
    let numbers: Vec<u32> = at.split(',').map(|n| n.parse().expect(card)).collect();
    let [x, y, w] = numbers[..] else {
        panic!("{card}: X,Y,W");
    };
    imageops::crop_imm(&load(path), x, y, w, w * CARD.1 / CARD.0).to_image()
}

fn save(image: &RgbaImage, path: &str) {
    image
        .save(Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
}

/// The wallpaper with each page as a rounded window where `WINDOWS` puts it.
fn desktop(pages: &[RgbaImage; 2]) -> RgbaImage {
    let mut screen = backdrop(SCREEN.0, SCREEN.1);
    for (page, (x, y, w, h)) in pages.iter().zip(WINDOWS) {
        assert_eq!(page.dimensions(), (w, h), "render the pages at {w}x{h}");
        for (px, py, pixel) in page.enumerate_pixels() {
            let cover = corner_cover(px as f32 + 0.5, py as f32 + 0.5, w as f32, h as f32);
            if cover > 0.0 {
                blend(screen.get_pixel_mut(x + px, y + py), pixel.0, cover);
            }
        }
    }
    screen
}

/// A vertical gradient from `TOP` to `BOTTOM`, dithered so its few 8-bit
/// steps do not show as bands.
fn backdrop(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let t = y as f32 / (h - 1) as f32;
        // Interleaved gradient noise, as the shelf's shadow dithers.
        let noise = (52.982_918 * (0.067_110_56 * x as f32 + 0.005_837_15 * y as f32).fract())
            .fract()
            - 0.5;
        let channel = |c: usize| (TOP[c] + (BOTTOM[c] - TOP[c]) * t + noise).round() as u8;
        image::Rgba([channel(0), channel(1), channel(2), 255])
    })
}

/// How much of the pixel centred at (`x`, `y`) lies inside a `w` × `h`
/// rectangle with `ROUNDING` corners.
fn corner_cover(x: f32, y: f32, w: f32, h: f32) -> f32 {
    let dx = (x.min(w - x) - ROUNDING).min(0.0);
    let dy = (y.min(h - y) - ROUNDING).min(0.0);
    let outside = (dx * dx + dy * dy).sqrt() - ROUNDING;
    (0.5 - outside).clamp(0.0, 1.0)
}

fn blend(under: &mut image::Rgba<u8>, over: [u8; 4], cover: f32) {
    let alpha = cover * f32::from(over[3]) / 255.0;
    for (dst, src) in under.0.iter_mut().zip(&over[..3]) {
        *dst = (f32::from(*src) * alpha + f32::from(*dst) * (1.0 - alpha)).round() as u8;
    }
}

/// The shelf with `cards`, newest first and hovered, as the overlay paints
/// it at `SCALE`, over the wallpaper.
fn shelf(cards: &[RgbaImage]) -> RgbaImage {
    let scaled = |v: u32| (v as f32 * SCALE).round() as u32;
    let size = (scaled(CARD.0), scaled(CARD.1));
    let thumbs: Vec<RgbaImage> = cards.iter().map(|card| fill(card, size)).collect();
    let base = LayoutConfig::default();
    let cfg = LayoutConfig {
        pad: scaled(base.pad),
        gap: scaled(base.gap),
        icon: scaled(base.icon),
        pad_icon: scaled(base.pad_icon),
    };
    let sizes: Vec<(u64, u32, u32)> = (0..thumbs.len() as u64)
        .map(|id| (id, size.0, size.1))
        .collect();
    let layout = Layout::compute(&sizes, &cfg);
    let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
    let frame = Frame {
        layout: &layout,
        cfg: &cfg,
        hovered: &[(0, 1.0)],
        anims: &[],
        save_flash: None,
        save_button: true,
        saving: &[],
        spin: 0.0,
        look: CardLook::default(),
        scale: SCALE,
    };
    draw_shelf(&mut canvas, layout.width, layout.height, &frame, |id| {
        thumbs
            .get(id as usize)
            .map(|thumb| (thumb, CardKind::Image))
    });
    let mut out = backdrop(layout.width + 2 * MARGIN, layout.height + 2 * MARGIN);
    for (i, bgra) in canvas.as_chunks::<4>().0.iter().enumerate() {
        let x = MARGIN + i as u32 % layout.width;
        let y = MARGIN + i as u32 / layout.width;
        // Premultiplied BGRA over the wallpaper.
        let under = out.get_pixel_mut(x, y);
        let keep = 1.0 - f32::from(bgra[3]) / 255.0;
        for (c, src) in [bgra[2], bgra[1], bgra[0]].into_iter().enumerate() {
            under.0[c] = (f32::from(src) + f32::from(under.0[c]) * keep).round() as u8;
        }
    }
    out
}

/// `image` cut to the card's proportions around its centre and shrunk to
/// `size`, as a screenshot fills its card.
fn fill(image: &RgbaImage, size: (u32, u32)) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (cw, ch) = if w * size.1 > h * size.0 {
        (h * size.0 / size.1, h)
    } else {
        (w, w * size.1 / size.0)
    };
    let part = imageops::crop_imm(image, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image();
    imageops::resize(&part, size.0, size.1, imageops::FilterType::Lanczos3)
}
