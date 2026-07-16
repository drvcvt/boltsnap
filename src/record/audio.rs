use crate::config::RecordAudioSource;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct AudioCapture {
    source: String,
    modules: Vec<u32>,
}

impl AudioCapture {
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn cleanup(self) -> Result<(), String> {
        unload_modules(&self.modules, run_pactl)
    }

    #[cfg(test)]
    pub(crate) fn for_test(source: &str) -> Self {
        Self {
            source: source.into(),
            modules: Vec::new(),
        }
    }
}

fn run_pactl(args: &[String]) -> Result<String, String> {
    let output = Command::new("pactl")
        .args(args)
        .output()
        .map_err(|error| format!("run pactl: {error}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| format!("read pactl output: {error}"))
    } else {
        Err(format!(
            "pactl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn call(
    run: &mut impl FnMut(&[String]) -> Result<String, String>,
    args: &[&str],
) -> Result<String, String> {
    run(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
}

fn one_line(output: String, what: &str) -> Result<String, String> {
    let value = output.lines().next().unwrap_or_default().trim();
    if value.is_empty() {
        Err(format!("pactl returned no {what}"))
    } else {
        Ok(value.to_owned())
    }
}

fn source_names(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect()
}

fn require_source(names: &[&str], source: &str) -> Result<(), String> {
    names
        .contains(&source)
        .then_some(())
        .ok_or_else(|| format!("audio source {source} is unavailable"))
}

fn load_module(
    args: Vec<String>,
    run: &mut impl FnMut(&[String]) -> Result<String, String>,
) -> Result<u32, String> {
    run(&args)?
        .trim()
        .parse()
        .map_err(|error| format!("invalid pactl module id: {error}"))
}

fn unload_modules(
    modules: &[u32],
    mut run: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<(), String> {
    let mut first_error = None;
    for module in modules.iter().rev() {
        let args = ["unload-module".to_owned(), module.to_string()];
        if let Err(error) = run(&args)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn prepare_audio_with(
    mode: RecordAudioSource,
    mix_name: &str,
    mut run: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<AudioCapture, String> {
    let direct = match mode {
        RecordAudioSource::System => {
            let sink = one_line(call(&mut run, &["get-default-sink"])?, "default sink")?;
            Some(format!("{sink}.monitor"))
        }
        RecordAudioSource::Mic => Some(one_line(
            call(&mut run, &["get-default-source"])?,
            "default source",
        )?),
        RecordAudioSource::SystemAndMic => None,
    };
    if let Some(source) = direct {
        let output = call(&mut run, &["list", "short", "sources"])?;
        require_source(&source_names(&output), &source)?;
        return Ok(AudioCapture {
            source,
            modules: Vec::new(),
        });
    }

    let sink = one_line(call(&mut run, &["get-default-sink"])?, "default sink")?;
    let mic = one_line(call(&mut run, &["get-default-source"])?, "default source")?;
    let system = format!("{sink}.monitor");
    let output = call(&mut run, &["list", "short", "sources"])?;
    let names = source_names(&output);
    require_source(&names, &system)?;
    require_source(&names, &mic)?;

    let mut modules = Vec::with_capacity(3);
    let setup = (|| -> Result<(), String> {
        modules.push(load_module(
            vec![
                "load-module".into(),
                "module-null-sink".into(),
                format!("sink_name={mix_name}"),
                "sink_properties=device.description=Boltsnap".into(),
            ],
            &mut run,
        )?);
        modules.push(load_module(
            vec![
                "load-module".into(),
                "module-loopback".into(),
                format!("source={system}"),
                format!("sink={mix_name}"),
            ],
            &mut run,
        )?);
        modules.push(load_module(
            vec![
                "load-module".into(),
                "module-loopback".into(),
                format!("source={mic}"),
                format!("sink={mix_name}"),
            ],
            &mut run,
        )?);
        Ok(())
    })();
    if let Err(error) = setup {
        return match unload_modules(&modules, &mut run) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; roll back audio mix: {cleanup}")),
        };
    }
    Ok(AudioCapture {
        source: format!("{mix_name}.monitor"),
        modules,
    })
}

static MIX_ID: AtomicU64 = AtomicU64::new(0);

pub fn prepare_audio(mode: RecordAudioSource) -> Result<AudioCapture, String> {
    let id = MIX_ID.fetch_add(1, Ordering::Relaxed);
    prepare_audio_with(
        mode,
        &format!("boltsnap_mix_{}_{}", std::process::id(), id),
        run_pactl,
    )
}

fn cleanup_stale_mixes_with(
    mut run: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<(), String> {
    let output = call(&mut run, &["list", "short", "modules"])?;
    let modules = output
        .lines()
        .filter(|line| line.contains("boltsnap_mix_"))
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .collect::<Vec<_>>();
    unload_modules(&modules, run)
}

pub fn cleanup_stale_mixes() -> Result<(), String> {
    cleanup_stale_mixes_with(run_pactl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakePactl {
        commands: Vec<String>,
        results: VecDeque<Result<String, String>>,
    }

    impl FakePactl {
        fn new<const N: usize>(outputs: [&str; N]) -> Self {
            Self::with_results(outputs.map(Ok))
        }

        fn with_results<const N: usize>(results: [Result<&str, &str>; N]) -> Self {
            Self {
                commands: Vec::new(),
                results: results
                    .into_iter()
                    .map(|result| result.map(str::to_owned).map_err(str::to_owned))
                    .collect(),
            }
        }

        fn run(&mut self, args: &[String]) -> Result<String, String> {
            self.commands.push(args.join(" "));
            self.results.pop_front().expect("queued pactl result")
        }
    }

    #[test]
    fn system_source_uses_default_sink_monitor() {
        let mut fake = FakePactl::new([
            "speakers\n",
            "1\talsa_output.monitor\n2\tspeakers.monitor\n",
        ]);
        let capture =
            prepare_audio_with(RecordAudioSource::System, "unused", |args| fake.run(args)).unwrap();
        assert_eq!(capture.source(), "speakers.monitor");
        assert_eq!(fake.commands, ["get-default-sink", "list short sources"]);
    }

    #[test]
    fn mic_source_uses_default_source() {
        let mut fake = FakePactl::new(["studio_mic\n", "1\tstudio_mic\n"]);
        let capture =
            prepare_audio_with(RecordAudioSource::Mic, "unused", |args| fake.run(args)).unwrap();
        assert_eq!(capture.source(), "studio_mic");
        assert_eq!(fake.commands, ["get-default-source", "list short sources"]);
    }

    #[test]
    fn combined_source_builds_mix_and_rolls_back_partial_failure() {
        let mut fake = FakePactl::with_results([
            Ok("speakers\n"),
            Ok("studio_mic\n"),
            Ok("1\tspeakers.monitor\n2\tstudio_mic\n"),
            Ok("41\n"),
            Ok("42\n"),
            Err("second loopback failed"),
            Ok(""),
            Ok(""),
        ]);
        let error = prepare_audio_with(
            RecordAudioSource::SystemAndMic,
            "boltsnap_mix_test",
            |args| fake.run(args),
        )
        .unwrap_err();
        assert!(error.contains("second loopback failed"));
        let unloads = fake
            .commands
            .iter()
            .filter(|command| command.starts_with("unload-module"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(unloads, ["unload-module 42", "unload-module 41"]);
    }

    #[test]
    fn stale_cleanup_only_unloads_boltsnap_modules_in_reverse_order() {
        let mut fake = FakePactl::new([
            "7\tmodule-always-sink\t\n41\tmodule-null-sink\tsink_name=boltsnap_mix_old\n42\tmodule-loopback\tsink=boltsnap_mix_old\n43\tmodule-loopback\tsink=boltsnap_mix_old\n",
            "",
            "",
            "",
        ]);
        cleanup_stale_mixes_with(|args| fake.run(args)).unwrap();
        assert_eq!(
            fake.commands,
            [
                "list short modules",
                "unload-module 43",
                "unload-module 42",
                "unload-module 41"
            ]
        );
    }
}
