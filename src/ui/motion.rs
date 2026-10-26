//! Rest-to-rest motion, evaluated from absolute time.

use std::time::{Duration, Instant};

use super::theme::{SCROLL_GLIDE, WHEEL_STEP};

/// Minimum-jerk position, 0 to 1, for `u` in 0 to 1.
pub fn min_jerk(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * u * (10.0 - 15.0 * u + 6.0 * u * u)
}

/// A value moving to `to`. A retarget starts from where the value is, with
/// the speed it has, so it never jumps or stops dead.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    /// Initial speed in units per whole duration.
    speed: f32,
    start: Instant,
    duration: Duration,
}

impl Tween {
    pub fn settled(value: f32, now: Instant) -> Self {
        Self {
            from: value,
            to: value,
            speed: 0.0,
            start: now,
            duration: Duration::ZERO,
        }
    }

    fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32())
            .min(1.0)
    }

    /// Quintic with the start speed, zero acceleration at both ends and zero
    /// speed at the end; without a start speed it is minimum-jerk.
    pub fn value(&self, now: Instant) -> f32 {
        let u = self.progress(now);
        let carry = u - 6.0 * u.powi(3) + 8.0 * u.powi(4) - 3.0 * u.powi(5);
        self.from + (self.to - self.from) * min_jerk(u) + self.speed * carry
    }

    /// Units per second.
    pub fn velocity(&self, now: Instant) -> f32 {
        let u = self.progress(now);
        if u >= 1.0 {
            return 0.0;
        }
        let ease = 30.0 * u * u - 60.0 * u.powi(3) + 30.0 * u.powi(4);
        let carry = 1.0 - 18.0 * u * u + 32.0 * u.powi(3) - 15.0 * u.powi(4);
        ((self.to - self.from) * ease + self.speed * carry) / self.duration.as_secs_f32()
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    pub fn retarget(&mut self, to: f32, duration: Duration, now: Instant) {
        if to == self.to {
            return;
        }
        let (from, velocity) = (self.value(now), self.velocity(now));
        *self = Self {
            from,
            to,
            speed: velocity * duration.as_secs_f32(),
            start: now,
            duration,
        };
    }

    /// Jump without motion.
    pub fn set(&mut self, value: f32, now: Instant) {
        *self = Self::settled(value, now);
    }
}

/// Vertical scroll of a page: wheel notches glide, a touchpad follows exactly.
#[derive(Clone, Copy, Debug)]
pub struct Scroll {
    offset: Tween,
    max: f32,
}

impl Scroll {
    pub fn new(now: Instant) -> Self {
        Self {
            offset: Tween::settled(0.0, now),
            max: 0.0,
        }
    }

    pub fn offset(&self, now: Instant) -> f32 {
        self.offset.value(now).clamp(0.0, self.max)
    }

    pub fn moving(&self, now: Instant) -> bool {
        !self.offset.done(now)
    }

    /// Content taller than the viewport by `max`.
    pub fn set_max(&mut self, max: f32, now: Instant) {
        self.max = max.max(0.0);
        if self.offset.target() > self.max {
            self.offset.set(self.offset(now), now);
            self.offset.retarget(self.max, SCROLL_GLIDE, now);
        }
    }

    /// `notches` from a wheel, or `pixels` from a touchpad when `notches` is 0.
    pub fn wheel(&mut self, notches: i32, pixels: f32, now: Instant) {
        if notches != 0 {
            let to = (self.offset.target() + notches as f32 * WHEEL_STEP).clamp(0.0, self.max);
            self.offset.retarget(to, SCROLL_GLIDE, now);
        } else {
            let to = (self.offset(now) + pixels).clamp(0.0, self.max);
            self.offset.set(to, now);
        }
    }

    /// Glide so `top..bottom` (content coordinates) is inside a viewport of
    /// `height`.
    pub fn reveal(&mut self, top: f32, bottom: f32, height: f32, now: Instant) {
        let current = self.offset.target();
        let to = if top < current {
            top
        } else if bottom > current + height {
            bottom - height
        } else {
            return;
        };
        self.offset
            .retarget(to.clamp(0.0, self.max), SCROLL_GLIDE, now);
    }
}
