//! Default arrow from the user's Xcursor theme, for export-time cursor rendering.

use image::{Rgba, RgbaImage, imageops};
use std::path::PathBuf;

pub struct Cursor {
    /// Straight (not premultiplied) RGBA.
    pub image: RgbaImage,
    pub hotspot: (u32, u32),
}

const IMAGE_TYPE: u32 = 0xfffd_0002;
const NAMES: [&str; 3] = ["left_ptr", "default", "arrow"];

/// Theme arrow at `XCURSOR_SIZE` times `scale` pixels. Falls back to a drawn
/// arrow when no theme provides one, so rendering never drops the cursor.
pub fn arrow(scale: f64) -> Cursor {
    let size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|s| (8..=256).contains(s))
        .unwrap_or(24);
    let target = ((f64::from(size) * scale).round() as u32).clamp(8, 512);
    let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
    find(&theme, &search_path(), 0)
        .and_then(|file| std::fs::read(file).ok())
        .and_then(|bytes| parse(&bytes, target))
        .map(|cursor| resize(cursor, target))
        .unwrap_or_else(|| drawn(target))
}

fn search_path() -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os("XCURSOR_PATH") {
        return std::env::split_paths(&path).collect();
    }
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/share/icons"));
        dirs.push(home.join(".icons"));
    }
    dirs.extend(["/usr/share/icons", "/usr/share/pixmaps"].map(PathBuf::from));
    dirs
}

/// Cursor file for the arrow in `theme`, following `Inherits=` a few levels deep.
fn find(theme: &str, dirs: &[PathBuf], depth: u32) -> Option<PathBuf> {
    if depth > 4 || theme.contains('/') {
        return None;
    }
    for dir in dirs {
        for name in NAMES {
            let file = dir.join(theme).join("cursors").join(name);
            if file.is_file() {
                return Some(file);
            }
        }
    }
    dirs.iter()
        .filter_map(|dir| std::fs::read_to_string(dir.join(theme).join("index.theme")).ok())
        .flat_map(|text| inherits(&text))
        .find_map(|parent| find(&parent, dirs, depth + 1))
}

fn inherits(index: &str) -> Vec<String> {
    index
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Inherits"))
        .filter_map(|rest| rest.trim_start().strip_prefix('='))
        .flat_map(|list| list.split([',', ';']))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// First image of the nominal size closest to `target` from an Xcursor file.
fn parse(bytes: &[u8], target: u32) -> Option<Cursor> {
    if bytes.get(..4)? != b"Xcur" {
        return None;
    }
    let header = u32_at(bytes, 4)? as usize;
    let count = (u32_at(bytes, 12)? as usize).min(4096);
    let entries: Vec<(u32, usize)> = (0..count)
        .filter_map(|i| {
            let entry = header.checked_add(i.checked_mul(12)?)?;
            (u32_at(bytes, entry)? == IMAGE_TYPE).then(|| {
                Some((
                    u32_at(bytes, entry + 4)?,
                    u32_at(bytes, entry + 8)? as usize,
                ))
            })?
        })
        .collect();
    let nominal = entries
        .iter()
        .map(|(size, _)| *size)
        .min_by_key(|size| size.abs_diff(target))?;
    let (_, position) = entries.iter().find(|(size, _)| *size == nominal)?;
    let width = u32_at(bytes, position + 16)?;
    let height = u32_at(bytes, position + 20)?;
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let hotspot = (
        u32_at(bytes, position + 24)?.min(width - 1),
        u32_at(bytes, position + 28)?.min(height - 1),
    );
    let pixels = position + u32_at(bytes, *position)? as usize;
    let data = bytes.get(pixels..pixels + width as usize * height as usize * 4)?;
    let mut image = RgbaImage::new(width, height);
    for (pixel, argb) in image.pixels_mut().zip(data.chunks_exact(4)) {
        let (b, g, r, a) = (argb[0], argb[1], argb[2], argb[3]);
        // Xcursor pixels are premultiplied; FFmpeg's overlay expects straight alpha.
        let straight = |c: u8| {
            if a == 0 {
                0
            } else {
                ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
            }
        };
        *pixel = Rgba([straight(r), straight(g), straight(b), a]);
    }
    Some(Cursor { image, hotspot })
}

fn resize(cursor: Cursor, target: u32) -> Cursor {
    let height = cursor.image.height();
    if height.abs_diff(target) <= 1 || target == 0 {
        return cursor;
    }
    let factor = f64::from(target) / f64::from(height);
    let width = ((f64::from(cursor.image.width()) * factor).round() as u32).max(1);
    Cursor {
        image: imageops::resize(&cursor.image, width, target, imageops::FilterType::Lanczos3),
        hotspot: (
            (f64::from(cursor.hotspot.0) * factor).round() as u32,
            (f64::from(cursor.hotspot.1) * factor).round() as u32,
        ),
    }
}

/// Plain white arrow with a dark outline, hotspot at the tip.
fn drawn(size: u32) -> Cursor {
    let unit = f64::from(size) / 24.0;
    let inside = |x: f64, y: f64, grow: f64| {
        // Arrow outline in 24x24 units: tip, left edge, notch, tail, right wing.
        let points = [
            (1.0, 1.0),
            (1.0, 18.0),
            (5.5, 14.0),
            (9.0, 21.5),
            (12.0, 20.0),
            (8.5, 13.0),
            (14.0, 13.0),
        ];
        contains(&points, x / unit, y / unit, grow)
    };
    let mut image = RgbaImage::new(size, size);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        if inside(fx, fy, -1.0) {
            *pixel = Rgba([255, 255, 255, 255]);
        } else if inside(fx, fy, 0.0) {
            *pixel = Rgba([20, 20, 20, 255]);
        }
    }
    Cursor {
        image,
        hotspot: (unit.round() as u32, unit.round() as u32),
    }
}

/// Point-in-polygon. A negative `grow` insets the polygon: the point and its four
/// neighbours at that distance must all be inside.
fn contains(points: &[(f64, f64)], x: f64, y: f64, grow: f64) -> bool {
    let test = |x: f64, y: f64| {
        let mut inside = false;
        let mut j = points.len() - 1;
        for i in 0..points.len() {
            let ((xi, yi), (xj, yj)) = (points[i], points[j]);
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
            j = i;
        }
        inside
    };
    if grow >= 0.0 {
        return test(x, y);
    }
    let d = -grow;
    [(0.0, 0.0), (d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)]
        .iter()
        .all(|(dx, dy)| test(x + dx, y + dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Xcursor file with one 2x2 image per nominal size.
    fn file(sizes: &[u32]) -> Vec<u8> {
        let mut bytes = b"Xcur".to_vec();
        let header = 16u32;
        bytes.extend(header.to_le_bytes());
        bytes.extend(0x1_0000u32.to_le_bytes());
        bytes.extend((sizes.len() as u32).to_le_bytes());
        let toc_end = header as usize + sizes.len() * 12;
        let chunk = 36 + 16;
        for (i, size) in sizes.iter().enumerate() {
            bytes.extend(IMAGE_TYPE.to_le_bytes());
            bytes.extend(size.to_le_bytes());
            bytes.extend(((toc_end + i * chunk) as u32).to_le_bytes());
        }
        for size in sizes {
            for value in [36, IMAGE_TYPE, *size, 1, 2, 2, 1, 1, 0] {
                bytes.extend(value.to_le_bytes());
            }
            // Premultiplied ARGB, stored B G R A: half-transparent white, then opaque red.
            bytes.extend([128, 128, 128, 128, 0, 0, 255, 255]);
            bytes.extend([0, 0, 0, 0, 0, 0, 0, *size as u8]);
        }
        bytes
    }

    #[test]
    fn parses_the_closest_size_and_unpremultiplies() {
        let bytes = file(&[24, 48]);
        let cursor = parse(&bytes, 40).unwrap();
        assert_eq!(cursor.image.dimensions(), (2, 2));
        assert_eq!(cursor.hotspot, (1, 1));
        assert_eq!(cursor.image.get_pixel(0, 0).0, [255, 255, 255, 128]);
        assert_eq!(cursor.image.get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert_eq!(cursor.image.get_pixel(1, 1)[3], 48);
        assert_eq!(parse(&bytes, 20).unwrap().image.get_pixel(1, 1)[3], 24);
        assert!(parse(b"nope", 24).is_none());
        assert!(parse(&bytes[..40], 24).is_none());
    }

    #[test]
    fn theme_lookup_follows_inherits() {
        let dir = std::env::temp_dir().join(format!("boltsnap-xcursor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("child")).unwrap();
        std::fs::create_dir_all(dir.join("parent/cursors")).unwrap();
        std::fs::write(
            dir.join("child/index.theme"),
            "[Icon Theme]\nInherits = parent\n",
        )
        .unwrap();
        std::fs::write(dir.join("parent/cursors/left_ptr"), file(&[24])).unwrap();
        let dirs = [dir.clone()];
        assert_eq!(
            find("child", &dirs, 0),
            Some(dir.join("parent/cursors/left_ptr"))
        );
        assert_eq!(find("missing", &dirs, 0), None);
        assert_eq!(find("../child", &dirs, 0), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drawn_fallback_is_an_opaque_arrow_with_the_tip_as_hotspot() {
        let cursor = drawn(24);
        assert_eq!(cursor.hotspot, (1, 1));
        assert_eq!(cursor.image.get_pixel(3, 8).0, [255, 255, 255, 255]);
        assert_eq!(cursor.image.get_pixel(20, 3)[3], 0);
        let big = resize(
            Cursor {
                image: RgbaImage::new(24, 24),
                hotspot: (4, 2),
            },
            48,
        );
        assert_eq!((big.image.dimensions(), big.hotspot), ((48, 48), (8, 4)));
    }
}
