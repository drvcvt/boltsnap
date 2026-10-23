use super::*;

#[test]
fn popup_keys_map_to_their_buttons() {
    assert_eq!(
        popup_key_button(Keysym::space),
        Some(PopupButton::PauseResume)
    );
    assert_eq!(
        popup_key_button(Keysym::Return),
        Some(PopupButton::SaveShelf)
    );
    assert_eq!(
        popup_key_button(Keysym::KP_Enter),
        Some(PopupButton::SaveShelf)
    );
    assert_eq!(popup_key_button(Keysym::d), Some(PopupButton::SaveDisk));
    assert_eq!(popup_key_button(Keysym::D), Some(PopupButton::SaveDisk));
    assert_eq!(popup_key_button(Keysym::Delete), Some(PopupButton::Discard));
    assert_eq!(popup_key_button(Keysym::x), None);
    assert_eq!(popup_key_button(Keysym::Escape), None);
}

#[test]
fn controls_pause_a_running_recording_before_they_show() {
    assert_eq!(
        controls_open_plan(SessionPhase::Recording),
        ControlsOpen::PauseFirst
    );
    assert_eq!(
        controls_open_plan(SessionPhase::Pausing),
        ControlsOpen::AfterPause
    );
    for phase in [SessionPhase::Paused, SessionPhase::Finalizing] {
        assert_eq!(controls_open_plan(phase), ControlsOpen::Now);
    }
}

#[test]
fn esc_resumes_only_what_the_controls_paused() {
    assert!(esc_resumes(true, SessionPhase::Paused, false));
    assert!(!esc_resumes(false, SessionPhase::Paused, false));
    assert!(!esc_resumes(true, SessionPhase::Paused, true));
    assert!(!esc_resumes(true, SessionPhase::Finalizing, false));
}

#[test]
fn discard_needs_a_second_press_in_time() {
    let now = std::time::Instant::now();
    assert!(!discard_confirmed(None, now));
    assert!(discard_confirmed(
        Some(now),
        now + std::time::Duration::from_secs(2)
    ));
    assert!(!discard_confirmed(Some(now), now + DISCARD_CONFIRM));
}
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn png_bytes() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[test]
fn pixels_prepare_preserves_rgb_and_unpublished_file_ownership() {
    use super::super::image_transfer::{Metadata, Pixels, sealed_pixels};
    let rgb =
        image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8, y as u8, (x + y) as u8]));
    let pixels = Pixels::map(sealed_pixels(rgb.as_raw()).unwrap(), rgb.as_raw().len()).unwrap();
    let metadata = || Metadata::new(64, 48, "area".into(), Some("DP-3".into()), false).unwrap();
    let add = prepare_pixels_with(
        metadata(),
        &pixels,
        |png| {
            assert_eq!(image::load_from_memory(png).unwrap().into_rgb8(), rgb);
            Ok(())
        },
        |_| panic!("copy disabled"),
    )
    .unwrap();
    assert_eq!(image::open(&add.path).unwrap().into_rgb8(), rgb);
    assert_eq!(
        add.thumb,
        crate::shelf::thumbnail::make_card_thumbnail(
            &image::DynamicImage::ImageRgb8(rgb).into_rgba8(),
            190,
            132
        )
    );
    let path = add.path.clone();
    drop(add);
    assert!(!path.exists(), "an unpublished item owns its file");
    assert!(
        prepare_pixels_with(
            metadata(),
            &pixels,
            |_| Err("injected publish failure".into()),
            |_| panic!()
        )
        .is_err()
    );
    let mut copy_meta = metadata();
    copy_meta.copy = true;
    let mut copy_path = None;
    let result = prepare_pixels_with(
        copy_meta,
        &pixels,
        |_| Ok(()),
        |path| {
            copy_path = Some(path.to_owned());
            assert!(path.is_file());
            Err("injected helper failure".into())
        },
    );
    assert!(result.err().unwrap().contains("last PNG saved"));
    assert!(!copy_path.unwrap().exists());
}

#[test]
fn pixel_offer_reserves_shared_capacity_before_ready_and_releases_after_bad_fd() {
    use super::super::image_transfer::{COMMAND, Metadata};
    use std::io::Write;
    let limits = ClientLimits::default();
    let first = limits.images.acquire().unwrap();
    let (tx, rx) = calloop::channel::channel();
    let (server, mut client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    spawn_client_reader(server, tx.clone(), &limits);
    let meta = Metadata::new(1, 1, "area".into(), None, false).unwrap();
    // Exercise a fragmented header, not just one socket write.
    let mut offer = Vec::new();
    crate::protocol::write_frame(&mut offer, &meta.header(), &[]).unwrap();
    for byte in offer {
        client.write_all(&[byte]).unwrap();
    }
    let (header, _) = crate::protocol::read_frame(&mut client).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&header).unwrap()["ready"],
        COMMAND
    );
    assert!(limits.images.acquire().is_none());
    let (server, mut excess) = UnixStream::pair().unwrap();
    spawn_client_reader(server, tx, &limits);
    crate::protocol::write_frame(&mut excess, &meta.header(), &[]).unwrap();
    assert!(!crate::protocol::Response::read(&mut excess).unwrap().ok);
    client.write_all(b"P").unwrap(); // no FD
    let event = receive_event(&rx);
    assert!(matches!(
        &event,
        DaemonEvent::AddPrepared { result: Err(_), .. }
    ));
    assert!(limits.images.acquire().is_none());
    drop(event);
    assert!(limits.images.acquire().is_some());
    drop(first);
}

#[test]
fn grayscale_png_cannot_expand_beyond_rgba_pixel_budget() {
    // Valid headers for 8193x8192 grayscale. Rejection must precede decoding
    // the deliberately incomplete compressed body or allocating RGBA.
    let png: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 32, 1, 0, 0, 32, 0, 8,
        0, 0, 0, 0, 184, 3, 254, 187, 0, 0, 0, 10, 73, 68, 65, 84, 120, 156, 99, 96, 0, 0, 0, 2, 0,
        1, 72, 175, 164, 113, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    assert!(
        decode_shelf_image(png)
            .unwrap_err()
            .contains("64 megapixel")
    );
}

#[test]
fn media_card_opener_uses_desktop_default() {
    for name in [
        "/tmp/example.png",
        "/tmp/recording with spaces.mp4",
        "/tmp/clip.webm",
    ] {
        let path = std::path::Path::new(name);
        let command = media_open_command(path);
        assert_eq!(command.get_program(), "xdg-open");
        assert_eq!(command.get_args().collect::<Vec<_>>(), [path.as_os_str()]);
    }
}

fn import_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "boltsnap-import-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("input")).unwrap();
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    dir
}

fn cache_entries(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir.join("cache")).unwrap().count()
}

#[test]
fn imported_videos_own_their_bytes() {
    let dir = import_dir("own");
    let input = dir.join("input");
    let real = input.join("real.mp4");
    std::fs::write(&real, b"video").unwrap();
    std::os::unix::fs::symlink("real.mp4", input.join("relative.mp4")).unwrap();
    std::os::unix::fs::symlink(&real, input.join("absolute.mp4")).unwrap();

    for name in ["real.mp4", "relative.mp4", "absolute.mp4"] {
        let imported =
            import_shelf_video(&input.join(name), &dir.join("cache"), |_| Ok(())).unwrap();
        let metadata = std::fs::symlink_metadata(&imported).unwrap();
        assert!(metadata.file_type().is_file(), "{name}");
        // An in-place write to the original must not reach the card.
        std::fs::write(&real, b"VIDEO").unwrap();
        assert_eq!(std::fs::read(&imported).unwrap(), b"video", "{name}");
        std::fs::write(&real, b"video").unwrap();
    }
    std::fs::remove_file(&real).unwrap();
    assert_eq!(cache_entries(&dir), 3);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unusable_video_sources_import_nothing() {
    let dir = import_dir("reject");
    let input = dir.join("input");
    std::fs::write(input.join("empty.mp4"), b"").unwrap();
    std::fs::create_dir(input.join("folder.mp4")).unwrap();
    std::os::unix::fs::symlink("gone.mp4", input.join("dangling.mp4")).unwrap();
    let fifo = std::ffi::CString::new(input.join("pipe.mp4").into_os_string().into_encoded_bytes())
        .unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::write(input.join("big.mp4"), b"video").unwrap();

    for name in ["empty.mp4", "folder.mp4", "dangling.mp4", "pipe.mp4"] {
        assert!(
            import_shelf_video(&input.join(name), &dir.join("cache"), |_| Ok(())).is_err(),
            "{name}"
        );
    }
    let refused = import_shelf_video(&input.join("big.mp4"), &dir.join("cache"), |bytes| {
        assert_eq!(bytes, 5);
        Err("cache full".into())
    });
    assert_eq!(refused.unwrap_err(), "cache full");
    assert_eq!(cache_entries(&dir), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dragged_files_are_streamed_in_small_writes() {
    /// Keeps what it gets and the largest single write.
    struct Receiver(Vec<u8>, usize);
    impl std::io::Write for Receiver {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.1 = self.1.max(bytes.len());
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let dir = import_dir("drag");
    let path = dir.join("input/clip.mp4");
    let bytes: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();

    let mut receiver = Receiver(Vec::new(), 0);
    assert_eq!(send_file(&path, &mut receiver).unwrap(), bytes.len() as u64);
    assert!(receiver.0 == bytes);
    assert!(receiver.1 <= 64 * 1024, "one write of {} bytes", receiver.1);

    // A receiver that goes away stops the transfer.
    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    drop(reader);
    assert!(send_file(&path, &mut writer).is_err());
    assert!(send_file(&dir.join("input/gone.mp4"), &mut Receiver(Vec::new(), 0)).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hung_up_watchers_are_noticed_without_a_status_change() {
    let (open, _open_peer) = UnixStream::pair().unwrap();
    let (chatty, mut chatty_peer) = UnixStream::pair().unwrap();
    chatty_peer.write_all(b"x").unwrap();
    let (closed, closed_peer) = UnixStream::pair().unwrap();
    drop(closed_peer);
    let (sent_then_closed, mut peer) = UnixStream::pair().unwrap();
    peer.write_all(b"x").unwrap();
    drop(peer);

    assert!(watcher_connected(&open));
    assert!(watcher_connected(&chatty));
    assert!(!watcher_connected(&closed));
    // Unread client bytes come first; the hang-up shows once they are read.
    assert!(watcher_connected(&sent_then_closed));
    // A blocking socket is peeked without blocking.
    open.set_nonblocking(false).unwrap();
    assert!(watcher_connected(&open));

    let mut watchers = Vec::new();
    for _ in 0..100 {
        let (daemon_side, client) = UnixStream::pair().unwrap();
        drop(client);
        watchers.retain(watcher_connected);
        watchers.push(daemon_side);
    }
    assert_eq!(watchers.len(), 1);
}

#[test]
fn drags_offer_the_mime_type_of_the_file() {
    let offer = |kind, name| drag_mime_types(kind, std::path::Path::new(name));
    assert_eq!(
        offer(CardKind::Image, "/c/shot.png"),
        ["image/png", "text/uri-list"]
    );
    assert_eq!(
        offer(CardKind::Video, "/c/rec.mp4"),
        ["video/mp4", "text/uri-list"]
    );
    assert_eq!(
        offer(CardKind::Video, "/c/clip-1.MKV"),
        ["video/x-matroska", "text/uri-list"]
    );
    assert_eq!(
        offer(CardKind::Video, "/c/in.webm"),
        ["video/webm", "text/uri-list"]
    );
    assert_eq!(offer(CardKind::Video, "/c/odd.avi"), ["text/uri-list"]);
    assert_eq!(offer(CardKind::Video, "/c/bare"), ["text/uri-list"]);
}

#[test]
fn cache_eviction_removes_oldest_until_count_and_image_bytes_fit() {
    let newest_first = [
        (5, CardKind::Image, FileLifetime::Temporary, 40),
        (4, CardKind::Image, FileLifetime::Temporary, 40),
        (3, CardKind::Video, FileLifetime::Permanent, 999),
        (2, CardKind::Image, FileLifetime::Temporary, 40),
        (1, CardKind::Image, FileLifetime::Temporary, 40),
    ];

    assert_eq!(
        shelf_cache_evictions_with_limits(&newest_first, 4, 100, |_| false),
        vec![1, 2]
    );
    // A card being saved is skipped; the next oldest goes instead.
    assert_eq!(
        shelf_cache_evictions_with_limits(&newest_first, 4, 100, |id| id == 1),
        vec![2, 3, 4]
    );
    assert_eq!(
        shelf_cache_evictions_with_limits(&newest_first, 1, 0, |id| id != 5),
        vec![5]
    );
}

fn client_thumb(width: u32, height: u32) -> crate::protocol::CardThumb {
    crate::protocol::CardThumb {
        width,
        height,
        rgba: vec![7; (width * height * 4) as usize],
    }
}

/// A PNG with a valid header whose image data is damaged.
fn png_with_broken_data() -> Vec<u8> {
    let rgb = image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8, y as u8, 9]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(rgb)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let idat = png.windows(4).position(|tag| tag == b"IDAT").unwrap();
    png[idat + 4..idat + 14].fill(0xff);
    png
}

#[test]
fn client_thumbnail_skips_decoding_the_png() {
    use crate::shelf::thumbnail::{CARD_H, CARD_W};
    let png = png_with_broken_data();
    assert!(prepare_add(png.clone(), "full".into(), None, None).is_err());
    let add = prepare_add(png, "full".into(), None, Some(client_thumb(CARD_W, CARD_H))).unwrap();
    assert_eq!(add.thumb.as_raw(), &vec![7; (CARD_W * CARD_H * 4) as usize]);
    // A thumbnail of the wrong size is ignored and the PNG decoded instead.
    let add = prepare_add(png_bytes(), "full".into(), None, Some(client_thumb(2, 2))).unwrap();
    assert_eq!(add.thumb.dimensions(), (CARD_W, CARD_H));
    assert_ne!(add.thumb.as_raw()[..4], [7, 7, 7, 7]);
}

#[test]
fn client_thumbnail_still_enforces_the_pixel_limit() {
    use crate::shelf::thumbnail::{CARD_H, CARD_W};
    use image::ImageEncoder;
    // 9000x9000 header, far above 64 MP; the body never gets read.
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[0; 3], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    png[16..20].copy_from_slice(&9000u32.to_be_bytes());
    png[20..24].copy_from_slice(&9000u32.to_be_bytes());
    let crc = crc32fast_ihdr(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    let Err(error) = prepare_add(png, "full".into(), None, Some(client_thumb(CARD_W, CARD_H)))
    else {
        panic!("oversized PNG was accepted")
    };
    assert!(error.contains("64 megapixel"), "{error}");
}

/// CRC-32 (PNG polynomial) of a chunk's type and data.
fn crc32fast_ihdr(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn cli_card_thumbnail_matches_the_decoded_one() {
    use crate::shelf::thumbnail::{CARD_H, CARD_W};
    let rgb = image::RgbImage::from_fn(1555, 1080, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 256) as u8])
    });
    let from_cli = crate::shelf::thumbnail::make_rgb_card_thumbnail(
        &image::ImageBuffer::from_raw(rgb.width(), rgb.height(), rgb.as_raw().as_slice()).unwrap(),
        CARD_W,
        CARD_H,
    );
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(rgb)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let decoded = crate::shelf::thumbnail::make_image_card_thumbnail(
        &decode_shelf_image(&png).unwrap(),
        CARD_W,
        CARD_H,
    );
    assert_eq!(from_cli, decoded);
}

#[test]
fn invalid_png_preparation_returns_an_ingest_error() {
    let result = prepare_add(vec![1, 2, 3], "area".into(), None, None);
    let Err(error) = result else {
        panic!("invalid PNG was accepted")
    };
    assert!(error.contains("decode PNG"), "{error}");
}

#[test]
fn idle_and_waiting_recordings_do_not_poll_but_active_recorders_do() {
    assert_eq!(dispatch_timeout(false, None), None);
    for phase in [
        SessionPhase::Paused,
        SessionPhase::Pausing,
        SessionPhase::Finalizing,
        SessionPhase::Discarding,
    ] {
        assert_eq!(dispatch_timeout(false, Some(phase)), None);
    }
    assert_eq!(
        dispatch_timeout(false, Some(SessionPhase::Recording)),
        Some(Duration::from_millis(250))
    );
    assert_eq!(
        dispatch_timeout(true, None),
        Some(Duration::from_millis(16))
    );
    assert_eq!(
        dispatch_timeout(true, Some(SessionPhase::Recording)),
        Some(Duration::from_millis(16))
    );
}

#[test]
fn saturated_images_leave_control_requests_available_and_release_capacity() {
    let limits = ClientLimits::default();
    let first = limits.images.acquire().unwrap();
    let second = limits.images.acquire().unwrap();
    let (tx, rx) = calloop::channel::channel();
    let (server, mut client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    spawn_client_reader(server, tx.clone(), &limits);
    // Only send the prefix: rejection must precede reading/allocating 256 MiB.
    client.write_all(&[0, 0, 0, 2, 16, 0, 0, 0]).unwrap();
    let response = crate::ipc::Response::read(&mut client).unwrap();
    assert!(!response.ok);
    assert!(response.error.unwrap().contains("busy"));

    let (server, mut control) = UnixStream::pair().unwrap();
    spawn_client_reader(server, tx.clone(), &limits);
    control
        .write_all(&crate::ipc::Request::Ping.encode())
        .unwrap();
    assert!(matches!(
        receive_event(&rx),
        DaemonEvent::ClientRequest {
            request: crate::ipc::Request::Ping,
            ..
        }
    ));

    drop(first);
    let (server, mut client) = UnixStream::pair().unwrap();
    spawn_client_reader(server, tx, &limits);
    client
        .write_all(
            &crate::ipc::Request::Add {
                source: "test".into(),
                png: vec![1],
                output: None,
                thumb: None,
            }
            .encode(),
        )
        .unwrap();
    let event = receive_event(&rx);
    assert!(matches!(
        &event,
        DaemonEvent::AddPrepared { result: Err(_), .. }
    ));
    assert!(
        limits.images.acquire().is_none(),
        "queued events retain their reservation"
    );
    drop(event);
    assert!(
        limits.images.acquire().is_some(),
        "failed work releases its reservation"
    );
    drop(second);
}

#[test]
fn add_is_prepared_before_it_reaches_the_ui_event_loop() {
    let (tx, rx) = calloop::channel::channel();
    let (server, mut client) = UnixStream::pair().unwrap();
    spawn_client_reader(server, tx, &ClientLimits::default());
    client
        .write_all(
            &crate::ipc::Request::Add {
                source: "area".into(),
                png: png_bytes(),
                output: Some("DP-3".into()),
                thumb: None,
            }
            .encode(),
        )
        .unwrap();

    match receive_event(&rx) {
        DaemonEvent::AddPrepared {
            result: Ok(add), ..
        } => {
            assert_eq!(add.source, "area");
            assert_eq!(add.output.as_deref(), Some("DP-3"));
            assert_eq!(add.thumb.dimensions(), (190, 132));
            assert!(add.path.is_file());
            let _ = std::fs::remove_file(&add.path);
        }
        _ => panic!("expected a prepared add event"),
    }
}

#[test]
fn two_dismissals_render_two_cached_replacements() {
    assert_eq!(animated_item_limit(8, false, 2), 7);
    assert_eq!(animated_item_limit(8, true, 2), 8);
}

#[test]
fn active_drag_defers_shelf_commits() {
    assert!(!shelf_commit_allowed(true));
    assert!(shelf_commit_allowed(false));
}

#[test]
fn five_card_viewport_reveals_cached_card_after_removal() {
    let mut model = ShelfModel::new();
    let ids: Vec<_> = (0..6)
        .map(|n| {
            model.add(
                std::path::PathBuf::from(format!("/tmp/card-{n}.png")),
                image::RgbaImage::new(1, 1),
                "area".into(),
            )
        })
        .collect();

    let visible: Vec<_> = visible_cards(&model).map(|card| card.id).collect();
    assert_eq!(visible, ids[1..].iter().rev().copied().collect::<Vec<_>>());
    assert!(model.get(ids[0]).is_some(), "oldest card stays cached");

    model.remove(ids[5]);
    let visible: Vec<_> = visible_cards(&model).map(|card| card.id).collect();
    assert_eq!(visible, ids[..5].iter().rev().copied().collect::<Vec<_>>());
}

#[test]
fn overflow_insertion_scrolls_oldest_below_viewport() {
    let cfg = LayoutConfig::default();
    let items: Vec<_> = (1..=6).rev().map(|id| (id, 190, 132, 0.0)).collect();

    let start = animated_card_layout(&items, &cfg, Some(0.0));
    assert_eq!(start.height, 744);
    assert_eq!(
        start.thumbs.iter().map(|card| card.y).collect::<Vec<_>>(),
        [22, 22, 164, 306, 448, 590]
    );

    let end = animated_card_layout(&items, &cfg, Some(1.0));
    assert_eq!(end.height, 744);
    assert_eq!(
        end.thumbs.iter().map(|card| card.y).collect::<Vec<_>>(),
        [22, 164, 306, 448, 590, 732]
    );
}

#[test]
fn dismissal_pulls_cached_card_back_into_viewport() {
    let cfg = LayoutConfig::default();
    let start: Vec<_> = (1..=6).rev().map(|id| (id, 190, 132, 0.0)).collect();
    let start = animated_card_layout(&start, &cfg, None);
    assert_eq!(start.height, 744);
    assert_eq!(start.thumbs[5].y, 732);

    let end: Vec<_> = (1..=6)
        .rev()
        .map(|id| (id, 190, 132, f32::from(id == 5)))
        .collect();
    let end = animated_card_layout(&end, &cfg, None);
    assert_eq!(end.height, 744);
    assert_eq!(end.thumbs[5].y, 590);
}

#[test]
fn each_cached_replacement_uses_its_own_dismiss_progress() {
    let dismissals = [0.8, 0.25];
    assert_eq!(overflow_card_opacity(5, None, &dismissals), Some(0.8));
    assert_eq!(overflow_card_opacity(6, None, &dismissals), Some(0.25));
    assert_eq!(overflow_card_opacity(5, Some(0.4), &dismissals), Some(0.6));
    assert_eq!(overflow_card_opacity(6, Some(0.4), &dismissals), Some(0.8));
    assert_eq!(overflow_card_opacity(7, Some(0.4), &dismissals), Some(0.25));
}

#[test]
fn animated_layout_hit_tracks_the_card_on_screen() {
    let cfg = LayoutConfig::default();
    let items = [(2, 190, 132, 1.0), (1, 190, 132, 0.0)];
    let animated = animated_card_layout(&items, &cfg, None);

    assert_eq!(
        animated.hit(60.0, 60.0, &cfg),
        Some(Hit::Body(1)),
        "input must follow the card after the first slot collapses"
    );
}

#[test]
fn requested_audio_respects_toggle_and_keeps_source_choice() {
    let mut prefs = crate::config::RecordingPrefs {
        audio_enabled: false,
        audio_source: crate::config::RecordAudioSource::System,
        ..Default::default()
    };
    assert_eq!(requested_audio(&prefs), None);
    prefs.audio_enabled = true;
    assert_eq!(
        requested_audio(&prefs),
        Some(crate::config::RecordAudioSource::System)
    );
}

fn receive_event(rx: &calloop::channel::Channel<DaemonEvent>) -> DaemonEvent {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match rx.try_recv() {
            Ok(event) => return event,
            Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                panic!("timed out waiting for daemon event")
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("daemon event channel disconnected")
            }
        }
    }
}

#[test]
fn partial_client_does_not_block_a_complete_client_or_drop_its_payload() {
    let (tx, rx) = calloop::channel::channel();
    let (slow_server, mut slow_client) = UnixStream::pair().unwrap();
    let png = png_bytes();
    let mut encoded = Vec::new();
    crate::ipc::write_frame(&mut encoded, br#"{"cmd":"add","source":"area"}"#, &png).unwrap();
    let split = encoded.len() / 2;
    slow_client.write_all(&encoded[..split]).unwrap();
    spawn_client_reader(slow_server, tx.clone(), &ClientLimits::default());

    let (fast_server, mut fast_client) = UnixStream::pair().unwrap();
    spawn_client_reader(fast_server, tx, &ClientLimits::default());
    fast_client
        .write_all(&crate::ipc::Request::Ping.encode())
        .unwrap();

    assert!(matches!(
        receive_event(&rx),
        DaemonEvent::ClientRequest {
            request: crate::ipc::Request::Ping,
            ..
        }
    ));

    slow_client.write_all(&encoded[split..]).unwrap();
    match receive_event(&rx) {
        DaemonEvent::AddPrepared {
            result: Ok(add), ..
        } => {
            assert_eq!(add.source, "area");
            assert_eq!(std::fs::read(&add.path).unwrap(), png);
            let _ = std::fs::remove_file(&add.path);
        }
        _ => panic!("expected prepared PNG request"),
    }
}

#[test]
fn unread_reply_does_not_block_another_reply() {
    let (mut blocked_server, _blocked_client) = UnixStream::pair().unwrap();
    blocked_server.set_nonblocking(true).unwrap();
    let fill = [0u8; 4096];
    loop {
        match blocked_server.write(&fill) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("fill reply socket: {error}"),
        }
    }
    blocked_server.set_nonblocking(false).unwrap();
    spawn_client_writer(blocked_server, vec![1]);

    let (fast_server, mut fast_client) = UnixStream::pair().unwrap();
    fast_client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    spawn_client_writer(fast_server, b"PONG".to_vec());
    let mut reply = [0u8; 4];
    fast_client.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"PONG");
}
