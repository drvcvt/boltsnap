use super::*;

/// RGB of every pixel, decoded by the `image` crate.
fn decoded(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .expect("valid PNG")
        .into_rgb8();
    (image.width(), image.height(), image.into_raw())
}

/// The zlib stream of all IDAT chunks inflated by a second decoder, which
/// also checks the Adler-32; and every chunk's CRC.
fn inflated(png: &[u8]) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut at = 8;
    while at < png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let body = &png[at + 4..at + 8 + length];
        let crc = u32::from_be_bytes(png[at + 8 + length..at + 12 + length].try_into().unwrap());
        assert_eq!(crc32fast::hash(body), crc, "CRC of {:?}", &body[..4]);
        if &body[..4] == b"IDAT" {
            stream.extend_from_slice(&body[4..]);
        }
        at += 12 + length;
    }
    miniz_oxide::inflate::decompress_to_vec_zlib(&stream).expect("valid zlib with Adler-32")
}

/// Bytes that look a bit like a desktop: flat areas, a gradient, noise.
fn sample(width: u32, height: u32, bytes: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_u32;
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state >> 24) as u8;
            let value = match (x / 16 + y / 16) % 3 {
                0 => 40,
                1 => (x + y) as u8,
                _ => noise,
            };
            std::iter::repeat_n(value, bytes)
                .enumerate()
                .map(|(c, v)| v.wrapping_add(c as u8 * 50))
        })
        .collect()
}

fn round_trip(width: u32, height: u32) {
    let rgb = sample(width, height, 3);
    let png = encode(
        width,
        height,
        &[Tile::whole(&rgb, Format::Rgb, width, height)],
    );
    assert_eq!(
        decoded(&png),
        (width, height, rgb.clone()),
        "{width} × {height}"
    );
    let raw = inflated(&png);
    assert_eq!(raw.len(), height as usize * (1 + width as usize * 3));
}

#[test]
fn every_shape_comes_back_pixel_for_pixel() {
    for (width, height) in [
        (1, 1),
        (1, 300),
        (300, 1),
        (7, 5),
        (257, 31),
        (640, 480),
        (1920, 33),
    ] {
        round_trip(width, height);
    }
}

#[test]
fn flat_and_noisy_pictures_come_back() {
    let flat = vec![200; 512 * 400 * 3];
    let png = encode(512, 400, &[Tile::whole(&flat, Format::Rgb, 512, 400)]);
    assert_eq!(decoded(&png).2, flat);
    // A flat picture is almost all runs.
    assert!(png.len() < 4096, "{} bytes", png.len());
    let mut state = 1_u32;
    let noise: Vec<u8> = (0..300 * 200 * 3)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    let png = encode(300, 200, &[Tile::whole(&noise, Format::Rgb, 300, 200)]);
    assert_eq!(decoded(&png).2, noise);
    inflated(&png);
}

#[test]
fn rgba_and_bgrx_lose_only_their_fourth_byte() {
    let (width, height) = (123, 77);
    let rgba = sample(width, height, 4);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let png = encode(
        width,
        height,
        &[Tile::whole(&rgba, Format::Rgba, width, height)],
    );
    assert_eq!(decoded(&png).2, rgb);
    let bgrx: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[2], p[1], p[0], 9])
        .collect();
    let png = encode(
        width,
        height,
        &[Tile::whole(&bgrx, Format::Bgrx, width, height)],
    );
    assert_eq!(decoded(&png).2, rgb);
}

#[test]
fn tiles_land_in_place_with_padded_rows_and_black_gaps() {
    // Two monitors side by side, the right one shorter: a gap below it.
    let left = sample(40, 50, 4);
    let mut right = sample(30, 20, 4);
    // Rows padded to 32 pixels.
    right = right
        .chunks_exact(30 * 4)
        .flat_map(|row| [row, &[7; 8][..]].concat())
        .collect();
    let tiles = [
        Tile::whole(&left, Format::Bgrx, 40, 50),
        Tile {
            pixels: &right,
            format: Format::Bgrx,
            x: 40,
            y: 0,
            width: 30,
            height: 20,
            stride: 32 * 4,
        },
    ];
    let (_, _, rgb) = decoded(&encode(70, 50, &tiles));
    let pixel = |x: usize, y: usize| &rgb[(y * 70 + x) * 3..][..3];
    let bgrx = |p: &[u8]| [p[2], p[1], p[0]];
    assert_eq!(pixel(39, 49), bgrx(&left[(49 * 40 + 39) * 4..]));
    assert_eq!(pixel(40, 0), bgrx(&right[..4]));
    assert_eq!(pixel(69, 19), bgrx(&right[19 * 128 + 29 * 4..]));
    assert_eq!(pixel(55, 30), [0, 0, 0]);
}

#[test]
fn adler_of_two_pieces_is_the_adler_of_both() {
    let data = sample(97, 13, 3);
    for split in [0, 1, 1000, data.len()] {
        let (a, b) = data.split_at(split);
        assert_eq!(
            adler_combine(adler32(a), adler32(b), b.len()),
            adler32(&data)
        );
    }
}

/// Local measurement on a real screenshot: `BOLTSNAP_PNG_SAMPLE=shot.png
/// cargo test --release --lib png::tests::speed -- --ignored --nocapture`.
#[test]
#[ignore]
fn speed() {
    let path = std::env::var("BOLTSNAP_PNG_SAMPLE").expect("BOLTSNAP_PNG_SAMPLE");
    let image = image::open(path).unwrap().into_rgb8();
    let (width, height) = image.dimensions();
    let ms = |start: std::time::Instant| start.elapsed().as_secs_f64() * 1000.0;
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let ours = encode(
            width,
            height,
            &[Tile::whole(image.as_raw(), Format::Rgb, width, height)],
        );
        let ours_ms = ms(start);
        let start = std::time::Instant::now();
        let mut theirs = Vec::new();
        image::DynamicImage::ImageRgb8(image.clone())
            .write_to(
                &mut std::io::Cursor::new(&mut theirs),
                image::ImageFormat::Png,
            )
            .unwrap();
        let theirs_ms = ms(start);
        println!(
            "ours {ours_ms:.1} ms {:.2} MB | image {theirs_ms:.1} ms {:.2} MB",
            ours.len() as f64 / 1e6,
            theirs.len() as f64 / 1e6
        );
        assert_eq!(decoded(&ours).2, *image.as_raw());
    }
}
