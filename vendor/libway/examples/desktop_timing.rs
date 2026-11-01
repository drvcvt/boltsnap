//! Time desktop capture on the session selected by the environment. This
//! captures the live desktop: run it only with the owner's okay. It writes no
//! image and keeps no pixels.
//!
//! `desktop_timing [RUNS]` (default 20) times two things after one warmup:
//! - `cold`: `Connection::connect` plus `capture_desktop`, as a one-shot
//!   screenshot tool does it;
//! - `warm`: `capture_desktop` on one connection that stays open;
//! - `tiles cold` and `tiles warm`: the same with `capture_desktop_tiles`, when
//!   the desktop allows tiles;
//! - `to_image` per output: the RGBA conversion composition does per frame;
//! - with `--features vulkan`: `fast warm` and `fast cold` for
//!   `capture_desktop_tiles_fast`, with the tier it used.
use libway::{CaptureOptions, Connection};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runs: usize = match std::env::args().nth(1) {
        Some(runs) => runs.parse()?,
        None => 20,
    };
    if runs == 0 {
        return Err("RUNS must be at least 1".into());
    }
    let options = CaptureOptions::default();
    let mut connection = Connection::connect(&options)?;
    for output in connection.outputs(&options)? {
        println!(
            "output {}: {}x{} at {:.3}",
            output.name,
            output.mode_size.0,
            output.mode_size.1,
            output.scale()
        );
    }
    connection.capture_desktop(&options)?;
    let cold = time(runs, || {
        Connection::connect(&options)?.capture_desktop(&options)?;
        Ok(())
    })?;
    let warm = time(runs, || {
        connection.capture_desktop(&options)?;
        Ok(())
    })?;
    report("cold", &cold);
    report("warm", &warm);
    if connection.capture_desktop_tiles(&options)?.is_none() {
        println!("tiles: not applicable (mixed scales, a rotation or a resize)");
        return Ok(());
    }
    let tiles_cold = time(runs, || {
        Connection::connect(&options)?.capture_desktop_tiles(&options)?;
        Ok(())
    })?;
    let tiles_warm = time(runs, || {
        connection.capture_desktop_tiles(&options)?;
        Ok(())
    })?;
    report("tiles cold", &tiles_cold);
    report("tiles warm", &tiles_warm);
    #[cfg(feature = "vulkan")]
    {
        // The dma-buf tiers: report the tier and the first fallback cause.
        connection.capture_desktop_tiles_fast(&options)?;
        let (tier, cause) = connection.tile_readback();
        println!("fast tier: {tier:?}, first fallback: {cause:?}");
        let fast_warm = time(runs, || {
            connection.capture_desktop_tiles_fast(&options)?;
            Ok(())
        })?;
        report("fast warm", &fast_warm);
        let fast_cold = time(runs, || {
            Connection::connect(&options)?.capture_desktop_tiles_fast(&options)?;
            Ok(())
        })?;
        report("fast cold", &fast_cold);
    }
    // Where composition time goes: RGBA conversion of one captured tile.
    let tiles = connection
        .capture_desktop_tiles(&options)?
        .ok_or("tiles became inapplicable")?;
    for tile in &tiles.tiles {
        let convert = time(runs, || {
            tile.frame.to_image()?;
            Ok(())
        })?;
        report(&format!("to_image {}", tile.frame.output.name), &convert);
    }
    Ok(())
}

fn time(runs: usize, mut run: impl FnMut() -> libway::Result<()>) -> libway::Result<Vec<Duration>> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        run()?;
        samples.push(start.elapsed());
    }
    samples.sort();
    Ok(samples)
}

fn report(name: &str, samples: &[Duration]) {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{name}: median {:.2} ms, min {:.2} ms, max {:.2} ms, runs {}",
        ms(samples[samples.len() / 2]),
        ms(samples[0]),
        ms(samples[samples.len() - 1]),
        samples.len()
    );
}
