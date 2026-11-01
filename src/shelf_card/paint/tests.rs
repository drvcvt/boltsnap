use super::*;

/// The card as it was blitted before the masks, pixel by pixel.
#[allow(clippy::too_many_arguments)]
fn reference_blit(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    img: &RgbaImage,
    dx: u32,
    dy: u32,
    scale: f32,
    opacity: f32,
    radius: f32,
) {
    let (iw, ih) = img.dimensions();
    let scale = scale.clamp(0.0, 1.0);
    let sw = ((iw as f32 * scale).round() as u32).max(1);
    let sh = ((ih as f32 * scale).round() as u32).max(1);
    let scaled;
    let src: &RgbaImage = if sw == iw && sh == ih {
        img
    } else {
        scaled = imageops::resize(img, sw, sh, imageops::FilterType::Triangle);
        &scaled
    };
    let (ox, oy) = (dx + (iw - sw) / 2, dy + (ih - sh) / 2);
    let (w, h) = (sw as f32, sh as f32);
    let r = radius.min(w / 2.0).min(h / 2.0);
    let global = opacity.clamp(0.0, 1.0);
    for sy in 0..sh {
        let py = oy + sy;
        if py >= ch {
            break;
        }
        for sx in 0..sw {
            let px = ox + sx;
            if px >= cw {
                break;
            }
            let cov = rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w, h, r);
            if cov <= 0.0 {
                continue;
            }
            let a = cov * global;
            let p = src.get_pixel(sx, sy).0;
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = (p[2] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 1] = (p[1] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 2] = (p[0] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[test]
fn shadow_and_card_paint_what_they_painted_before() {
    // (canvas scale, card at, card size, card scale, opacity)
    let cases = [
        (1.0, (22, 22), (190, 132), 1.0, 0.8),
        (1.0, (22, 22), (190, 132), 1.0, 1.0),
        (1.0, (22, 22), (190, 132), 0.94, 0.5),
        (1.25, (28, 28), (237, 165), 1.0, 0.8),
        // Cut off at the canvas's right and bottom edges, then outside it.
        (1.0, (180, 90), (190, 132), 1.0, 0.8),
        (1.0, (400, 10), (190, 132), 1.0, 0.8),
    ];
    let (cw, ch) = (300, 220);
    for (canvas_scale, (x, y), (w, h), scale, opacity) in cases {
        let thumb = RgbaImage::from_fn(w, h, |px, py| {
            image::Rgba([(px * 7) as u8, (py * 5) as u8, (px ^ py) as u8, 255])
        });
        let r = ThumbRect { id: 1, x, y, w, h };
        let radius = CARD_RADIUS * canvas_scale;
        let background: Vec<u8> = (0..cw * ch * 4).map(|i| (i * 37 % 251) as u8).collect();
        let settled = scale == 1.0;
        let mut before = background.clone();
        if settled {
            super::super::shadow::tests::reference_shadow(&mut before, cw, ch, &r, canvas_scale);
        }
        reference_blit(&mut before, cw, ch, &thumb, x, y, scale, opacity, radius);
        let mut now = background.clone();
        if settled {
            draw_card_shadow(&mut now, cw, ch, &r, canvas_scale, true);
        }
        blit_thumb_card_anim(&mut now, cw, ch, &thumb, x, y, scale, opacity, radius);
        assert!(
            before == now,
            "{canvas_scale} {x},{y} {w}x{h} {scale} {opacity}"
        );
    }
}

fn plain(w: u32, h: u32, rgba: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, image::Rgba(rgba))
}

/// One card of `thumb` alone on a canvas, `hovered` or not.
fn shelf_of(thumb: &RgbaImage, kind: CardKind, hovered: bool, look: CardLook) -> (Vec<u8>, Layout) {
    let cfg = LayoutConfig::default();
    let layout = Layout::compute(&[(1, thumb.width(), thumb.height())], &cfg);
    let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
    let frame = Frame {
        layout: &layout,
        cfg: &cfg,
        hovered: if hovered { &[(1, 1.0)] } else { &[] },
        anims: &[],
        save_flash: None,
        save_button: true,
        saving: &[],
        spin: 0.0,
        look,
        scale: 1.0,
    };
    draw_shelf(&mut canvas, layout.width, layout.height, &frame, |id| {
        (id == 1).then_some((thumb, kind))
    });
    (canvas, layout)
}

fn alpha_at(canvas: &[u8], layout: &Layout, x: u32, y: u32) -> u8 {
    canvas[((y * layout.width + x) * 4 + 3) as usize]
}

#[test]
fn card_rounds_corners_no_border() {
    let img = plain(20, 20, [10, 120, 240, 255]); // low R so we can tell it from white
    let mut buf = vec![0u8; 20 * 20 * 4];
    blit_thumb_card_anim(&mut buf, 20, 20, &img, 0, 0, 1.0, 0.8, CARD_RADIUS);
    // far corner is outside the radius -> transparent
    assert_eq!(buf[3], 0, "corner should be transparent");
    // centre carries the card alpha (~0.8*255), the thumbnail colour (low R)
    let c = ((10 * 20 + 10) * 4) as usize;
    assert!(
        buf[c + 3] > 180 && buf[c + 3] < 220,
        "centre alpha ~0.8, got {}",
        buf[c + 3]
    );
    assert!(buf[c + 2] < 60, "centre R should be the thumbnail's");
    // left-edge midpoint is the IMAGE colour now, NOT a white border
    let e = ((10 * 20) * 4) as usize;
    assert!(
        buf[e + 3] > 150,
        "left edge should carry card alpha, got {}",
        buf[e + 3]
    );
    assert!(
        buf[e + 2] < 60,
        "left edge R should be the image's, not white"
    );
}

#[test]
fn anim_scale_and_opacity_shrink_and_fade() {
    let img = plain(40, 40, [10, 120, 240, 255]);
    // Half scale, half opacity: card occupies the centre, corners of the slot
    // are untouched (transparent), centre alpha ~ 0.5 * 0.8.
    let mut buf = vec![0u8; 40 * 40 * 4];
    blit_thumb_card_anim(&mut buf, 40, 40, &img, 0, 0, 0.5, 0.8 * 0.5, CARD_RADIUS);
    // slot corner (well outside the centred 20x20 card) stays transparent
    let corner = ((2 * 40 + 2) * 4) as usize;
    assert_eq!(buf[corner + 3], 0, "slot corner should be untouched");
    let c = ((20 * 40 + 20) * 4) as usize;
    assert!(
        buf[c + 3] > 70 && buf[c + 3] < 120,
        "centre alpha ~0.4, got {}",
        buf[c + 3]
    );
}

#[test]
fn clear_zeros_buffer() {
    let mut buf = vec![9u8; 16];
    clear(&mut buf);
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn fill_circle_blends_center_opaqueish() {
    let mut buf = vec![0u8; 24 * 24 * 4];
    fill_circle(&mut buf, 24, 24, 12.0, 12.0, 8.0, (10, 20, 30), 0.5);
    let idx = ((12 * 24 + 12) * 4) as usize;
    // center alpha ~ 0.5*255
    assert!(
        buf[idx + 3] > 100 && buf[idx + 3] < 160,
        "got a={}",
        buf[idx + 3]
    );
    // corner stays transparent
    assert_eq!(buf[3], 0);
}

#[test]
fn video_card_draws_play_badge() {
    let thumb = plain(260, 180, [10, 20, 200, 255]);
    let (canvas, layout) = shelf_of(&thumb, CardKind::Video, false, CardLook::default());
    let r = &layout.thumbs[0];
    // plain thumb color sampled away from center
    let plain = canvas[(((r.y + 8) * layout.width + r.x + 8) * 4 + 2) as usize];
    // badge at the card center
    let cidx = (((r.y + r.h / 2) * layout.width + r.x + r.w / 2) * 4) as usize;
    assert_ne!(
        canvas[cidx + 2],
        plain,
        "video card center should carry the play badge"
    );
}

#[test]
fn hovered_thumb_draws_icon_pixels() {
    let thumb = plain(260, 180, [10, 20, 200, 255]);
    let (canvas, layout) = shelf_of(&thumb, CardKind::Image, true, CardLook::default());
    let cfg = LayoutConfig::default();
    // thumb body opaque
    let r = &layout.thumbs[0];
    let (px, py) = (r.x + r.w / 2, r.y + r.h / 2);
    let idx = ((py * layout.width + px) * 4) as usize;
    assert_eq!(canvas[idx + 3], 255, "the hovered card is solid");
    // a glyph pixel near the close-button centre should differ from the
    // plain thumbnail colour (button bg darkened or glyph stroke).
    let bx = r.x + r.w - cfg.pad_icon - cfg.icon / 2;
    let by = r.y + cfg.pad_icon + cfg.icon / 2;
    let bidx = ((by * layout.width + bx) * 4) as usize;
    let plain = canvas[idx + 2]; // R of plain thumb region
    assert_ne!(
        canvas[bidx + 2],
        plain,
        "button area should not be plain thumb colour"
    );
    // a glyph pixel near the save-button (top-left) centre should differ from
    // the plain thumbnail colour.
    let sx = r.x + cfg.pad_icon + cfg.icon / 2;
    let sy = r.y + cfg.pad_icon + cfg.icon / 2;
    let sidx = ((sy * layout.width + sx) * 4) as usize;
    assert_ne!(
        canvas[sidx + 2],
        plain,
        "save button area should not be plain thumb colour"
    );
}

#[test]
fn the_look_sets_opacity_hover_and_shadow() {
    let thumb = plain(190, 132, [200, 200, 200, 255]);
    let look = CardLook {
        opacity: 0.5,
        solid_on_hover: false,
        shadow: false,
    };
    let centre = |(canvas, layout): &(Vec<u8>, Layout)| {
        let r = &layout.thumbs[0];
        alpha_at(canvas, layout, r.x + r.w / 2, r.y + r.h / 2)
    };
    // Beside the card, inside its shadow's reach.
    let beside = |(canvas, layout): &(Vec<u8>, Layout)| {
        let r = &layout.thumbs[0];
        alpha_at(canvas, layout, r.x + r.w / 2, r.y + r.h + 4)
    };
    let rest = shelf_of(&thumb, CardKind::Image, false, look);
    assert_eq!(centre(&rest), 128);
    assert_eq!(centre(&shelf_of(&thumb, CardKind::Image, true, look)), 128);
    assert_eq!(beside(&rest), 0, "no shadow");
    let default = CardLook::default();
    assert_eq!(
        centre(&shelf_of(&thumb, CardKind::Image, false, default)),
        204
    );
    assert_eq!(
        centre(&shelf_of(&thumb, CardKind::Image, true, default)),
        255
    );
    assert!(beside(&shelf_of(&thumb, CardKind::Image, false, default)) > 0);
}

#[test]
fn the_preview_leaves_the_save_button_out() {
    let thumb = plain(190, 132, [10, 20, 200, 255]);
    let cfg = LayoutConfig::default();
    let layout = Layout::compute(&[(1, 190, 132)], &cfg);
    let draw = |save_button| {
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        let frame = Frame {
            layout: &layout,
            cfg: &cfg,
            hovered: &[(1, 1.0)],
            anims: &[],
            save_flash: None,
            save_button,
            saving: &[],
            spin: 0.0,
            look: CardLook::default(),
            scale: 1.0,
        };
        draw_shelf(&mut canvas, layout.width, layout.height, &frame, |_| {
            Some((&thumb, CardKind::Image))
        });
        canvas
    };
    let (with, without) = (draw(true), draw(false));
    let (sx, sy, _, _) = save_cell(&layout.thumbs[0], &cfg);
    let (cx, cy, _, _) = close_cell(&layout.thumbs[0], &cfg);
    let at = |x: u32, y: u32| ((y * layout.width + x) * 4) as usize;
    let centre = |x: u32, y: u32| at(x + cfg.icon / 2, y + cfg.icon / 2);
    assert_ne!(with[centre(sx, sy)..][..4], without[centre(sx, sy)..][..4]);
    assert_eq!(with[centre(cx, cy)..][..4], without[centre(cx, cy)..][..4]);
}

#[test]
fn half_a_hover_fade_is_half_way_to_solid_with_half_its_buttons() {
    let thumb = plain(190, 132, [200, 200, 200, 255]);
    let cfg = LayoutConfig::default();
    let layout = Layout::compute(&[(1, 190, 132)], &cfg);
    let draw = |lift: f32| {
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        let hovered = [(1, lift)];
        let frame = Frame {
            layout: &layout,
            cfg: &cfg,
            hovered: if lift > 0.0 { &hovered } else { &[] },
            anims: &[],
            save_flash: None,
            save_button: true,
            saving: &[],
            spin: 0.0,
            look: CardLook::default(),
            scale: 1.0,
        };
        draw_shelf(&mut canvas, layout.width, layout.height, &frame, |_| {
            Some((&thumb, CardKind::Image))
        });
        canvas
    };
    let r = &layout.thumbs[0];
    let centre = (((r.y + r.h / 2) * layout.width + r.x + r.w / 2) * 4 + 3) as usize;
    let (rest, half, full) = (draw(0.0), draw(0.5), draw(1.0));
    assert_eq!((rest[centre], full[centre]), (204, 255));
    assert!((half[centre] as i32 - 230).abs() <= 1, "{}", half[centre]);
    // The close button's middle: half its darkening at half the fade.
    let (bx, by, _, _) = close_cell(r, &cfg);
    let button = (((by + cfg.icon / 2) * layout.width + bx + 2) * 4) as usize;
    let shade = |canvas: &[u8]| i32::from(full[centre - 3]) - i32::from(canvas[button]);
    assert!(shade(&half) > 0 && shade(&half) < shade(&full));
}

/// What a shelf frame of six settled cards costs, with one appearing and
/// one hovered: `cargo test --release --lib frame_cost -- --ignored
/// --nocapture`. Frames must fit 4.2 ms (240 Hz).
#[test]
#[ignore]
fn frame_cost() {
    let cfg = LayoutConfig::default();
    let thumbs: Vec<RgbaImage> = (0..6u32)
        .map(|i| {
            RgbaImage::from_fn(190, 132, |x, y| {
                image::Rgba([(x * 7 + i) as u8, (y * 3) as u8, (x ^ y) as u8, 255])
            })
        })
        .collect();
    let sizes: Vec<_> = (0..6u64).map(|id| (id, 190, 132)).collect();
    let layout = Layout::compute(&sizes, &cfg);
    let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
    for (name, anims, hovered) in [
        ("rest", vec![], vec![]),
        ("appear+hover", vec![(0, 0.94, 0.5)], vec![(2, 0.5)]),
    ] {
        let frame = Frame {
            layout: &layout,
            cfg: &cfg,
            hovered: &hovered,
            anims: &anims,
            save_flash: None,
            save_button: true,
            saving: &[],
            spin: 0.0,
            look: CardLook::default(),
            scale: 1.0,
        };
        let mut times: Vec<f64> = (0..300)
            .map(|_| {
                let start = std::time::Instant::now();
                draw_shelf(&mut canvas, layout.width, layout.height, &frame, |id| {
                    thumbs.get(id as usize).map(|t| (t, CardKind::Image))
                });
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        println!(
            "{name}: median {:.3} ms, p95 {:.3} ms",
            times[150], times[285]
        );
    }
}

/// `SHADOW_OUT=file.png cargo test --lib shadow_shot -- --ignored`: one
/// card and its shadow, straight alpha, to look at over a dark desktop.
#[test]
#[ignore]
fn shadow_shot() {
    let out = std::env::var("SHADOW_OUT").expect("SHADOW_OUT");
    let thumb = plain(190, 132, [60, 60, 60, 255]);
    let (canvas, layout) = shelf_of(&thumb, CardKind::Image, false, CardLook::default());
    let mut image = RgbaImage::new(layout.width, layout.height);
    for (pixel, bgra) in image.pixels_mut().zip(canvas.as_chunks::<4>().0.iter()) {
        // Premultiplied BGRA to straight RGBA.
        let a = bgra[3];
        let straight = |c: u8| {
            if a == 0 {
                0
            } else {
                (u32::from(c) * 255 / u32::from(a)) as u8
            }
        };
        *pixel = image::Rgba([straight(bgra[2]), straight(bgra[1]), straight(bgra[0]), a]);
    }
    image.save(out).unwrap();
}
