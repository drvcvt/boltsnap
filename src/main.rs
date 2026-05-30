use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use image::{DynamicImage, Rgba, RgbaImage, imageops};
use serde_json::Value;

mod capture;
mod clipboard;
mod editor;
mod ipc;
mod paths;
mod select;
mod shelf;

pub type DynResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    Auto,
    X11,
    Wayland,
}

impl Backend {
    fn parse(value: &str) -> DynResult<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "x11" => Ok(Self::X11),
            "wayland" => Ok(Self::Wayland),
            other => Err(format!("unknown backend '{other}', use auto/x11/wayland").into()),
        }
    }

    fn resolved(self) -> DynResult<Self> {
        match self {
            Self::Auto => detect_backend(),
            other => Ok(other),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureMode {
    Area,
    Full,
    Window,
    ActiveWindow,
}

impl CaptureMode {
    fn parse(command: &str) -> DynResult<Self> {
        match command {
            "area" | "select" | "selection" | "region" => Ok(Self::Area),
            "full" | "screen" | "fullscreen" => Ok(Self::Full),
            "window" | "win" | "select-window" => Ok(Self::Window),
            "active" | "active-window" | "current" | "current-window" => Ok(Self::ActiveWindow),
            other => Err(format!("unknown capture command '{other}'").into()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Area => "area",
            Self::Full => "full",
            Self::Window => "window",
            Self::ActiveWindow => "active-window",
        }
    }
}

#[derive(Clone, Debug)]
struct Args {
    command: String,
    command_explicit: bool,
    image: Option<PathBuf>,
    edit: bool,
    copy: bool,
    save: bool,
    output: Option<PathBuf>,
    backend: Backend,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "area".to_string(),
            command_explicit: false,
            image: None,
            edit: false,
            copy: true,
            save: false,
            output: None,
            backend: Backend::Auto,
        }
    }
}

fn usage() -> &'static str {
    "\
Usage:
  boltsnap [area|full|window|active-window] [-o PATH|-] [--save] [--no-copy] [--backend auto|x11|wayland]
  boltsnap --edit                         open last screenshot in editor
  boltsnap [area|window|full] --edit      capture then edit
  boltsnap edit [IMAGE] [-o PATH] [--no-copy]
  boltsnap doctor

Examples:
  boltsnap                                area, copy PNG, remember as last
  boltsnap --edit                         open last screenshot in editor
  boltsnap window                         pick window, copy PNG
  boltsnap full --no-copy -o /tmp/x.png   write file, no clipboard
  boltsnap area --no-copy -o - | swappy -f -    pipe to external editor
"
}

fn parse_args(raw: &[String]) -> DynResult<Args> {
    let mut args = Args::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            "--version" => {
                println!("boltsnap {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--edit" => args.edit = true,
            "--copy" => args.copy = true,
            "--no-copy" => args.copy = false,
            "--save" => args.save = true,
            "-o" | "--output" => {
                i += 1;
                let Some(path) = raw.get(i) else {
                    return Err("--output needs a path".into());
                };
                args.output = Some(PathBuf::from(path));
            }
            "--backend" => {
                i += 1;
                let Some(value) = raw.get(i) else {
                    return Err("--backend needs auto, x11, or wayland".into());
                };
                args.backend = Backend::parse(value)?;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option '{value}'\n{}", usage()).into());
            }
            value => positional.push(value.to_string()),
        }
        i += 1;
    }

    if let Some(command) = positional.first() {
        args.command = command.to_lowercase();
        args.command_explicit = true;
    }
    if positional.len() > 1 {
        args.image = Some(PathBuf::from(&positional[1]));
    }
    Ok(args)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("boltsnap: {err}");
        std::process::exit(2);
    }
}

fn run() -> DynResult<()> {
    let raw: Vec<String> = env::args().collect();
    let args = parse_args(&raw)?;

    if args.edit && !args.command_explicit {
        return edit_last_screenshot(&args);
    }

    match args.command.as_str() {
        "doctor" => {
            print_doctor();
            Ok(())
        }
        "self-test" => self_test(),
        "__serve-clipboard" => {
            // Detached child kept alive to serve Wayland paste requests.
            let path = args
                .image
                .clone()
                .ok_or("__serve-clipboard needs a PNG path")?;
            return serve_wayland_clipboard(&path);
        }
        "edit" => {
            let image = args.image.clone().unwrap_or(last_screenshot_path()?);
            ensure_file(&image)?;
            let output = edit_output_path(&args);
            let result = run_editor(normalize_path(&image), output, args.copy, args.backend)?;
            remember_last_screenshot(&result)?;
            println!("Edited image ready: {}", result.display());
            Ok(())
        }
        command => {
            CaptureMode::parse(command)?;
            capture_flow(&args)
        }
    }
}

fn edit_last_screenshot(args: &Args) -> DynResult<()> {
    let image = last_screenshot_path()?;
    ensure_file(&image)?;
    let result = run_editor(
        image.clone(),
        edit_output_path(args),
        args.copy,
        args.backend,
    )?;
    remember_last_screenshot(&result)?;
    println!("Edited last screenshot: {}", result.display());
    Ok(())
}

fn capture_flow(args: &Args) -> DynResult<()> {
    let mode = CaptureMode::parse(&args.command)?;

    if is_stdout_target(args) {
        return capture_to_stdout(mode, args.backend);
    }

    let raw_is_temp = args.edit;
    let raw_path = if args.edit {
        temp_png("raw")
    } else {
        target_path(args)
    };
    let backend = capture(mode, &raw_path, args.backend)?;
    let _ = strip_uniform_border(&raw_path);
    let mut final_path = raw_path.clone();

    if args.edit {
        let output = edit_output_path(args);
        final_path = run_editor(raw_path.clone(), output, args.copy, args.backend)?;
        remember_last_screenshot(&final_path)?;
        if raw_is_temp && final_path != raw_path {
            let _ = fs::remove_file(&raw_path);
        }
    } else if args.copy {
        copy_to_clipboard(&raw_path, args.backend)?;
        remember_last_screenshot(&raw_path)?;
    } else {
        remember_last_screenshot(&raw_path)?;
    }

    let verb = if args.copy { "copied" } else { "captured" };
    println!(
        "Boltsnap {verb} {} via {}: {}",
        mode.label(),
        backend.as_str(),
        final_path.display()
    );
    Ok(())
}

fn is_stdout_target(args: &Args) -> bool {
    args.output.as_deref().and_then(|p| p.to_str()) == Some("-")
}

fn capture_to_stdout(mode: CaptureMode, backend: Backend) -> DynResult<()> {
    let tmp = temp_png("stdout");
    capture(mode, &tmp, backend)?;
    let bytes = fs::read(&tmp)?;
    let _ = fs::remove_file(&tmp);
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}

fn detect_backend() -> DynResult<Backend> {
    let session = env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_lowercase();
    if session == "wayland" || (session.is_empty() && env::var_os("WAYLAND_DISPLAY").is_some()) {
        Ok(Backend::Wayland)
    } else if session == "x11" || env::var_os("DISPLAY").is_some() {
        Ok(Backend::X11)
    } else {
        Err(
            "could not detect X11 or Wayland session; pass --backend x11 or --backend wayland"
                .into(),
        )
    }
}

fn capture(mode: CaptureMode, output: &Path, backend: Backend) -> DynResult<Backend> {
    let backend = backend.resolved()?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    match backend {
        Backend::X11 => {
            capture_x11(mode, output)?;
            // Strip alpha + iCCP that ImageMagick/maim may emit, otherwise
            // some viewers composite onto white and show a halo.
            flatten_to_rgb(output)?;
        }
        Backend::Wayland => capture_wayland(mode, output)?,
        Backend::Auto => unreachable!(),
    }
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(format!("capture helper did not create {}", output.display()).into());
    }
    Ok(backend)
}

fn flatten_to_rgb(path: &Path) -> DynResult<()> {
    let img = image::open(path)?;
    if matches!(img, DynamicImage::ImageRgb8(_)) {
        return Ok(());
    }
    img.to_rgb8().save(path)?;
    Ok(())
}

// Strip up to 4 px of uniform grayscale ring (Hypr d0d0d0 active-window
// border). Bails out if the inner content is itself uniform.
fn strip_uniform_border(path: &Path) -> DynResult<()> {
    let img = image::open(path)?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    if w < 32 || h < 32 {
        return Ok(());
    }

    let edge_pixel = *img.get_pixel(0, 0);
    let is_grayish = {
        let mn = edge_pixel[0].min(edge_pixel[1]).min(edge_pixel[2]) as i16;
        let mx = edge_pixel[0].max(edge_pixel[1]).max(edge_pixel[2]) as i16;
        (mx - mn) <= 8
    };
    if !is_grayish {
        return Ok(());
    }

    let mut peel: u32 = 0;
    while peel < 4 {
        let p = peel;
        let row_uniform = |y: u32| -> bool {
            (p..w - p).all(|x| *img.get_pixel(x, y) == edge_pixel)
        };
        let col_uniform = |x: u32| -> bool {
            (p..h - p).all(|y| *img.get_pixel(x, y) == edge_pixel)
        };
        if !(row_uniform(p) && row_uniform(h - 1 - p)
            && col_uniform(p) && col_uniform(w - 1 - p))
        {
            break;
        }
        peel += 1;
    }
    if peel == 0 {
        return Ok(());
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
        return Ok(());
    }

    let cropped = imageops::crop_imm(&img, peel, peel, w - 2 * peel, h - 2 * peel).to_image();
    cropped.save(path)?;
    Ok(())
}

fn capture_x11(mode: CaptureMode, output: &Path) -> DynResult<()> {
    match mode {
        CaptureMode::Full => {
            let img = x11_capture_root(None)?;
            img.save(output)?;
            Ok(())
        }
        CaptureMode::Area => capture_x11_area(output),
        CaptureMode::Window => capture_x11_window(output),
        CaptureMode::ActiveWindow => capture_x11_active_window(output),
    }
}

// Area selection on X11: kick off the root capture in a worker thread
// while we boot the eframe overlay in the main thread, then crop in
// memory once the user confirms a drag rect.
fn capture_x11_area(output: &Path) -> DynResult<()> {
    let cropped = run_select_with_parallel_capture(|| -> Result<RgbaImage, String> {
        x11_capture_root(None).map_err(|e| e.to_string())
    })?
    .ok_or("selection cancelled")?;
    cropped.save(output)?;
    Ok(())
}

fn capture_x11_window(output: &Path) -> DynResult<()> {
    match x11_pick_window_id()? {
        Some(id) => capture_x11_window_id(id, output),
        None => capture_x11_area(output),
    }
}

fn capture_x11_active_window(output: &Path) -> DynResult<()> {
    match x11_active_window_id()? {
        Some(id) => capture_x11_window_id(id, output),
        None => capture_x11_window(output),
    }
}

fn capture_x11_window_id(win: u32, output: &Path) -> DynResult<()> {
    let geom = x11_window_geometry(win)?;
    let img = x11_capture_root(Some(geom))?;
    img.save(output)?;
    Ok(())
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
        cursor, cursor_font, cursor_font, 34, 35, 0, 0, 0, 0xffff, 0xffff, 0xffff,
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

fn capture_wayland(mode: CaptureMode, output: &Path) -> DynResult<()> {
    match mode {
        CaptureMode::Full => {
            let conn = libwayshot::WayshotConnection::new()
                .map_err(|e| format!("wayland connection failed: {e}"))?;
            let img = conn
                .screenshot_all(false)
                .map_err(|e| format!("wayshot screenshot_all failed: {e}"))?;
            img.to_rgb8()
                .save(output)
                .map_err(|e| format!("png encode failed: {e}"))?;
            Ok(())
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
            img.to_rgb8()
                .save(output)
                .map_err(|e| format!("png encode failed: {e}"))?;
            Ok(())
        }
        CaptureMode::Area | CaptureMode::Window => {
            // Run libwayshot capture on a worker so it overlaps with
            // eframe + GL init in the main thread. Capture only the
            // focused output instead of stitching every monitor.
            let cropped = run_select_with_parallel_capture(|| -> Result<RgbaImage, String> {
                let conn = libwayshot::WayshotConnection::new()
                    .map_err(|e| format!("wayland connection failed: {e}"))?;
                let out_info = pick_focused_wl_output(&conn)
                    .map_err(|e| format!("output pick failed: {e}"))?;
                let img = conn
                    .screenshot_single_output(&out_info, false)
                    .map_err(|e| format!("wayshot single-output failed: {e}"))?;
                Ok(img.to_rgba8())
            })?
            .ok_or("selection cancelled")?;
            image::DynamicImage::ImageRgba8(cropped)
                .to_rgb8()
                .save(output)
                .map_err(|e| format!("png encode failed: {e}"))?;
            Ok(())
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
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && has_cmd("hyprctl") {
        if let Ok(out) =
            run_capture(Command::new("hyprctl").args(["monitors", "-j"]))
        {
            let lossy = String::from_utf8_lossy(&out);
            if let Ok(Value::Array(monitors)) = serde_json::from_str::<Value>(&lossy) {
                for m in &monitors {
                    if m.get("focused").and_then(Value::as_bool) == Some(true) {
                        if let Some(name) = m.get("name").and_then(Value::as_str) {
                            if let Some(o) = outputs.iter().find(|o| o.name == name) {
                                return Ok(o.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(outputs[0].clone())
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

fn serve_wayland_clipboard(path: &Path) -> DynResult<()> {
    use wl_clipboard_rs::copy::{MimeType, Options, Source};
    let data = fs::read(path)?;
    let mut opts = Options::new();
    opts.foreground(true);
    let prepared = opts
        .prepare_copy(
            Source::Bytes(data.into_boxed_slice()),
            MimeType::Specific("image/png".into()),
        )
        .map_err(|e| format!("prepare_copy failed: {e}"))?;
    prepared
        .serve()
        .map_err(|e| format!("clipboard serve failed: {e}"))?;
    Ok(())
}

fn copy_to_clipboard(path: &Path, backend: Backend) -> DynResult<Backend> {
    let backend = backend.resolved()?;
    match backend {
        Backend::Wayland => {
            // Spawn a detached self holding the clipboard data source open
            // so the foreground shell returns immediately.
            let exe = env::current_exe()?;
            Command::new(exe)
                .arg("__serve-clipboard")
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        }
        Backend::X11 => {
            // arboard's set_image owns the X11 selection; on X11 it forks a
            // helper that lives until another client takes the selection,
            // so we don't need an external xclip helper.
            let img = image::open(path)?.to_rgba8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let bytes = std::borrow::Cow::Owned(img.into_raw());
            let mut clipboard = arboard::Clipboard::new()
                .map_err(|e| format!("X11 clipboard open failed: {e}"))?;
            clipboard
                .set_image(arboard::ImageData {
                    width: w,
                    height: h,
                    bytes,
                })
                .map_err(|e| format!("X11 clipboard set_image failed: {e}"))?;
        }
        Backend::Auto => unreachable!(),
    }
    Ok(backend)
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

fn has_cmd(name: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| dir.join(name).is_file())
}

fn print_doctor() {
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
    println!("Boltsnap doctor");
    println!("================");
    println!("Session: {session}");
    println!();
    println!("Optional compositor IPC (only used for active-window on Wayland):");
    for cmd in ["hyprctl"] {
        println!(
            "  {cmd:<10} {}",
            if has_cmd(cmd) { "ok" } else { "missing" }
        );
    }
    println!();
    println!("Capabilities (no external screenshot/clipboard helpers required):");
    println!("  X11 capture:       in-process via x11rb (root pixmap GetImage)");
    println!("  X11 area/window:   in-process selection overlay (eframe)");
    println!("  X11 active win:    in-process via x11rb (_NET_ACTIVE_WINDOW)");
    println!("  Wayland capture:   in-process via libwayshot (wlr-screencopy)");
    println!("  Wayland area/win:  in-process selection overlay (eframe)");
    println!("  Wayland active win: hyprctl on Hyprland");
    println!("  Clipboard:         in-process via arboard (X11) and wl-clipboard-rs (Wayland)");
}

fn self_test() -> DynResult<()> {
    print_doctor();
    println!("\nInternal parser/render tests are covered by `cargo test`.");
    Ok(())
}

fn target_path(args: &Args) -> PathBuf {
    if let Some(path) = &args.output {
        normalize_path(path)
    } else if args.save {
        default_save_path()
    } else {
        cache_dir().join("last.png")
    }
}

fn edit_output_path(args: &Args) -> Option<PathBuf> {
    if let Some(path) = &args.output {
        Some(normalize_path(path))
    } else if args.save {
        Some(default_save_path())
    } else {
        Some(cache_dir().join("last-edited.png"))
    }
}

fn cache_dir() -> PathBuf {
    if let Some(cache) = env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(cache).join("boltsnap")
    } else if let Some(home) = env::var_os("HOME") {
        PathBuf::from(home).join(".cache").join("boltsnap")
    } else {
        env::temp_dir().join("boltsnap")
    }
}

fn last_pointer_path() -> PathBuf {
    cache_dir().join("last.txt")
}

fn remember_last_screenshot(path: &Path) -> DynResult<()> {
    let path = normalize_path(path);
    if !path.is_file() {
        return Ok(());
    }
    fs::create_dir_all(cache_dir())?;
    fs::write(last_pointer_path(), path.to_string_lossy().as_bytes())?;
    Ok(())
}

fn last_screenshot_path() -> DynResult<PathBuf> {
    let pointer = last_pointer_path();
    if pointer.is_file() {
        let value = fs::read_to_string(pointer)?;
        let path = PathBuf::from(value.trim());
        if path.is_file() {
            return Ok(path);
        }
    }
    let fallback = cache_dir().join("last.png");
    if fallback.is_file() {
        Ok(fallback)
    } else {
        Err("no last screenshot yet; run `boltsnap` first, then `boltsnap --edit`".into())
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn ensure_file(path: &Path) -> DynResult<()> {
    if normalize_path(path).is_file() {
        Ok(())
    } else {
        Err(format!("file not found: {}", path.display()).into())
    }
}

fn default_save_path() -> PathBuf {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Pictures")
        .join("Screenshots")
        .join(format!("boltsnap-{}.png", timestamp()))
}

fn temp_png(prefix: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "boltsnap-{prefix}-{}-{}.png",
        std::process::id(),
        timestamp()
    ))
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

const SIDEBAR_W: f32 = 68.0;
const STATUSBAR_H: f32 = 32.0;
const ICON_BTN: f32 = 44.0;
const ICON_RADIUS: f32 = 7.0;

const MONO_BG: egui::Color32 = egui::Color32::from_gray(16);
const MONO_PANEL: egui::Color32 = egui::Color32::from_gray(24);
const MONO_INK: egui::Color32 = egui::Color32::from_gray(10);
const MONO_HOVER: egui::Color32 = egui::Color32::from_gray(38);
const MONO_SELECTED: egui::Color32 = egui::Color32::from_gray(62);
const MONO_BORDER: egui::Color32 = egui::Color32::from_gray(34);
const MONO_BORDER_HI: egui::Color32 = egui::Color32::from_gray(120);
const MONO_TEXT: egui::Color32 = egui::Color32::from_gray(190);
const MONO_TEXT_HI: egui::Color32 = egui::Color32::from_gray(245);
const MONO_TEXT_DIM: egui::Color32 = egui::Color32::from_gray(120);
const MONO_TEXT_DARK: egui::Color32 = egui::Color32::from_gray(14);
const MONO_ACCENT: egui::Color32 = egui::Color32::from_gray(225);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tool {
    Move,
    Arrow,
    Pen,
    Rect,
    Highlight,
    Redact,
    Blur,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Arrow => "Arrow",
            Self::Pen => "Pen",
            Self::Rect => "Box",
            Self::Highlight => "Highlight",
            Self::Redact => "Redact",
            Self::Blur => "Blur",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Move => "M",
            Self::Arrow => "A",
            Self::Pen => "P",
            Self::Rect => "R",
            Self::Highlight => "H",
            Self::Redact => "X",
            Self::Blur => "B",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionIcon {
    Undo,
    Clear,
    Save,
}

impl ActionIcon {
    fn tooltip(self) -> &'static str {
        match self {
            Self::Undo => "Undo (Ctrl+Z)",
            Self::Clear => "Clear all annotations",
            Self::Save => "Save & copy (Space)",
        }
    }
}

fn thin_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON_BTN, 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + 8.0, rect.center().y),
            egui::pos2(rect.right() - 8.0, rect.center().y),
        ],
        egui::Stroke::new(1.0, MONO_BORDER),
    );
}

fn tool_glyph(tool: Tool) -> &'static str {
    use egui_phosphor::regular as ph;
    match tool {
        Tool::Move => ph::ARROWS_OUT_CARDINAL,
        Tool::Arrow => ph::ARROW_UP_RIGHT,
        Tool::Pen => ph::PENCIL_SIMPLE,
        Tool::Rect => ph::RECTANGLE,
        Tool::Highlight => ph::HIGHLIGHTER,
        Tool::Redact => ph::EYE_SLASH,
        Tool::Blur => ph::DROP_HALF,
    }
}

fn action_glyph(action: ActionIcon) -> &'static str {
    use egui_phosphor::regular as ph;
    match action {
        ActionIcon::Undo => ph::ARROW_COUNTER_CLOCKWISE,
        ActionIcon::Clear => ph::TRASH,
        ActionIcon::Save => ph::FLOPPY_DISK,
    }
}

#[derive(Clone, Debug)]
struct Annotation {
    tool: Tool,
    points: Vec<[f32; 2]>,
}

struct EditorApp {
    output_path: PathBuf,
    copy_after: bool,
    backend: Backend,
    base: RgbaImage,
    texture: egui::TextureHandle,
    annotations: Vec<Annotation>,
    current: Vec<[f32; 2]>,
    tool: Tool,
    status: String,
    finished: bool,
    zoom: f32,
    pan: egui::Vec2,
    show_help: bool,
    saving: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl EditorApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        image_path: PathBuf,
        output_path: PathBuf,
        copy_after: bool,
        backend: Backend,
    ) -> DynResult<Self> {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = MONO_BG;
        visuals.panel_fill = MONO_BG;
        visuals.extreme_bg_color = MONO_INK;
        visuals.faint_bg_color = MONO_PANEL;
        visuals.widgets.noninteractive.bg_fill = MONO_PANEL;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT);
        visuals.widgets.inactive.bg_fill = MONO_PANEL;
        visuals.widgets.inactive.weak_bg_fill = MONO_PANEL;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT);
        visuals.widgets.hovered.bg_fill = MONO_HOVER;
        visuals.widgets.hovered.weak_bg_fill = MONO_HOVER;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER_HI);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT_HI);
        visuals.widgets.active.bg_fill = MONO_HOVER;
        visuals.widgets.active.weak_bg_fill = MONO_HOVER;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER_HI);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT_HI);
        visuals.selection.bg_fill = MONO_HOVER;
        visuals.selection.stroke = egui::Stroke::new(1.0, MONO_TEXT_HI);
        // Hide egui's resize-grip glyph; it shows in the canvas corner
        // when CSD is off.
        visuals.resize_corner_size = 0.0;
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        let r: egui::CornerRadius = 4.into();
        style.visuals.widgets.active.corner_radius = r;
        style.visuals.widgets.hovered.corner_radius = r;
        style.visuals.widgets.inactive.corner_radius = r;
        style.visuals.widgets.noninteractive.corner_radius = r;
        cc.egui_ctx.set_global_style(style);

        let base = image::open(&image_path)?.to_rgba8();
        let size = [base.width() as usize, base.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, base.as_raw());
        let texture =
            cc.egui_ctx
                .load_texture("screenshot", color_image, egui::TextureOptions::LINEAR);
        Ok(Self {
            output_path,
            copy_after,
            backend,
            base,
            texture,
            annotations: Vec::new(),
            current: Vec::new(),
            tool: Tool::Arrow,
            status: String::new(),
            finished: false,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            show_help: false,
            saving: None,
        })
    }

    fn save_and_maybe_copy(&mut self) {
        if self.saving.is_some() {
            return;
        }
        let base = self.base.clone();
        let annotations = self.annotations.clone();
        let output_path = self.output_path.clone();
        let copy_after = self.copy_after;
        let backend = self.backend;
        self.status = "Saving…".to_string();
        self.saving = Some(std::thread::spawn(move || {
            let rendered = render_annotations(&base, &annotations);
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // Save as RGB; all annotation passes write alpha=255.
            DynamicImage::ImageRgba8(rendered)
                .to_rgb8()
                .save(&output_path)
                .map_err(|e| e.to_string())?;
            if copy_after {
                copy_to_clipboard(&output_path, backend).map_err(|e| e.to_string())?;
            }
            Ok(())
        }));
    }

    fn poll_saving(&mut self, ctx: &egui::Context) {
        let Some(handle) = self.saving.take() else { return };
        if handle.is_finished() {
            match handle.join() {
                Ok(Ok(())) => {
                    self.status = if self.copy_after {
                        format!("saved + copied {}", self.output_path.display())
                    } else {
                        format!("saved {}", self.output_path.display())
                    };
                    self.finished = true;
                }
                Ok(Err(err)) => self.status = format!("save failed: {err}"),
                Err(_) => self.status = "save failed: worker panicked".to_string(),
            }
        } else {
            self.saving = Some(handle);
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }

    fn image_from_pointer(&self, pos: egui::Pos2, rect: egui::Rect) -> [f32; 2] {
        let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * self.base.width() as f32;
        let y = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0) * self.base.height() as f32;
        [x, y]
    }

    fn screen_point(&self, point: [f32; 2], rect: egui::Rect) -> egui::Pos2 {
        egui::pos2(
            rect.left() + point[0] / self.base.width() as f32 * rect.width(),
            rect.top() + point[1] / self.base.height() as f32 * rect.height(),
        )
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let none = egui::Modifiers::NONE;
        let cmd = egui::Modifiers::COMMAND;

        if ctx.input_mut(|i| i.consume_key(none, egui::Key::M)) {
            self.tool = Tool::Move;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::A)) {
            self.tool = Tool::Arrow;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::P)) {
            self.tool = Tool::Pen;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::R)) {
            self.tool = Tool::Rect;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::H)) {
            self.tool = Tool::Highlight;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::X)) {
            self.tool = Tool::Redact;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::B)) {
            self.tool = Tool::Blur;
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Z)) {
            self.annotations.pop();
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Num0)) {
            self.zoom = 1.0;
            self.pan = egui::Vec2::ZERO;
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Plus)) {
            self.zoom = (self.zoom * 1.15).min(6.0);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Minus)) {
            self.zoom = (self.zoom / 1.15).max(0.25);
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::F1)) {
            self.show_help = !self.show_help;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::Enter))
            || ctx.input_mut(|i| i.consume_key(none, egui::Key::Space))
        {
            self.save_and_maybe_copy();
        }
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            let factor = if scroll > 0.0 { 1.06 } else { 1.0 / 1.06 };
            self.zoom = (self.zoom * factor).clamp(0.1, 12.0);
        }
    }

    fn tool_icon_button(&mut self, ui: &mut egui::Ui, tool: Tool) {
        let selected = self.tool == tool;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ICON_BTN, ICON_BTN), egui::Sense::click());
        let painter = ui.painter_at(rect);

        let (fill, fg, border) = if selected {
            (MONO_SELECTED, MONO_TEXT_HI, MONO_BORDER_HI)
        } else if response.hovered() {
            (MONO_HOVER, MONO_TEXT_HI, MONO_BORDER)
        } else {
            (MONO_PANEL, MONO_TEXT, MONO_PANEL)
        };
        painter.rect(
            rect.shrink(0.5),
            ICON_RADIUS,
            fill,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        if selected {
            let strip = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 3.0, rect.top() + 11.0),
                egui::pos2(rect.left() + 5.0, rect.bottom() - 11.0),
            );
            painter.rect_filled(strip, 1.5, MONO_ACCENT);
        }
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tool_glyph(tool),
            egui::FontId::proportional(20.0),
            fg,
        );
        painter.text(
            rect.right_bottom() + egui::vec2(-4.0, -2.0),
            egui::Align2::RIGHT_BOTTOM,
            tool.shortcut(),
            egui::FontId::monospace(8.5),
            MONO_TEXT_DIM,
        );

        let _ = response
            .clone()
            .on_hover_text(format!("{} ({})", tool.label(), tool.shortcut()));
        if response.clicked() {
            self.tool = tool;
        }
    }

    fn action_icon_button(
        &mut self,
        ui: &mut egui::Ui,
        action: ActionIcon,
        primary: bool,
    ) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ICON_BTN, ICON_BTN), egui::Sense::click());
        let hovered = response.hovered();
        let painter = ui.painter_at(rect);

        let (fill, fg, border) = if primary && hovered {
            (MONO_TEXT_HI, MONO_TEXT_DARK, MONO_TEXT_HI)
        } else if primary {
            (MONO_ACCENT, MONO_TEXT_DARK, MONO_ACCENT)
        } else if hovered {
            (MONO_HOVER, MONO_TEXT_HI, MONO_BORDER)
        } else {
            (MONO_PANEL, MONO_TEXT, MONO_PANEL)
        };
        painter.rect(
            rect.shrink(0.5),
            ICON_RADIUS,
            fill,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            action_glyph(action),
            egui::FontId::proportional(20.0),
            fg,
        );
        response.on_hover_text(action.tooltip())
    }

    fn toolbar_ui(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 5.0);
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            for tool in [
                Tool::Move,
                Tool::Arrow,
                Tool::Pen,
                Tool::Rect,
                Tool::Highlight,
                Tool::Redact,
                Tool::Blur,
            ] {
                self.tool_icon_button(ui, tool);
            }

            ui.add_space(8.0);
            thin_separator(ui);
            ui.add_space(8.0);

            if self
                .action_icon_button(ui, ActionIcon::Undo, false)
                .clicked()
            {
                self.annotations.pop();
            }
            if self
                .action_icon_button(ui, ActionIcon::Clear, false)
                .clicked()
            {
                self.annotations.clear();
            }

            ui.add_space(8.0);
            thin_separator(ui);
            ui.add_space(8.0);

            if self.action_icon_button(ui, ActionIcon::Save, true).clicked() {
                self.save_and_maybe_copy();
            }
        });
    }

    fn statusbar_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

            ui.label(
                egui::RichText::new(self.tool.label())
                    .color(MONO_TEXT_HI)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!("[{}]", self.tool.shortcut()))
                    .color(MONO_TEXT_DIM)
                    .monospace()
                    .small(),
            );
            ui.label(egui::RichText::new("·").color(MONO_TEXT_DIM));
            ui.label(
                egui::RichText::new(format!("{}%", (self.zoom * 100.0).round() as i32))
                    .color(MONO_TEXT)
                    .monospace()
                    .small(),
            );

            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let hint = if self.saving.is_some() {
                        ("saving…", MONO_TEXT_HI)
                    } else if self.status.starts_with("save failed") {
                        (
                            self.status.as_str(),
                            egui::Color32::from_rgb(220, 110, 110),
                        )
                    } else if !self.status.is_empty() {
                        (self.status.as_str(), MONO_TEXT)
                    } else {
                        ("space save & copy · esc close", MONO_TEXT_DIM)
                    };
                    ui.label(egui::RichText::new(hint.0).color(hint.1).small());
                },
            );
        });
    }

    fn paint_annotation(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        ann: &Annotation,
        preview: bool,
    ) {
        if ann.points.len() < 2 {
            return;
        }
        let stroke = match ann.tool {
            Tool::Highlight => egui::Stroke::new(3.0, egui::Color32::from_rgb(255, 230, 40)),
            Tool::Redact => egui::Stroke::new(3.0, egui::Color32::BLACK),
            Tool::Blur => egui::Stroke::new(3.0, egui::Color32::from_rgb(115, 190, 255)),
            _ => egui::Stroke::new(
                4.0,
                if preview {
                    egui::Color32::LIGHT_RED
                } else {
                    egui::Color32::from_rgb(255, 70, 70)
                },
            ),
        };
        match ann.tool {
            Tool::Move => {}
            Tool::Pen => {
                for pair in ann.points.windows(2) {
                    painter.line_segment(
                        [
                            self.screen_point(pair[0], rect),
                            self.screen_point(pair[1], rect),
                        ],
                        stroke,
                    );
                }
            }
            Tool::Arrow => {
                let a = self.screen_point(ann.points[0], rect);
                let b = self.screen_point(*ann.points.last().unwrap(), rect);
                painter.line_segment([a, b], stroke);
                let dir = (a - b).normalized();
                let left = egui::vec2(dir.x * 22.0 - dir.y * 10.0, dir.y * 22.0 + dir.x * 10.0);
                let right = egui::vec2(dir.x * 22.0 + dir.y * 10.0, dir.y * 22.0 - dir.x * 10.0);
                painter.line_segment([b, b + left], stroke);
                painter.line_segment([b, b + right], stroke);
            }
            Tool::Rect | Tool::Highlight | Tool::Redact | Tool::Blur => {
                let a = self.screen_point(ann.points[0], rect);
                let b = self.screen_point(ann.points[1], rect);
                let r = egui::Rect::from_two_pos(a, b);
                match ann.tool {
                    Tool::Redact => {
                        painter.rect_filled(r, 3.0, egui::Color32::BLACK);
                    }
                    Tool::Highlight => {
                        painter.rect_filled(
                            r,
                            3.0,
                            egui::Color32::from_rgba_unmultiplied(255, 230, 0, 72),
                        );
                        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                    }
                    Tool::Blur => {
                        painter.rect_filled(
                            r,
                            3.0,
                            egui::Color32::from_rgba_unmultiplied(80, 160, 255, 44),
                        );
                        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                    }
                    Tool::Rect => {
                        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                    }
                    _ => {}
                }
            }
        }
    }

    fn draw_canvas(&mut self, ui: &mut egui::Ui) {
        let img_native = egui::vec2(self.base.width() as f32, self.base.height() as f32);
        let avail = ui.available_size().max(egui::vec2(200.0, 200.0));
        let fit_scale = (avail.x / img_native.x).min(avail.y / img_native.y);
        let scale = (fit_scale * self.zoom).clamp(0.05, 12.0);
        let img_size = img_native * scale;

        let (response, painter) = ui.allocate_painter(avail, egui::Sense::click_and_drag());
        let outer = response.rect;
        painter.rect_filled(outer, 0.0, MONO_INK);

        // Middle-mouse pan in any tool. egui's drag sense only reports
        // primary-button drags so we read raw pointer delta.
        let ctx = ui.ctx().clone();
        let pointer_in = ctx
            .input(|i| i.pointer.hover_pos().map(|p| outer.contains(p)))
            .unwrap_or(false);
        if pointer_in
            && ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
        {
            let delta = ctx.input(|i| i.pointer.delta());
            if delta != egui::Vec2::ZERO {
                self.pan += delta;
            }
        }

        let img_rect =
            egui::Rect::from_center_size(outer.center() + self.pan, img_size);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(self.texture.id(), img_rect, uv, egui::Color32::WHITE);

        // Annotations are primary-button only; egui's drag events fire
        // for any button, so we filter explicitly.
        let primary = egui::PointerButton::Primary;
        if self.tool == Tool::Move {
            if response.dragged_by(primary) {
                self.pan += response.drag_delta();
            }
            if response.double_clicked() {
                self.pan = egui::Vec2::ZERO;
                self.zoom = 1.0;
            }
        }
        if self.tool != Tool::Move {
            if response.drag_started_by(primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.current = vec![self.image_from_pointer(pos, img_rect)];
            }
            if response.dragged_by(primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                let p = self.image_from_pointer(pos, img_rect);
                if self.tool == Tool::Pen || self.current.len() < 2 {
                    self.current.push(p);
                } else {
                    self.current[1] = p;
                }
            }
            if response.drag_stopped_by(primary) && self.current.len() >= 2 {
                self.annotations.push(Annotation {
                    tool: self.tool,
                    points: self.current.clone(),
                });
                self.current.clear();
            }
        }

        let clip = ui.painter_at(img_rect);
        for ann in &self.annotations {
            self.paint_annotation(&clip, img_rect, ann, false);
        }
        if self.current.len() >= 2 {
            let preview = Annotation {
                tool: self.tool,
                points: self.current.clone(),
            };
            self.paint_annotation(&clip, img_rect, &preview, true);
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.poll_saving(&ctx);

        if ctx.input(|i| i.viewport().close_requested()) {
            self.finished = true;
        }
        if self.finished {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let toolbar_frame = egui::Frame::new()
            .fill(MONO_PANEL)
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 12,
                bottom: 12,
            });
        egui::Panel::left("boltsnap-tools")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(toolbar_frame)
            .show_inside(ui, |ui| {
                self.toolbar_ui(ui);
            });

        let status_frame = egui::Frame::new()
            .fill(MONO_PANEL)
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin::symmetric(14, 7));
        egui::Panel::bottom("boltsnap-status")
            .exact_size(STATUSBAR_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(status_frame)
            .show_inside(ui, |ui| {
                self.statusbar_ui(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(MONO_INK).inner_margin(0))
            .show_inside(ui, |ui| {
                self.draw_canvas(ui);
            });
    }
}

fn run_editor(
    image_path: PathBuf,
    output_path: Option<PathBuf>,
    copy_after: bool,
    backend: Backend,
) -> DynResult<PathBuf> {
    let output = output_path.unwrap_or_else(|| temp_png("edited"));
    let out_clone = output.clone();

    let (img_w, img_h) = image::image_dimensions(&image_path)?;
    let max_w = 1700.0_f32;
    let max_h = 980.0_f32;
    let canvas_max_w = max_w - SIDEBAR_W;
    let canvas_max_h = max_h - STATUSBAR_H;
    let scale = (canvas_max_w / img_w as f32)
        .min(canvas_max_h / img_h as f32)
        .min(1.0);
    // Toolbar needs ~560 px or the Save button clips off the bottom.
    const MIN_TOOLBAR_H: f32 = 560.0;
    let win_w = (img_w as f32 * scale + SIDEBAR_W).max(420.0);
    let win_h = (img_h as f32 * scale + STATUSBAR_H).max(MIN_TOOLBAR_H);

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Boltsnap")
            .with_app_id("boltsnap-editor")
            .with_window_type(egui::X11WindowType::Dialog)
            .with_inner_size([win_w, win_h])
            .with_min_inner_size([420.0, MIN_TOOLBAR_H])
            .with_resizable(false)
            .with_decorations(false)
            .with_always_on_top(),
        ..Default::default()
    };

    prep_compositor_for("boltsnap-editor", false);
    eframe::run_native(
        "Boltsnap Editor",
        native,
        Box::new(move |cc| {
            Ok(Box::new(
                EditorApp::new(cc, image_path, output.clone(), copy_after, backend)
                    .map_err(|e| e.to_string())?,
            ))
        }),
    )?;
    Ok(out_clone)
}

// Push windowrules upfront so the window appears already floating with
// no fade-in. Beats polling after the window opens — no race, no jank,
// no compositor animation hitting the user before we react.
fn prep_compositor_for(class: &str, fullscreen: bool) {
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && has_cmd("hyprctl") {
        let selector = format!("class:^({class})$");
        let mut rules: Vec<String> = vec![
            format!("noanim, {selector}"),
            format!("noblur, {selector}"),
            format!("noshadow, {selector}"),
            format!("float, {selector}"),
            format!("center, {selector}"),
            format!("pin, {selector}"),
        ];
        if fullscreen {
            rules.push(format!("fullscreen, {selector}"));
            rules.push(format!("noborder, {selector}"));
            rules.push(format!("rounding 0, {selector}"));
        }
        for rule in &rules {
            let _ = Command::new("hyprctl")
                .args(["keyword", "windowrulev2", rule])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    if env::var_os("SWAYSOCK").is_some() && has_cmd("swaymsg") {
        let _ = Command::new("swaymsg")
            .args([
                "for_window",
                &format!("[app_id=\"{class}\"]"),
                if fullscreen {
                    "floating enable, fullscreen enable, border none"
                } else {
                    "floating enable"
                },
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

// Single-process selection: spawn the compositor capture on a worker
// thread so it overlaps with eframe's winit + GL init in the main
// thread, then run the SelectApp inline. No child process, no PNG/raw
// handoff over a pipe, no extra Rust cold-start.
//
// Returns the cropped RgbaImage on confirm, or None on Esc/cancel.
fn run_select_with_parallel_capture<F>(capture: F) -> DynResult<Option<RgbaImage>>
where
    F: FnOnce() -> Result<RgbaImage, String> + Send + 'static,
{
    let capture_handle = std::thread::spawn(capture);

    let result: std::sync::Arc<std::sync::Mutex<Option<RgbaImage>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let result_clone = result.clone();

    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        persist_window: false,
        persistence_path: None,
        viewport: egui::ViewportBuilder::default()
            .with_title("boltsnap-select")
            .with_app_id("boltsnap-select")
            .with_decorations(false)
            .with_resizable(false)
            .with_fullscreen(true)
            .with_active(true)
            .with_always_on_top()
            .with_window_type(egui::X11WindowType::Splash),
        ..Default::default()
    };

    // Sync: the hyprctl/swaymsg call is fast (<15 ms) and the rule
    // MUST be in the compositor's table before eframe maps the
    // toplevel, or the default fade-in plays and the overlay feels
    // laggy. Off-threading this loses that race.
    prep_compositor_for("boltsnap-select", true);

    eframe::run_native(
        "boltsnap-select",
        native,
        Box::new(move |cc| {
            // By the time eframe gets here it has spent its winit + GL
            // setup time; any leftover wait against the capture thread
            // is the *parallel* overlap we wanted.
            let image = capture_handle
                .join()
                .map_err(|_| "capture worker panicked".to_string())??;
            Ok(Box::new(SelectApp::new(cc, image, result_clone)))
        }),
    )?;

    Ok(result.lock().unwrap().take())
}

struct SelectApp {
    base_w: u32,
    base_h: u32,
    texture: egui::TextureHandle,
    drag_start: Option<egui::Pos2>,
    drag_now: Option<egui::Pos2>,
    finalized: bool,
    base: RgbaImage,
    result: std::sync::Arc<std::sync::Mutex<Option<RgbaImage>>>,
}

impl SelectApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        base: RgbaImage,
        result: std::sync::Arc<std::sync::Mutex<Option<RgbaImage>>>,
    ) -> Self {
        let (w, h) = (base.width(), base.height());
        let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], base.as_raw());
        let texture =
            cc.egui_ctx
                .load_texture("boltsnap-select-bg", color, egui::TextureOptions::LINEAR);
        Self {
            base_w: w,
            base_h: h,
            texture,
            drag_start: None,
            drag_now: None,
            finalized: false,
            base,
            result,
        }
    }

    fn rect_to_image(&self, p: egui::Pos2, rect: egui::Rect) -> (u32, u32) {
        let nx = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let ny = ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        (
            (nx * self.base_w as f32).round() as u32,
            (ny * self.base_h as f32).round() as u32,
        )
    }
}

impl eframe::App for SelectApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.finalized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
            *self.result.lock().unwrap() = None;
            self.finalized = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let frame = egui::Frame::new()
            .fill(egui::Color32::BLACK)
            .inner_margin(0);
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                let painter = ui.painter();

                let uv = egui::Rect::from_min_max(
                    egui::pos2(0.0, 0.0),
                    egui::pos2(1.0, 1.0),
                );
                painter.image(self.texture.id(), rect, uv, egui::Color32::WHITE);

                if response.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(p) = response.interact_pointer_pos() {
                        self.drag_start = Some(p);
                        self.drag_now = Some(p);
                    }
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(p) = ctx.pointer_latest_pos() {
                        self.drag_now = Some(p);
                    }
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b)) = (self.drag_start, self.drag_now) {
                        let (ax, ay) = self.rect_to_image(a, rect);
                        let (bx, by) = self.rect_to_image(b, rect);
                        let x = ax.min(bx);
                        let y = ay.min(by);
                        let w = ax.max(bx).saturating_sub(x);
                        let h = ay.max(by).saturating_sub(y);
                        if w > 1 && h > 1 {
                            // Crop here so the parent doesn't have to keep
                            // a copy of the full base image around.
                            let cropped =
                                image::imageops::crop_imm(&self.base, x, y, w, h)
                                    .to_image();
                            *self.result.lock().unwrap() = Some(cropped);
                            self.finalized = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else {
                            self.drag_start = None;
                            self.drag_now = None;
                        }
                    }
                }

                let dim = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110);
                match (self.drag_start, self.drag_now) {
                    (Some(a), Some(b)) => {
                        let sel = egui::Rect::from_two_pos(a, b);
                        let outside_top = egui::Rect::from_min_max(
                            rect.min,
                            egui::pos2(rect.right(), sel.top()),
                        );
                        let outside_bottom = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), sel.bottom()),
                            rect.max,
                        );
                        let outside_left = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), sel.top()),
                            egui::pos2(sel.left(), sel.bottom()),
                        );
                        let outside_right = egui::Rect::from_min_max(
                            egui::pos2(sel.right(), sel.top()),
                            egui::pos2(rect.right(), sel.bottom()),
                        );
                        for r in [outside_top, outside_bottom, outside_left, outside_right]
                        {
                            if r.width() > 0.0 && r.height() > 0.0 {
                                painter.rect_filled(r, 0.0, dim);
                            }
                        }
                        painter.rect_stroke(
                            sel,
                            0.0,
                            egui::Stroke::new(1.5, egui::Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                        let label = format!(
                            "{}x{}",
                            sel.width().round() as i32,
                            sel.height().round() as i32
                        );
                        painter.text(
                            sel.left_top() + egui::vec2(6.0, -4.0),
                            egui::Align2::LEFT_BOTTOM,
                            label,
                            egui::FontId::monospace(12.0),
                            egui::Color32::WHITE,
                        );
                    }
                    _ => {
                        painter.rect_filled(rect, 0.0, dim);
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "drag to select • Esc to cancel",
                            egui::FontId::proportional(14.0),
                            egui::Color32::from_white_alpha(220),
                        );
                    }
                }
            });
    }
}

fn render_annotations(base: &RgbaImage, annotations: &[Annotation]) -> RgbaImage {
    let mut out = base.clone();
    for ann in annotations {
        if ann.points.len() < 2 {
            continue;
        }
        match ann.tool {
            Tool::Move => {}
            Tool::Pen => {
                for pair in ann.points.windows(2) {
                    draw_thick_line(&mut out, pair[0], pair[1], Rgba([255, 48, 48, 255]), 5.0);
                }
            }
            Tool::Arrow => {
                let a = ann.points[0];
                let b = *ann.points.last().unwrap();
                draw_thick_line(&mut out, a, b, Rgba([255, 48, 48, 255]), 5.0);
                let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
                let len = 26.0;
                for spread in [2.55_f32, -2.55_f32] {
                    let end = [
                        b[0] + len * (angle + spread).cos(),
                        b[1] + len * (angle + spread).sin(),
                    ];
                    draw_thick_line(&mut out, b, end, Rgba([255, 48, 48, 255]), 5.0);
                }
            }
            Tool::Rect => draw_rect_outline(
                &mut out,
                ann.points[0],
                ann.points[1],
                Rgba([255, 48, 48, 255]),
                5,
            ),
            Tool::Highlight => fill_rect_alpha(
                &mut out,
                ann.points[0],
                ann.points[1],
                Rgba([255, 230, 0, 90]),
            ),
            Tool::Redact => fill_rect(&mut out, ann.points[0], ann.points[1], Rgba([0, 0, 0, 255])),
            Tool::Blur => blur_rect(&mut out, ann.points[0], ann.points[1], 10.0),
        }
    }
    out
}

fn blur_rect(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], sigma: f32) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    if x2 <= x1 || y2 <= y1 {
        return;
    }
    let w = (x2 - x1 + 1) as u32;
    let h = (y2 - y1 + 1) as u32;
    let crop = imageops::crop_imm(img, x1 as u32, y1 as u32, w, h).to_image();
    let blurred = imageops::blur(&crop, sigma);
    imageops::replace(img, &blurred, x1 as i64, y1 as i64);
}

fn draw_thick_line(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>, radius: f32) {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let steps = dx.abs().max(dy.abs()).max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        draw_disc(img, a[0] + dx * t, a[1] + dy * t, radius, color);
    }
}

fn draw_disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: Rgba<u8>) {
    let r2 = r * r;
    let min_x = (cx - r).floor() as i32;
    let max_x = (cx + r).ceil() as i32;
    let min_y = (cy - r).floor() as i32;
    let max_y = (cy + r).ceil() as i32;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2) <= r2 {
                put_pixel_checked(img, x, y, color);
            }
        }
    }
}

fn draw_rect_outline(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>, width: i32) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    for w in 0..width {
        for x in x1..=x2 {
            put_pixel_checked(img, x, y1 + w, color);
            put_pixel_checked(img, x, y2 - w, color);
        }
        for y in y1..=y2 {
            put_pixel_checked(img, x1 + w, y, color);
            put_pixel_checked(img, x2 - w, y, color);
        }
    }
}

fn fill_rect(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    for y in y1..=y2 {
        for x in x1..=x2 {
            put_pixel_checked(img, x, y, color);
        }
    }
}

fn fill_rect_alpha(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    let alpha = color[3] as f32 / 255.0;
    for y in y1..=y2 {
        for x in x1..=x2 {
            if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
                continue;
            }
            let p = img.get_pixel_mut(x as u32, y as u32);
            for c in 0..3 {
                p[c] = ((p[c] as f32 * (1.0 - alpha)) + (color[c] as f32 * alpha)).round() as u8;
            }
            p[3] = 255;
        }
    }
}

fn rect_bounds(img: &RgbaImage, a: [f32; 2], b: [f32; 2]) -> (i32, i32, i32, i32) {
    let max_x = img.width().saturating_sub(1) as i32;
    let max_y = img.height().saturating_sub(1) as i32;
    let x1 = (a[0].min(b[0]).round() as i32).clamp(0, max_x);
    let y1 = (a[1].min(b[1]).round() as i32).clamp(0, max_y);
    let x2 = (a[0].max(b[0]).round() as i32).clamp(0, max_x);
    let y2 = (a[1].max(b[1]).round() as i32).clamp(0, max_y);
    (x1, y1, x2, y2)
}

fn put_pixel_checked(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && x < img.width() as i32 && y < img.height() as i32 {
        img.put_pixel(x as u32, y as u32, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;

    #[test]
    fn parser_defaults_to_area_copy() {
        let args = parse_args(&["boltsnap".into()]).unwrap();
        assert_eq!(args.command, "area");
        assert!(!args.command_explicit);
        assert!(args.copy);
    }

    #[test]
    fn parser_edit_without_command_means_edit_last() {
        let args = parse_args(&["boltsnap".into(), "--edit".into()]).unwrap();
        assert!(args.edit);
        assert!(!args.command_explicit);
        assert_eq!(args.command, "area");
    }

    #[test]
    fn parser_handles_window_modes() {
        let args = parse_args(&["boltsnap".into(), "window".into(), "--edit".into()]).unwrap();
        assert_eq!(
            CaptureMode::parse(&args.command).unwrap(),
            CaptureMode::Window
        );
        assert!(args.edit);
        assert_eq!(
            CaptureMode::parse("active-window").unwrap(),
            CaptureMode::ActiveWindow
        );
        assert_eq!(CaptureMode::parse("select").unwrap(), CaptureMode::Area);
    }

    #[test]
    fn parser_handles_edit_output_no_copy() {
        let args = parse_args(&[
            "boltsnap".into(),
            "edit".into(),
            "a.png".into(),
            "--no-copy".into(),
            "-o".into(),
            "b.png".into(),
        ])
        .unwrap();
        assert_eq!(args.command, "edit");
        assert_eq!(args.image.unwrap(), PathBuf::from("a.png"));
        assert!(!args.copy);
        assert_eq!(args.output.unwrap(), PathBuf::from("b.png"));
    }

    #[test]
    fn parse_hypr_geometry() {
        let json = r#"{"at":[100,200],"size":[900,700],"title":"x"}"#;
        assert_eq!(
            parse_hypr_window_geometry(json).as_deref(),
            Some("100,200 900x700")
        );
    }

    #[test]
    fn render_redaction_blacks_region() {
        let base: RgbaImage = ImageBuffer::from_pixel(50, 50, Rgba([255, 255, 255, 255]));
        let ann = Annotation {
            tool: Tool::Redact,
            points: vec![[10.0, 10.0], [20.0, 20.0]],
        };
        let out = render_annotations(&base, &[ann]);
        assert_eq!(*out.get_pixel(15, 15), Rgba([0, 0, 0, 255]));
        assert_eq!(*out.get_pixel(5, 5), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn render_arrow_draws_end() {
        let base: RgbaImage = ImageBuffer::from_pixel(80, 80, Rgba([255, 255, 255, 255]));
        let ann = Annotation {
            tool: Tool::Arrow,
            points: vec![[5.0, 5.0], [60.0, 60.0]],
        };
        let out = render_annotations(&base, &[ann]);
        assert_ne!(*out.get_pixel(60, 60), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn render_blur_changes_noisy_region() {
        let mut base: RgbaImage = ImageBuffer::from_pixel(40, 40, Rgba([255, 255, 255, 255]));
        for x in 0..20 {
            for y in 0..40 {
                base.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let ann = Annotation {
            tool: Tool::Blur,
            points: vec![[8.0, 0.0], [25.0, 39.0]],
        };
        let out = render_annotations(&base, &[ann]);
        assert_ne!(*out.get_pixel(18, 20), *base.get_pixel(18, 20));
        assert_eq!(*out.get_pixel(35, 20), Rgba([255, 255, 255, 255]));
    }
}
