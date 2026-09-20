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

fn capture_image(
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
    let capture_output = crate::platform::shelf::focused_monitor_name();
    match mode {
        CaptureMode::Full => {
            let img = wayland_full_image()?;
            Ok((img, capture_output))
        }
        CaptureMode::ActiveWindow => {
            let conn = libwayshot::WayshotConnection::new()
                .map_err(|e| format!("wayland connection failed: {e}"))?;
            let geometry = hyprland_active_window_geometry()?
                .ok_or("active-window on Wayland requires Hyprland (hyprctl)")?;
            let region = parse_geometry(&geometry)?;
            let img = conn
                .screenshot(region, false)
                .map_err(|e| format!("wayshot screenshot active failed: {e}"))?;
            Ok((img.into_rgba8(), capture_output))
        }
        CaptureMode::Area | CaptureMode::Window => {
            // Run libwayshot capture on a worker so it overlaps with the
            // selector's Wayland init. Capture only the focused output.
            let grab = || -> Result<RgbaImage, String> {
                let conn = libwayshot::WayshotConnection::new()
                    .map_err(|e| format!("wayland connection failed: {e}"))?;
                let out_info = pick_focused_wl_output(&conn)
                    .map_err(|e| format!("output pick failed: {e}"))?;
                match conn.screenshot_outputs(std::slice::from_ref(&out_info), false) {
                    Ok(img) => Ok(img.to_rgba8()),
                    Err(err) => {
                        // No wlr-screencopy (KWin): grab the whole desktop
                        // through the portal and cut out the focused output.
                        let full = super::portal::screenshot().map_err(|pe| {
                            format!(
                                "wayshot single-output failed: {err}; portal fallback failed: {pe}"
                            )
                        })?;
                        let outputs = conn.get_all_outputs();
                        Ok(crop_to_output(full, &outputs, &out_info))
                    }
                }
            };
            let cropped =
                crate::platform::select_skia::run_select_with_parallel_capture(grab, instant)?
                    .ok_or("selection cancelled")?;
            Ok((cropped, capture_output))
        }
    }
}

// Pick the Wayland output the user is actually looking at. On Hyprland
// we ask hyprctl for the focused monitor; everywhere else we fall back
// to the first output, which is correct for single-monitor setups and
// "good enough" for everyone else (still way better than stitching).
fn pick_focused_wl_output(
    conn: &libwayshot::WayshotConnection,
) -> DynResult<libwayshot::output::OutputInfo> {
    let outputs = conn.get_all_outputs();
    if outputs.is_empty() {
        return Err("no Wayland outputs available".into());
    }
    if outputs.len() == 1 {
        return Ok(outputs[0].clone());
    }
    if let Some(name) = crate::platform::shelf::focused_monitor_name() {
        if let Some(o) = outputs.iter().find(|o| o.name == name) {
            return Ok(o.clone());
        }
    }
    Ok(outputs[0].clone())
}

/// Whole-desktop capture: libwayshot (wlr-screencopy) first, portal second.
fn wayland_full_image() -> Result<RgbaImage, String> {
    let wayshot = libwayshot::WayshotConnection::new()
        .map_err(|e| format!("wayland connection failed: {e}"))
        .and_then(|conn| {
            conn.screenshot_all(false)
                .map(|img| img.to_rgba8())
                .map_err(|e| format!("wayshot screenshot_all failed: {e}"))
        });
    wayshot.or_else(|err| {
        super::portal::screenshot().map_err(|pe| format!("{err}; portal fallback failed: {pe}"))
    })
}

/// Cut one output out of a whole-desktop image. The portal image is in device
/// pixels while output regions are logical, so scale by the desktop extent.
fn crop_to_output(
    full: RgbaImage,
    outputs: &[libwayshot::output::OutputInfo],
    target: &libwayshot::output::OutputInfo,
) -> RgbaImage {
    let (right, bottom) = outputs.iter().fold((0i64, 0i64), |(r, b), o| {
        let reg = &o.logical_region.inner;
        (
            r.max(i64::from(reg.position.x) + i64::from(reg.size.width)),
            b.max(i64::from(reg.position.y) + i64::from(reg.size.height)),
        )
    });
    if right <= 0 || bottom <= 0 {
        return full;
    }
    let sx = f64::from(full.width()) / right as f64;
    let sy = f64::from(full.height()) / bottom as f64;
    let reg = &target.logical_region.inner;
    let x = (f64::from(reg.position.x) * sx).round().max(0.0) as u32;
    let y = (f64::from(reg.position.y) * sy).round().max(0.0) as u32;
    let w = ((f64::from(reg.size.width) * sx).round() as u32).min(full.width().saturating_sub(x));
    let h = ((f64::from(reg.size.height) * sy).round() as u32).min(full.height().saturating_sub(y));
    if w == 0 || h == 0 {
        return full;
    }
    imageops::crop_imm(&full, x, y, w, h).to_image()
}

fn parse_geometry(geometry: &str) -> DynResult<libwayshot::region::LogicalRegion> {
    use libwayshot::region::{LogicalRegion, Position, Region, Size};
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
    Ok(LogicalRegion {
        inner: Region {
            position: Position { x, y },
            size: Size {
                width: w,
                height: h,
            },
        },
    })
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
    let out = cmd.output()?;
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
}
