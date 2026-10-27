//! What a card does: arriving, leaving, copy, save, click, and the shelf drawn
//! with them.

use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::QueueHandle;

use crate::record::finalize::promote_recording;
use crate::shelf::layout::Hit;
use crate::shelf::model::{CardKind, FileLifetime};

use super::anim::AnimKind;
use super::events::DaemonEvent;
use super::{Daemon, notify, save};

pub(crate) struct PreparedAdd {
    pub(super) path: std::path::PathBuf,
    pub(super) thumb: image::RgbaImage,
    pub(super) trace: Option<u64>,
    pub(super) source: String,
    pub(super) output: Option<String>,
}

impl Drop for PreparedAdd {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// How long the ✓ "saved" flash stays on the Save button.
const SAVE_FLASH_MS: u128 = 700;

pub(super) fn shelf_commit_allowed(drag_active: bool) -> bool {
    !drag_active
}

pub(super) const MAX_CACHED_CARDS: usize = 20;

pub(super) const MAX_CACHED_IMAGE_BYTES: u64 = 256 * 1024 * 1024;

/// Oldest cards to drop until the cache fits. `saving` cards still count
/// but are never dropped, so the shelf can briefly exceed its limits.
pub(super) fn shelf_cache_evictions_with_limits(
    newest_first: &[(u64, CardKind, FileLifetime, u64)],
    max_cards: usize,
    max_image_bytes: u64,
    saving: impl Fn(u64) -> bool,
) -> Vec<u64> {
    let mut cards = newest_first.len();
    let mut image_bytes = newest_first
        .iter()
        .filter(|(_, kind, lifetime, _)| {
            *kind == CardKind::Image && *lifetime == FileLifetime::Temporary
        })
        .map(|(_, _, _, bytes)| bytes)
        .sum::<u64>();
    let mut evictions = Vec::new();
    for (id, kind, lifetime, bytes) in newest_first.iter().rev() {
        if cards <= max_cards && image_bytes <= max_image_bytes {
            break;
        }
        if saving(*id) {
            continue;
        }
        evictions.push(*id);
        cards -= 1;
        if *kind == CardKind::Image && *lifetime == FileLifetime::Temporary {
            image_bytes = image_bytes.saturating_sub(*bytes);
        }
    }
    evictions
}

impl Daemon {
    pub(super) fn add_prepared(&mut self, mut add: PreparedAdd, qh: &QueueHandle<Self>) {
        // A compositor can omit the terminal drag event and leave redraws
        // blocked. A fresh capture supersedes that stale interaction.
        if self.drag_source.is_some() {
            self.clear_drag();
        }
        let id = self.model.add(
            std::mem::take(&mut add.path),
            std::mem::take(&mut add.thumb),
            std::mem::take(&mut add.source),
        );
        self.fit_card(id);
        if let Some(trace) = add.trace {
            if self.pending_image_traces.len() == MAX_CACHED_CARDS {
                self.pending_image_traces.remove(0);
            }
            self.pending_image_traces.push(trace);
        }
        self.start_anim(id, AnimKind::Appear);
        self.trim_shelf_cache();
        self.relayout();
        self.place_on_output(add.output.as_deref(), qh);
        self.draw(qh);
    }

    pub(super) fn trim_shelf_cache(&mut self) {
        let cards = self
            .model
            .newest_first()
            .map(|card| {
                let bytes =
                    if card.kind == CardKind::Image && card.lifetime == FileLifetime::Temporary {
                        std::fs::metadata(&card.png_path)
                            .map(|metadata| metadata.len())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                (card.id, card.kind, card.lifetime, bytes)
            })
            .collect::<Vec<_>>();
        for id in shelf_cache_evictions_with_limits(
            &cards,
            MAX_CACHED_CARDS,
            MAX_CACHED_IMAGE_BYTES,
            |id| self.saves.is_saving(id) || self.is_pending(id),
        ) {
            self.remove_card_now(id);
        }
    }

    /// Drop a card and its temporary file. A card being saved stays; it closes
    /// once the save succeeds.
    pub(super) fn remove_card_now(&mut self, id: u64) -> bool {
        if !self.saves.request_close(id) {
            return false;
        }
        let Some(card) = self.model.remove(id) else {
            return false;
        };
        self.pending.close(id);
        self.drop_card_file(&card);
        self.anims.retain(|animation| animation.id != id);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if self.press.as_ref().is_some_and(|press| press.id == id) {
            self.press = None;
        }
        if self.save_flash.is_some_and(|(saved, _)| saved == id) {
            self.save_flash = None;
        }
        true
    }

    /// Copy the card under the cursor to the clipboard: an image as `image/png`,
    /// a video as a `text/uri-list` file reference (path only, never bytes).
    pub(super) fn copy_card(&mut self, id: u64) {
        if self.is_pending(id) {
            return;
        }
        if let Some(t) = self.model.get(id) {
            let path = t.png_path.clone();
            if t.kind == crate::shelf::model::CardKind::Video {
                // Video: copy a file REFERENCE (path), not bytes — instant for any
                // clip length and pasteable as a file. No auto-clipboard elsewhere.
                if let Err(e) = crate::clipboard::copy_uri_to_clipboard(&path) {
                    eprintln!("boltsnap daemon: video copy failed: {e}");
                }
                return;
            }
            if let Err(e) = crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland) {
                eprintln!("boltsnap daemon: copy failed: {e}");
            }
        }
    }

    /// Save the full-res file of card `id` and flash a ✓ on its Save button.
    /// Image cards are copied to `<save_dir>/boltsnap-<ts>[-N].png`; video cards go to
    /// `<record_dir>/boltsnap-<ts>.<ext>` (ext from the source file, usually mp4).
    fn save_card(&mut self, id: u64) {
        let (src, kind) = match self.model.get(id) {
            Some(t) if t.lifetime == FileLifetime::Permanent => {
                eprintln!("boltsnap daemon: already saved {}", t.png_path.display());
                notify("Already saved");
                self.flash_saved(id);
                return;
            }
            Some(t) => (t.png_path.clone(), t.kind),
            None => return,
        };
        // Off the event loop: a slow target must not stall the shelf. The
        // card stays protected until the result arrives.
        if !self.saves.start(id) {
            return;
        }
        let tx = self.event_tx.clone();
        if kind == CardKind::Video {
            let dir = crate::config::resolve_record_dir(&crate::config::Config::load());
            std::thread::spawn(move || {
                let result = promote_recording(&src, &dir, None);
                let _ = tx.send(DaemonEvent::CardSaved {
                    id,
                    promote: true,
                    result,
                });
            });
        } else {
            let dir = crate::config::resolve_save_dir(
                self.save_dir_cli.as_deref(),
                &crate::config::Config::load(),
            );
            std::thread::spawn(move || {
                let result = save::save_image(&src, &dir).map_err(|error| error.to_string());
                let _ = tx.send(DaemonEvent::CardSaved {
                    id,
                    promote: false,
                    result,
                });
            });
        }
    }

    /// Act on a completed (non-drag) left click.
    pub(super) fn on_click(&mut self, hit: Hit, redraw: &mut bool) {
        match hit {
            // Nothing to open or save before the clip is written.
            Hit::Body(id) | Hit::Save(id) if self.is_pending(id) => {}
            Hit::Body(id) => self.open_media_card(id),
            Hit::Save(id) => {
                self.save_card(id);
                *redraw = true;
            }
            Hit::Close(id) => {
                // Animate the card out (tick_animations then removes it and its
                // file); with `animations = false` start_anim removes it at once.
                if self.hovered == Some(id) {
                    self.hovered = None;
                }
                // A card being saved stays until the save ends.
                if self.saves.request_close(id) {
                    self.start_anim(id, AnimKind::Disappear);
                }
                *redraw = true;
            }
        }
    }

    /// Time until the save checkmark disappears.
    pub(super) fn save_flash_remaining(&self) -> Option<std::time::Duration> {
        self.save_flash.map(|(_, started)| {
            std::time::Duration::from_millis(SAVE_FLASH_MS as u64).saturating_sub(started.elapsed())
        })
    }

    /// Show the saved checkmark on a card; `expire_save_flash` removes it.
    pub(super) fn flash_saved(&mut self, id: u64) {
        self.save_flash = Some((id, std::time::Instant::now()));
        if let Some(qh) = self.qh.clone() {
            self.draw(&qh);
        }
    }

    pub(super) fn expire_save_flash(&mut self, qh: &QueueHandle<Self>) {
        if self
            .save_flash_remaining()
            .is_some_and(|left| left.is_zero())
        {
            self.save_flash = None;
            self.draw(qh);
        }
    }

    pub(super) fn hit_test(&self, x: f64, y: f64) -> Option<Hit> {
        self.animated_layout().hit(x, y, &self.cfg)
    }

    pub(super) fn draw(&mut self, qh: &QueueHandle<Self>) {
        if self.release_empty_surface() || !self.shelf_configured {
            self.shelf_pending_draw = true;
            return;
        }
        // Committing the origin surface during the implicit grab makes some
        // wlroots compositors cancel the drag as it crosses to another output.
        // clear_drag() redraws after the terminal event; a fresh pointer press
        // clears a compositor-stuck drag before processing the new interaction.
        if !shelf_commit_allowed(self.drag_source.is_some()) {
            return;
        }
        // Render against an animated layout that collapses any disappearing card's
        // slot, and resize the (bottom-anchored) surface to match so the cards
        // above slide down smoothly during the dismiss instead of snapping at the
        // end. When nothing is animating this equals the settled layout.
        let render = self.animated_layout();
        let (w, h) = (render.width.max(1), render.height.max(1));
        let size_changed = w != self.width || h != self.height;
        self.width = w;
        self.height = h;
        // Per-card (scale, opacity) for in-flight appear/dismiss animations. Built
        // before create_buffer because `canvas` mutably borrows self.pool, which
        // would otherwise block the immutable self borrow anim_factor needs.
        let anims: Vec<(u64, f32, f32)> = render
            .thumbs
            .iter()
            .filter_map(|r| {
                let (s, o) = self.anim_factor(r.id);
                if (s - 1.0).abs() < f32::EPSILON && (o - 1.0).abs() < f32::EPSILON {
                    None
                } else {
                    Some((r.id, s, o))
                }
            })
            .collect();
        let hovered = self.hover_lifts();
        let saving = self.saving_frame();
        let layer = match self.layer.as_ref() {
            Some(l) => l,
            None => return,
        };
        if size_changed {
            layer.set_size(w, h);
        }
        let format = wayland_client::protocol::wl_shm::Format::Argb8888;
        let Some((buffer, canvas)) =
            self.buffers
                .next(&mut self.pool, (w as i32, h as i32), format)
        else {
            return;
        };

        crate::shelf::paint::draw_shelf_saving(
            canvas,
            w,
            h,
            &render,
            &self.model,
            &hovered,
            &self.cfg,
            &anims,
            self.save_flash.map(|(id, _)| id),
            self.shelf_prefs.look(),
            (&saving.0, saving.1),
        );

        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_err() {
            return;
        }
        // One callback in flight: a hover or configure redraw mid-frame must
        // not add another tick per frame.
        if self.animating() && self.frame_requested.is_none() {
            surface.frame(qh, surface.clone());
            self.frame_requested = Some(std::time::Instant::now());
        }
        layer.commit();
        for trace in self.pending_image_traces.drain(..) {
            super::super::timing::mark_request("shelf_commit", Some(trace));
        }
    }

    // ----- Recording lifecycle -------------------------------------------
}
