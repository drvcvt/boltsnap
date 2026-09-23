#[cfg(any(feature = "native-cursor", test))]
#[path = "../../../../../record/cursor.rs"]
mod cursor_motion;
mod export;
mod media;
#[cfg(feature = "native-cursor")]
mod native;
#[allow(dead_code)]
#[path = "../../../../../replay/mod.rs"]
mod replay;
mod ring;

use std::path::PathBuf;
use std::time::Instant;

fn run() -> Result<(), String> {
    ffmpeg_next::init().map_err(|e| format!("initialize FFmpeg: {e}"))?;
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Warning);
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|s| s.to_str()) {
        #[cfg(feature = "native-cursor")]
        Some(command @ ("cursor-fixture" | "cursor-probe" | "cursor-record" | "cursor-record-fixture")) => native::run(command, args.collect()),
        Some("check-encoder") => {
            let encoder = args.next().and_then(|s| s.into_string().ok()).ok_or("missing encoder")?;
            if args.next().is_some() { return Err("unexpected encoder probe option".into()); }
            crop::check_encoder(&encoder)
        },
        Some("live") => {
            let name = args.next().and_then(|s| s.into_string().ok()).ok_or("live requires socket name")?;
            let directory = PathBuf::from(args.next().ok_or("live requires output directory")?);
            let seconds: u64 = args.next().and_then(|s| s.to_str().and_then(|s| s.parse().ok())).ok_or("live requires seconds")?;
            let memory: usize = args.next().and_then(|s| s.to_str().and_then(|s| s.parse().ok())).ok_or("live requires memory MiB")?;
            let encoder = args.next().and_then(|s| s.into_string().ok()).ok_or("live requires crop encoder")?;
            if args.next().is_some() || !(1..=600).contains(&seconds) || !(64..=4096).contains(&memory) {
                return Err("invalid live options".into());
            }
            if !directory.is_dir() { return Err("live output directory is missing".into()); }
            live::run(&name, directory, seconds, memory * 1024 * 1024, encoder)
        },
        Some("capabilities") if args.next().is_none() => {
            println!("{}", media::capabilities());
            Ok(())
        },
        Some("probe") => {
            let mut input = None;
            let mut output = None;
            let defaults = crate::replay::settings::Settings::default();
            let mut seconds = defaults.seconds;
            let mut memory_mib = defaults.memory_mib;
            let mut closed_gop = false;
            while let Some(arg) = args.next() {
                match arg.to_str() {
                    Some("--closed-gop") => closed_gop = true,
                    Some("--input") => input = Some(PathBuf::from(args.next().ok_or("--input requires a path")?)),
                    Some("--output") => output = Some(PathBuf::from(args.next().ok_or("--output requires a path")?)),
                    Some("--seconds") => seconds = args.next().and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
                        .ok_or("--seconds requires an integer")?,
                    Some("--memory-mib") => memory_mib = args.next().and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
                        .ok_or("--memory-mib requires an integer")?,
                    _ => return Err(format!("unknown probe option: {}", arg.to_string_lossy())),
                }
            }
            if !closed_gop { return Err("probe requires --closed-gop for a known closed-GOP, non-reordered test stream".into()); }
            if !(1..=600).contains(&seconds) || !(1..=4096).contains(&memory_mib) {
                return Err("seconds must be 1..600 and memory-mib must be 1..4096".into());
            }
            let input = input.ok_or("--input is required")?;
            let output = output.ok_or("--output is required")?;
            let budget = memory_mib.checked_mul(1024 * 1024).ok_or("memory budget overflow")?;
            let started = Instant::now();
            let recording = media::ingest(&input, (seconds * 1_000_000) as i64, budget)?;
            let ingest_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (start, end) = recording.ring.bounds()?;
            let saved_bytes = export::remux(&recording, &output, 2 * budget as u64)?;
            println!("{}", serde_json::json!({"mode": "probe", "start_us": start, "end_us": end,
                "duration_us": end - start, "accounted_bytes": recording.ring.bytes(),
                "peak_accounted_bytes": recording.ring.peak_bytes(), "budget_bytes": budget,
                "evicted_gops": recording.ring.evicted_gops(), "packets_read": recording.packets_read,
                "ingest_ms": ingest_ms, "output_bytes": saved_bytes, "output": output.to_string_lossy()}));
            Ok(())
        },
        _ => Err("usage: boltsnap-replay-worker capabilities | check-encoder ENCODER | live SOCKET DIRECTORY SECONDS MEMORY_MIB ENCODER | probe --input PATH|- --output FILE.mkv --closed-gop [--seconds N] [--memory-mib N]".into()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("replay worker: {error}");
        std::process::exit(1);
    }
}
mod crop;
mod live;
#[path = "../../process.rs"]
mod process;
#[path = "../../socket.rs"]
mod socket;
