//! Export-time cursor rendering: recorded cursor tracks, spring smoothing and
//! the FFmpeg overlay script. Platform-neutral; capture lives in the OS backend.

use std::fmt::Write as _;
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

/// Cursor position for each output frame, or `None` while it is hidden. The
/// spring follows the latest raw sample; appearing snaps to the raw position.
pub fn smooth(
    samples: &[Sample],
    fps: u32,
    frames: usize,
    preset: Preset,
) -> Vec<Option<(f64, f64)>> {
    const STEP: f64 = 0.001;
    let mut spring = Spring::default();
    let mut result = Vec::with_capacity(frames);
    let mut next = 0;
    let mut step = 0u64;
    for frame in 0..frames {
        let frame_ms = frame as f64 * 1000.0 / f64::from(fps);
        // Integer step counting keeps the 1 ms simulation grid drift-free.
        while (step as f64) < frame_ms {
            while next < samples.len() && samples[next].ms() <= step as f64 {
                spring.apply(samples[next]);
                next += 1;
            }
            spring.advance(preset, STEP);
            step += 1;
        }
        while next < samples.len() && samples[next].ms() <= frame_ms {
            spring.apply(samples[next]);
            next += 1;
        }
        result.push(spring.target.map(|_| spring.position));
    }
    result
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

/// FFmpeg `sendcmd` script moving `overlay@cursor` to each frame's position.
/// `hotspot` is subtracted so the image's hotspot lands on the position.
pub fn overlay_script(positions: &[Option<(f64, f64)>], fps: u32, hotspot: (f64, f64)) -> String {
    let mut script = String::new();
    let mut last = None;
    for (frame, position) in positions.iter().enumerate() {
        // Hidden cursors move far outside the frame instead of toggling inputs.
        let (x, y) = position
            .map(|(x, y)| {
                (
                    (x - hotspot.0).round() as i64,
                    (y - hotspot.1).round() as i64,
                )
            })
            .unwrap_or((-100_000, -100_000));
        if last == Some((x, y)) {
            continue;
        }
        last = Some((x, y));
        let seconds = frame as f64 / f64::from(fps);
        let _ = writeln!(
            script,
            "{seconds:.6} overlay@cursor x {x}, overlay@cursor y {y};"
        );
    }
    script
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
    fn overlay_script_emits_changes_only_and_hides_off_frame() {
        let script = overlay_script(
            &[
                Some((10.0, 20.0)),
                Some((10.2, 20.2)),
                None,
                Some((4.0, 4.0)),
            ],
            4,
            (2.0, 1.0),
        );
        assert_eq!(
            script,
            "0.000000 overlay@cursor x 8, overlay@cursor y 19;\n\
             0.500000 overlay@cursor x -100000, overlay@cursor y -100000;\n\
             0.750000 overlay@cursor x 2, overlay@cursor y 3;\n"
        );
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
