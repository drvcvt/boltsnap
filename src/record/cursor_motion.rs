//! Spring-smoothed cursor motion, motion blur taps, the drawn arrow and the
//! live feed into the gpu-screen-recorder plugin. Platform-neutral and
//! self-contained: the plugin crate includes this file with `#[path]`.

use std::collections::VecDeque;

/// Spring parameters in Screen Studio's terms. Both presets are critically
/// damped, so the cursor settles without overshoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub tension: f64,
    pub friction: f64,
    pub mass: f64,
}

/// 90 % of a jump after about 0.5 s: the cursor glides behind the pointer.
pub const MELLOW: Preset = Preset {
    tension: 60.0,
    friction: 15.5,
    mass: 1.0,
};
/// 90 % of a jump after about 0.16 s.
pub const QUICK: Preset = Preset {
    tension: 600.0,
    friction: 49.0,
    mass: 1.0,
};

/// Preset for a `record_cursor` key.
pub fn preset(name: &str) -> Option<Preset> {
    match name {
        "mellow" => Some(MELLOW),
        "quick" => Some(QUICK),
        _ => None,
    }
}

/// Nominal arrow size in logical pixels, like the desktop cursor.
pub fn cursor_size() -> f32 {
    std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|size| (8.0..=256.0).contains(size))
        .unwrap_or(24.0)
}

/// One observation in clip pixels and milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    /// Cursor at (x, y) in pixels of the final frame.
    At { ms: f64, x: f64, y: f64 },
    /// Cursor left the recorded area.
    Gone { ms: f64 },
}

impl Sample {
    pub fn ms(&self) -> f64 {
        match *self {
            Sample::At { ms, .. } | Sample::Gone { ms } => ms,
        }
    }
}

/// Maps compositor-global logical coordinates to clip pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    /// Global logical origin of the clip's top-left pixel.
    pub origin: (f64, f64),
    /// Clip pixels per logical unit.
    pub scale: f64,
}

impl Mapping {
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.origin.0) * self.scale,
            (y - self.origin.1) * self.scale,
        )
    }
}

/// Exposure per frame for motion blur: a fixed 1/120 s, so the trail looks the
/// same at 60 and 240 FPS. Longer than a 240 FPS frame, so exposures overlap.
pub const SHUTTER_MS: usize = 8;
/// Sub-positions averaged per frame.
pub const BLUR_SAMPLES: usize = 8;
/// Longer gaps between frames restart the simulation settled at the target.
const MAX_GAP_MS: i64 = 2000;

#[derive(Default)]
struct Spring {
    target: Option<(f64, f64)>,
    position: (f64, f64),
    velocity: (f64, f64),
}

/// Pointer moves smaller than this (clip pixels) around the current target are
/// hand tremor, not intent, and do not move the smoothed cursor.
const SHAKE_PX: f64 = 2.0;

impl Spring {
    fn apply(&mut self, sample: Sample) {
        match sample {
            Sample::At { x, y, .. } => match self.target {
                None => {
                    self.position = (x, y);
                    self.velocity = (0.0, 0.0);
                    self.target = Some((x, y));
                }
                Some((tx, ty)) if (x - tx).hypot(y - ty) < SHAKE_PX => {}
                Some(_) => self.target = Some((x, y)),
            },
            Sample::Gone { .. } => self.target = None,
        }
    }

    /// Semi-implicit Euler; stable for these stiffnesses at 1 ms.
    fn advance(&mut self, preset: Preset, dt: f64) {
        let Some((tx, ty)) = self.target else {
            return;
        };
        let accel = |p: f64, v: f64, t: f64| {
            (-preset.tension * (p - t) - preset.friction * v) / preset.mass
        };
        self.velocity.0 += accel(self.position.0, self.velocity.0, tx) * dt;
        self.velocity.1 += accel(self.position.1, self.velocity.1, ty) * dt;
        self.position.0 += self.velocity.0 * dt;
        self.position.1 += self.velocity.1 * dt;
    }

    fn settle(&mut self) {
        if let Some(target) = self.target {
            self.position = target;
            self.velocity = (0.0, 0.0);
        }
    }
}

/// Live spring simulation in 1 ms steps. The spring follows the latest raw
/// sample; appearing snaps to the raw position; samples at a millisecond apply
/// before it is recorded.
pub struct Motion {
    preset: Preset,
    spring: Spring,
    /// Not yet applied samples, ordered by time.
    pending: VecDeque<Sample>,
    /// Next millisecond to simulate.
    next_ms: Option<i64>,
    /// Positions of the last `SHUTTER_MS + 1` simulated milliseconds, newest last.
    recent: VecDeque<Option<(f64, f64)>>,
}

impl Motion {
    pub fn new(preset: Preset) -> Self {
        Self {
            preset,
            spring: Spring::default(),
            pending: VecDeque::new(),
            next_ms: None,
            recent: VecDeque::with_capacity(SHUTTER_MS + 2),
        }
    }

    pub fn push(&mut self, sample: Sample) {
        let at = self.pending.partition_point(|s| s.ms() <= sample.ms());
        self.pending.insert(at, sample);
    }

    /// Simulate up to and including millisecond `ms`. The first call (and one
    /// after a long gap) starts a shutter before `ms` from the settled state,
    /// so the first frame is neither faded in nor gliding from old samples.
    pub fn advance_to(&mut self, ms: i64) {
        let mut tick = match self.next_ms {
            Some(next) if ms - next <= MAX_GAP_MS => next,
            _ => {
                let start = ms - SHUTTER_MS as i64;
                while self.pending.front().is_some_and(|s| s.ms() < start as f64) {
                    let sample = self.pending.pop_front().unwrap();
                    self.spring.apply(sample);
                }
                self.spring.settle();
                self.recent.clear();
                start
            }
        };
        while tick <= ms {
            while self.pending.front().is_some_and(|s| s.ms() <= tick as f64) {
                let sample = self.pending.pop_front().unwrap();
                self.spring.apply(sample);
            }
            if self.recent.len() > SHUTTER_MS {
                self.recent.pop_front();
            }
            self.recent
                .push_back(self.spring.target.map(|_| self.spring.position));
            self.spring.advance(self.preset, 0.001);
            tick += 1;
        }
        self.next_ms = Some(tick);
    }

    /// Smoothed position at the last simulated millisecond, `None` while hidden.
    pub fn position(&self) -> Option<(f64, f64)> {
        self.recent.back().copied().flatten()
    }

    /// Sprite top-left per blur tap over the exposure ending at the last
    /// simulated millisecond, `None` where the cursor was hidden. At rest all
    /// taps coincide; appearing and leaving fade over the exposure.
    pub fn taps(&self, hotspot: (f64, f64)) -> [Option<(i64, i64)>; BLUR_SAMPLES] {
        let mut taps = [None; BLUR_SAMPLES];
        let Some(newest) = self.recent.len().checked_sub(1) else {
            return taps;
        };
        for (tap, slot) in taps.iter_mut().enumerate() {
            let back = SHUTTER_MS * (BLUR_SAMPLES - 1 - tap) / (BLUR_SAMPLES - 1);
            *slot = self.recent[newest.saturating_sub(back)].map(|(x, y)| {
                (
                    (x - hotspot.0).round() as i64,
                    (y - hotspot.1).round() as i64,
                )
            });
        }
        taps
    }
}

/// Live feed line from a daemon cursor tracker to the plugin: `SOURCE US p X Y`
/// (global logical position) or `SOURCE US l` (left the output). `US` is
/// CLOCK_MONOTONIC microseconds.
pub fn feed_line(source: usize, us: u64, position: Option<(f64, f64)>) -> String {
    match position {
        Some((x, y)) => format!("{source} {us} p {x} {y}\n"),
        None => format!("{source} {us} l\n"),
    }
}

pub type FeedEvent = (usize, u64, Option<(f64, f64)>);

pub fn parse_feed_line(line: &str) -> Option<FeedEvent> {
    let fields: Vec<&str> = line.split_ascii_whitespace().collect();
    let number = |s: &str| s.parse::<f64>().ok().filter(|n| n.is_finite());
    match fields.as_slice() {
        [source, us, "p", x, y] => Some((
            source.parse().ok()?,
            us.parse().ok()?,
            Some((number(x)?, number(y)?)),
        )),
        [source, us, "l"] => Some((source.parse().ok()?, us.parse().ok()?, None)),
        _ => None,
    }
}

/// Turns feed events of several trackers into one sample stream: the cursor is
/// on one output at a time, so a leave counts only when no tracker sees it.
#[derive(Default)]
pub struct Sources {
    visible: Vec<bool>,
}

impl Sources {
    pub fn sample(
        &mut self,
        (source, us, position): FeedEvent,
        mapping: &Mapping,
    ) -> Option<Sample> {
        if source >= 64 {
            return None;
        }
        if self.visible.len() <= source {
            self.visible.resize(source + 1, false);
        }
        self.visible[source] = position.is_some();
        let ms = us as f64 / 1000.0;
        match position {
            Some((x, y)) => {
                let (x, y) = mapping.apply(x, y);
                Some(Sample::At { ms, x, y })
            }
            None if self.visible.iter().any(|v| *v) => None,
            None => Some(Sample::Gone { ms }),
        }
    }
}

/// Environment variable carrying the plugin configuration.
pub const PLUGIN_ENV: &str = "BOLTSNAP_CURSOR";

/// What the daemon tells a gsr plugin instance, as
/// `fd=5 preset=mellow size=24 origin=1920,0 width=1280`.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginConfig {
    /// Inherited read end of the cursor feed.
    pub fd: i32,
    /// `record_cursor` key of a smooth preset.
    pub preset: String,
    /// Nominal arrow size in logical pixels.
    pub size: f32,
    /// Global logical coordinate of the video's top-left pixel.
    pub origin: (f64, f64),
    /// Logical width the video covers.
    pub width: f64,
}

impl PluginConfig {
    pub fn to_env(&self) -> String {
        format!(
            "fd={} preset={} size={} origin={},{} width={}",
            self.fd, self.preset, self.size, self.origin.0, self.origin.1, self.width
        )
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let field = |key: &str| {
            text.split_ascii_whitespace()
                .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
                .ok_or_else(|| format!("{PLUGIN_ENV} lacks {key}"))
        };
        let bad = |key: &str| format!("{PLUGIN_ENV} has an invalid {key}");
        let finite = |key: &str, value: &str| {
            value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| bad(key))
        };
        let fd = field("fd")?
            .parse::<i32>()
            .ok()
            .filter(|fd| *fd >= 0)
            .ok_or_else(|| bad("fd"))?;
        let preset = field("preset")?;
        if self::preset(preset).is_none() {
            return Err(bad("preset"));
        }
        let size = finite("size", field("size")?)? as f32;
        if !(8.0..=256.0).contains(&size) {
            return Err(bad("size"));
        }
        let (x, y) = field("origin")?
            .split_once(',')
            .ok_or_else(|| bad("origin"))?;
        let width = finite("width", field("width")?)?;
        if width <= 0.0 {
            return Err(bad("width"));
        }
        Ok(Self {
            fd,
            preset: preset.to_owned(),
            size,
            origin: (finite("origin", x)?, finite("origin", y)?),
            width,
        })
    }

    /// Mapping into a video `video_width` pixels wide.
    pub fn mapping(&self, video_width: u32) -> Mapping {
        Mapping {
            origin: self.origin,
            scale: f64::from(video_width) / self.width,
        }
    }
}

/// Anti-aliased arrow in the macOS style (black, white rim, soft shadow),
/// `size` pixels nominal like an Xcursor size. Straight-alpha RGBA.
pub struct Arrow {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The tip, in pixels.
    pub hotspot: (f64, f64),
}

pub fn arrow(size: f32) -> Arrow {
    use tiny_skia::{Color, FillRule, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
    // Outline in units where the arrow is about 19 tall; tip at the origin.
    const POINTS: [(f32, f32); 7] = [
        (0.0, 0.0),
        (0.0, 16.6),
        (4.1, 12.9),
        (6.9, 19.2),
        (9.6, 18.1),
        (6.9, 11.9),
        (12.3, 11.9),
    ];
    let scale = (size / 22.0).max(0.5);
    let pad = 2.5 * scale;
    let width = ((12.3 + 5.5) * scale).ceil() as u32;
    let height = ((19.2 + 5.5) * scale).ceil() as u32;
    let mut builder = PathBuilder::new();
    builder.move_to(POINTS[0].0, POINTS[0].1);
    for (x, y) in &POINTS[1..] {
        builder.line_to(*x, *y);
    }
    builder.close();
    let path = builder.finish().expect("arrow outline is a valid path");
    let mut pixmap = Pixmap::new(width, height).expect("arrow size is non-zero");
    let place = |dx: f32, dy: f32| {
        Transform::from_row(scale, 0.0, 0.0, scale, pad + dx * scale, pad + dy * scale)
    };
    let paint = |r: u8, g: u8, b: u8, a: u8| {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(r, g, b, a));
        paint.anti_alias = true;
        paint
    };
    let stroke = |width: f32| Stroke {
        width,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    // Soft shadow: a wide faint rim and a denser core, offset down-right.
    pixmap.stroke_path(
        &path,
        &paint(0, 0, 0, 30),
        &stroke(3.6),
        place(0.3, 0.9),
        None,
    );
    pixmap.fill_path(
        &path,
        &paint(0, 0, 0, 60),
        FillRule::Winding,
        place(0.3, 0.9),
        None,
    );
    pixmap.stroke_path(
        &path,
        &paint(255, 255, 255, 255),
        &stroke(2.2),
        place(0.0, 0.0),
        None,
    );
    pixmap.fill_path(
        &path,
        &paint(12, 12, 12, 255),
        FillRule::Winding,
        place(0.0, 0.0),
        None,
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let c = pixel.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Arrow {
        rgba,
        width,
        height,
        hotspot: (f64::from(pad), f64::from(pad)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoothed position per output frame, like the old save-time renderer.
    fn smooth(
        samples: &[Sample],
        fps: u32,
        frames: usize,
        preset: Preset,
    ) -> Vec<Option<(f64, f64)>> {
        let mut motion = Motion::new(preset);
        for sample in samples {
            motion.push(*sample);
        }
        (0..frames)
            .map(|frame| {
                motion.advance_to((frame as u64 * 1000 / u64::from(fps)) as i64);
                motion.position()
            })
            .collect()
    }

    fn at(ms: f64, x: f64, y: f64) -> Sample {
        Sample::At { ms, x, y }
    }

    #[test]
    fn spring_settles_without_overshoot_and_quick_is_faster() {
        let samples = [at(0.0, 0.0, 0.0), at(10.0, 100.0, 0.0)];
        for preset in [MELLOW, QUICK] {
            let frames = smooth(&samples, 240, 480, preset);
            let xs: Vec<f64> = frames.iter().map(|p| p.unwrap().0).collect();
            assert_eq!(xs[0], 0.0);
            assert!(xs.windows(2).all(|w| w[1] >= w[0] - 1e-9), "monotonic");
            assert!(xs.iter().all(|x| *x <= 100.0 + 1e-6), "no overshoot");
            assert!((xs[479] - 100.0).abs() < 0.5, "settles");
        }
        let at = |preset| smooth(&samples, 240, 48, preset)[47].unwrap().0;
        assert!(at(QUICK) > at(MELLOW));
    }

    #[test]
    fn small_shakes_do_not_move_the_cursor() {
        let mut samples = vec![at(0.0, 100.0, 100.0)];
        for (i, (dx, dy)) in [(1.0, 0.0), (-1.2, 0.8), (0.5, -1.5), (1.8, 0.0)]
            .into_iter()
            .enumerate()
        {
            samples.push(at(10.0 * (i + 1) as f64, 100.0 + dx, 100.0 + dy));
        }
        let frames = smooth(&samples, 100, 20, MELLOW);
        assert!(frames.iter().all(|p| *p == Some((100.0, 100.0))));
        samples.push(at(60.0, 103.0, 100.0));
        let frames = smooth(&samples, 100, 200, MELLOW);
        assert!((frames[199].unwrap().0 - 103.0).abs() < 0.05);
    }

    #[test]
    fn mellow_glides_about_half_a_second() {
        let jump = [at(0.0, 0.0, 0.0), at(1.0, 100.0, 0.0)];
        let frames = smooth(&jump, 1000, 1500, MELLOW);
        let ninety = frames.iter().position(|p| p.unwrap().0 >= 90.0).unwrap();
        assert!((400..=600).contains(&ninety), "{ninety} ms");
    }

    #[test]
    fn appearing_snaps_and_leaving_hides_immediately() {
        let samples = [
            Sample::Gone { ms: 0.0 },
            at(100.0, 500.0, 300.0),
            Sample::Gone { ms: 200.0 },
        ];
        let frames = smooth(&samples, 100, 30, MELLOW);
        assert_eq!(frames[0], None);
        assert_eq!(frames[9], None);
        assert_eq!(frames[10], Some((500.0, 300.0)));
        assert!(frames[15].is_some());
        assert_eq!(frames[20], None);
    }

    #[test]
    fn late_and_unordered_samples_still_apply_in_time_order() {
        let mut motion = Motion::new(QUICK);
        motion.push(at(0.0, 0.0, 0.0));
        motion.advance_to(5);
        // Two trackers deliver out of order; the newer target must win.
        motion.push(at(9.0, 50.0, 0.0));
        motion.push(at(8.0, 20.0, 0.0));
        motion.advance_to(400);
        assert!((motion.position().unwrap().0 - 50.0).abs() < 0.5);
    }

    #[test]
    fn first_frame_and_long_gaps_start_settled() {
        let mut motion = Motion::new(MELLOW);
        // Samples from before the recording started: the latest position wins
        // without gliding, and the first frame is not faded in.
        motion.push(at(1000.0, 10.0, 10.0));
        motion.push(at(1500.0, 300.0, 40.0));
        motion.advance_to(5000);
        assert_eq!(motion.position(), Some((300.0, 40.0)));
        assert!(
            motion
                .taps((0.0, 0.0))
                .iter()
                .all(|t| *t == Some((300, 40)))
        );
        motion.push(at(5001.0, 600.0, 40.0));
        motion.advance_to(60_000);
        assert_eq!(motion.position(), Some((600.0, 40.0)));
    }

    #[test]
    fn taps_blur_motion_and_stay_sharp_at_rest() {
        let mut still = Motion::new(QUICK);
        still.push(at(0.0, 5.0, 5.0));
        still.advance_to(100);
        assert_eq!(still.taps((1.0, 1.0)), [Some((4, 4)); BLUR_SAMPLES]);

        let mut moving = Motion::new(QUICK);
        moving.push(at(0.0, 0.0, 0.0));
        moving.advance_to(0);
        moving.push(at(1.0, 400.0, 0.0));
        moving.advance_to(40);
        let taps = moving.taps((0.0, 0.0));
        assert!(taps.iter().all(Option::is_some));
        let xs: Vec<i64> = taps.iter().map(|t| t.unwrap().0).collect();
        assert!(xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");

        let mut appearing = Motion::new(QUICK);
        appearing.push(Sample::Gone { ms: 0.0 });
        appearing.advance_to(30);
        appearing.push(at(34.0, 0.0, 0.0));
        appearing.advance_to(38);
        let shown = appearing.taps((0.0, 0.0)).iter().flatten().count();
        assert!(shown > 0 && shown < BLUR_SAMPLES, "{shown}");
    }

    #[test]
    fn feed_lines_round_trip() {
        let line = feed_line(1, 29_630_209_157, Some((1930.5, -4.0)));
        assert_eq!(line, "1 29630209157 p 1930.5 -4\n");
        assert_eq!(
            parse_feed_line(&line),
            Some((1, 29_630_209_157, Some((1930.5, -4.0))))
        );
        assert_eq!(parse_feed_line(&feed_line(0, 7, None)), Some((0, 7, None)));
        for bad in ["", "0 7", "0 7 p 1", "x 7 l", "0 7 p nan 1", "0 7 e"] {
            assert_eq!(parse_feed_line(bad), None, "{bad}");
        }
    }

    fn config(origin: (f64, f64), width: f64) -> PluginConfig {
        PluginConfig {
            fd: 5,
            preset: "mellow".into(),
            size: 24.0,
            origin,
            width,
        }
    }

    #[test]
    fn plugin_config_round_trips_and_rejects_bad_values() {
        let config = config((1920.0, -1080.5), 1280.0);
        assert_eq!(
            config.to_env(),
            "fd=5 preset=mellow size=24 origin=1920,-1080.5 width=1280"
        );
        assert_eq!(PluginConfig::parse(&config.to_env()), Ok(config));
        for bad in [
            "",
            "fd=5 preset=mellow size=24 origin=0,0",
            "fd=-1 preset=mellow size=24 origin=0,0 width=10",
            "fd=5 preset=system size=24 origin=0,0 width=10",
            "fd=5 preset=quick size=2 origin=0,0 width=10",
            "fd=5 preset=quick size=24 origin=0 width=10",
            "fd=5 preset=quick size=24 origin=0,0 width=0",
            "fd=5 preset=quick size=24 origin=0,inf width=10",
        ] {
            assert!(PluginConfig::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mapping_covers_output_region_and_combined_at_any_scale() {
        // DP-1 at x=1920, scale 1.5: 1280 logical, 1920 video pixels.
        let output = config((1920.0, 0.0), 1280.0).mapping(1920);
        assert_eq!(output.apply(1920.0 + 100.0, 50.0), (150.0, 75.0));
        // Region 800x600 at (2000, 100) on that output, recorded at 1200 px.
        let region = config((2000.0, 100.0), 800.0).mapping(1200);
        assert_eq!(region.apply(2010.0, 120.0), (15.0, 30.0));
        // Combined: DP-3 (0..1920) and DP-1 each get every tracker's events.
        let left = config((0.0, 0.0), 1920.0).mapping(1920);
        let right = output;
        let mut sources = (Sources::default(), Sources::default());
        let on_left = (0, 1000, Some((1900.0, 10.0)));
        assert_eq!(
            sources.0.sample(on_left, &left),
            Some(at(1.0, 1900.0, 10.0))
        );
        // The right video sees it left of its edge and clips it.
        assert_eq!(
            sources.1.sample(on_left, &right),
            Some(at(1.0, -30.0, 15.0))
        );
        // Crossing: the right tracker sees it before the left one reports the leave.
        let crossing = (1, 2000, Some((1925.0, 10.0)));
        assert_eq!(
            sources.0.sample(crossing, &left),
            Some(at(2.0, 1925.0, 10.0))
        );
        assert_eq!(sources.0.sample((0, 2100, None), &left), None);
        assert_eq!(
            sources.0.sample((1, 3000, None), &left),
            Some(Sample::Gone { ms: 3.0 })
        );
    }

    #[test]
    fn arrow_is_antialiased_with_the_tip_as_hotspot() {
        let arrow = arrow(24.0);
        let alpha = |x: u32, y: u32| arrow.rgba[((y * arrow.width + x) * 4 + 3) as usize];
        let (tx, ty) = (arrow.hotspot.0 as u32, arrow.hotspot.1 as u32);
        // Opaque body just inside the tip, transparent far corner, soft edges.
        assert_eq!(alpha(tx + 2, ty + 6), 255);
        assert_eq!(alpha(arrow.width - 1, 0), 0);
        assert!(arrow.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        let body = ((ty + 6) * arrow.width + tx + 2) as usize * 4;
        assert!(arrow.rgba[body] < 40, "dark fill");
        assert!(super::arrow(48.0).height > arrow.height);
    }
}
