//! Export-time cursor rendering: recorded cursor tracks, spring smoothing and
//! the FFmpeg overlay script. Platform-neutral; capture lives in the OS backend.

use std::path::{Path, PathBuf};

/// Sidecars beside a clip `X.mp4`: the cursor-free video `X.clean.mp4`.
pub fn clean_path(clip: &Path) -> PathBuf {
    clip.with_extension("clean.mp4")
}

/// Sidecars beside a clip `X.mp4`: the raw cursor track `X.cursor.json`.
pub fn sidecar_path(clip: &Path) -> PathBuf {
    clip.with_extension("cursor.json")
}

/// Spring parameters in Screen Studio's terms. Both presets are critically
/// damped, so the cursor settles without overshoot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub tension: f64,
    pub friction: f64,
    pub mass: f64,
}

pub const MELLOW: Preset = Preset {
    tension: 170.0,
    friction: 26.0,
    mass: 1.0,
};
pub const QUICK: Preset = Preset {
    tension: 600.0,
    friction: 49.0,
    mass: 1.0,
};

/// One recorded observation on a clip timeline, in clip pixels.
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

/// Raw per-segment track as written during capture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    /// Logical compositor origin of the output the positions are relative to.
    pub output: (f64, f64),
    pub events: Vec<Event>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Enter { us: u64 },
    Leave { us: u64 },
    Position { us: u64, x: f64, y: f64 },
}

pub const TRACK_HEADER: &str = "boltsnap-cursor 1";

/// Track file: a header line, `output X Y`, then `US e`, `US l` or `US p X Y`
/// lines with monotonic microseconds and output-local logical coordinates.
pub fn parse_track(text: &str) -> Result<Track, String> {
    let mut lines = text.lines();
    if lines.next() != Some(TRACK_HEADER) {
        return Err("not a boltsnap cursor track".into());
    }
    let mut track = Track::default();
    for (index, line) in lines.enumerate() {
        let fields: Vec<&str> = line.split_ascii_whitespace().collect();
        let bad = || format!("invalid cursor track line {}", index + 2);
        let number = |s: &str| s.parse::<f64>().ok().filter(|n| n.is_finite());
        match fields.as_slice() {
            ["output", x, y] => {
                track.output = (number(x).ok_or_else(bad)?, number(y).ok_or_else(bad)?)
            }
            [us, kind, rest @ ..] => {
                let us = us.parse::<u64>().map_err(|_| bad())?;
                track.events.push(match (*kind, rest) {
                    ("e", []) => Event::Enter { us },
                    ("l", []) => Event::Leave { us },
                    ("p", [x, y]) => Event::Position {
                        us,
                        x: number(x).ok_or_else(bad)?,
                        y: number(y).ok_or_else(bad)?,
                    },
                    _ => return Err(bad()),
                });
            }
            // A capture interrupted mid-write leaves a partial last line.
            [] | [_] => {}
        }
    }
    Ok(track)
}

/// Maps compositor-global logical coordinates to clip pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapping {
    /// Global logical origin of the clip's top-left pixel.
    pub origin: (f64, f64),
    /// Clip pixels per logical unit.
    pub scale: f64,
}

/// One recorded segment placed on the clip timeline.
pub struct Placed<'a> {
    pub track: &'a Track,
    /// Monotonic microseconds of the segment's first video frame.
    pub first_frame_us: u64,
    /// Start of the segment on the clip timeline.
    pub offset_ms: f64,
    pub duration_ms: f64,
    pub mapping: Mapping,
}

/// Concatenate segment tracks into clip-timeline samples, ordered by time. The
/// cursor is gone at every segment start until the track proves otherwise.
pub fn timeline(segments: &[Placed]) -> Vec<Sample> {
    let mut samples = Vec::new();
    for segment in segments {
        let end = segment.offset_ms + segment.duration_ms;
        let clip_ms =
            |us: u64| segment.offset_ms + (us as f64 - segment.first_frame_us as f64) / 1000.0;
        let (ox, oy) = segment.track.output;
        let Mapping { origin, scale } = segment.mapping;
        let mut last: Option<Sample> = None;
        let mut started = false;
        for event in &segment.track.events {
            // Visibility follows positions; enter carries no coordinates.
            let sample = match *event {
                Event::Enter { .. } => continue,
                Event::Leave { us } => Sample::Gone { ms: clip_ms(us) },
                Event::Position { us, x, y } => Sample::At {
                    ms: clip_ms(us),
                    x: (ox + x - origin.0) * scale,
                    y: (oy + y - origin.1) * scale,
                },
            };
            let ms = sample.ms();
            if ms < segment.offset_ms {
                // Before the first frame: remember the state at the start.
                last = Some(sample);
                continue;
            }
            if ms >= end {
                break;
            }
            if !started {
                started = true;
                samples.push(at_start(last, segment.offset_ms));
            }
            samples.push(sample);
        }
        if !started {
            samples.push(at_start(last, segment.offset_ms));
        }
    }
    samples.sort_by(|a, b| a.ms().total_cmp(&b.ms()));
    samples
}

/// State at a segment start: the last position seen before its first frame.
fn at_start(last: Option<Sample>, ms: f64) -> Sample {
    match last {
        Some(Sample::At { x, y, .. }) => Sample::At { ms, x, y },
        _ => Sample::Gone { ms },
    }
}

/// Merge the timelines of several outputs recorded together. The cursor is on
/// one output at a time; a `Gone` from one output is dropped while another
/// output still shows it.
pub fn merge(timelines: &[Vec<Sample>]) -> Vec<Sample> {
    let mut all: Vec<(usize, Sample)> = timelines
        .iter()
        .enumerate()
        .flat_map(|(source, samples)| samples.iter().map(move |s| (source, *s)))
        .collect();
    all.sort_by(|a, b| a.1.ms().total_cmp(&b.1.ms()));
    let mut visible = vec![false; timelines.len()];
    let mut merged = Vec::with_capacity(all.len());
    for (source, sample) in all {
        visible[source] = matches!(sample, Sample::At { .. });
        match sample {
            Sample::Gone { .. } if visible.iter().any(|v| *v) => {}
            _ => merged.push(sample),
        }
    }
    merged
}

/// Spring-smoothed cursor position per millisecond from 0 to `end_ms`, or `None`
/// while hidden. The spring follows the latest raw sample; appearing snaps to
/// the raw position. Samples at a millisecond apply before it is recorded.
pub fn trajectory(samples: &[Sample], end_ms: usize, preset: Preset) -> Vec<Option<(f64, f64)>> {
    let mut spring = Spring::default();
    let mut result = Vec::with_capacity(end_ms + 1);
    let mut next = 0;
    for ms in 0..=end_ms {
        while next < samples.len() && samples[next].ms() <= ms as f64 {
            spring.apply(samples[next]);
            next += 1;
        }
        result.push(spring.target.map(|_| spring.position));
        spring.advance(preset, 0.001);
    }
    result
}

/// Cursor position for each output frame, or `None` while it is hidden.
#[cfg(test)]
pub fn smooth(
    samples: &[Sample],
    fps: u32,
    frames: usize,
    preset: Preset,
) -> Vec<Option<(f64, f64)>> {
    let path = trajectory(samples, frame_ms(frames, fps), preset);
    (0..frames)
        .map(|frame| path[frame_ms(frame, fps)])
        .collect()
}

fn frame_ms(frame: usize, fps: u32) -> usize {
    (frame as u64 * 1000 / u64::from(fps.max(1))) as usize
}

/// Exposure per frame for motion blur: a fixed 1/120 s, so the trail looks the
/// same at 60 and 240 FPS. Longer than a 240 FPS frame, so exposures overlap.
pub const SHUTTER_MS: usize = 8;
/// Sub-positions averaged per frame.
pub const BLUR_SAMPLES: usize = 8;

/// Cursor positions within frame `frame`'s exposure, ending at the frame time,
/// as distinct pixel positions with their share of `BLUR_SAMPLES`. Hidden
/// moments contribute nothing, so appearing and leaving fade over the exposure.
pub fn exposure(
    path: &[Option<(f64, f64)>],
    frame: usize,
    fps: u32,
    offset: (f64, f64),
) -> Vec<((i64, i64), u32)> {
    let end = frame_ms(frame, fps).min(path.len().saturating_sub(1));
    let mut taps: Vec<((i64, i64), u32)> = Vec::new();
    for tap in 0..BLUR_SAMPLES {
        let back = SHUTTER_MS * (BLUR_SAMPLES - 1 - tap) / (BLUR_SAMPLES - 1);
        let Some((x, y)) = path[end.saturating_sub(back)] else {
            continue;
        };
        let at = ((x - offset.0).round() as i64, (y - offset.1).round() as i64);
        match taps.iter_mut().find(|(p, _)| *p == at) {
            Some((_, weight)) => *weight += 1,
            None => taps.push((at, 1)),
        }
    }
    taps
}

#[derive(Default)]
struct Spring {
    target: Option<(f64, f64)>,
    position: (f64, f64),
    velocity: (f64, f64),
}

impl Spring {
    fn apply(&mut self, sample: Sample) {
        match sample {
            Sample::At { x, y, .. } => {
                if self.target.is_none() {
                    self.position = (x, y);
                    self.velocity = (0.0, 0.0);
                }
                self.target = Some((x, y));
            }
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
}

/// Cursor image converted once to the video's YUV space.
pub struct Sprite {
    width: usize,
    height: usize,
    /// Per pixel: luma, Cb, Cr and alpha (0-255).
    pixels: Vec<[f32; 4]>,
}

impl Sprite {
    /// `rgba` is straight alpha. `bt709` picks the matrix (else BT.601);
    /// `full` picks full instead of limited range.
    pub fn new(rgba: &[u8], width: usize, height: usize, bt709: bool, full: bool) -> Self {
        let (kr, kb) = if bt709 {
            (0.2126, 0.0722)
        } else {
            (0.299, 0.114)
        };
        let (y_scale, y_offset, c_scale) = if full {
            (255.0, 0.0, 255.0)
        } else {
            (219.0, 16.0, 224.0)
        };
        let pixels = rgba
            .chunks_exact(4)
            .take(width * height)
            .map(|p| {
                let [r, g, b] = [p[0], p[1], p[2]].map(|c| f32::from(c) / 255.0);
                let y = kr * r + (1.0 - kr - kb) * g + kb * b;
                let cb = (b - y) / (2.0 * (1.0 - kb));
                let cr = (r - y) / (2.0 * (1.0 - kr));
                [
                    y_offset + y_scale * y,
                    128.0 + c_scale * cb,
                    128.0 + c_scale * cr,
                    f32::from(p[3]),
                ]
            })
            .collect();
        Self {
            width,
            height,
            pixels,
        }
    }

    fn at(&self, x: i64, y: i64) -> Option<[f32; 4]> {
        (x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height)
            .then(|| self.pixels[y as usize * self.width + x as usize])
    }
}

/// Bytes of one yuv420p frame.
pub fn yuv420_len(width: usize, height: usize) -> usize {
    width * height + 2 * width.div_ceil(2) * height.div_ceil(2)
}

/// Alpha-blend `sprite` with its top-left at (`left`, `top`) into a yuv420p
/// frame, clipped to the frame. Chroma averages the covered 2x2 luma block.
#[cfg(test)]
pub fn blend_yuv420(
    frame: &mut [u8],
    width: usize,
    height: usize,
    sprite: &Sprite,
    left: i64,
    top: i64,
) {
    blend_exposure(frame, width, height, sprite, &[((left, top), 1)], 1);
}

/// Blend the average of `sprite` placed at each tap (top-left, weight) over
/// `total` weights: motion blur when the taps differ, a plain blend when they
/// coincide.
pub fn blend_exposure(
    frame: &mut [u8],
    width: usize,
    height: usize,
    sprite: &Sprite,
    taps: &[((i64, i64), u32)],
    total: u32,
) {
    let (Some(min_x), Some(min_y)) = (
        taps.iter().map(|((x, _), _)| *x).min(),
        taps.iter().map(|((_, y), _)| *y).min(),
    ) else {
        return;
    };
    let max_x = taps.iter().map(|((x, _), _)| *x).max().unwrap_or(min_x);
    let max_y = taps.iter().map(|((_, y), _)| *y).max().unwrap_or(min_y);
    let (w, h) = (width as i64, height as i64);
    let x0 = min_x.max(0);
    let y0 = min_y.max(0);
    let x1 = (max_x + sprite.width as i64).min(w);
    let y1 = (max_y + sprite.height as i64).min(h);
    if x0 >= x1 || y0 >= y1 || total == 0 {
        return;
    }
    let mix = |dst: u8, src: f32, alpha: f32| {
        (src * alpha + f32::from(dst) * (1.0 - alpha))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    // Coverage-weighted colour of the averaged cursor over one or more pixels.
    let sample = |points: &[(i64, i64)], plane: usize| {
        let (mut coverage, mut value) = (0.0f32, 0.0f32);
        for &((left, top), weight) in taps {
            for &(x, y) in points {
                if let Some(pixel) = sprite.at(x - left, y - top) {
                    let a = pixel[3] * weight as f32;
                    coverage += a;
                    value += pixel[plane] * a;
                }
            }
        }
        (coverage, value)
    };
    let norm = 255.0 * total as f32;
    for y in y0..y1 {
        for x in x0..x1 {
            let (coverage, luma) = sample(&[(x, y)], 0);
            if coverage > 0.0 {
                let i = (y * w + x) as usize;
                frame[i] = mix(frame[i], luma / coverage, coverage / norm);
            }
        }
    }
    let (cw, ch) = (width.div_ceil(2) as i64, height.div_ceil(2) as i64);
    let (u_plane, v_plane) = (width * height, width * height + (cw * ch) as usize);
    for cy in y0 / 2..=(y1 - 1) / 2 {
        for cx in x0 / 2..=(x1 - 1) / 2 {
            if cx >= cw || cy >= ch {
                continue;
            }
            let block = [
                (cx * 2, cy * 2),
                (cx * 2 + 1, cy * 2),
                (cx * 2, cy * 2 + 1),
                (cx * 2 + 1, cy * 2 + 1),
            ];
            let (coverage, cb) = sample(&block, 1);
            if coverage > 0.0 {
                let (_, cr) = sample(&block, 2);
                let i = (cy * cw + cx) as usize;
                let alpha = coverage / (4.0 * norm);
                frame[u_plane + i] = mix(frame[u_plane + i], cb / coverage, alpha);
                frame[v_plane + i] = mix(frame[v_plane + i], cr / coverage, alpha);
            }
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

/// Raw samples as the `boltsnap.cursor` v1 JSON sidecar read by Eddy.
pub fn sidecar_json(
    samples: &[Sample],
    size: (u32, u32),
    clean_video: Option<&str>,
    image: Option<(&str, (f64, f64), f64)>,
    preset: &str,
) -> serde_json::Value {
    let id = image.map(|_| "arrow");
    let samples: Vec<serde_json::Value> = samples
        .iter()
        .map(|sample| match *sample {
            Sample::At { ms, x, y } => match id {
                Some(id) => serde_json::json!([round3(ms), round3(x), round3(y), id]),
                None => serde_json::json!([round3(ms), round3(x), round3(y)]),
            },
            Sample::Gone { ms } => serde_json::json!([round3(ms), null, null]),
        })
        .collect();
    let mut value = serde_json::json!({
        "format": "boltsnap.cursor",
        "version": 1,
        "width": size.0,
        "height": size.1,
        "cursor_in_video": true,
        "samples": samples,
        "render": {"preset": preset},
    });
    if let Some(clean) = clean_video {
        value["clean_video"] = clean.into();
    }
    if let Some((png, hotspot, scale)) = image {
        value["images"] = serde_json::json!({
            "arrow": {"png": png, "hotspot": [hotspot.0, hotspot.1], "scale": scale}
        });
    }
    value
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(text: &str) -> Track {
        parse_track(&format!("{TRACK_HEADER}\noutput 1920 0\n{text}")).unwrap()
    }

    #[test]
    fn track_parses_events_and_tolerates_a_truncated_last_line() {
        let parsed = track("1000 e\n2000 p 10 20.5\n3000 l\n4000");
        assert_eq!(parsed.output, (1920.0, 0.0));
        assert_eq!(
            parsed.events,
            [
                Event::Enter { us: 1000 },
                Event::Position {
                    us: 2000,
                    x: 10.0,
                    y: 20.5
                },
                Event::Leave { us: 3000 },
            ]
        );
        assert!(parse_track("nope").is_err());
        assert!(parse_track(&format!("{TRACK_HEADER}\n5 p x 1")).is_err());
    }

    #[test]
    fn timeline_maps_segments_to_clip_time_and_pixels() {
        let first = track("500 e\n900 p 5 5\n2000 p 10 20\n9000 p 99 99");
        let second = track("100000 p 30 40\n101000 l");
        let mapping = Mapping {
            origin: (1920.0, 0.0),
            scale: 2.0,
        };
        let samples = timeline(&[
            Placed {
                track: &first,
                first_frame_us: 1000,
                offset_ms: 0.0,
                duration_ms: 5.0,
                mapping,
            },
            Placed {
                track: &second,
                first_frame_us: 100_000,
                offset_ms: 5.0,
                duration_ms: 10.0,
                mapping,
            },
        ]);
        assert_eq!(
            samples,
            [
                // The position before the first frame carries into the clip start.
                Sample::At {
                    ms: 0.0,
                    x: 10.0,
                    y: 10.0
                },
                Sample::At {
                    ms: 1.0,
                    x: 20.0,
                    y: 40.0
                },
                // Nothing known yet at the second segment's start.
                Sample::Gone { ms: 5.0 },
                Sample::At {
                    ms: 5.0,
                    x: 60.0,
                    y: 80.0
                },
                Sample::Gone { ms: 6.0 },
            ]
        );
    }

    #[test]
    fn merge_keeps_the_cursor_visible_while_it_crosses_outputs() {
        let left = vec![
            Sample::At {
                ms: 0.0,
                x: 1.0,
                y: 1.0,
            },
            Sample::Gone { ms: 2.0 },
        ];
        let right = vec![
            Sample::Gone { ms: 0.0 },
            Sample::At {
                ms: 1.5,
                x: 9.0,
                y: 1.0,
            },
        ];
        assert_eq!(
            merge(&[left, right]),
            [
                Sample::At {
                    ms: 0.0,
                    x: 1.0,
                    y: 1.0
                },
                Sample::At {
                    ms: 1.5,
                    x: 9.0,
                    y: 1.0
                },
            ]
        );
    }

    #[test]
    fn spring_settles_without_overshoot_and_quick_is_faster() {
        let samples = [
            Sample::At {
                ms: 0.0,
                x: 0.0,
                y: 0.0,
            },
            Sample::At {
                ms: 10.0,
                x: 100.0,
                y: 0.0,
            },
        ];
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
    fn appearing_snaps_and_leaving_hides_immediately() {
        let samples = [
            Sample::Gone { ms: 0.0 },
            Sample::At {
                ms: 100.0,
                x: 500.0,
                y: 300.0,
            },
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
    fn sprite_uses_the_bt709_limited_range_matrix() {
        let sprite = Sprite::new(
            &[255, 255, 255, 255, 0, 0, 0, 128, 255, 0, 0, 255],
            3,
            1,
            true,
            false,
        );
        let [y, cb, cr, a] = sprite.pixels[0];
        assert_eq!(
            (y.round(), cb.round(), cr.round(), a),
            (235.0, 128.0, 128.0, 255.0)
        );
        assert_eq!(sprite.pixels[1][0].round(), 16.0);
        assert_eq!(sprite.pixels[1][3], 128.0);
        // BT.709 red: Y 63, Cb 102, Cr 240 in limited range.
        let red = sprite.pixels[2].map(f32::round);
        assert_eq!(&red[..3], &[63.0, 102.0, 240.0]);
    }

    #[test]
    fn blend_writes_luma_and_chroma_and_clips_at_edges() {
        let (w, h) = (6, 4);
        let mut frame = vec![0u8; yuv420_len(w, h)];
        frame[w * h..].fill(128);
        // 2x2 opaque white sprite at (1, 1): covers luma (1..3, 1..3), which
        // touches chroma pixels (0, 0), (1, 0), (0, 1), (1, 1).
        let sprite = Sprite::new(&[255; 16], 2, 2, true, false);
        blend_yuv420(&mut frame, w, h, &sprite, 1, 1);
        let luma = |x: usize, y: usize| frame[y * w + x];
        assert_eq!(
            (luma(0, 0), luma(1, 1), luma(2, 2), luma(3, 2)),
            (0, 235, 235, 0)
        );
        // White keeps chroma neutral.
        assert!(frame[w * h..].iter().all(|c| *c == 128));
        let mut edge = vec![16u8; yuv420_len(w, h)];
        blend_yuv420(&mut edge, w, h, &sprite, 5, 3);
        assert_eq!(edge[3 * w + 5], 235);
        blend_yuv420(&mut edge, w, h, &sprite, -9, 2);
        blend_yuv420(&mut edge, w, h, &sprite, 6, 0);
        assert_eq!(edge.iter().filter(|v| **v == 235).count(), 1);
    }

    #[test]
    fn blend_mixes_partial_alpha_and_averages_chroma_coverage() {
        let (w, h) = (2, 2);
        let mut frame = vec![0u8; yuv420_len(w, h)];
        frame[4..].fill(128);
        // One half-transparent pure red pixel in the 2x2 block.
        let sprite = Sprite::new(&[255, 0, 0, 128], 1, 1, true, false);
        blend_yuv420(&mut frame, w, h, &sprite, 0, 0);
        let red_luma = 16.0f32 + 219.0 * 0.2126;
        assert_eq!(frame[0], (red_luma * 128.0 / 255.0).round() as u8);
        // Coverage is a quarter of half alpha: Cr moves an eighth toward 240.
        assert_eq!(frame[5], (240.0f32 * 0.125 + 128.0 * 0.875).round() as u8);
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

    #[test]
    fn exposure_blurs_motion_and_stays_sharp_at_rest() {
        let moving: Vec<_> = (0..40).map(|ms| Some((ms as f64, 0.0))).collect();
        let taps = exposure(&moving, 8, 240, (0.0, 0.0));
        assert_eq!(
            taps.iter().map(|(_, w)| w).sum::<u32>(),
            BLUR_SAMPLES as u32
        );
        assert!(taps.len() > 4, "{taps:?}");
        let still = vec![Some((5.0, 5.0)); 40];
        assert_eq!(
            exposure(&still, 8, 240, (1.0, 1.0)),
            [((4, 4), BLUR_SAMPLES as u32)]
        );
        let appearing: Vec<_> = (0..40).map(|ms| (ms >= 30).then_some((0.0, 0.0))).collect();
        let partial: u32 = exposure(&appearing, 8, 240, (0.0, 0.0))
            .iter()
            .map(|(_, w)| w)
            .sum();
        assert!(partial > 0 && partial < BLUR_SAMPLES as u32);
    }

    #[test]
    fn blurred_blend_spreads_coverage_over_the_path() {
        let (w, h) = (8, 2);
        let mut frame = vec![0u8; yuv420_len(w, h)];
        frame[w * h..].fill(128);
        let sprite = Sprite::new(&[255; 4], 1, 1, true, false);
        // Half the exposure at x=0, half at x=4: each gets half coverage.
        blend_exposure(&mut frame, w, h, &sprite, &[((0, 0), 1), ((4, 0), 1)], 2);
        assert_eq!(frame[0], (235.0f32 / 2.0).round() as u8);
        assert_eq!(frame[4], (235.0f32 / 2.0).round() as u8);
        assert_eq!(frame[2], 0);
    }

    #[test]
    fn sidecar_matches_the_v1_contract() {
        let value = sidecar_json(
            &[
                Sample::At {
                    ms: 1.23456,
                    x: 2.0,
                    y: 3.0,
                },
                Sample::Gone { ms: 4.0 },
            ],
            (1920, 1080),
            Some("clip.clean.mp4"),
            Some(("UE5H", (1.0, 2.0), 1.0)),
            "mellow",
        );
        assert_eq!(value["format"], "boltsnap.cursor");
        assert_eq!(value["version"], 1);
        assert_eq!(value["width"], 1920);
        assert_eq!(value["clean_video"], "clip.clean.mp4");
        assert_eq!(
            value["samples"][0],
            serde_json::json!([1.235, 2.0, 3.0, "arrow"])
        );
        assert_eq!(value["samples"][1], serde_json::json!([4.0, null, null]));
        assert_eq!(
            value["images"]["arrow"]["hotspot"],
            serde_json::json!([1.0, 2.0])
        );
        assert_eq!(value["render"]["preset"], "mellow");
        assert!(value.get("clicks").is_none());
    }
}
#[cfg(test)]
#[test]
#[ignore = "writes a preview PNG"]
fn arrow_preview() {
    let mut sheet = image::RgbaImage::from_pixel(220, 90, image::Rgba([90, 110, 140, 255]));
    let mut x = 10;
    for size in [24.0, 36.0, 60.0] {
        let a = arrow(size);
        let img = image::RgbaImage::from_raw(a.width, a.height, a.rgba).unwrap();
        image::imageops::overlay(&mut sheet, &img, x, 10);
        x += i64::from(a.width) + 20;
    }
    sheet.save("/tmp/bs-arrow-preview.png").unwrap();
}
