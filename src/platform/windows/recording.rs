use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::DynResult;
use crate::protocol::{PublicRecordingState, RecordingSnapshot};

type CaptureError = Box<dyn Error + Send + Sync>;

#[derive(Clone)]
pub(crate) struct RecordingSettings {
    output: PathBuf,
    crop: Option<(u32, u32, u32, u32)>,
    width: u32,
    height: u32,
    audio_enabled: bool,
    audio_receiver: Option<Arc<Mutex<Receiver<Vec<u8>>>>>,
}

pub(crate) struct RecordingHandler {
    encoder: Option<VideoEncoder>,
    crop: Option<(u32, u32, u32, u32)>,
    paused: bool,
    paused_at: Option<Instant>,
    paused_ticks: i64,
    audio_receiver: Option<Arc<Mutex<Receiver<Vec<u8>>>>>,
}

impl RecordingHandler {
    fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        self.paused = paused;
        if paused {
            self.paused_at = Some(Instant::now());
        } else if let Some(started) = self.paused_at.take() {
            self.paused_ticks = self
                .paused_ticks
                .saturating_add(duration_ticks(started.elapsed()));
        }
    }

    fn finish(&mut self) -> Result<(), CaptureError> {
        self.drain_audio(true)?;
        if let Some(encoder) = self.encoder.take() {
            encoder.finish()?;
        }
        Ok(())
    }

    fn drain_audio(&mut self, encode: bool) -> Result<(), CaptureError> {
        let Some(receiver) = &self.audio_receiver else {
            return Ok(());
        };
        let chunks: Vec<Vec<u8>> = {
            let receiver = receiver
                .lock()
                .map_err(|_| "Windows audio queue is poisoned")?;
            receiver.try_iter().collect()
        };
        if encode {
            if let Some(encoder) = self.encoder.as_mut() {
                for chunk in chunks {
                    encoder.send_audio_buffer(&chunk, 0)?;
                }
            }
        }
        Ok(())
    }
}

impl GraphicsCaptureApiHandler for RecordingHandler {
    type Flags = RecordingSettings;
    type Error = CaptureError;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let settings = context.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(settings.width, settings.height)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(30)
                .bitrate(12_000_000),
            AudioSettingsBuilder::default().disabled(!settings.audio_enabled),
            ContainerSettingsBuilder::default(),
            &settings.output,
        )?;
        Ok(Self {
            encoder: Some(encoder),
            crop: settings.crop,
            paused: false,
            paused_at: None,
            paused_ticks: 0,
            audio_receiver: settings.audio_receiver,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.paused {
            self.drain_audio(false)?;
            return Ok(());
        }
        if self.encoder.is_none() {
            return Ok(());
        }
        self.drain_audio(true)?;
        let timestamp = frame
            .timestamp()?
            .Duration
            .saturating_sub(self.paused_ticks);
        let buffer = match self.crop {
            Some((left, top, right, bottom)) => frame.buffer_crop(left, top, right, bottom)?,
            None => frame.buffer()?,
        };
        let width = buffer.width() as usize;
        let height = buffer.height() as usize;
        let mut packed = Vec::new();
        let bytes = buffer.as_nopadding_buffer(&mut packed);
        let stride = width * 4;
        let mut bottom_up = vec![0_u8; bytes.len()];
        for row in 0..height {
            let source = row * stride;
            let destination = (height - 1 - row) * stride;
            bottom_up[destination..destination + stride]
                .copy_from_slice(&bytes[source..source + stride]);
        }
        self.encoder
            .as_mut()
            .expect("encoder checked above")
            .send_frame_buffer(&bottom_up, timestamp)?;
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.finish()
    }
}

pub(crate) struct WindowsRecording {
    control: Option<CaptureControl<RecordingHandler, CaptureError>>,
    output: PathBuf,
    state: PublicRecordingState,
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    scope: String,
    audio: Option<crate::platform::windows::audio::AudioCapture>,
}

impl WindowsRecording {
    pub fn start_area(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        audio_enabled: bool,
        audio_source: crate::config::RecordAudioSource,
    ) -> DynResult<Self> {
        let (monitor, monitor_rect) = monitor_for_point(x, y)?;
        let local_x = x.saturating_sub(monitor_rect.left) as u32;
        let local_y = y.saturating_sub(monitor_rect.top) as u32;
        let width = even(width.min((monitor_rect.right - x).max(0) as u32));
        let height = even(height.min((monitor_rect.bottom - y).max(0) as u32));
        if width < 2 || height < 2 {
            return Err("recording selection is outside the selected monitor".into());
        }
        Self::start(
            monitor,
            Some((local_x, local_y, local_x + width, local_y + height)),
            width,
            height,
            "area".into(),
            audio_enabled,
            audio_source,
        )
    }

    pub fn start_focused(
        audio_enabled: bool,
        audio_source: crate::config::RecordAudioSource,
    ) -> DynResult<Self> {
        let rect = crate::platform::windows::select_skia::focused_monitor_rect()?;
        let (monitor, _) = monitor_for_point(rect.left, rect.top)?;
        Self::start(
            monitor,
            None,
            even((rect.right - rect.left) as u32),
            even((rect.bottom - rect.top) as u32),
            "focused".into(),
            audio_enabled,
            audio_source,
        )
    }

    fn start(
        monitor: Monitor,
        crop: Option<(u32, u32, u32, u32)>,
        width: u32,
        height: u32,
        scope: String,
        audio_enabled: bool,
        audio_source: crate::config::RecordAudioSource,
    ) -> DynResult<Self> {
        let output = crate::paths::rec_file("capture", "mp4");
        let (audio, audio_receiver) = if audio_enabled {
            let (audio, receiver) =
                crate::platform::windows::audio::AudioCapture::start(audio_source)?;
            (Some(audio), Some(receiver))
        } else {
            (None, None)
        };
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Include,
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(33)),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            RecordingSettings {
                output: output.clone(),
                crop,
                width,
                height,
                audio_enabled,
                audio_receiver,
            },
        );
        let control = match RecordingHandler::start_free_threaded(settings) {
            Ok(control) => control,
            Err(error) => {
                if let Some(audio) = audio {
                    let _ = audio.stop();
                }
                return Err(error.into());
            }
        };
        Ok(Self {
            control: Some(control),
            output,
            state: PublicRecordingState::Recording,
            started: Instant::now(),
            paused_at: None,
            paused_total: Duration::ZERO,
            scope,
            audio,
        })
    }

    pub fn pause(&mut self) -> DynResult<()> {
        if self.state != PublicRecordingState::Recording {
            return Err("recording is not active".into());
        }
        if let Some(control) = &self.control {
            control.callback().lock().set_paused(true);
        }
        self.paused_at = Some(Instant::now());
        self.state = PublicRecordingState::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> DynResult<()> {
        if self.state != PublicRecordingState::Paused {
            return Err("recording is not paused".into());
        }
        if let Some(control) = &self.control {
            control.callback().lock().set_paused(false);
        }
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_total += paused_at.elapsed();
        }
        self.state = PublicRecordingState::Recording;
        Ok(())
    }

    pub fn finish(mut self) -> DynResult<PathBuf> {
        self.state = PublicRecordingState::Finalizing;
        if let Some(audio) = self.audio.take() {
            audio.stop()?;
        }
        let control = self.control.take().ok_or("recording control is missing")?;
        control
            .callback()
            .lock()
            .finish()
            .map_err(|error| format!("failed to finalize Windows recording: {error}"))?;
        control.stop()?;
        if !self.output.is_file() || self.output.metadata()?.len() == 0 {
            return Err("Media Foundation did not create a recording".into());
        }
        Ok(self.output)
    }

    pub fn snapshot(&self) -> RecordingSnapshot {
        let now = Instant::now();
        let paused = self
            .paused_at
            .map(|paused_at| now.saturating_duration_since(paused_at))
            .unwrap_or_default();
        let elapsed = now
            .saturating_duration_since(self.started)
            .saturating_sub(self.paused_total)
            .saturating_sub(paused);
        RecordingSnapshot {
            state: self.state,
            elapsed_ms: elapsed.as_millis() as u64,
            scope: self.scope.clone(),
            outputs: Vec::new(),
            actions_enabled: matches!(
                self.state,
                PublicRecordingState::Recording | PublicRecordingState::Paused
            ),
            error: None,
        }
    }
}

fn monitor_for_point(x: i32, y: i32) -> DynResult<(Monitor, windows::Win32::Foundation::RECT)> {
    for monitor in Monitor::enumerate()? {
        let rect = monitor_rect(monitor)?;
        if x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom {
            return Ok((monitor, rect));
        }
    }
    Err(format!("no monitor contains recording origin {x},{y}").into())
}

fn monitor_rect(monitor: Monitor) -> DynResult<windows::Win32::Foundation::RECT> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &mut info) }.as_bool() {
        return Err("GetMonitorInfoW failed for recording target".into());
    }
    Ok(info.rcMonitor)
}

fn even(value: u32) -> u32 {
    value.saturating_sub(value % 2)
}

fn duration_ticks(duration: Duration) -> i64 {
    (duration.as_nanos() / 100) as i64
}

pub(crate) fn move_to_recording_dir(path: &Path) -> DynResult<PathBuf> {
    let config = crate::config::Config::load();
    let directory = crate::config::resolve_record_dir(&config);
    std::fs::create_dir_all(&directory)?;
    let destination = crate::paths::unique_recording_path(&directory, None);
    std::fs::rename(path, &destination).or_else(|_| {
        std::fs::copy(path, &destination)?;
        std::fs::remove_file(path)
    })?;
    Ok(destination)
}
