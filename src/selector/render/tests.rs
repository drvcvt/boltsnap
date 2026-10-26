//! Pure rendering + geometry tests for the selector.

use super::*;

/// FNV-1a over pixels, to pin a render without keeping a copy of it.
fn fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(hash, |h, b| (h ^ *b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The frozen Windows backend draws exactly what it drew before the
/// Linux selector moved to the app's design system: pill, audio button,
/// badge, border and handles in both Windows styles, loupe, all with the
/// embedded face Windows uses.
#[test]
fn windows_pixels_stay_pinned() {
    let (w, h) = (400u32, 300u32);
    let mut base = Pixmap::new(w, h).unwrap();
    for (i, pixel) in base.data_mut().chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i as u32 % w), (i as u32 / w));
        pixel.copy_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 90, 255]);
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for (index, sel) in [
        (120.0, 150.0, 160.0, 90.0),
        (0.0, 0.0, 400.0, 300.0),
        (300.5, 10.25, 90.0, 60.0),
        (8.0, 220.0, 160.0, 70.0),
    ]
    .into_iter()
    .enumerate()
    {
        for style in [WINDOWS_RECORD_OVERLAY_STYLE, SCREENSHOT_OVERLAY_STYLE] {
            let mut pm = base.clone();
            dim_and_restore_with_style(&mut pm, Some(sel), style);
            draw_border_with_style(&mut pm, sel, style);
            draw_handles_with_style(&mut pm, sel, style);
            draw_rec_pill(&mut pm, sel, w, h);
            draw_record_audio_button(&mut pm, sel, w, h, index % 2 == 0);
            draw_badge(&mut pm, sel, w, h);
            draw_magnifier(
                &mut pm,
                &base,
                (sel.0 as f64 + 5.0, sel.1 as f64 + 5.0),
                w,
                h,
            );
            hash = fnv(hash, pm.data());
        }
        for rect in [
            rec_pill_rect(sel, w, h),
            record_audio_button_rect(sel, w, h),
            badge_bounds(sel, w, h),
        ]
        .into_iter()
        .flatten()
        {
            for v in [rect.0, rect.1, rect.2, rect.3] {
                hash = fnv(hash, &v.to_bits().to_le_bytes());
            }
        }
    }
    assert_eq!(hash, 0x34ed_7442_fdc3_e91d, "windows render hash {hash:#x}");
}

#[test]
fn rect_to_image_forward_and_reversed_match() {
    // 1:1 surface->image. Drag (10,20)->(110,220) and the reverse must
    // produce the same crop rect.
    let fwd = rect_to_image((10.0, 20.0), (110.0, 220.0), 200, 400, 200, 400);
    let rev = rect_to_image((110.0, 220.0), (10.0, 20.0), 200, 400, 200, 400);
    assert_eq!(fwd, Some((10, 20, 100, 200)));
    assert_eq!(fwd, rev);
}

#[test]
fn rect_to_image_clamps_out_of_bounds() {
    let r = rect_to_image((-50.0, -50.0), (10_000.0, 10_000.0), 200, 400, 200, 400);
    assert_eq!(r, Some((0, 0, 200, 400)));
}

#[test]
fn rect_to_image_rejects_subpixel() {
    assert_eq!(
        rect_to_image((10.0, 10.0), (10.5, 10.5), 200, 400, 200, 400),
        None
    );
    assert_eq!(
        rect_to_image((10.0, 10.0), (10.0, 10.0), 200, 400, 200, 400),
        None
    );
}

#[test]
fn pixmap_to_argb8888_swaps_r_and_b_keeps_premult() {
    use tiny_skia::Pixmap;
    let mut pm = Pixmap::new(1, 1).unwrap();
    // premultiplied RGBA sample with alpha < 255 and channels <= alpha.
    pm.data_mut().copy_from_slice(&[10, 20, 30, 200]); // R,G,B,A
    let mut canvas = [0u8; 4];
    pixmap_to_argb8888(&pm, &mut canvas);
    assert_eq!(canvas, [30, 20, 10, 200]); // B,G,R,A
    // premultiplied invariant survives: no colour channel exceeds alpha.
    assert!(canvas[0] <= canvas[3] && canvas[1] <= canvas[3] && canvas[2] <= canvas[3]);
}

#[test]
fn base_pixmap_copies_opaque_image_1to1() {
    use image::{Rgba, RgbaImage};
    let mut img = RgbaImage::new(2, 2);
    for p in img.pixels_mut() {
        *p = Rgba([10, 20, 30, 255]);
    }
    let pm = base_pixmap_from_image(&img, 2, 2);
    assert_eq!(pm.width(), 2);
    assert_eq!(&pm.data()[0..4], &[10, 20, 30, 255]);
}

#[test]
fn render_overlay_dims_outside_keeps_inside_bright() {
    use tiny_skia::Pixmap;
    // Opaque red base.
    let mut pm = Pixmap::new(200, 100).unwrap();
    for px in pm.data_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 0, 0, 255]);
    }
    dim_and_restore(&mut pm, Some((40.0, 30.0, 60.0, 20.0)));
    let at = |x: u32, y: u32| {
        let i = ((y * pm.width() + x) * 4) as usize;
        pm.data()[i] // red channel
    };
    // Inside the selection: still bright red.
    assert!(
        at(60, 38) > 250,
        "inside should stay bright, got {}",
        at(60, 38)
    );
    // Outside: dimmed by the ~43% black overlay.
    assert!(at(0, 0) < 200, "outside should be dimmed, got {}", at(0, 0));
}

#[test]
fn cached_overlay_matches_original_across_moves_and_transparency() {
    for alpha in [0, 128, 255] {
        let mut base = Pixmap::new(80, 60).unwrap();
        base.fill(tiny_skia::Color::from_rgba8(170, 90, 35, alpha));
        let mut cache = CachedOverlay::new(&base);
        for selection in [
            None,
            Some((12.5, 9.5, 35.0, 22.0)),
            Some((-5.5, -3.0, 20.0, 90.0)),
            Some((70.0, 50.0, 40.0, 20.0)),
            Some((2.0, 2.0, 0.5, 10.0)),
            None,
        ] {
            let mut expected = base.clone();
            dim_and_restore(&mut expected, selection);
            let actual = cache.reset(&base, selection);
            assert_eq!(
                actual.pixmap.data(),
                expected.data(),
                "alpha={alpha}, selection={selection:?}"
            );
            // Simulate controls drawn after the background pass. The next
            // reset must remove their pixels even when the selection moves.
            actual.pixmap.data_mut()[0..4].fill(255);
        }
    }
}

#[test]
fn windows_record_overlay_dims_selection_less_than_overview() {
    let mut default = Pixmap::new(100, 100).unwrap();
    let mut windows = Pixmap::new(100, 100).unwrap();
    for pixmap in [&mut default, &mut windows] {
        for pixel in pixmap.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    let selection = Some((20.0, 20.0, 60.0, 60.0));
    dim_and_restore(&mut default, selection);
    dim_and_restore_with_style(&mut windows, selection, WINDOWS_RECORD_OVERLAY_STYLE);

    let pixel = |pixmap: &Pixmap, x: u32, y: u32| {
        let offset = ((y * pixmap.width() + x) * 4) as usize;
        <[u8; 4]>::try_from(&pixmap.data()[offset..offset + 4]).unwrap()
    };
    assert_eq!(pixel(&default, 0, 0), [145, 145, 145, 255]);
    assert_eq!(pixel(&windows, 0, 0), [145, 145, 145, 255]);
    assert_eq!(pixel(&windows, 50, 50), [231, 231, 231, 255]);
}

#[test]
fn windows_record_border_is_single_and_handles_use_neutral_accent() {
    let mut pm = Pixmap::new(120, 120).unwrap();
    for pixel in pm.data_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&[64, 64, 64, 255]);
    }
    let selection = (30.0, 30.0, 60.0, 60.0);
    draw_border_with_style(&mut pm, selection, WINDOWS_RECORD_OVERLAY_STYLE);

    let pixel = |pixmap: &Pixmap, x: u32, y: u32| {
        let offset = ((y * pixmap.width() + x) * 4) as usize;
        <[u8; 4]>::try_from(&pixmap.data()[offset..offset + 4]).unwrap()
    };
    assert_eq!(pixel(&pm, 28, 60), [64, 64, 64, 255]);
    assert!(pixel(&pm, 30, 60)[0] > 200);

    draw_handles_with_style(&mut pm, selection, WINDOWS_RECORD_OVERLAY_STYLE);
    assert_eq!(pixel(&pm, 30, 30), [255, 255, 255, 255]);
}

#[test]
fn render_overlay_none_dims_everything() {
    use tiny_skia::Pixmap;
    let mut pm = Pixmap::new(20, 20).unwrap();
    for px in pm.data_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 0, 0, 255]);
    }
    dim_and_restore(&mut pm, None);
    let i = ((10 * pm.width() + 10) * 4) as usize;
    assert!(pm.data()[i] < 200, "whole surface should be dimmed");
}

#[test]
fn render_overlay_dim_has_no_undimmed_row_seam() {
    use tiny_skia::Pixmap;
    let mut pm = Pixmap::new(200, 100).unwrap();
    for px in pm.data_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&[255, 0, 0, 255]);
    }
    // Fractional *.5 edges are the worst case for non-AA rect tiling: the
    // boundary pixel row gets ~0.5 coverage from each neighbouring dim rect
    // and can end up undimmed. x=0 is far left of the selection (sx=40.5),
    // so EVERY row there must be fully dimmed (red ~145). A seam shows up as
    // a bright row.
    dim_and_restore(&mut pm, Some((40.5, 30.5, 60.0, 20.0)));
    for y in 0..100u32 {
        let r = pm.data()[((y * 200) * 4) as usize];
        assert!(r < 160, "left column row {y} not fully dimmed: r={r}");
    }
}

#[test]
fn draw_rec_pill_paints_red_dot_and_pill() {
    use tiny_skia::Pixmap;
    let mut pm = Pixmap::new(400, 300).unwrap();
    for px in pm.data_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&[0, 0, 0, 255]); // opaque black backdrop
    }
    // Selection well clear of the top so the pill sits above it on-screen.
    let sel = (120.0, 150.0, 160.0, 90.0);
    draw_rec_pill(&mut pm, sel, 400, 300);
    // Somewhere in the pill band above the selection there must be a strongly
    // red pixel (the record dot). Scan the rows just above the selection top.
    let at = |x: u32, y: u32| {
        let i = ((y * pm.width() + x) * 4) as usize;
        (pm.data()[i], pm.data()[i + 1], pm.data()[i + 2])
    };
    let mut found_red = false;
    for y in 110..150u32 {
        for x in 120..280u32 {
            let (r, g, b) = at(x, y);
            if r > 180 && g < 100 && b < 100 {
                found_red = true;
            }
        }
    }
    assert!(found_red, "REC pill should paint a red record dot");
}

#[test]
fn rec_pill_rect_is_above_selection_and_hittable() {
    // Selection well clear of the top edge → pill sits above it.
    let sel = (120.0, 150.0, 160.0, 90.0);
    let (bx, by, bw, bh) = rec_pill_rect(sel, 400, 300).expect("pill rect for a real selection");
    assert!(bw > 0.0 && bh > 0.0, "pill has positive size");
    // Placed above the selection top (badge_rect places above-left here).
    assert!(by + bh <= sel.1 as f64, "pill sits above the selection");
    // Its own centre is inside the rect (so a click there hits the button).
    let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
    assert!(cx >= bx && cx < bx + bw && cy >= by && cy < by + bh);
    // Sub-pixel selection → no pill.
    assert!(rec_pill_rect((10.0, 10.0, 0.0, 0.0), 400, 300).is_none());
}

#[test]
fn frame_checkbox_sits_beside_rec_without_overlap() {
    let sel = (100.0, 100.0, 800.0, 500.0);
    let rec = rec_pill_rect(sel, 1920, 1080).unwrap();
    let check = record_frame_checkbox_rect(sel, 1920, 1080).unwrap();
    assert!(check.0 >= rec.0 + rec.2);
    assert!(
        check.2 > check.3 * 3.0,
        "control reserves room for its label"
    );
    assert!(check.0 + check.2 <= 1920.0);
    assert!(check.1 >= 0.0 && check.1 + check.3 <= 1080.0);
}

#[test]
fn frame_checkbox_stays_on_screen_at_selection_edges() {
    for sel in [
        (0.0, 0.0, 120.0, 80.0),
        (280.0, 0.0, 120.0, 80.0),
        (0.0, 220.0, 120.0, 80.0),
        (280.0, 220.0, 120.0, 80.0),
    ] {
        let rec = rec_pill_rect(sel, 400, 300).unwrap();
        let check = record_frame_checkbox_rect(sel, 400, 300).unwrap();
        assert!(check.0 >= 0.0 && check.0 + check.2 <= 400.0);
        assert!(check.1 >= 0.0 && check.1 + check.3 <= 300.0);
        assert!(
            check.0 + check.2 <= rec.0
                || rec.0 + rec.2 <= check.0
                || check.1 + check.3 <= rec.1
                || rec.1 + rec.3 <= check.1
        );
    }
}

#[test]
fn checked_frame_control_draws_a_checkmark_instead_of_a_solid_fill() {
    use tiny_skia::Pixmap;
    let sel = (100.0, 100.0, 200.0, 100.0);
    let rect = record_frame_checkbox_rect(sel, 400, 300).unwrap();
    let mut checked = Pixmap::new(400, 300).unwrap();
    draw_record_frame_checkbox(&mut checked, sel, 400, 300, true);
    let mut red = 0;
    for y in rect.1 as u32..(rect.1 + rect.3) as u32 {
        for x in rect.0 as u32..(rect.0 + rect.3) as u32 {
            let i = ((y * checked.width() + x) * 4) as usize;
            let pixel = &checked.data()[i..i + 4];
            red += usize::from(pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100);
        }
    }
    assert!(red > 0, "checked control paints a red checkmark");
    assert!(red < 100, "checked control must not use a solid red fill");
}

#[test]
fn frame_control_draws_its_explanatory_label() {
    use tiny_skia::Pixmap;
    let sel = (100.0, 100.0, 200.0, 100.0);
    let rect = record_frame_checkbox_rect(sel, 400, 300).unwrap();
    let mut pm = Pixmap::new(400, 300).unwrap();
    draw_record_frame_checkbox(&mut pm, sel, 400, 300, false);
    let mut found_text = false;
    for y in rect.1 as u32..(rect.1 + rect.3) as u32 {
        for x in (rect.0 + rect.3) as u32..(rect.0 + rect.2) as u32 {
            let i = ((y * pm.width() + x) * 4) as usize;
            let pixel = &pm.data()[i..i + 4];
            found_text |= pixel[0] > 100 && pixel[1] > 100 && pixel[2] > 100;
        }
    }
    assert!(found_text, "control paints a readable label");
}

#[test]
fn record_audio_button_stays_visible_and_clear_of_other_controls() {
    let overlaps = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
    };
    for sel in [
        (8.0, 8.0, 160.0, 100.0),
        (232.0, 8.0, 160.0, 100.0),
        (8.0, 192.0, 160.0, 100.0),
        (232.0, 192.0, 160.0, 100.0),
    ] {
        let rec = rec_pill_rect(sel, 400, 300).unwrap();
        let frame = record_frame_checkbox_rect(sel, 400, 300).unwrap();
        let audio = record_audio_button_rect(sel, 400, 300).unwrap();
        assert!(!overlaps(audio, rec));
        assert!(!overlaps(audio, frame));
        assert!(audio.0 >= 0.0 && audio.1 >= 0.0);
        assert!(audio.0 + audio.2 <= 400.0 && audio.1 + audio.3 <= 300.0);
    }
}

#[test]
fn record_audio_button_has_distinct_on_and_off_rendering() {
    let (w, h) = (400, 300);
    let sel = (80.0, 80.0, 200.0, 120.0);
    let mut on = Pixmap::new(w, h).unwrap();
    let mut off = Pixmap::new(w, h).unwrap();
    draw_record_audio_button(&mut on, sel, w, h, true);
    draw_record_audio_button(&mut off, sel, w, h, false);
    assert_ne!(on.data(), off.data());
    assert!(on.data().iter().any(|byte| *byte != 0));
    assert!(off.data().iter().any(|byte| *byte != 0));
}
