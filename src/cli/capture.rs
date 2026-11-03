//! A screenshot from capture to its sink: stdout, a file, the clipboard or
//! the shelf.

use std::fs;
use std::io::Write;

use super::require_daemon_success;
use crate::capture::capture;
#[cfg(target_os = "windows")]
use crate::capture::strip_uniform_border;
use crate::clipboard::copy_to_clipboard;
use crate::config;
use crate::paths::target_path;
#[cfg(target_os = "windows")]
use crate::paths::temp_png;
use crate::{Args, Backend, CaptureMode, DynResult};

/// What to do with a freshly captured screenshot, after backend resolution.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PostCapture {
    Stdout,
    File { copy: bool },
    Shelf { copy: bool },
    CopyOnly,
}

/// Decide the post-capture sink. `backend` is already resolved (no Auto).
/// `copy_to_clipboard` is the config's wish for Wayland shelf captures; a
/// `--copy`/`--no-copy` flag goes first.
pub(super) fn decide_post_capture(
    args: &Args,
    backend: Backend,
    copy_to_clipboard: bool,
) -> PostCapture {
    if is_stdout_target(args) {
        return PostCapture::Stdout;
    }
    if args.output.is_some() || args.save {
        return PostCapture::File { copy: args.copy };
    }
    match backend {
        Backend::Wayland => PostCapture::Shelf {
            copy: if args.copy_explicit {
                args.copy
            } else {
                copy_to_clipboard
            },
        },
        Backend::Windows => PostCapture::Shelf {
            copy: args.copy_explicit && args.copy,
        },
        _ => PostCapture::CopyOnly,
    }
}

pub(crate) fn capture_flow(args: &Args) -> DynResult<()> {
    let mode = CaptureMode::parse(&args.command)?;

    if is_stdout_target(args) {
        return capture_to_stdout(mode, args.backend, args.instant);
    }

    #[cfg(target_os = "linux")]
    if mode == CaptureMode::Full
        && args.backend.resolved()? == Backend::Wayland
        && full_from_daemon(args)?
    {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    if args.output.is_none() && !args.save && args.backend.resolved()? == Backend::Wayland {
        return wayland_to_shelf(args, mode);
    }

    capture_to_file(args, mode)
}

/// A full-desktop PNG from the running daemon skips connecting to the
/// compositor and handing the PNG over. False: this sink or this daemon
/// cannot, so the CLI captures itself.
#[cfg(target_os = "linux")]
fn full_from_daemon(args: &Args) -> DynResult<bool> {
    let copy_wish = config::Config::load().copy_to_clipboard();
    let (output, copy) = match decide_post_capture(args, Backend::Wayland, copy_wish) {
        PostCapture::File { copy: false } => (Some(std::path::absolute(target_path(args))?), false),
        PostCapture::Shelf { copy } => (None, copy),
        _ => return Ok(false),
    };
    Ok(crate::capture::full_in_daemon(output, copy)?)
}

/// Wayland without `-o`/`--save`: the capture goes straight to the shelf.
#[cfg(target_os = "linux")]
fn wayland_to_shelf(args: &Args, mode: CaptureMode) -> DynResult<()> {
    let (resolved, capture_output, image) =
        crate::capture::capture_image(mode, args.backend, args.instant, true)?;
    let copy = matches!(
        decide_post_capture(args, resolved, config::Config::load().copy_to_clipboard()),
        PostCapture::Shelf { copy: true }
    );
    let image = image.into_rgb8();
    // Measured desktop regressions keep the raw transport experimental.
    if std::env::var("BOLTSNAP_RAW_TRANSFER").as_deref() == Ok("1")
        && crate::platform::image_transfer::try_add(
            &image,
            mode.label(),
            capture_output.clone(),
            copy,
        )?
    {
        println!(
            "Boltsnap sent {} to shelf{}",
            mode.label(),
            if copy {
                " (clipboard helper started)"
            } else {
                ""
            }
        );
        return Ok(());
    }
    // Default PNG transport, also used after an older daemon closes the
    // nonmutating raw offer. No image has been accepted on that path yet.
    let thumb = card_thumb(&image);
    let png = crate::capture::encode_png(&image)?;
    if copy {
        // The clipboard helper outlives this CLI. Keep its uniquely-owned
        // source; last.png can be replaced by another concurrent capture.
        let output = target_path(args);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, &png)?;
        copy_temporary_capture(&output, resolved)?;
    }
    let last = crate::paths::publish_last_png(&png)?;
    send_shelf_add(crate::ipc::Request::Add {
        source: mode.label().into(),
        png,
        output: capture_output,
        thumb: Some(thumb),
    })
    .map_err(|error| format!("{error}; the screenshot is kept at {}", last.display()))?;
    println!(
        "Boltsnap sent {} to shelf{}",
        mode.label(),
        if copy { " (copied)" } else { "" }
    );
    Ok(())
}

/// The card thumbnail comes along so the shelf need not decode the PNG.
#[cfg(target_os = "linux")]
fn card_thumb(image: &image::RgbImage) -> crate::protocol::CardThumb {
    let _timing = crate::platform::timing::Span::new("card_thumbnail");
    let card = crate::config::Config::load().shelf_prefs().card;
    let thumb = crate::shelf::thumbnail::make_rgb_card_thumbnail(
        &image::ImageBuffer::from_raw(image.width(), image.height(), image.as_raw().as_slice())
            .expect("RGB view matches its image"),
        card.w,
        card.h,
    );
    crate::protocol::CardThumb {
        width: thumb.width(),
        height: thumb.height(),
        rgba: thumb.into_raw(),
    }
}

/// Every other sink: the capture lands in a file first, then goes to the
/// clipboard, the shelf or stays where `-o`/`--save` put it.
fn capture_to_file(args: &Args, mode: CaptureMode) -> DynResult<()> {
    let output = target_path(args);
    let (resolved, capture_output) = capture(mode, &output, args.backend, args.instant)?;
    #[cfg(target_os = "windows")]
    if matches!(mode, CaptureMode::Window | CaptureMode::ActiveWindow) {
        let _ = strip_uniform_border(&output);
    }
    let default_png = (args.output.is_none() && !args.save)
        .then(|| fs::read(&output))
        .transpose()?;

    match decide_post_capture(args, resolved, config::Config::load().copy_to_clipboard()) {
        PostCapture::Stdout => unreachable!("handled above"),
        PostCapture::File { copy } => {
            if copy {
                copy_to_clipboard(&output, resolved)?;
            }
            let verb = if copy { "copied" } else { "captured" };
            println!(
                "Boltsnap {verb} {} via {}: {}",
                mode.label(),
                resolved.as_str(),
                output.display()
            );
        }
        PostCapture::CopyOnly => {
            copy_temporary_capture(&output, resolved)?;
            #[cfg(target_os = "linux")]
            if let Some(png) = default_png {
                crate::paths::publish_last_png(&png)?;
                let _ = fs::remove_file(&output);
            }
            println!(
                "Boltsnap copied {} via {}: {}",
                mode.label(),
                resolved.as_str(),
                output.display()
            );
        }
        PostCapture::Shelf { copy } => {
            let png = match default_png {
                Some(png) => png,
                None => fs::read(&output)?,
            };
            if copy {
                if args.output.is_none() && !args.save {
                    copy_temporary_capture(&output, resolved)?;
                } else {
                    copy_to_clipboard(&output, resolved)?;
                }
            }
            #[cfg(target_os = "linux")]
            if args.output.is_none() && !args.save {
                crate::paths::publish_last_png(&png)?;
                if !copy {
                    let _ = fs::remove_file(&output);
                }
            }
            send_shelf_add(crate::ipc::Request::Add {
                source: mode.label().to_string(),
                png,
                output: capture_output,
                thumb: None,
            })?;
            let suffix = if copy { " (copied)" } else { "" };
            println!("Boltsnap sent {} to shelf{}", mode.label(), suffix);
        }
    }
    Ok(())
}

fn send_shelf_add(request: crate::ipc::Request) -> DynResult<()> {
    #[cfg(target_os = "linux")]
    {
        let _timing = crate::platform::timing::Span::new("png_transfer_and_shelf_ack");
        let response = crate::ipc::add_to_shelf(request)
            .map_err(|error| format!("shelf daemon unavailable: {error}"))?;
        require_daemon_success(response, "shelf ingest failed")?;
    }
    #[cfg(target_os = "windows")]
    crate::ipc::send_to_shelf(request)?;
    Ok(())
}

fn copy_temporary_capture(path: &std::path::Path, backend: Backend) -> DynResult<Backend> {
    #[cfg(target_os = "linux")]
    {
        crate::clipboard::copy_temporary_to_clipboard(path, backend)
    }
    #[cfg(target_os = "windows")]
    {
        copy_to_clipboard(path, backend)
    }
}

fn is_stdout_target(args: &Args) -> bool {
    args.output.as_deref().and_then(|p| p.to_str()) == Some("-")
}

#[cfg(target_os = "linux")]
fn capture_to_stdout(mode: CaptureMode, backend: Backend, instant: bool) -> DynResult<()> {
    let (_, _, bytes) = crate::capture::capture_png(mode, backend, instant, false)?;
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn capture_to_stdout(mode: CaptureMode, backend: Backend, instant: bool) -> DynResult<()> {
    let tmp = temp_png("stdout");
    capture(mode, &tmp, backend, instant)?;
    let bytes = fs::read(&tmp)?;
    let _ = fs::remove_file(&tmp);
    std::io::stdout().lock().write_all(&bytes)?;
    Ok(())
}
