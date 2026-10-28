//! The snap search on a page like the probe's (snap plan, P6): a header, a
//! black page, an embedded video with texture, objects and subtitles, a
//! title under it.

use image::{Rgba, RgbaImage};

use super::super::edit::Rect;
use super::{Edges, MIN_SIDE, levels};

const VIDEO: (u32, u32, u32, u32) = (382, 138, 1702, 880);

/// A small, repeatable noise in -8..8.
fn noise(x: u32, y: u32) -> i32 {
    let mut n = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263);
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    (n >> 28) as i32 - 8
}

fn grey(v: i32) -> Rgba<u8> {
    let v = v.clamp(0, 255) as u8;
    Rgba([v, v, v, 255])
}

fn fill(image: &mut RgbaImage, (x0, y0, x1, y1): (u32, u32, u32, u32), colour: Rgba<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            image.put_pixel(x, y, colour);
        }
    }
}

/// Words: a row of letters, each a white block with a black outline.
fn words(image: &mut RgbaImage, x0: u32, y0: u32, letters: u32) {
    for i in 0..letters {
        let x = x0 + i * 18;
        fill(image, (x, y0, x + 14, y0 + 24), grey(0));
        fill(image, (x + 2, y0 + 2, x + 12, y0 + 22), grey(255));
    }
}

/// The page, with the video's content `base` bright.
fn page(base: i32) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(1920, 1080, grey(0));
    fill(&mut image, (0, 0, 1920, VIDEO.1), grey(0x27));
    words(&mut image, 40, 50, 12);
    let (x0, y0, x1, y1) = VIDEO;
    for y in y0..y1 {
        for x in x0..x1 {
            // A soft gradient with grain: no long straight edges.
            let v = base + ((x - x0) as i32 / 40) + noise(x, y);
            image.put_pixel(x, y, grey(v));
        }
    }
    // Objects in the picture: a bright lamp, a dark shelf.
    fill(&mut image, (600, 300, 660, 520), grey(240));
    fill(&mut image, (1300, 600, 1650, 640), grey(20));
    // Subtitles near the bottom, and the title under the video.
    words(&mut image, 700, 800, 30);
    words(&mut image, 520, 905, 40);
    image
}

#[test]
fn the_edges_find_the_embedded_video_from_anywhere_in_it() {
    let edges = Edges::new(&page(120)).unwrap();
    let started = std::time::Instant::now();
    for point in [(1040, 500), (700, 815), (394, 150), (1690, 868)] {
        assert_eq!(edges.around(point), [VIDEO], "from {point:?}");
    }
    // On a sharp shape in the picture: the shape first, the video next out.
    assert_eq!(edges.around((630, 400)), [(600, 300, 660, 520), VIDEO]);
    println!("five searches: {:?}", started.elapsed());
}

#[test]
fn text_and_the_whole_screen_are_no_inner_rectangle() {
    let image = page(120);
    let started = std::time::Instant::now();
    let edges = Edges::new(&image).unwrap();
    println!("edges of 1920 x 1080: {:?}", started.elapsed());
    // On the title's letters: never the video, never a letter.
    for point in [(700, 915), (530, 910), (60, 60)] {
        for (x0, y0, x1, y1) in edges.around(point) {
            assert_ne!((x0, y0, x1, y1), VIDEO, "from {point:?}");
            assert!(x1 - x0 >= MIN_SIDE && y1 - y0 >= MIN_SIDE, "from {point:?}");
        }
    }
    // A video that fills the screen is the screen's level, not an inner one.
    let full = RgbaImage::from_fn(640, 360, |x, y| grey(120 + noise(x, y)));
    assert!(Edges::new(&full).unwrap().around((320, 180)).is_empty());
}

#[test]
fn a_border_too_dark_to_see_never_cuts_into_the_video() {
    // Content as dark as the page: its left and right borders vanish.
    let edges = Edges::new(&page(4)).unwrap();
    for (x0, y0, x1, y1) in edges.around((1040, 500)) {
        let (vx0, vy0, vx1, vy1) = VIDEO;
        assert!(x0 <= vx0 && y0 <= vy0 && x1 >= vx1 && y1 >= vy1);
    }
}

/// A terminal: rules across its whole width, its sides too faint to see
/// (a translucent window over the wallpaper). Bands between the rules are no
/// rectangles, so nothing inner is offered and the window comes first.
#[test]
fn bands_between_lines_across_the_window_are_no_rectangle() {
    let mut image = RgbaImage::from_fn(1920, 1080, |x, y| grey(30 + noise(x, y) / 4));
    for y in [300, 609, 889] {
        fill(&mut image, (0, y, 1920, y + 1), grey(200));
    }
    let edges = Edges::new(&image).unwrap();
    for point in [(400, 200), (960, 600), (1800, 900)] {
        assert!(edges.around(point).is_empty(), "from {point:?}");
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn levels_go_from_inside_out_each_once_and_only_where_the_pointer_is() {
    let video = rect(382.0, 138.0, 1320.0, 742.0);
    let window = rect(10.0, 38.0, 1900.0, 1032.0);
    let other = rect(1930.0, 38.0, 1900.0, 1032.0);
    let screen = rect(0.0, 0.0, 1920.0, 1080.0);
    let at = (1040.0, 500.0);
    assert_eq!(
        levels(&[video], &[other, window], screen, at),
        [video, window, screen]
    );
    // Outside the video only the window and the screen hold the pointer.
    assert_eq!(
        levels(&[video], &[window], screen, (100.0, 1000.0)),
        [window, screen]
    );
    // A window as large as the screen (within 4 px) is one level.
    let full = rect(2.0, 0.0, 1918.0, 1078.0);
    assert_eq!(levels(&[], &[full], screen, at), [full]);
    // The picture's reading of a window's edges, 2 px inside it: the
    // window's own rectangle is what shows.
    let read = rect(12.0, 40.0, 1896.0, 1028.0);
    assert_eq!(levels(&[read], &[window], screen, at), [window, screen]);
    // Nothing inside, no windows: the screen alone.
    assert_eq!(levels(&[], &[], screen, at), [screen]);
}
