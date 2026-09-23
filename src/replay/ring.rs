use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Clone)]
pub struct Entry<T> {
    pub payload: T,
    pub keyframe: bool,
    pub start_us: i64,
    pub end_us: i64,
    pub bytes: usize,
}

#[derive(Clone)]
struct Gop<T> {
    start_us: i64,
    packets: VecDeque<Entry<T>>,
    bytes: usize,
}

pub struct Ring<T> {
    video: VecDeque<Arc<Gop<T>>>,
    audio: VecDeque<Entry<T>>,
    duration_us: i64,
    limit: usize,
    bytes: usize,
    peak_bytes: usize,
    last_video_us: Option<i64>,
    last_audio_us: Option<i64>,
    end_us: Option<i64>,
    evicted_gops: u64,
}

impl<T: Clone> Ring<T> {
    pub fn new(duration_us: i64, limit: usize) -> Result<Self, String> {
        if duration_us <= 0 || limit == 0 {
            return Err("duration and memory budget must be positive".into());
        }
        Ok(Self {
            video: VecDeque::new(),
            audio: VecDeque::new(),
            duration_us,
            limit,
            bytes: 0,
            peak_bytes: 0,
            last_video_us: None,
            last_audio_us: None,
            end_us: None,
            evicted_gops: 0,
        })
    }

    /// A rejected packet ends the input session.
    pub fn push(&mut self, entry: Entry<T>, video: bool) -> Result<(), String> {
        if entry.end_us <= entry.start_us || entry.bytes == 0 {
            return Err("invalid packet interval or accounted size".into());
        }
        if entry.bytes > self.limit {
            return Err("packet exceeds memory budget".into());
        }
        let last = if video {
            self.last_video_us
        } else {
            self.last_audio_us
        };
        if last.is_some_and(|last| entry.start_us <= last) {
            return Err("non-increasing stream timestamps".into());
        }
        if video {
            self.last_video_us = Some(entry.start_us);
        } else {
            self.last_audio_us = Some(entry.start_us);
        }

        if video && entry.keyframe {
            self.video.push_back(Arc::new(Gop {
                start_us: entry.start_us,
                packets: VecDeque::new(),
                bytes: 0,
            }));
        }
        if self.video.is_empty() {
            return Ok(());
        }
        if !video && entry.end_us <= self.video[0].start_us {
            return Ok(());
        }
        let prospective_end = if video {
            entry.end_us
        } else {
            self.end_us.unwrap_or(entry.end_us)
        };
        let target = prospective_end.saturating_sub(self.duration_us);
        while self.video.front().is_some_and(|gop| gop.start_us < target) {
            self.evict_gop();
        }
        if self.video.is_empty() {
            return Ok(());
        }
        while entry.bytes > self.limit - self.bytes && self.video.len() > 1 {
            self.evict_gop();
        }
        if entry.bytes > self.limit - self.bytes {
            return Err(format!(
                "active GOP and audio exceed memory budget ({} held + {} incoming > {} limit)",
                self.bytes, entry.bytes, self.limit
            ));
        }
        self.bytes += entry.bytes;
        self.peak_bytes = self.peak_bytes.max(self.bytes);
        if video {
            self.end_us = Some(entry.end_us);
            // A frozen snapshot shares completed GOPs. Only the active GOP
            // needs copying, once, when ingest first appends after a snapshot.
            let gop = Arc::make_mut(self.video.back_mut().expect("nonempty video ring"));
            gop.bytes += entry.bytes;
            gop.packets.push_back(entry);
        } else {
            self.audio.push_back(entry);
        }
        Ok(())
    }

    fn evict_gop(&mut self) {
        let gop = self.video.pop_front().expect("eviction requires a GOP");
        self.bytes -= gop.bytes;
        self.evicted_gops += 1;
        let start = self.video.front().map(|gop| gop.start_us);
        if start.is_none() {
            self.end_us = None;
        }
        while self
            .audio
            .front()
            .is_some_and(|entry| start.is_none_or(|start| entry.end_us <= start))
        {
            self.bytes -= self.audio.pop_front().expect("audio front exists").bytes;
        }
    }

    pub fn bounds(&self) -> Result<(i64, i64), String> {
        let start = self
            .video
            .front()
            .ok_or("no video keyframe received")?
            .start_us;
        let end = self.end_us.ok_or("no complete video frame received")?;
        if end.checked_sub(start).is_none_or(|duration| duration <= 0) {
            return Err("invalid video time span".into());
        }
        Ok((start, end))
    }

    pub fn video(&self) -> impl Iterator<Item = &Entry<T>> {
        self.video.iter().flat_map(|gop| &gop.packets)
    }

    /// Share immutable video GOPs; audio descriptors are copied independently.
    /// Appending and eviction cannot change a previously returned snapshot.
    pub fn snapshot(&self) -> Self {
        Self {
            video: self.video.clone(),
            audio: self.audio.clone(),
            duration_us: self.duration_us,
            limit: self.limit,
            bytes: self.bytes,
            peak_bytes: self.bytes,
            last_video_us: self.last_video_us,
            last_audio_us: self.last_audio_us,
            end_us: self.end_us,
            evicted_gops: self.evicted_gops,
        }
    }

    pub fn tail_snapshot(
        &self,
        mut reference: impl FnMut(&T) -> Result<T, String>,
    ) -> Result<Self, String> {
        let gop = self.video.back().ok_or("no video keyframe received")?;
        let mut copy = Self::new(self.duration_us, self.limit)?;
        for (entry, video) in gop.packets.iter().map(|e| (e, true)).chain(
            self.audio()
                .filter(|e| e.end_us > gop.start_us)
                .map(|e| (e, false)),
        ) {
            copy.push(
                Entry {
                    payload: reference(&entry.payload)?,
                    keyframe: entry.keyframe,
                    start_us: entry.start_us,
                    end_us: entry.end_us,
                    bytes: entry.bytes,
                },
                video,
            )?;
        }
        Ok(copy)
    }

    pub fn audio(&self) -> impl Iterator<Item = &Entry<T>> {
        let end = self.end_us.unwrap_or(i64::MIN);
        self.audio
            .iter()
            .take_while(move |entry| entry.start_us < end)
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn peak_bytes(&self) -> usize {
        self.peak_bytes
    }
    pub fn evicted_gops(&self) -> u64 {
        self.evicted_gops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time: i64, keyframe: bool, bytes: usize) -> Entry<()> {
        Entry {
            payload: (),
            keyframe,
            start_us: time * 1_000_000,
            end_us: (time + 1) * 1_000_000,
            bytes,
        }
    }

    #[test]
    fn time_eviction_starts_at_a_keyframe_inside_the_window() {
        let mut ring = Ring::new(3_000_000, 100_000).unwrap();
        for n in 0..10 {
            ring.push(entry(n, n % 2 == 0, 20), true).unwrap();
            let (start, end) = ring.bounds().unwrap();
            assert!(end - start <= 3_000_000);
        }
        assert_eq!(ring.bounds().unwrap(), (8_000_000, 10_000_000));
        assert_eq!(ring.video().count(), 2);
        assert!(ring.video().next().unwrap().keyframe);
    }

    #[test]
    fn snapshot_keeps_shared_payloads_after_live_eviction() {
        use std::sync::Arc;
        let payload = Arc::new([1, 2, 3]);
        let mut ring = Ring::new(1_000_000, 1000).unwrap();
        ring.push(
            Entry {
                payload: payload.clone(),
                keyframe: true,
                start_us: 0,
                end_us: 1_000_000,
                bytes: 30,
            },
            true,
        )
        .unwrap();
        let frozen = ring.snapshot();
        ring.push(
            Entry {
                payload: Arc::new([4, 5, 6]),
                keyframe: true,
                start_us: 1_000_000,
                end_us: 2_000_000,
                bytes: 30,
            },
            true,
        )
        .unwrap();
        assert_eq!(frozen.bounds().unwrap(), (0, 1_000_000));
        assert_eq!(ring.bounds().unwrap(), (1_000_000, 2_000_000));
        assert!(Arc::ptr_eq(
            &frozen.video().next().unwrap().payload,
            &payload
        ));
        assert_eq!(Arc::strong_count(&payload), 2);
        drop(frozen);
        assert_eq!(Arc::strong_count(&payload), 1);
    }

    #[test]
    fn snapshot_shares_gops_and_only_copies_the_active_gop_on_append() {
        let mut ring = Ring::new(60_000_000, 100_000).unwrap();
        for n in 0..8 {
            ring.push(entry(n, n % 4 == 0, 20), true).unwrap();
            ring.push(entry(n, false, 10), false).unwrap();
        }
        let frozen = ring.snapshot();
        assert!(Arc::ptr_eq(&ring.video[0], &frozen.video[0]));
        assert!(Arc::ptr_eq(&ring.video[1], &frozen.video[1]));
        ring.push(entry(8, false, 20), true).unwrap();
        assert!(Arc::ptr_eq(&ring.video[0], &frozen.video[0]));
        assert!(!Arc::ptr_eq(&ring.video[1], &frozen.video[1]));
        let active = Arc::as_ptr(&ring.video[1]);
        ring.push(entry(9, false, 20), true).unwrap();
        assert_eq!(active, Arc::as_ptr(&ring.video[1]));
        ring.push(entry(8, false, 10), false).unwrap();
        assert_eq!(frozen.video().count(), 8);
        assert_eq!(frozen.audio().count(), 8);
        assert_eq!(frozen.bounds().unwrap(), (0, 8_000_000));
        assert_eq!(frozen.bytes(), 240);
        assert_eq!(ring.video().count(), 10);
        assert_eq!(ring.audio().count(), 9);
        assert_eq!(ring.bytes(), 290);
        // Eviction and new keyframes preserve the shared snapshot too.
        ring.push(entry(70, true, 20), true).unwrap();
        assert_eq!(ring.video().count(), 1);
        assert_eq!(frozen.video().count(), 8);
        assert_eq!(frozen.audio().count(), 8);
    }

    #[test]
    fn configured_duration_is_the_maximum_not_a_fixed_sixty_seconds() {
        for seconds in [1, 30, 60, 120] {
            let mut ring = Ring::new(seconds * 1_000_000, 100_000).unwrap();
            for n in 0..250 {
                ring.push(entry(n, true, 20), true).unwrap();
                let (start, end) = ring.bounds().unwrap();
                assert_eq!(end, (n + 1) * 1_000_000);
                assert_eq!(end - start, (n + 1).min(seconds) * 1_000_000);
            }
        }
    }

    #[test]
    fn expired_active_gop_is_discarded_until_the_next_keyframe() {
        let mut ring = Ring::new(2_000_000, 100_000).unwrap();
        ring.push(entry(0, true, 20), true).unwrap();
        ring.push(entry(0, false, 10), false).unwrap();
        ring.push(entry(1, false, 20), true).unwrap();
        assert_eq!(ring.bounds().unwrap(), (0, 2_000_000));

        for n in 2..5 {
            ring.push(entry(n, false, 20), true).unwrap();
            ring.push(entry(n, false, 10), false).unwrap();
            assert!(ring.bounds().is_err());
            assert_eq!(ring.video().count(), 0);
            assert_eq!(ring.audio().count(), 0);
            assert_eq!(ring.bytes(), 0);
        }
        ring.push(entry(5, true, 20), true).unwrap();
        ring.push(entry(5, false, 10), false).unwrap();
        assert_eq!(ring.bounds().unwrap(), (5_000_000, 6_000_000));
        assert_eq!(ring.bytes(), 30);
    }

    #[test]
    fn frame_longer_than_the_window_is_not_retained() {
        let mut ring = Ring::new(500_000, 100_000).unwrap();
        ring.push(entry(0, true, 20), true).unwrap();
        assert!(ring.bounds().is_err());
        assert_eq!(ring.bytes(), 0);
        let mut frame = entry(1, true, 20);
        frame.end_us = 1_500_000;
        ring.push(frame, true).unwrap();
        assert_eq!(ring.bounds().unwrap(), (1_000_000, 1_500_000));
    }

    #[test]
    fn byte_eviction_shortens_history_without_breaking_gops() {
        let cost = entry(0, true, 100).bytes;
        let mut ring = Ring::new(60_000_000, cost * 3).unwrap();
        for n in 0..10 {
            ring.push(entry(n, n % 2 == 0, 100), true).unwrap();
        }
        assert_eq!(ring.bounds().unwrap(), (8_000_000, 10_000_000));
        assert!(ring.peak_bytes <= ring.limit);
    }

    #[test]
    fn oversized_active_gop_fails_without_exceeding_budget() {
        let cost = entry(0, true, 100).bytes;
        let mut ring = Ring::new(60_000_000, cost * 2).unwrap();
        ring.push(entry(0, true, 100), true).unwrap();
        ring.push(entry(1, false, 100), true).unwrap();
        assert!(ring.push(entry(2, false, 100), true).is_err());
        assert_eq!(ring.bytes, cost * 2);
    }

    #[test]
    fn audio_is_evicted_with_video_and_tail_is_bounded() {
        let mut ring = Ring::new(2_000_000, 100_000).unwrap();
        for n in 0..8 {
            ring.push(entry(n, n % 2 == 0, 20), true).unwrap();
            ring.push(entry(n, false, 10), false).unwrap();
        }
        ring.push(entry(8, false, 10), false).unwrap();
        assert_eq!(
            ring.audio().map(|e| e.start_us).collect::<Vec<_>>(),
            vec![6_000_000, 7_000_000]
        );
    }

    #[test]
    fn non_increasing_timestamps_are_rejected() {
        let mut ring = Ring::new(2_000_000, 100_000).unwrap();
        ring.push(entry(0, true, 10), true).unwrap();
        assert!(ring.push(entry(0, false, 10), true).is_err());
    }

    #[test]
    fn invalid_entries_do_not_start_a_history() {
        let mut ring = Ring::new(2_000_000, 100).unwrap();
        assert!(ring.push(entry(0, true, 101), true).is_err());
        assert!(ring.bounds().is_err());
        assert_eq!(ring.bytes(), 0);
        assert!(ring.push(entry(0, true, 0), true).is_err());
        let mut invalid = entry(0, true, 10);
        invalid.end_us = invalid.start_us;
        assert!(ring.push(invalid, true).is_err());
    }

    #[test]
    fn sustained_byte_pressure_keeps_a_decodable_bounded_history() {
        let mut ring = Ring::new(60_000_000, 8000).unwrap();
        let mut seed = 17_u32;
        for n in 0..10_000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ring.push(entry(n, n % 10 == 0, 10 + (seed % 491) as usize), true)
                .unwrap();
            ring.push(entry(n, false, 50), false).unwrap();
            assert!(ring.bytes() <= 8000);
            assert!(ring.video().next().unwrap().keyframe);
            let accounted: usize = ring
                .video()
                .chain(ring.audio())
                .map(|entry| entry.bytes)
                .sum();
            assert_eq!(accounted, ring.bytes());
            assert_eq!(ring.bounds().unwrap().1, (n + 1) * 1_000_000);
        }
    }
}
