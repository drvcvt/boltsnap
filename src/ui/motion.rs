//! Springs, evaluated from absolute time (`/ui` motion: every visible state
//! change is a spring; finite curves are for fixed-length choreography only).

use std::time::Instant;

use super::theme::{GLIDE, WHEEL_STEP};

/// A spring by its perceptual duration: at `duration` about 98.6 % of the
/// way is covered. Critically damped (bounce 0), so it never overshoots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token {
    pub duration: f32,
}

impl Token {
    fn omega(self) -> f32 {
        std::f32::consts::TAU / self.duration
    }
}

/// A value moving to `target`. A retarget starts from where the value is,
/// with the speed it has, so it never jumps or stops dead.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    target: f32,
    /// Offset from `target` and speed (units per second) at `start`.
    y0: f32,
    v0: f32,
    start: Instant,
    omega: f32,
    /// Seconds after `start` at which the value is committed to `target`.
    end: f32,
    /// Distance from the target that counts as there, in units.
    precision: f32,
}

/// Refresh rate the end test assumes for "moves less than `precision` in a
/// frame"; a faster display only makes the remaining step smaller.
const SLOWEST_FRAME_HZ: f32 = 60.0;

impl Spring {
    pub fn settled(value: f32, now: Instant) -> Self {
        Self {
            target: value,
            y0: 0.0,
            v0: 0.0,
            start: now,
            omega: 1.0,
            end: 0.0,
            precision: 0.001,
        }
    }

    /// The same spring, counting as settled within `precision` units
    /// (pixels want 0.05, fractions the default 0.001).
    pub fn with_precision(mut self, precision: f32) -> Self {
        self.precision = precision;
        self
    }

    fn elapsed(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.start).as_secs_f32()
    }

    /// Offset and speed `t` seconds after `start`.
    fn at(&self, t: f32) -> (f32, f32) {
        let b = self.v0 + self.omega * self.y0;
        let e = (-self.omega * t).exp();
        ((self.y0 + b * t) * e, (self.v0 - self.omega * b * t) * e)
    }

    pub fn value(&self, now: Instant) -> f32 {
        let t = self.elapsed(now);
        if t >= self.end {
            return self.target;
        }
        self.target + self.at(t).0
    }

    /// Units per second.
    pub fn velocity(&self, now: Instant) -> f32 {
        let t = self.elapsed(now);
        if t >= self.end { 0.0 } else { self.at(t).1 }
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn done(&self, now: Instant) -> bool {
        self.elapsed(now) >= self.end
    }

    pub fn retarget(&mut self, to: f32, token: Token, now: Instant) {
        if to == self.target {
            return;
        }
        let (value, velocity) = (self.value(now), self.velocity(now));
        self.target = to;
        self.y0 = value - to;
        self.v0 = velocity;
        self.start = now;
        self.omega = token.omega();
        self.end = self.settle_time(token);
    }

    /// First time at which the spring is within `precision` and moves less
    /// than that per frame; from then on it is exactly at the target.
    fn settle_time(&self, token: Token) -> f32 {
        let step = token.duration / 200.0;
        let near = |t: f32| {
            let (y, v) = self.at(t);
            y.abs() <= self.precision && v.abs() / SLOWEST_FRAME_HZ <= self.precision
        };
        // A critically damped spring crosses the target at most once, so the
        // last time it is not near is found by walking back from far out.
        let mut t = 10.0 * token.duration;
        while t > 0.0 && near(t - step) {
            t -= step;
        }
        t
    }

    /// Jump without motion.
    pub fn set(&mut self, value: f32, now: Instant) {
        *self = Self::settled(value, now).with_precision(self.precision);
    }
}

/// Vertical scroll of a page: wheel notches glide, a touchpad follows exactly.
#[derive(Clone, Copy, Debug)]
pub struct Scroll {
    offset: Spring,
    max: f32,
}

impl Scroll {
    pub fn new(now: Instant) -> Self {
        Self {
            offset: Spring::settled(0.0, now).with_precision(0.05),
            max: 0.0,
        }
    }

    pub fn offset(&self, now: Instant) -> f32 {
        self.offset.value(now).clamp(0.0, self.max)
    }

    pub fn max(&self) -> f32 {
        self.max
    }

    pub fn moving(&self, now: Instant) -> bool {
        !self.offset.done(now)
    }

    /// Content taller than the viewport by `max`. A resize follows at once
    /// (on the no-animation list): the offset is clamped, not glided.
    pub fn set_max(&mut self, max: f32, now: Instant) {
        self.max = max.max(0.0);
        if self.offset.target() > self.max {
            self.offset.set(self.offset(now).min(self.max), now);
        }
    }

    /// `notches` from a wheel, or `pixels` from a touchpad when `notches` is 0.
    pub fn wheel(&mut self, notches: i32, pixels: f32, now: Instant) {
        if notches != 0 {
            let to = (self.offset.target() + notches as f32 * WHEEL_STEP).clamp(0.0, self.max);
            self.offset.retarget(to, GLIDE, now);
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
        self.offset.retarget(to.clamp(0.0, self.max), GLIDE, now);
    }
}
