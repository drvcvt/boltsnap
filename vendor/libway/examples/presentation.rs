//! Live presentation feedback check: opens one window, presents 240 tagged CPU
//! frames (dark gray, one slowly moving thin line, no flashing) and prints what
//! the compositor reported. Run inside a Wayland session.
use libway::window::{
    Configuration, Host, PresentOutcome, PresentationOutcome, PresentationTiming, WindowEvent,
    WindowOptions,
};
use std::time::{Duration, Instant};

const FRAMES: u64 = 240;
const BACKGROUND: [u8; 4] = [0x20, 0x20, 0x20, 0xff];
const LINE: [u8; 4] = [0x70, 0x70, 0x70, 0xff];

fn main() -> libway::Result<()> {
    let mut host = Host::connect()?;
    let start = Instant::now();
    let deadline = start + Duration::from_secs(15);
    let mut window = None;
    let mut configuration: Option<Configuration> = None;
    let (mut tag, mut dirty) = (0, true);
    let mut answers: Vec<PresentationOutcome> = Vec::new();
    while (answers.len() as u64) < FRAMES && Instant::now() < deadline {
        host.pump(Some(Instant::now() + Duration::from_millis(50)))
            .map_err(|failure| failure.error)?;
        for event in host.events() {
            match event {
                WindowEvent::Ready => {
                    let id = host.create_window(WindowOptions {
                        preferred_width: 480,
                        preferred_height: 270,
                        title: "libway presentation".into(),
                        app_id: "libway.presentation".into(),
                    })?;
                    host.enable_cpu(id)?;
                    window = Some(id);
                }
                WindowEvent::Configured {
                    configuration: next,
                    ..
                } => (configuration, dirty) = (Some(next), true),
                WindowEvent::RedrawRequested { .. } => dirty = true,
                WindowEvent::Presentation { outcome, .. } => answers.push(outcome),
                WindowEvent::CloseRequested { .. } => return Ok(()),
                _ => {}
            }
        }
        let (Some(id), Some(config)) = (window, configuration) else {
            continue;
        };
        if !dirty || tag >= FRAMES {
            continue;
        }
        host.request_presentation(id, tag)?;
        let pixels = frame(config, start.elapsed());
        if host.present_cpu(id, config.revision, &pixels)? == PresentOutcome::Presented {
            tag += 1;
        }
        dirty = false;
    }
    report(&answers);
    if let Some(id) = window {
        host.close(id)?;
    }
    host.shutdown();
    Ok(())
}

/// A thin line moving 60 pixels per second over a dark gray background.
fn frame(config: Configuration, elapsed: Duration) -> Vec<u8> {
    let (width, height) = (config.pixel_width as usize, config.pixel_height as usize);
    let line = (elapsed.as_secs_f64() * 60.0) as usize % width;
    let mut pixels = BACKGROUND.repeat(width * height);
    for row in 0..height {
        let start = (row * width + line) * 4;
        pixels[start..start + 4].copy_from_slice(&LINE);
    }
    pixels
}

fn report(answers: &[PresentationOutcome]) {
    let presented: Vec<PresentationTiming> = answers
        .iter()
        .filter_map(|outcome| match outcome {
            PresentationOutcome::Presented(timing) => Some(*timing),
            _ => None,
        })
        .collect();
    let discarded = answers
        .iter()
        .filter(|outcome| **outcome == PresentationOutcome::Discarded)
        .count();
    println!(
        "answers {}: presented {}, discarded {}, unavailable {}",
        answers.len(),
        presented.len(),
        discarded,
        answers.len() - presented.len() - discarded
    );
    let Some(first) = presented.first() else {
        return;
    };
    println!("clock {} flags {:#x}", first.clock, first.flags.bits());
    let mut intervals: Vec<Duration> = presented
        .windows(2)
        .map(|pair| pair[1].timestamp.saturating_sub(pair[0].timestamp))
        .collect();
    let mut refresh: Vec<Duration> = presented.iter().filter_map(|t| t.refresh).collect();
    println!(
        "median interval {:?}, median refresh {:?}",
        median(&mut intervals),
        median(&mut refresh)
    );
}

fn median(values: &mut [Duration]) -> Option<Duration> {
    values.sort();
    values.get(values.len() / 2).copied()
}
