//! Starting a recording: which outputs, which audio, the first segment.

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_output;

use crate::record::audio::AudioCapture;
use crate::record::finalize::{check_recording_cache_limit, check_recording_reserve};
use crate::record::session::{
    CaptureScope, RecorderTools, RecordingSession, spawn_segment, start_plan,
};

use super::clients::validate_recording_dimensions;
use super::placement::monitor_for_geometry;
use super::{Daemon, notify};

pub(super) fn prepare_recording_cache() -> Result<(), String> {
    let dir = crate::paths::rec_dir();
    std::fs::create_dir_all(&dir).map_err(|error| format!("create recording cache: {error}"))?;
    check_recording_cache_limit(&dir)?;
    check_recording_reserve(&dir)
}

pub(super) fn default_start_plan(
    prefs: &crate::config::RecordingPrefs,
    query: &Result<Vec<crate::record::Monitor>, String>,
    wayland: &[crate::record::Monitor],
) -> Result<crate::record::session::StartPlan, String> {
    let mut monitors: Vec<_> = match query {
        Ok(fresh) => fresh
            .iter()
            .filter(|monitor| wayland.iter().any(|output| output.name == monitor.name))
            .cloned()
            .collect(),
        Err(_) => wayland
            .iter()
            .cloned()
            .map(|mut monitor| {
                monitor.focused = false;
                monitor
            })
            .collect(),
    };
    if query.is_err() {
        let needs_focused_fallback = match &prefs.default_target {
            crate::config::RecordDefaultTarget::Focused => true,
            crate::config::RecordDefaultTarget::Output(name) => {
                !monitors.iter().any(|monitor| monitor.name == *name)
            }
            crate::config::RecordDefaultTarget::Both => false,
        };
        if needs_focused_fallback && let Some(first) = monitors.first_mut() {
            first.focused = true;
        }
    }
    start_plan(prefs, &monitors).map_err(|error| match query {
        Ok(_) => error,
        Err(query_error) => format!("{query_error}: {error}"),
    })
}

pub(super) fn resolve_output_name<'a>(
    requested: Option<&str>,
    available: &'a [String],
) -> Option<&'a str> {
    requested
        .and_then(|requested| {
            available
                .iter()
                .find(|name| name.as_str() == requested)
                .map(String::as_str)
        })
        .or_else(|| available.first().map(String::as_str))
}

pub(super) fn transformed_mode_size(
    (width, height): (i32, i32),
    transform: wl_output::Transform,
) -> (i32, i32) {
    if matches!(
        transform,
        wl_output::Transform::_90
            | wl_output::Transform::_270
            | wl_output::Transform::Flipped90
            | wl_output::Transform::Flipped270
    ) {
        (height, width)
    } else {
        (width, height)
    }
}

pub(super) fn requested_audio(
    prefs: &crate::config::RecordingPrefs,
) -> Option<crate::config::RecordAudioSource> {
    prefs.audio_enabled.then_some(prefs.audio_source)
}

fn spawn_initial_segment(
    scope: &CaptureScope,
    codec: &str,
    profile: crate::config::RecordProfile,
    tools: &RecorderTools,
    prefs: &crate::config::RecordingPrefs,
    combined: Option<&[crate::record::Monitor]>,
) -> Result<
    (
        Vec<crate::record::session::ActiveRecorder>,
        Option<AudioCapture>,
    ),
    String,
> {
    let mut audio = requested_audio(prefs)
        .map(|source| crate::record::audio::prepare_audio(source, tools.needs_audio_mix()))
        .transpose()?;
    match spawn_segment(
        scope,
        codec,
        profile,
        audio.as_ref().map_or(&[][..], AudioCapture::inputs),
        prefs.cursor,
        combined,
        tools,
    ) {
        Ok(active) => Ok((active, audio)),
        Err(error) => {
            let Some(audio) = audio.take() else {
                return Err(error);
            };
            match audio.cleanup() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}; clean up recording audio: {cleanup}")),
            }
        }
    }
}

pub(super) fn cleanup_audio_async(audio: Option<AudioCapture>) {
    if let Some(audio) = audio {
        std::thread::spawn(move || {
            if let Err(error) = audio.cleanup() {
                eprintln!("boltsnap daemon: clean up recording audio: {error}");
            }
        });
    }
}

impl Daemon {
    pub(super) fn start_recording(
        &mut self,
        geo: crate::record::Geometry,
        show_frame: bool,
        audio_enabled: bool,
        qh: &QueueHandle<Self>,
    ) -> Result<(), String> {
        if self.recording.is_some() {
            return Err("a recording is already in progress".into());
        }
        validate_recording_dimensions(geo.w, geo.h)?;
        self.refresh_prefs();
        let prefs = crate::config::RecordingPrefs {
            audio_enabled,
            ..self.recording_prefs.clone()
        };
        let config = crate::config::Config::load();
        let tools = RecorderTools::default();
        let codec = tools.session_codec(&crate::config::resolve_record_codec(None, &config))?;
        prepare_recording_cache()?;
        let scope = CaptureScope::Area(geo);
        let profile = config.record_profile()?;
        let monitors = self.cached_monitors();
        let monitors = monitor_for_geometry(&monitors, &geo)
            .cloned()
            .into_iter()
            .collect();
        let (active, audio) = spawn_initial_segment(&scope, &codec, profile, &tools, &prefs, None)?;
        self.recording = Some(RecordingSession::new(
            scope,
            monitors,
            codec.clone(),
            profile,
            crate::config::RecordBothMode::Separate,
            show_frame,
            audio,
            active,
            std::time::Instant::now(),
        ));
        let session = self.recording.as_mut().unwrap();
        session.cursor = self.recording_prefs.cursor;
        session.shelf_to_disk_after = crate::config::Config::load().shelf_to_disk_after();
        if show_frame {
            self.create_marker(&geo, qh);
        }
        self.publish_recording_snapshot();
        eprintln!("boltsnap daemon: recording {} -> {codec:?}", geo.to_arg());
        Ok(())
    }

    pub(super) fn start_default_recording(
        &mut self,
        focus_snapshot: &Result<Vec<crate::record::Monitor>, String>,
    ) -> Result<(), String> {
        self.refresh_prefs();
        let plan = default_start_plan(
            &self.recording_prefs,
            focus_snapshot,
            &self.cached_monitors(),
        )?;
        let notice = plan.notice.clone();
        self.start_recording_outputs(plan.outputs, plan.both_mode)?;
        if let Some(notice) = notice {
            notify(&notice);
        }
        Ok(())
    }

    pub(super) fn start_named_recording_outputs(
        &mut self,
        names: Vec<String>,
        both_mode: crate::config::RecordBothMode,
    ) -> Result<(), String> {
        let connected = self.cached_monitors();
        let monitors = names
            .iter()
            .map(|name| {
                connected
                    .iter()
                    .find(|monitor| monitor.name == *name)
                    .cloned()
                    .ok_or_else(|| format!("recording output {name} is disconnected"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.start_recording_outputs(monitors, both_mode)
    }

    fn start_recording_outputs(
        &mut self,
        monitors: Vec<crate::record::Monitor>,
        both_mode: crate::config::RecordBothMode,
    ) -> Result<(), String> {
        if self.recording.is_some() {
            return Err("a recording is already in progress".into());
        }
        self.refresh_prefs();
        let config = crate::config::Config::load();
        let tools = RecorderTools::default();
        let codec = tools.session_codec(&crate::config::resolve_record_codec(None, &config))?;
        prepare_recording_cache()?;
        if monitors.is_empty() {
            return Err("no recording output was selected".into());
        }
        let names = monitors
            .iter()
            .map(|monitor| monitor.name.clone())
            .collect::<Vec<_>>();
        let scope = CaptureScope::Outputs(names.clone());
        let profile = config.record_profile()?;
        let (active, audio) = spawn_initial_segment(
            &scope,
            &codec,
            profile,
            &tools,
            &self.recording_prefs,
            (both_mode == crate::config::RecordBothMode::Combined).then_some(monitors.as_slice()),
        )?;
        self.recording = Some(RecordingSession::new(
            scope,
            monitors,
            codec,
            profile,
            both_mode,
            false,
            audio,
            active,
            std::time::Instant::now(),
        ));
        let session = self.recording.as_mut().unwrap();
        session.cursor = self.recording_prefs.cursor;
        session.shelf_to_disk_after = crate::config::Config::load().shelf_to_disk_after();
        self.publish_recording_snapshot();
        Ok(())
    }
}
