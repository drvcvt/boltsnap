use std::fs;
use std::path::Path;
use std::process::Command;

use image::{DynamicImage, RgbaImage, imageops};
use serde_json::Value;

use crate::paths::has_cmd;
use crate::{Backend, CaptureMode, DynResult};

pub fn capture(
    mode: CaptureMode,
    output: &Path,
    backend: Backend,
    instant: bool,
) -> DynResult<(Backend, Option<String>)> {
    let (backend, capture_output, image) = capture_image(mode, backend, instant, true)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    image.save(output)?;
    Ok((backend, capture_output))
}

pub fn capture_png(
    mode: CaptureMode,
    backend: Backend,
    instant: bool,
    trim: bool,
) -> DynResult<(Backend, Option<String>, Vec<u8>)> {
    let (backend, output, image) = capture_image(mode, backend, instant, trim)?;
    let mut bytes = Vec::new();
    image.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;
    Ok((backend, output, bytes))
}

pub(crate) fn capture_image(
    mode: CaptureMode,
    backend: Backend,
    instant: bool,
    trim: bool,
) -> DynResult<(Backend, Option<String>, DynamicImage)> {
    let backend = backend.resolved()?;
    let (image, output) = match backend {
        Backend::X11 => (capture_x11(mode)?, None),
        Backend::Wayland => capture_wayland(mode, instant)?,
        Backend::Windows => return Err("Windows capture is unavailable on Linux".into()),
        Backend::Auto => unreachable!(),
    };
    // Match the previous RGB output before evaluating a window border, without
    // encoding and decoding an intermediate file.
    let _timing = super::timing::Span::new("rgb_and_trim");
    let image = DynamicImage::ImageRgba8(image).into_rgb8();
    let image = if trim && matches!(mode, CaptureMode::Window | CaptureMode::ActiveWindow) {
        strip_uniform_border(image)
    } else {
        image
    };
    Ok((backend, output, DynamicImage::ImageRgb8(image)))
}

// Strip up to 4 px of uniform grayscale ring (Hypr d0d0d0 active-window
// border). Bails out if the inner content is itself uniform.
fn strip_uniform_border(img: image::RgbImage) -> image::RgbImage {
    let (w, h) = (img.width(), img.height());
    if w < 32 || h < 32 {
        return img;
    }

    let edge_pixel = *img.get_pixel(0, 0);
    let is_grayish = {
        let mn = edge_pixel[0].min(edge_pixel[1]).min(edge_pixel[2]) as i16;
        let mx = edge_pixel[0].max(edge_pixel[1]).max(edge_pixel[2]) as i16;
        (mx - mn) <= 8
    };
    if !is_grayish {
        return img;
    }

    let mut peel: u32 = 0;
    while peel < 4 {
        let p = peel;
        let row_uniform =
            |y: u32| -> bool { (p..w - p).all(|x| *img.get_pixel(x, y) == edge_pixel) };
        let col_uniform =
            |x: u32| -> bool { (p..h - p).all(|y| *img.get_pixel(x, y) == edge_pixel) };
        if !(row_uniform(p) && row_uniform(h - 1 - p) && col_uniform(p) && col_uniform(w - 1 - p)) {
            break;
        }
        peel += 1;
    }
    if peel == 0 {
        return img;
    }

    // Confirm interior is not itself a uniform field (solid background).
    let inner = *img.get_pixel(peel + 1, peel + 1);
    let mut differs = false;
    'scan: for y in (peel + 1..h - peel - 1).step_by(8) {
        for x in (peel + 1..w - peel - 1).step_by(8) {
            if *img.get_pixel(x, y) != inner {
                differs = true;
                break 'scan;
            }
        }
    }
    if !differs {
        return img;
    }

    imageops::crop_imm(&img, peel, peel, w - 2 * peel, h - 2 * peel).to_image()
}

fn capture_x11(mode: CaptureMode) -> DynResult<RgbaImage> {
    let window = match mode {
        CaptureMode::Full => return x11_capture_root(None),
        CaptureMode::Area => None,
        CaptureMode::Window => x11_pick_window_id()?,
        CaptureMode::ActiveWindow => match x11_active_window_id()? {
            Some(id) => Some(id),
            None => x11_pick_window_id()?,
        },
    };
    match window {
        Some(id) => x11_capture_root(Some(x11_window_geometry(id)?)),
        None => Err("interactive region selection requires Wayland (wlr-layer-shell); on X11 use `boltsnap full`, capture a window, or force `--backend wayland`".into()),
    }
}

// In-process X11 capture via x11rb. Reads the root window's pixels with
// GetImage in ZPixmap format, swizzles BGRX/BGRA into RGBA8.
fn x11_capture_root(rect: Option<(i16, i16, u16, u16)>) -> DynResult<RgbaImage> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

    let (conn, screen_num) =
        x11rb::connect(None).map_err(|e| format!("X11 connect failed: {e}"))?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let (x, y, w, h) = match rect {
        Some(v) => v,
        None => {
            let g = conn
                .get_geometry(root)
                .map_err(|e| format!("get_geometry root: {e}"))?
                .reply()
                .map_err(|e| format!("get_geometry root reply: {e}"))?;
            (0, 0, g.width, g.height)
        }
    };
    if w == 0 || h == 0 {
        return Err(format!("zero-sized X11 capture rect {w}x{h}").into());
    }
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root, x, y, w, h, !0u32)
        .map_err(|e| format!("X11 get_image: {e}"))?
        .reply()
        .map_err(|e| format!("X11 get_image reply: {e}"))?;

    let stride = reply.data.len() / h as usize;
    let bpp = stride / w as usize;
    if bpp != 4 {
        return Err(format!(
            "unexpected X11 pixmap stride: {bpp} bytes per pixel (depth {})",
            reply.depth
        )
        .into());
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for chunk in reply.data.chunks_exact(4) {
        rgba.push(chunk[2]);
        rgba.push(chunk[1]);
        rgba.push(chunk[0]);
        rgba.push(255);
    }
    RgbaImage::from_raw(w as u32, h as u32, rgba)
        .ok_or_else(|| format!("could not build RgbaImage {w}x{h}").into())
}

fn x11_active_window_id() -> DynResult<Option<u32>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, screen_num) =
        x11rb::connect(None).map_err(|e| format!("X11 connect failed: {e}"))?;
    let screen = &conn.setup().roots[screen_num];
    let atom = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .map_err(|e| format!("intern_atom: {e}"))?
        .reply()
        .map_err(|e| format!("intern_atom reply: {e}"))?
        .atom;
    let prop = conn
        .get_property(false, screen.root, atom, AtomEnum::WINDOW, 0, 1)
        .map_err(|e| format!("get_property: {e}"))?
        .reply()
        .map_err(|e| format!("get_property reply: {e}"))?;
    let Some(mut iter) = prop.value32() else {
        return Ok(None);
    };
    Ok(iter.next().filter(|w| *w != 0))
}

fn x11_window_geometry(win: u32) -> DynResult<(i16, i16, u16, u16)> {
    use x11rb::protocol::xproto::ConnectionExt;

    let (conn, _screen_num) =
        x11rb::connect(None).map_err(|e| format!("X11 connect failed: {e}"))?;
    let geom = conn
        .get_geometry(win)
        .map_err(|e| format!("get_geometry: {e}"))?
        .reply()
        .map_err(|e| format!("get_geometry reply: {e}"))?;
    let trans = conn
        .translate_coordinates(win, geom.root, 0, 0)
        .map_err(|e| format!("translate_coordinates: {e}"))?
        .reply()
        .map_err(|e| format!("translate_coordinates reply: {e}"))?;
    Ok((trans.dst_x, trans.dst_y, geom.width, geom.height))
}

// Crosshair window picker: grab pointer with crosshair cursor, wait for
// click, hand back whatever child window was clicked.
fn x11_pick_window_id() -> DynResult<Option<u32>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{ConnectionExt, EventMask, GrabMode, GrabStatus};

    let (conn, screen_num) =
        x11rb::connect(None).map_err(|e| format!("X11 connect failed: {e}"))?;
    let screen = &conn.setup().roots[screen_num];

    let cursor_font = conn
        .generate_id()
        .map_err(|e| format!("generate_id font: {e}"))?;
    conn.open_font(cursor_font, b"cursor")
        .map_err(|e| format!("open_font cursor: {e}"))?;
    let cursor = conn
        .generate_id()
        .map_err(|e| format!("generate_id cursor: {e}"))?;
    // 34 = XC_crosshair, 35 = the mask glyph paired with it.
    conn.create_glyph_cursor(
        cursor,
        cursor_font,
        cursor_font,
        34,
        35,
        0,
        0,
        0,
        0xffff,
        0xffff,
        0xffff,
    )
    .map_err(|e| format!("create_glyph_cursor: {e}"))?;
    conn.flush().map_err(|e| format!("flush: {e}"))?;

    let grab = conn
        .grab_pointer(
            false,
            screen.root,
            EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            cursor,
            x11rb::CURRENT_TIME,
        )
        .map_err(|e| format!("grab_pointer: {e}"))?
        .reply()
        .map_err(|e| format!("grab_pointer reply: {e}"))?;
    if grab.status != GrabStatus::SUCCESS {
        return Ok(None);
    }

    let target: Option<u32> = loop {
        let event = conn
            .wait_for_event()
            .map_err(|e| format!("wait_for_event: {e}"))?;
        if let Event::ButtonPress(ev) = event {
            break Some(if ev.child != 0 { ev.child } else { ev.event });
        }
    };
    let _ = conn.ungrab_pointer(x11rb::CURRENT_TIME);
    let _ = conn.free_cursor(cursor);
    let _ = conn.close_font(cursor_font);
    let _ = conn.flush();
    Ok(target.filter(|w| *w != 0))
}

fn capture_wayland(mode: CaptureMode, instant: bool) -> DynResult<(RgbaImage, Option<String>)> {
    let _timing = super::timing::Span::new("capture_wayland_total");
    let capture_output = crate::platform::shelf::focused_monitor_name();
    super::timing::mark("target_resolved");
    match mode {
        CaptureMode::Full => {
            let img = wayland_full_image()?;
            Ok((img, capture_output))
        }
        CaptureMode::ActiveWindow => {
            let options = libway::CaptureOptions::default();
            let mut conn = libway::Connection::connect(&options)
                .map_err(|e| format!("wayland connection failed: {e}"))?;
            let geometry = hyprland_active_window_geometry()?
                .ok_or("active-window on Wayland requires Hyprland (hyprctl)")?;
            let region = parse_geometry(&geometry)?;
            let img = conn
                .capture_region(region, &options)
                .map_err(|e| format!("libway screenshot active failed: {e}"))?;
            Ok((img.image, capture_output))
        }
        CaptureMode::Area | CaptureMode::Window => {
            // Freeze the complete desktop before mapping any selector surfaces.
            let grab = move || -> Result<super::select_skia::CapturedDesktop, String> {
                let options = libway::CaptureOptions::default();
                let mut conn = libway::Connection::connect(&options)
                    .map_err(|e| format!("wayland connection failed: {e}"))?;
                super::timing::mark("libway_connected");
                let outputs = conn.outputs(&options).map_err(|e| e.to_string())?;
                let monitors: Vec<_> = outputs.iter().map(captured_monitor).collect();
                let regions: Vec<_> = monitors
                    .iter()
                    .map(|m| (m.origin.0, m.origin.1, m.logical_size.0, m.logical_size.1))
                    .collect();
                let (_, _, width, height) = crate::selector::desktop::bounds(&regions)
                    .ok_or("invalid or oversized desktop layout")?;
                // Compose all outputs at the highest logical-to-pixel scale.
                let scale = outputs
                    .iter()
                    .map(libway::Output::scale)
                    .fold(1.0_f64, f64::max);
                if f64::from(width) * f64::from(height) * scale * scale > 64_000_000.0 {
                    return Err("desktop capture exceeds 64 million pixels".into());
                }
                super::timing::mark("capture_start");
                let image = match conn.capture_desktop(&options) {
                    Ok(desktop) => {
                        if desktop.outputs != outputs {
                            return Err("desktop layout changed during capture; retry".into());
                        }
                        desktop.image
                    }
                    Err(libway::Error::LayoutChanged | libway::Error::OutputGone) => {
                        return Err("desktop layout changed during capture; retry".into());
                    }
                    Err(err) => {
                        let image = super::portal::screenshot().map_err(|pe| {
                            format!("libway desktop failed: {err}; portal fallback failed: {pe}")
                        })?;
                        if conn.outputs(&options).map_err(|e| e.to_string())? != outputs {
                            return Err(
                                "desktop layout changed during portal capture; retry".into()
                            );
                        }
                        image
                    }
                };
                super::timing::mark("capture_ready");
                Ok(super::select_skia::CapturedDesktop {
                    image,
                    monitors,
                    preferred_output: capture_output,
                })
            };
            let cropped =
                crate::platform::select_skia::run_select_with_parallel_capture(grab, instant)?
                    .ok_or("selection cancelled")?;
            Ok(cropped)
        }
    }
}

fn captured_monitor(output: &libway::Output) -> super::select_skia::CapturedMonitor {
    use libway::Transform as T;
    use wayland_client::protocol::wl_output::Transform as W;
    super::select_skia::CapturedMonitor {
        name: output.name.clone(),
        origin: (output.logical.x, output.logical.y),
        logical_size: (output.logical.width, output.logical.height),
        transform: match output.transform {
            T::Normal => W::Normal,
            T::Rotate90 => W::_90,
            T::Rotate180 => W::_180,
            T::Rotate270 => W::_270,
            T::Flipped => W::Flipped,
            T::Flipped90 => W::Flipped90,
            T::Flipped180 => W::Flipped180,
            T::Flipped270 => W::Flipped270,
        },
    }
}

/// Whole-desktop capture: libway (EXT, then WLR) first, portal second.
fn wayland_full_image() -> Result<RgbaImage, String> {
    let options = libway::CaptureOptions::default();
    let capture = libway::Connection::connect(&options)
        .map_err(|e| format!("wayland connection failed: {e}"))
        .and_then(|mut conn| {
            conn.capture_desktop(&options)
                .map(|desktop| desktop.image)
                .map_err(|e| format!("libway desktop capture failed: {e}"))
        });
    capture.or_else(|err| {
        super::portal::screenshot().map_err(|pe| format!("{err}; portal fallback failed: {pe}"))
    })
}

fn parse_geometry(geometry: &str) -> DynResult<libway::Rect> {
    let (pos, size) = geometry
        .split_once(' ')
        .ok_or_else(|| format!("bad geometry '{geometry}'"))?;
    let (x, y) = pos
        .split_once(',')
        .ok_or_else(|| format!("bad geometry position '{pos}'"))?;
    let (w, h) = size
        .split_once('x')
        .ok_or_else(|| format!("bad geometry size '{size}'"))?;
    let x: i32 = x.trim().parse()?;
    let y: i32 = y.trim().parse()?;
    let w: u32 = w.trim().parse()?;
    let h: u32 = h.trim().parse()?;
    if w == 0 || h == 0 {
        return Err(format!("zero-sized region '{geometry}'").into());
    }
    Ok(libway::Rect {
        x,
        y,
        width: w,
        height: h,
    }
    .validate()?)
}

fn hyprland_active_window_geometry() -> DynResult<Option<String>> {
    if !has_cmd("hyprctl") {
        return Ok(None);
    }
    let out = run_capture(Command::new("hyprctl").arg("-j").arg("activewindow"))?;
    Ok(parse_hypr_window_geometry(&String::from_utf8_lossy(&out)))
}

fn parse_hypr_window_geometry(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    let at = v.get("at")?.as_array()?;
    let size = v.get("size")?.as_array()?;
    geometry_from_json_arrays(at, size)
}

fn geometry_from_json_arrays(at: &[Value], size: &[Value]) -> Option<String> {
    if at.len() < 2 || size.len() < 2 {
        return None;
    }
    let x = at[0].as_i64()?;
    let y = at[1].as_i64()?;
    let w = size[0].as_i64()?;
    let h = size[1].as_i64()?;
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(format!("{x},{y} {w}x{h}"))
}

fn run_capture(cmd: &mut Command) -> DynResult<Vec<u8>> {
    let debug = format!("{:?}", cmd);
    let out = super::replay::process::output_setup(cmd)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "command failed {debug}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn owned_rgba_is_reused() {
        let rgba = image::RgbaImage::new(23, 17);
        let allocation = rgba.as_ptr();
        let owned = image::DynamicImage::ImageRgba8(rgba).into_rgba8();
        assert_eq!(owned.as_ptr(), allocation);
    }

    #[test]
    fn border_trimming_preserves_pixels_and_uniform_images() {
        let border = image::Rgb([30, 30, 30]);
        let mut image = image::RgbImage::from_pixel(64, 48, border);
        assert_eq!(super::strip_uniform_border(image.clone()), image);
        for y in 2..46 {
            for x in 2..62 {
                image.put_pixel(x, y, image::Rgb([x as u8, y as u8, 180]));
            }
        }
        let expected = image::imageops::crop_imm(&image, 2, 2, 60, 44).to_image();
        assert_eq!(super::strip_uniform_border(image.clone()), expected);
        image.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        assert_eq!(super::strip_uniform_border(image.clone()), image);
    }

    use super::*;

    #[test]
    fn parse_hypr_geometry() {
        let json = r#"{"at":[100,200],"size":[900,700],"title":"x"}"#;
        assert_eq!(
            parse_hypr_window_geometry(json).as_deref(),
            Some("100,200 900x700")
        );
    }

    #[test]
    fn libway_region_preserves_signed_origin_and_rejects_invalid_sizes() {
        assert_eq!(
            parse_geometry("-1920,-200 900x700").unwrap(),
            libway::Rect {
                x: -1920,
                y: -200,
                width: 900,
                height: 700
            }
        );
        for invalid in [
            "0,0 0x10",
            "0,0 10x0",
            "0,0 4294967295x2",
            "2147483648,0 1x1",
            "bad",
        ] {
            assert!(parse_geometry(invalid).is_err(), "{invalid}");
        }
    }
}
