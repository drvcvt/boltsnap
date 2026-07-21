use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::DynResult;
use crate::config::RecordAudioSource;

const SAMPLE_RATE: usize = 48_000;
const CHANNELS: usize = 2;
const BYTES_PER_SAMPLE: usize = 2;
const CHUNK_FRAMES: usize = 480;
const CHUNK_BYTES: usize = CHUNK_FRAMES * CHANNELS * BYTES_PER_SAMPLE;

pub(crate) struct AudioCapture {
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<Result<(), String>>>,
}

impl AudioCapture {
    pub fn start(source: RecordAudioSource) -> DynResult<(Self, Arc<Mutex<Receiver<Vec<u8>>>>)> {
        let stop = Arc::new(AtomicBool::new(false));
        let (output_tx, output_rx) = mpsc::channel();
        let mut handles = Vec::new();

        match source {
            RecordAudioSource::System => {
                handles.push(spawn_source(Direction::Render, output_tx, stop.clone())?);
            }
            RecordAudioSource::Mic => {
                handles.push(spawn_source(Direction::Capture, output_tx, stop.clone())?);
            }
            RecordAudioSource::SystemAndMic => {
                let (system_tx, system_rx) = mpsc::channel();
                let (mic_tx, mic_rx) = mpsc::channel();
                handles.push(spawn_source(Direction::Render, system_tx, stop.clone())?);
                handles.push(spawn_source(Direction::Capture, mic_tx, stop.clone())?);
                let mixer_stop = stop.clone();
                handles.push(
                    thread::Builder::new()
                        .name("boltsnap-audio-mixer".into())
                        .spawn(move || mix_sources(system_rx, mic_rx, output_tx, mixer_stop))?,
                );
            }
        }

        Ok((Self { stop, handles }, Arc::new(Mutex::new(output_rx))))
    }

    pub fn stop(mut self) -> DynResult<()> {
        self.stop.store(true, Ordering::Release);
        let mut first_error = None;
        for handle in self.handles.drain(..) {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
                Err(_) if first_error.is_none() => {
                    first_error = Some("Windows audio thread panicked".into())
                }
                _ => {}
            }
        }
        first_error.map_or(Ok(()), |error| Err(error.into()))
    }
}

fn spawn_source(
    device_direction: Direction,
    output: Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
) -> DynResult<JoinHandle<Result<(), String>>> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let name = match device_direction {
        Direction::Render => "boltsnap-system-audio",
        Direction::Capture => "boltsnap-microphone",
    };
    let handle = thread::Builder::new().name(name.into()).spawn(move || {
        let result = capture_source(device_direction, output, stop, &ready_tx);
        if let Err(error) = &result {
            let _ = ready_tx.send(Err(error.clone()));
        }
        result
    })?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(error)) => {
            let _ = handle.join();
            Err(error.into())
        }
        Err(error) => Err(format!("Windows audio source did not initialize: {error}").into()),
    }
}

fn capture_source(
    device_direction: Direction,
    output: Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    initialize_mta()
        .ok()
        .map_err(|error| format!("initialize WASAPI COM apartment: {error}"))?;
    let enumerator = DeviceEnumerator::new().map_err(|error| error.to_string())?;
    let device = enumerator
        .get_default_device(&device_direction)
        .map_err(|error| format!("open default {device_direction:?} audio device: {error}"))?;
    let mut client = device
        .get_iaudioclient()
        .map_err(|error| error.to_string())?;
    let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE, CHANNELS, None);
    let (_, minimum_period) = client
        .get_device_period()
        .map_err(|error| error.to_string())?;
    client
        .initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: minimum_period,
            },
        )
        .map_err(|error| format!("initialize WASAPI stream: {error}"))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|error| error.to_string())?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|error| error.to_string())?;
    client
        .start_stream()
        .map_err(|error| format!("start WASAPI stream: {error}"))?;
    let _ = ready.send(Ok(()));

    let mut samples = VecDeque::with_capacity(CHUNK_BYTES * 8);
    while !stop.load(Ordering::Acquire) {
        let _ = event.wait_for_event(100);
        loop {
            let available = capture
                .get_next_packet_size()
                .map_err(|error| error.to_string())?
                .unwrap_or(0);
            if available == 0 {
                break;
            }
            capture
                .read_from_device_to_deque(&mut samples)
                .map_err(|error| error.to_string())?;
        }
        while samples.len() >= CHUNK_BYTES {
            let chunk = samples.drain(..CHUNK_BYTES).collect();
            if output.send(chunk).is_err() {
                stop.store(true, Ordering::Release);
                break;
            }
        }
    }
    client.stop_stream().map_err(|error| error.to_string())
}

fn mix_sources(
    system: Receiver<Vec<u8>>,
    microphone: Receiver<Vec<u8>>,
    output: Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    while !stop.load(Ordering::Acquire) {
        let system_chunk = system.recv_timeout(Duration::from_millis(20)).ok();
        let microphone_chunk = microphone.recv_timeout(Duration::from_millis(20)).ok();
        if system_chunk.is_none() && microphone_chunk.is_none() {
            continue;
        }
        let mut mixed = vec![0_u8; CHUNK_BYTES];
        for offset in (0..CHUNK_BYTES).step_by(2) {
            let system_sample = sample_at(system_chunk.as_deref(), offset);
            let microphone_sample = sample_at(microphone_chunk.as_deref(), offset);
            let sample = system_sample.saturating_add(microphone_sample);
            mixed[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
        }
        if output.send(mixed).is_err() {
            break;
        }
    }
    Ok(())
}

fn sample_at(chunk: Option<&[u8]>, offset: usize) -> i16 {
    chunk
        .and_then(|chunk| chunk.get(offset..offset + 2))
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .unwrap_or_default()
}
