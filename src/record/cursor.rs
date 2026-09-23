//! Receipt-time resampling. No prediction, no input-device manipulation.
use std::{collections::VecDeque, time::Duration};

pub const LOOKAHEAD: Duration = Duration::from_millis(8);
const HISTORY: Duration = Duration::from_millis(125);
const GAP: Duration = Duration::from_millis(32);
const CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug)]
struct Sample {
    time: Duration,
    position: Option<(f64, f64)>,
    generation: u64,
}
#[derive(Default)]
pub struct Motion {
    samples: VecDeque<Sample>,
    generation: u64,
    visible: bool,
}
impl Motion {
    pub fn visibility(&mut self, time: Duration, visible: bool) {
        self.visible = visible;
        self.generation = self.generation.wrapping_add(1);
        self.push(Sample {
            time,
            position: None,
            generation: self.generation,
        });
    }
    pub fn position(&mut self, time: Duration, x: i32, y: i32) {
        if !self.visible {
            return;
        }
        let point = (f64::from(x), f64::from(y));
        if let Some(previous) = self.samples.back() {
            let discontinuity = time < previous.time
                || time.saturating_sub(previous.time) > GAP
                || previous
                    .position
                    .is_some_and(|p| (p.0 - point.0).hypot(p.1 - point.1) > 256.0);
            if discontinuity {
                self.generation = self.generation.wrapping_add(1);
            }
        }
        self.push(Sample {
            time,
            position: Some(point),
            generation: self.generation,
        });
    }
    fn push(&mut self, mut sample: Sample) {
        if self.samples.back().is_some_and(|s| sample.time < s.time)
            || self.samples.len() == CAPACITY
        {
            self.samples.clear();
            self.generation = self.generation.wrapping_add(1);
            sample.generation = self.generation;
        }
        while self.samples.len() > 1 && sample.time.saturating_sub(self.samples[1].time) > HISTORY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
    pub fn at(&self, time: Duration) -> Option<(f32, f32)> {
        let index = self.samples.iter().rposition(|s| s.time <= time)?;
        let before = self.samples[index];
        let mut point = before.position?;
        if let Some(after) = self.samples.get(index + 1)
            && after.generation == before.generation
            && after.time > before.time
            && after.time - before.time <= GAP
            && after.time - time <= LOOKAHEAD
            && let Some(next) = after.position
        {
            let fraction =
                (time - before.time).as_secs_f64() / (after.time - before.time).as_secs_f64();
            point.0 += (next.0 - point.0) * fraction;
            point.1 += (next.1 - point.1) * fraction;
        }
        Some((point.0 as f32, point.1 as f32))
    }
}
/// Integer frame-index timing avoids accumulation of rounded period errors.
pub fn frame_time(index: u64, fps: u32) -> Option<Duration> {
    if !(1..=240).contains(&fps) {
        return None;
    }
    let nanos = u128::from(index).checked_mul(1_000_000_000)? / u128::from(fps);
    Some(Duration::new(
        u64::try_from(nanos / 1_000_000_000).ok()?,
        (nanos % 1_000_000_000) as u32,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    #[test]
    fn interpolates_jitter_without_extrapolating_or_overshooting() {
        let mut m = Motion::default();
        m.visibility(ms(0), true);
        m.position(ms(1), 0, 10);
        m.position(ms(9), 80, 10);
        assert_eq!(m.at(ms(5)), Some((40., 10.)));
        assert_eq!(m.at(ms(100)), Some((80., 10.)));
        m.position(ms(17), 0, 10);
        assert_eq!(m.at(ms(13)), Some((40., 10.)));
    }
    #[test]
    fn visibility_gaps_warps_and_equal_timestamps_are_boundaries() {
        let mut m = Motion::default();
        m.visibility(ms(0), true);
        m.position(ms(1), 10, 20);
        m.visibility(ms(6), false);
        m.visibility(ms(7), true);
        m.position(ms(8), 90, 90);
        assert_eq!(m.at(ms(5)), Some((10., 20.)));
        assert_eq!(m.at(ms(6)), None);
        assert_eq!(m.at(ms(7)), None);
        m.position(ms(9), 2000, -300);
        assert_eq!(m.at(ms(8)), Some((90., 90.)));
        m.position(ms(100), 2020, -300);
        assert_eq!(m.at(ms(99)), Some((2000., -300.)));
        m.position(ms(100), 2030, -300);
        assert_eq!(m.at(ms(100)), Some((2030., -300.)));
    }
    #[test]
    fn bounded_history_and_clock_reset() {
        let mut m = Motion::default();
        m.visibility(ms(0), true);
        for n in 0..1000 {
            m.position(Duration::from_micros(n), n as i32, 0);
            assert!(m.samples.len() <= CAPACITY);
        }
        m.position(ms(0), -3, 4);
        assert_eq!(m.samples.len(), 1);
        assert_eq!(m.at(ms(0)), Some((-3., 4.)));
        for n in 1..1000 {
            m.position(ms(n), n as i32, 0);
        }
        assert!(m.samples.len() <= 128);
    }
    #[test]
    fn frame_clock_does_not_accumulate_rounding_error() {
        for fps in [60, 120, 144, 240] {
            assert_eq!(
                frame_time(u64::from(fps) * 3600, fps),
                Some(Duration::from_secs(3600))
            );
        }
        assert_eq!(frame_time(1, 0), None);
        assert_eq!(frame_time(1, 241), None);
    }
}
