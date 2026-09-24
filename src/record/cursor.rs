//! Recorded cursor tracks and the `X.cursor.json` sidecar. Platform-neutral;
//! capture lives in the OS backend, drawing in the gsr plugin.

pub use super::cursor_motion::{Mapping, Sample};
use std::path::{Path, PathBuf};

/// Sidecar beside a clip `X.mp4`: the raw cursor track `X.cursor.json`.
pub fn sidecar_path(clip: &Path) -> PathBuf {
    clip.with_extension("cursor.json")
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
        let mut last: Option<Sample> = None;
        let mut started = false;
        for event in &segment.track.events {
            // Visibility follows positions; enter carries no coordinates.
            let sample = match *event {
                Event::Enter { .. } => continue,
                Event::Leave { us } => Sample::Gone { ms: clip_ms(us) },
                Event::Position { us, x, y } => {
                    let (x, y) = segment.mapping.apply(ox + x, oy + y);
                    Sample::At {
                        ms: clip_ms(us),
                        x,
                        y,
                    }
                }
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

/// Samples closer than this after the previous kept one are left out of the
/// sidecar: 120 per second is plenty for editors and keeps hour-long tracks small.
pub const SIDECAR_MIN_GAP_MS: f64 = 1000.0 / 120.0;

/// Stream raw samples as the `boltsnap.cursor` v1 JSON sidecar read by Eddy,
/// one sample at a time, so long recordings need no large buffer. Positions
/// are thinned to `SIDECAR_MIN_GAP_MS`; leaves and the first position after
/// one are always kept, as is the last sample.
pub fn write_sidecar(
    out: &mut impl std::io::Write,
    samples: &[Sample],
    size: (u32, u32),
    image: Option<(&str, (f64, f64), f64)>,
    preset: &str,
) -> std::io::Result<()> {
    let id = image.map(|_| "arrow");
    let mut head = serde_json::json!({
        "format": "boltsnap.cursor",
        "version": 1,
        "width": size.0,
        "height": size.1,
        "cursor_in_video": true,
        "render": {"preset": preset},
    });
    if let Some((png, hotspot, scale)) = image {
        head["images"] = serde_json::json!({
            "arrow": {"png": png, "hotspot": [hotspot.0, hotspot.1], "scale": scale}
        });
    }
    let head = head.to_string();
    out.write_all(&head.as_bytes()[..head.len() - 1])?;
    out.write_all(b",\"samples\":[")?;
    let mut kept: Option<Sample> = None;
    for (index, sample) in samples.iter().enumerate() {
        let keep = match (kept, *sample) {
            (Some(Sample::At { ms: last, .. }), Sample::At { ms, .. }) => {
                // Half a millisecond of slack: two 240 Hz steps are a hair
                // under the gap in floating point.
                ms - last >= SIDECAR_MIN_GAP_MS - 0.5 || index + 1 == samples.len()
            }
            _ => true,
        };
        if !keep {
            continue;
        }
        if kept.is_some() {
            out.write_all(b",")?;
        }
        let value = match *sample {
            Sample::At { ms, x, y } => match id {
                Some(id) => serde_json::json!([round3(ms), round3(x), round3(y), id]),
                None => serde_json::json!([round3(ms), round3(x), round3(y)]),
            },
            Sample::Gone { ms } => serde_json::json!([round3(ms), null, null]),
        };
        serde_json::to_writer(&mut *out, &value)?;
        kept = Some(*sample);
    }
    out.write_all(b"]}")
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

    fn sidecar(samples: &[Sample]) -> serde_json::Value {
        let mut out = Vec::new();
        write_sidecar(
            &mut out,
            samples,
            (1920, 1080),
            Some(("UE5H", (1.0, 2.0), 1.0)),
            "mellow",
        )
        .unwrap();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn sidecar_matches_the_v1_contract() {
        let value = sidecar(&[
            Sample::At {
                ms: 1.23456,
                x: 2.0,
                y: 3.0,
            },
            Sample::Gone { ms: 4.0 },
        ]);
        assert_eq!(value["format"], "boltsnap.cursor");
        assert_eq!(value["version"], 1);
        assert_eq!(value["width"], 1920);
        assert!(value.get("clean_video").is_none());
        assert_eq!(value["cursor_in_video"], true);
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

    #[test]
    fn sidecar_thins_positions_to_120_hz_but_keeps_leaves() {
        let at = |ms: f64| Sample::At { ms, x: ms, y: 0.0 };
        // 240 Hz for one second, a leave, a return, and a final position.
        let mut samples: Vec<Sample> = (0..240).map(|i| at(i as f64 * 1000.0 / 240.0)).collect();
        samples.push(Sample::Gone { ms: 1001.0 });
        samples.push(at(1002.0));
        samples.push(at(1003.0));
        let value = sidecar(&samples);
        let kept = value["samples"].as_array().unwrap();
        assert!((118..=124).contains(&kept.len()), "{}", kept.len());
        let times: Vec<f64> = kept.iter().map(|s| s[0].as_f64().unwrap()).collect();
        assert!(times.windows(2).all(|w| w[1] > w[0]));
        assert!(
            kept.iter().any(|s| s[0] == 1001.0 && s[1].is_null()),
            "leave kept"
        );
        assert!(times.contains(&1002.0), "return kept");
        assert_eq!(times.last(), Some(&1003.0), "last kept");
    }
}
