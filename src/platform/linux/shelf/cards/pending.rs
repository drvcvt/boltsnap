//! A replay clip's card from the moment of the save: it shows the frozen
//! frame while the worker writes the clip, an arc turning in its badge, and
//! becomes the finished card when the clip is there (replay plan §13 C1).

use std::path::PathBuf;
use std::time::Instant;

use boltsnap::shelf_card::HOVER_MS;

use crate::record::finalize::FinalizedClip;
use crate::shelf::model::{CardKind, FileLifetime};

use super::anim::AnimKind;
use super::placement::{card_output, focused_monitor_name};
use super::{Daemon, cards};

/// A clip being written, by the token the replay control gave its save.
struct Pending {
    token: u64,
    /// `None` once its card was closed: the clip goes when it arrives.
    card: Option<u64>,
    /// The card shows the frozen frame (a region clip), not a placeholder.
    framed: bool,
    /// When the clip arrived; the badge then fades from the arc to the ▶.
    ready: Option<Instant>,
}

/// What came of a clip that arrived.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Arrival {
    /// Its card is waiting for it; `framed`: it already shows the clip's
    /// last frame.
    Card { card: u64, framed: bool },
    /// Its card was closed meanwhile: the file is not wanted.
    Closed,
    /// No card waits for this token (a control older than this shelf).
    Unknown,
}

#[derive(Default)]
pub(super) struct PendingClips {
    clips: Vec<Pending>,
}

impl PendingClips {
    pub(super) fn start(&mut self, token: u64, card: u64, framed: bool) {
        self.clips.push(Pending {
            token,
            card: Some(card),
            framed,
            ready: None,
        });
    }

    pub(super) fn arrive(&mut self, token: u64, now: Instant) -> Arrival {
        let Some(index) = self
            .clips
            .iter()
            .position(|clip| clip.token == token && clip.ready.is_none())
        else {
            return Arrival::Unknown;
        };
        match self.clips[index].card {
            Some(card) => {
                self.clips[index].ready = Some(now);
                Arrival::Card {
                    card,
                    framed: self.clips[index].framed,
                }
            }
            None => {
                self.clips.remove(index);
                Arrival::Closed
            }
        }
    }

    /// The save failed: the card to take away, if it is still there.
    pub(super) fn fail(&mut self, token: u64) -> Option<u64> {
        let index = self
            .clips
            .iter()
            .position(|clip| clip.token == token && clip.ready.is_none())?;
        self.clips.remove(index).card
    }

    /// Card `card` went (closed, or out of the cache).
    pub(super) fn close(&mut self, card: u64) {
        self.clips
            .retain(|clip| clip.card != Some(card) || clip.ready.is_none());
        for clip in &mut self.clips {
            if clip.card == Some(card) {
                clip.card = None;
            }
        }
    }

    /// Its clip is still being written: no opening, dragging, copying or
    /// saving yet.
    pub(super) fn is_saving(&self, card: u64) -> bool {
        self.clips
            .iter()
            .any(|clip| clip.card == Some(card) && clip.ready.is_none())
    }

    /// (card, how far to ready) for the painter; with `fade` false the arc
    /// goes at once.
    pub(super) fn lifts(&self, now: Instant, fade: bool) -> Vec<(u64, f32)> {
        self.clips
            .iter()
            .filter_map(|clip| {
                let ready = match clip.ready {
                    None => 0.0,
                    Some(_) if !fade => return None,
                    Some(at) => {
                        let ms = now.saturating_duration_since(at).as_secs_f32() * 1000.0;
                        (ms / HOVER_MS as f32).min(1.0)
                    }
                };
                Some((clip.card?, ready))
            })
            .collect()
    }

    /// Forget the clips whose fade is over.
    pub(super) fn retire(&mut self, now: Instant) {
        self.clips.retain(|clip| {
            clip.ready
                .is_none_or(|at| now.saturating_duration_since(at).as_millis() < HOVER_MS)
        });
    }

    /// Something is still turning or fading: keep drawing frames.
    pub(super) fn active(&self) -> bool {
        self.clips.iter().any(|clip| clip.card.is_some())
    }
}

impl Daemon {
    /// The replay control accepted a save: the card shows now, with `thumb`
    /// (the frozen frame cut like the clip) or, for a full-screen clip, a
    /// placeholder until its first frame is read.
    pub(super) fn add_pending_clip(
        &mut self,
        token: u64,
        output: String,
        thumb: Option<image::RgbaImage>,
    ) {
        let Some(qh) = self.qh.clone() else {
            return;
        };
        let size = cards::card_size();
        let thumb = thumb.filter(|thumb| thumb.dimensions() == size.dims());
        let framed = thumb.is_some();
        let thumb = thumb.unwrap_or_else(|| {
            image::RgbaImage::from_pixel(size.w, size.h, image::Rgba([32, 32, 40, 255]))
        });
        let on_output = card_output(
            &[FinalizedClip {
                path: PathBuf::new(),
                output: Some(output.clone()),
                permanent: false,
            }],
            focused_monitor_name,
        );
        let id = self.model.add_kind_with_lifetime(
            PathBuf::new(),
            thumb,
            format!("record:{output}"),
            CardKind::Video,
            FileLifetime::Temporary,
        );
        self.pending.start(token, id, framed);
        self.start_anim(id, AnimKind::Appear);
        self.trim_shelf_cache();
        self.relayout();
        self.place_on_output(on_output.as_deref(), &qh);
        self.draw(&qh);
    }

    /// The clip of save `token` is written. Its card takes the file, or the
    /// file goes when the card was closed; without a waiting card it arrives
    /// as before.
    pub(super) fn finish_pending_clip(&mut self, token: u64, path: PathBuf, output: String) {
        match self.pending.arrive(token, Instant::now()) {
            Arrival::Card { card, framed } => {
                self.model.set_file(card, path.clone());
                // A full-screen card has only the placeholder; a region card
                // keeps its frozen frame.
                if !framed {
                    self.thumbnails.submit(card, path);
                }
                if let Some(qh) = self.qh.clone() {
                    self.draw(&qh);
                }
            }
            Arrival::Closed => {
                let _ = std::fs::remove_file(&path);
            }
            Arrival::Unknown => self.add_finalized_cards(
                vec![FinalizedClip {
                    path,
                    output: Some(output),
                    permanent: false,
                }],
                false,
            ),
        }
    }

    /// The save of `token` failed: its card leaves.
    pub(super) fn fail_pending_clip(&mut self, token: u64) {
        if let Some(id) = self.pending.fail(token) {
            self.start_anim(id, AnimKind::Disappear);
            if let Some(qh) = self.qh.clone() {
                self.draw(&qh);
            }
        }
    }

    /// The painter's saving cards and how far their arc has turned: a turn a
    /// second, standing still with `[shelf] animations = false`.
    pub(super) fn saving_frame(&self) -> (Vec<(u64, f32)>, f32) {
        static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let animations = self.shelf_prefs.animations;
        let spin = if animations {
            // Whole turns dropped first: an f32 of days of seconds jitters.
            (EPOCH.get_or_init(Instant::now).elapsed().as_millis() % 1000) as f32 / 1000.0
        } else {
            0.0
        };
        (self.pending.lifts(Instant::now(), animations), spin)
    }

    pub(super) fn is_pending(&self, id: u64) -> bool {
        self.pending.is_saving(id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Arrival, PendingClips};

    #[test]
    fn a_clip_arrives_at_its_card_and_its_badge_fades_to_ready() {
        let now = Instant::now();
        let mut clips = PendingClips::default();
        clips.start(7, 1, true);
        assert!(clips.is_saving(1) && clips.active());
        assert_eq!(clips.lifts(now, true), [(1, 0.0)]);
        assert_eq!(
            clips.arrive(7, now),
            Arrival::Card {
                card: 1,
                framed: true
            }
        );
        assert!(!clips.is_saving(1));
        let half = now + Duration::from_millis(60);
        assert_eq!(clips.lifts(half, true), [(1, 0.5)]);
        // Without animations the arc goes at once.
        assert!(clips.lifts(half, false).is_empty());
        clips.retire(now + Duration::from_millis(200));
        assert!(!clips.active() && clips.lifts(now, true).is_empty());
        // The token is spent.
        assert_eq!(clips.arrive(7, now), Arrival::Unknown);
    }

    #[test]
    fn a_closed_card_drops_its_clip_and_a_failed_save_takes_the_card() {
        let now = Instant::now();
        let mut clips = PendingClips::default();
        clips.start(1, 10, false);
        clips.start(2, 11, true);
        clips.close(10);
        assert!(!clips.is_saving(10) && clips.is_saving(11));
        assert_eq!(clips.arrive(1, now), Arrival::Closed);
        assert_eq!(clips.fail(2), Some(11));
        assert!(!clips.active());
        // A failure after the card was closed has nothing to take.
        clips.start(3, 12, false);
        clips.close(12);
        assert_eq!(clips.fail(3), None);
        assert_eq!(clips.arrive(9, now), Arrival::Unknown);
    }
}
