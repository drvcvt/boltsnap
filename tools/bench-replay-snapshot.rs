// Compile with rustc -O; see docs/recording-smoothing.md.
#![allow(dead_code)]
mod before {
    include!(env!("BOLTSNAP_BASELINE_RING"));
}
#[path = "../src/replay/ring.rs"]
mod after;
use std::{hint::black_box, sync::Arc, time::Instant};
fn before(seconds: i64, fps: i64) {
    let mut ring = before::Ring::new(seconds * 1_000_000, 1_000_000_000).unwrap();
    let payload = Arc::new([0u8; 512]);
    for n in 0..seconds * fps {
        ring.push(
            before::Entry {
                payload: payload.clone(),
                keyframe: n % fps == 0,
                start_us: n * 1_000_000 / fps,
                end_us: (n + 1) * 1_000_000 / fps,
                bytes: 700,
            },
            true,
        )
        .unwrap();
        if n % fps < 47 {
            ring.push(
                before::Entry {
                    payload: payload.clone(),
                    keyframe: false,
                    start_us: n * 1_000_000 / fps,
                    end_us: (n + 1) * 1_000_000 / fps,
                    bytes: 300,
                },
                false,
            )
            .unwrap();
        }
    }
    let mut times = Vec::new();
    for i in 0..105 {
        let start = Instant::now();
        let copy = ring.snapshot(|p| Ok::<_, String>(p.clone())).unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        black_box(&copy);
        if i >= 5 {
            times.push(elapsed);
        }
        drop(copy);
    }
    let mut append_times = Vec::new();
    for _ in 0..100 {
        let mut live = ring.snapshot(|p| Ok::<_, String>(p.clone())).unwrap();
        let n = seconds * fps;
        let packet = before::Entry {
            payload: payload.clone(),
            keyframe: false,
            start_us: n * 1_000_000 / fps,
            end_us: (n + 1) * 1_000_000 / fps,
            bytes: 700,
        };
        let start = Instant::now();
        live.push(packet, true).unwrap();
        append_times.push(start.elapsed().as_secs_f64() * 1e6);
        black_box(&live);
    }
    append_times.sort_by(f64::total_cmp);
    println!(
        "before first append after freeze median={:.2}us p95={:.2}us",
        append_times[50], append_times[94]
    );
    times.sort_by(f64::total_cmp);
    println!(
        "before {seconds}s {fps}fps median={:.2}us p95={:.2}us",
        times[50], times[94]
    );
}
fn after(seconds: i64, fps: i64) {
    let mut ring = after::Ring::new(seconds * 1_000_000, 1_000_000_000).unwrap();
    let payload = Arc::new([0u8; 512]);
    for n in 0..seconds * fps {
        ring.push(
            after::Entry {
                payload: payload.clone(),
                keyframe: n % fps == 0,
                start_us: n * 1_000_000 / fps,
                end_us: (n + 1) * 1_000_000 / fps,
                bytes: 700,
            },
            true,
        )
        .unwrap();
        if n % fps < 47 {
            ring.push(
                after::Entry {
                    payload: payload.clone(),
                    keyframe: false,
                    start_us: n * 1_000_000 / fps,
                    end_us: (n + 1) * 1_000_000 / fps,
                    bytes: 300,
                },
                false,
            )
            .unwrap();
        }
    }
    let mut times = Vec::new();
    for i in 0..105 {
        let start = Instant::now();
        let copy = ring.snapshot();
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        black_box(&copy);
        if i >= 5 {
            times.push(elapsed);
        }
        drop(copy);
    }
    let mut append_times = Vec::new();
    for _ in 0..100 {
        let mut live = ring.snapshot();
        let n = seconds * fps;
        let packet = after::Entry {
            payload: payload.clone(),
            keyframe: false,
            start_us: n * 1_000_000 / fps,
            end_us: (n + 1) * 1_000_000 / fps,
            bytes: 700,
        };
        let start = Instant::now();
        live.push(packet, true).unwrap();
        append_times.push(start.elapsed().as_secs_f64() * 1e6);
        black_box(&live);
    }
    append_times.sort_by(f64::total_cmp);
    println!(
        "after first append after freeze median={:.2}us p95={:.2}us",
        append_times[50], append_times[94]
    );
    times.sort_by(f64::total_cmp);
    println!(
        "after {seconds}s {fps}fps median={:.2}us p95={:.2}us",
        times[50], times[94]
    );
}
fn main() {
    for (s, f) in [(60, 60), (600, 240)] {
        before(s, f);
        after(s, f);
    }
}
