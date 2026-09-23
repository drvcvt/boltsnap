//! Launcher for the optional native cursor producer. Legacy launches stay intact.
use crate::{
    config::RecordProfile,
    record::session::{ActiveRecorder, CaptureScope, RecorderTools, segment_path},
};
use std::{
    process::{Command, Stdio},
    time::Duration,
};

pub fn probe(output: &str, codec: &str, profile: RecordProfile) -> Result<(), String> {
    supported_profile(profile)?;
    let result = super::replay::process::output(
        Command::new(super::replay::companion_path("boltsnap-replay-worker")).args([
            "cursor-probe",
            output,
            codec,
        ]),
        Duration::from_secs(8),
    )?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr)
            .trim()
            .chars()
            .take(400)
            .collect());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&result.stdout).map_err(|e| e.to_string())?;
    if value["cursor_smoothing"] != true {
        return Err("worker lacks native cursor support".into());
    }
    Ok(())
}
pub fn spawn(
    scope: &CaptureScope,
    codec: &str,
    profile: RecordProfile,
    audio: Option<&str>,
    tools: &RecorderTools,
) -> Result<Vec<ActiveRecorder>, String> {
    supported_profile(profile)?;
    let output = target(scope, codec)?;
    std::fs::create_dir_all(&tools.segment_dir).map_err(|e| e.to_string())?;
    let path = segment_path(&tools.segment_dir, Some(output));
    let fps = match profile {
        RecordProfile::Quality => "240",
        RecordProfile::Quiet => "60",
    };
    let mut command = Command::new(super::replay::companion_path("boltsnap-replay-worker"));
    command
        .args(["cursor-record", output, fps, codec, audio.unwrap_or("-")])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let child = super::replay::process::spawn(&mut command, None)
        .map_err(|e| format!("start native cursor recorder: {e}"))?;
    Ok(vec![ActiveRecorder {
        output: Some(output.to_owned()),
        path,
        child,
    }])
}
fn supported_profile(profile: RecordProfile) -> Result<(), String> {
    if profile != RecordProfile::Quiet {
        return Err("Experimental cursor smoothing requires record_profile = \"quiet\" (60 FPS); the 240 FPS path has insufficient measured headroom.".into());
    }
    Ok(())
}
fn target<'a>(scope: &'a CaptureScope, codec: &str) -> Result<&'a str, String> {
    if codec != "libx264" {
        return Err("Cursor smoothing currently requires explicit libx264; hardware encoding is not approved yet.".into());
    }
    match scope {
        CaptureScope::Outputs(outputs) if outputs.len()==1 && !outputs[0].is_empty()=>Ok(&outputs[0]),
        _=>Err("Cursor smoothing currently supports one fullscreen output. Disable it for region or multi-output recordings.".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_targets_and_codecs_never_fall_back() {
        assert!(supported_profile(RecordProfile::Quality).is_err());
        assert!(supported_profile(RecordProfile::Quiet).is_ok());
        assert!(target(&CaptureScope::Outputs(vec!["DP-1".into()]), "libx264").is_ok());
        assert!(target(&CaptureScope::Outputs(vec!["DP-1".into()]), "h264_vulkan").is_err());
        assert!(
            target(
                &CaptureScope::Outputs(vec!["DP-1".into(), "DP-2".into()]),
                "libx264"
            )
            .is_err()
        );
        assert!(
            target(
                &CaptureScope::Area(crate::record::Geometry {
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 100
                }),
                "libx264"
            )
            .is_err()
        );
    }
}
