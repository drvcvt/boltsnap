//! A PNG encoder for screenshots that uses every core. The picture is cut
//! into bands of rows; each band is filtered and deflated on its own thread
//! into its own IDAT chunk. The bands still form one zlib stream: every band
//! but the last ends on a byte with an empty stored block (a sync flush), and
//! the Adler-32 is combined from the bands' own.
//!
//! The picture comes as tiles (one per monitor, say) in the layout they were
//! captured in; rows are put together and turned into RGB while filtering,
//! so no full-size copy is made first. Gaps between tiles are black.

mod deflate;
mod filter;

use deflate::{Bits, deflate};

/// Fewer rows than this are not worth a thread of their own.
const MIN_BAND_ROWS: usize = 32;
const ADLER_MOD: u32 = 65521;

/// How a tile's pixels are laid out in memory, 8 bits a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgb,
    /// Alpha is left out.
    Rgba,
    /// Wayland's XRGB8888 and ARGB8888 on a little-endian machine; the
    /// fourth byte is left out.
    Bgrx,
}

impl Format {
    fn bytes(self) -> usize {
        match self {
            Format::Rgb => 3,
            Format::Rgba | Format::Bgrx => 4,
        }
    }
}

/// Part of the picture: `width` × `height` pixels at (`x`, `y`), rows
/// `stride` bytes apart.
#[derive(Clone, Copy, Debug)]
pub struct Tile<'a> {
    pub pixels: &'a [u8],
    pub format: Format,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
}

impl<'a> Tile<'a> {
    /// A whole image of tightly packed rows.
    pub fn whole(pixels: &'a [u8], format: Format, width: u32, height: u32) -> Self {
        Self {
            pixels,
            format,
            x: 0,
            y: 0,
            width,
            height,
            stride: width as usize * format.bytes(),
        }
    }
}

/// An 8-bit RGB PNG of the `width` × `height` picture made of `tiles`, which
/// must lie inside it and not overlap.
pub fn encode(width: u32, height: u32, tiles: &[Tile]) -> Vec<u8> {
    assert!(width > 0 && height > 0, "PNG needs at least one pixel");
    for tile in tiles {
        assert!(tile.x + tile.width <= width && tile.y + tile.height <= height);
        assert!(tile.stride >= tile.width as usize * tile.format.bytes());
        let rows = tile.height as usize;
        let needed =
            rows.saturating_sub(1) * tile.stride + tile.width as usize * tile.format.bytes();
        assert!(
            rows == 0 || tile.pixels.len() >= needed,
            "tile pixels match its size"
        );
    }
    let area: u64 = tiles
        .iter()
        .map(|t| u64::from(t.width) * u64::from(t.height))
        .sum();
    let picture = Picture {
        width: width as usize,
        tiles,
        covered: area == u64::from(width) * u64::from(height),
    };
    let rows = height as usize;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let bands = cores.min(rows.div_ceil(MIN_BAND_ROWS)).max(1);
    let band = |index: usize| {
        let range = rows * index / bands..rows * (index + 1) / bands;
        encode_band(&picture, range, index == 0, index + 1 == bands)
    };
    let encoded: Vec<Band> = std::thread::scope(|scope| {
        let workers: Vec<_> = (1..bands).map(|i| scope.spawn(move || band(i))).collect();
        let first = band(0);
        std::iter::once(first)
            .chain(
                workers
                    .into_iter()
                    .map(|w| w.join().expect("PNG band thread")),
            )
            .collect()
    });

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // 8 bits, RGB, deflate, filter method 0, no interlace.
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let adler = encoded
        .iter()
        .fold(1, |sum, band| adler_combine(sum, band.adler, band.filtered));
    let size: usize = encoded.iter().map(|band| band.chunk.len()).sum();
    let mut png = Vec::with_capacity(size + 64);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    chunk(&mut png, b"IHDR", &ihdr);
    for band in &encoded {
        png.extend_from_slice(&band.chunk);
    }
    chunk(&mut png, b"IDAT", &adler.to_be_bytes());
    chunk(&mut png, b"IEND", &[]);
    png
}

struct Picture<'a> {
    width: usize,
    tiles: &'a [Tile<'a>],
    /// The tiles fill the picture, so no row needs blacking out first.
    covered: bool,
}

impl Picture<'_> {
    /// Row `y` as RGB into `rgb`.
    fn row(&self, y: usize, rgb: &mut [u8]) {
        if !self.covered {
            rgb.fill(0);
        }
        for tile in self.tiles {
            let top = tile.y as usize;
            if y < top || y >= top + tile.height as usize {
                continue;
            }
            let bytes = tile.format.bytes();
            let source = &tile.pixels[(y - top) * tile.stride..][..tile.width as usize * bytes];
            let target = &mut rgb[tile.x as usize * 3..][..tile.width as usize * 3];
            match tile.format {
                Format::Rgb => target.copy_from_slice(source),
                Format::Rgba => {
                    for (pixel, out) in source.chunks_exact(4).zip(target.chunks_exact_mut(3)) {
                        out.copy_from_slice(&pixel[..3]);
                    }
                }
                Format::Bgrx => {
                    for (pixel, out) in source.chunks_exact(4).zip(target.chunks_exact_mut(3)) {
                        out.copy_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                    }
                }
            }
        }
    }
}

/// One band's IDAT chunk, and the Adler-32 and length of what it deflated.
struct Band {
    chunk: Vec<u8>,
    adler: u32,
    filtered: usize,
}

fn encode_band(picture: &Picture, rows: std::ops::Range<usize>, first: bool, last: bool) -> Band {
    let rgb_len = picture.width * 3;
    let mut filtered = Vec::with_capacity(rows.len() * (rgb_len + 1));
    let mut previous = vec![0; rgb_len];
    if rows.start > 0 {
        picture.row(rows.start - 1, &mut previous);
    }
    let mut current = vec![0; rgb_len];
    for y in rows {
        picture.row(y, &mut current);
        filter::filter_row(&current, &previous, &mut filtered);
        std::mem::swap(&mut current, &mut previous);
    }

    // Length (set below), type and, in the first band, the zlib header:
    // deflate with a 32 KiB window, fastest level, check bits.
    let mut start = vec![0, 0, 0, 0, b'I', b'D', b'A', b'T'];
    if first {
        start.extend_from_slice(&[0x78, 0x01]);
    }
    let mut bits = Bits::new(start);
    deflate(&filtered, last, &mut bits);
    let mut chunk = bits.finish();
    let length = (chunk.len() - 8) as u32;
    chunk[..4].copy_from_slice(&length.to_be_bytes());
    let crc = crc32fast::hash(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    Band {
        chunk,
        adler: adler32(&filtered),
        filtered: filtered.len(),
    }
}

fn adler32(data: &[u8]) -> u32 {
    let mut adler = simd_adler32::Adler32::new();
    adler.write(data);
    adler.finish()
}

/// The Adler-32 of two pieces back to back, from each piece's own and the
/// second one's length (as zlib's `adler32_combine`).
fn adler_combine(first: u32, second: u32, second_len: usize) -> u32 {
    let m = u64::from(ADLER_MOD);
    let rem = second_len as u64 % m;
    let (a1, b1) = (u64::from(first & 0xffff), u64::from(first >> 16));
    let (a2, b2) = (u64::from(second & 0xffff), u64::from(second >> 16));
    let a = (a1 + a2 + m - 1) % m;
    let b = (rem * a1 + b1 + b2 + m - rem) % m;
    ((b << 16) | a) as u32
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32fast::hash(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

#[cfg(test)]
mod tests;
