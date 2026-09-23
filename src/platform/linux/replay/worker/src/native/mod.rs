//! Optional capture/compositing implementation, kept out of the main executable.
mod gpu;
use crate::cursor_motion as motion;
use gpu::Gpu;
use libway::{
    Backend, BufferKind, CaptureOptions, Connection, CursorEvent, FrameStorage, Output, Transform,
};
use motion::{LOOKAHEAD, Motion, frame_time};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::OpenOptions,
    os::fd::{AsFd, OwnedFd},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}
fn signals() -> Result<(), String> {
    // Only this standalone producer installs handlers; never a library caller.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = stop as *const () as usize;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    for signal in [libc::SIGTERM, libc::SIGINT] {
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}
fn node() -> Result<PathBuf, String> {
    let mut nodes = std::fs::read_dir("/dev/dri")
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().as_encoded_bytes().starts_with(b"renderD"))
        .map(|e| e.path())
        .collect::<Vec<_>>();
    if nodes.len() != 1 {
        return Err(
            "native cursor recording currently requires exactly one DRM render node".into(),
        );
    }
    Ok(nodes.pop().unwrap())
}
fn selected(
    c: &mut Connection,
    name: &str,
    expected: Option<&Output>,
    options: &CaptureOptions,
) -> Result<Output, String> {
    let outputs = c.outputs(options).map_err(|e| e.to_string())?;
    let mut matches = outputs.into_iter().filter(|o| o.name == name);
    let output = matches.next().ok_or("cursor capture output unavailable")?;
    if matches.next().is_some() {
        return Err("ambiguous capture output".into());
    }
    if output.transform != Transform::Normal
        || output.mode_size.0 % 2 != 0
        || output.mode_size.1 % 2 != 0
    {
        return Err("native cursor capture requires an unrotated, even-sized output".into());
    }
    if let Some(old) = expected
        && (old.logical != output.logical
            || old.mode_size != output.mode_size
            || old.description != output.description
            || old.transform != output.transform)
    {
        return Err("capture output changed during discovery".into());
    }
    if !c.capabilities().ext_output_capture {
        return Err("separate cursor capture requires EXT image-copy-capture".into());
    }
    Ok(output)
}
struct Background {
    format: u32,
    modifier: u64,
    planes: Vec<(OwnedFd, u32, u32)>,
    time: Instant,
    ack: Option<mpsc::SyncSender<()>>,
}
impl Drop for Background {
    fn drop(&mut self) {
        if let Some(ack) = self.ack.take() {
            let _ = ack.try_send(());
        }
    }
}
struct Image {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    hotspot: (i32, i32),
    count: Arc<AtomicUsize>,
}
impl Drop for Image {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}
enum Observation {
    Visibility(Instant, bool),
    Position(Instant, i32, i32),
    Image(Instant, Image),
}
struct Producers {
    cancel: libway::Cancellation,
    video: mpsc::Receiver<Result<Background, String>>,
    cursor: mpsc::Receiver<Result<Observation, String>>,
    threads: Vec<thread::JoinHandle<()>>,
}
impl Drop for Producers {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Drop queued leases before joining the video thread waiting for an ack.
        while self.video.try_recv().is_ok() {}
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
fn send<T>(
    tx: &mpsc::SyncSender<Result<T, String>>,
    value: Result<T, String>,
    cancel: &libway::Cancellation,
) -> Result<(), String> {
    let mut value = value;
    loop {
        match tx.try_send(value) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err("consumer stopped".into()),
            Err(mpsc::TrySendError::Full(v)) => value = v,
        }
        if cancel.is_cancelled() {
            return Err("capture cancelled".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
}
// EXT timestamps use the system monotonic clock. Keep source presentation time
// rather than dating a delayed GPU copy by its receipt time.
fn presented(frame: &libway::Frame) -> Result<Instant, String> {
    let stamp = frame
        .presentation_time
        .ok_or("video presentation timestamp missing")?;
    let instant = Instant::now();
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let clock = Duration::new(
        u64::try_from(now.tv_sec).map_err(|_| "invalid monotonic clock")?,
        u32::try_from(now.tv_nsec).map_err(|_| "invalid monotonic clock")?,
    );
    if stamp > clock + Duration::from_secs(1) {
        return Err("video timestamp is outside the monotonic timeline".into());
    }
    if stamp <= clock {
        instant.checked_sub(clock - stamp)
    } else {
        instant.checked_add(stamp - clock)
    }
    .ok_or_else(|| "video timestamp overflow".into())
}

fn producers(output: &Output, node: &Path) -> Producers {
    let cancel = libway::Cancellation::default();
    let mut threads = Vec::new();
    let (video_tx, video) = mpsc::sync_channel(1);
    let (cursor_tx, cursor) = mpsc::sync_channel(256);
    let opts = CaptureOptions {
        backend: Backend::Ext,
        cursor: false,
        cancellation: cancel.clone(),
        ..Default::default()
    };
    let expected = output.clone();
    let render = node.to_owned();
    let options = opts.clone();
    threads.push(thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut c = Connection::connect(&options).map_err(|e| e.to_string())?;
            let output = selected(&mut c, &expected.name, Some(&expected), &options)?;
            let allocator = libway::gpu::GpuAllocator::open(render).map_err(|e| e.to_string())?;
            let mut idle_options = options.clone();
            idle_options.timeout = Duration::from_secs(3600);
            let mut stream = c
                .stream(output.id, idle_options, BufferKind::Gpu(allocator))
                .map_err(|e| e.to_string())?;
            while !options.cancellation.is_cancelled() {
                let frame = stream.next_frame().map_err(|e| e.to_string())?;
                if frame.transform != Transform::Normal
                    || frame.y_inverted
                    || (frame.width, frame.height) != expected.mode_size
                {
                    return Err("video layout changed or unsupported buffer orientation".into());
                }
                let FrameStorage::Gpu(buffer) = &frame.storage else {
                    return Err("expected GPU capture storage".into());
                };
                let planes = buffer
                    .planes()
                    .iter()
                    .map(|p| {
                        Ok((
                            p.fd().try_clone_to_owned().map_err(|e| e.to_string())?,
                            p.stride(),
                            p.offset(),
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let (ack, wait) = mpsc::sync_channel(1);
                send(
                    &video_tx,
                    Ok(Background {
                        format: buffer.format() as u32,
                        modifier: buffer.allocation_modifier(),
                        planes,
                        time: presented(&frame)?,
                        ack: Some(ack),
                    }),
                    &options.cancellation,
                )?;
                loop {
                    match wait.recv_timeout(Duration::from_millis(20)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout)
                            if options.cancellation.is_cancelled() =>
                        {
                            return Ok(());
                        }
                        Err(_) => {}
                    }
                }
                drop(frame);
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = send(&video_tx, Err(e), &options.cancellation);
        }
    }));
    let expected = output.clone();
    threads.push(thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut c = Connection::connect(&opts).map_err(|e| e.to_string())?;
            let output = selected(&mut c, &expected.name, Some(&expected), &opts)?;
            let mut stream = c
                .cursor_stream(output.id, opts.clone())
                .map_err(|e| e.to_string())?;
            let count = Arc::new(AtomicUsize::new(0));
            while !opts.cancellation.is_cancelled() {
                let observation = match stream
                    .poll_event(Duration::from_millis(20))
                    .map_err(|e| e.to_string())?
                {
                    None => continue,
                    Some(CursorEvent::Enter { received_at }) => {
                        Observation::Visibility(received_at, true)
                    }
                    Some(CursorEvent::Leave { received_at }) => {
                        Observation::Visibility(received_at, false)
                    }
                    Some(CursorEvent::Position { received_at, x, y }) => {
                        Observation::Position(received_at, x, y)
                    }
                    Some(CursorEvent::Image {
                        received_at,
                        frame,
                        hotspot,
                        ..
                    }) => {
                        if frame.transform != Transform::Normal || frame.y_inverted {
                            return Err("unsupported cursor image orientation".into());
                        }
                        if count.load(Ordering::Relaxed) >= 4 {
                            return Err("cursor image queue exhausted".into());
                        }
                        let rgba = frame.rgba8().map_err(|e| e.to_string())?;
                        count.fetch_add(1, Ordering::Relaxed);
                        Observation::Image(
                            received_at,
                            Image {
                                w: frame.width,
                                h: frame.height,
                                rgba,
                                hotspot,
                                count: count.clone(),
                            },
                        )
                    }
                };
                // Metadata must not silently disappear under pressure.
                cursor_tx
                    .try_send(Ok(observation))
                    .map_err(|_| "cursor observation queue exhausted/disconnected")?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = send(&cursor_tx, Err(e), &opts.cancellation);
        }
    }));
    Producers {
        cancel,
        video,
        cursor,
        threads,
    }
}

pub fn run(command: &str, args: Vec<OsString>) -> Result<(), String> {
    match command {
        "cursor-fixture" => fixture(args),
        "cursor-probe" => {
            if args.len() != 2 {
                return Err("cursor-probe OUTPUT CODEC".into());
            }
            let name = args[0].to_str().ok_or("invalid output name")?;
            let codec = args[1].to_str().ok_or("invalid codec")?;
            check_codec(codec)?;
            let options = CaptureOptions::default();
            let mut c = Connection::connect(&options).map_err(|e| e.to_string())?;
            let output = selected(&mut c, name, None, &options)?;
            let _cursor = c
                .cursor_stream(output.id, options)
                .map_err(|e| e.to_string())?;
            let mut gpu = Gpu::new(&node()?, output.mode_size.0, output.mode_size.1)?;
            let discard = OpenOptions::new()
                .write(true)
                .open("/dev/null")
                .map_err(|e| e.to_string())?;
            gpu.encoder(codec, 60, discard.as_fd())?;
            println!(
                "{}",
                serde_json::json!({"ok":true,"cursor_smoothing":true,"output":name,"codec":codec})
            );
            Ok(())
        }
        "cursor-record" => record(args, false),
        "cursor-record-fixture" => record(args, true),
        _ => Err("unknown native cursor command".into()),
    }
}
fn check_codec(codec: &str) -> Result<(), String> {
    if codec != "libx264" {
        return Err("cursor smoothing currently requires explicit libx264; the local Vulkan encoder fails validation, no automatic codec fallback".into());
    }
    Ok(())
}
fn fixture(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 3 && args.len() != 5 {
        return Err("cursor-fixture RENDER_NODE CODEC OUTPUT.nut [WIDTH HEIGHT]".into());
    }
    let codec = args[1].to_str().ok_or("invalid codec")?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&args[2])
        .map_err(|e| e.to_string())?;
    let dimensions = if args.len() == 5 {
        (
            args[3]
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or("invalid width")?,
            args[4]
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or("invalid height")?,
        )
    } else {
        (320, 180)
    };
    let mut gpu = Gpu::new(Path::new(&args[0]), dimensions.0, dimensions.1)?;
    gpu.rgba_background(
        &[16u8, 32, 48, 255].repeat(dimensions.0 as usize * dimensions.1 as usize),
    )?;
    gpu.cursor(16, 16, &[128u8, 0, 0, 128].repeat(16 * 16))?;
    gpu.encoder(codec, 60, file.as_fd())?;
    let mut motion = Motion::default();
    motion.visibility(Duration::ZERO, true);
    let started = Instant::now();
    for index in 0..120 {
        let time = frame_time(index, 60).ok_or("frame time overflow")?;
        motion.position(time, index as i32 * 2 - 8, 80);
        motion.position(time + LOOKAHEAD, index as i32 * 2 - 7, 80);
        gpu.draw(motion.at(time + Duration::from_millis(4)))?;
        if index == 0 {
            let pixels = gpu.readback()?;
            if pixels[..4] != [16, 32, 48, 255] {
                return Err("GPU readback pixel mismatch".into());
            }
        }
        gpu.encode(index as i64)?;
    }
    gpu.finish()?;
    file.sync_all().map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::json!({"frames":120,"width":dimensions.0,"height":dimensions.1,"elapsed_seconds":started.elapsed().as_secs_f64()})
    );
    Ok(())
}
fn record(args: Vec<OsString>, fixture_audio: bool) -> Result<(), String> {
    if args.len() != 5 {
        return Err("cursor-record OUTPUT FPS CODEC AUDIO_SOURCE|- DESTINATION.mp4".into());
    }
    let name = args[0].to_str().ok_or("invalid output")?;
    let fps: u32 = args[1]
        .to_str()
        .and_then(|s| s.parse().ok())
        .filter(|n| (1..=240).contains(n))
        .ok_or("invalid fps")?;
    let codec = args[2].to_str().ok_or("invalid codec")?;
    check_codec(codec)?;
    if fps != 60 {
        return Err("experimental cursor recording currently supports 60 FPS only".into());
    }
    let audio = args[3].to_str().ok_or("invalid audio source")?;
    signals()?;
    let options = CaptureOptions::default();
    let mut c = Connection::connect(&options).map_err(|e| e.to_string())?;
    let output = selected(&mut c, name, None, &options)?;
    drop(c);
    let render = node()?;
    let mut gpu = Gpu::new(&render, output.mode_size.0, output.mode_size.1)?;
    let producers = producers(&output, &render);
    let initial = producers
        .video
        .recv_timeout(Duration::from_secs(8))
        .map_err(|e| format!("initial video frame: {e}"))??;
    import(&mut gpu, &initial)?;
    // Keep existing outputs intact. A failed session leaves its private segment
    // available to the caller's normal recovery path.
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&args[4])
        .map_err(|e| e.to_string())?;
    let mut command = Command::new("ffmpeg");
    command.args([
        "-v",
        "warning",
        "-nostdin",
        "-copyts",
        "-start_at_zero",
        "-f",
        "nut",
        "-i",
        "pipe:0",
    ]);
    let has_audio = audio != "-" || fixture_audio;
    if fixture_audio {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs_f64();
        let source = format!("sine=frequency=1000:sample_rate=48000,asetpts=PTS+{epoch:.6}/TB");
        command.args(["-isync", "0", "-re", "-f", "lavfi", "-i", &source]);
    } else if has_audio {
        command.args(["-isync", "0", "-f", "pulse", "-i", audio]);
    }
    command.args(["-map", "0:v:0", "-c:v", "copy"]);
    if has_audio {
        command.args(["-map", "1:a:0", "-c:a", "aac", "-b:a", "128k", "-shortest"]);
    }
    command
        .args([
            "-f",
            "mp4",
            "-movflags",
            "frag_keyframe+delay_moov",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(file))
        .stderr(Stdio::inherit());
    let mut mux = crate::process::spawn(&mut command, None).map_err(|e| e.to_string())?;
    let pipe = mux.stdin.take().ok_or("missing mux input")?;
    let result = (|| {
        gpu.encoder(codec, fps, pipe.as_fd())?;
        let origin = Instant::now();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        let pts_origin = i64::try_from(epoch.as_nanos() * u128::from(fps) / 1_000_000_000)
            .map_err(|_| "timestamp overflow")?;
        let mut motion = Motion::default();
        let mut images = VecDeque::new();
        let mut current_image: Option<Image> = None;
        let mut background = Some(initial);
        let mut pending_video = None;
        let mut index = 0;
        while !STOP.load(Ordering::Relaxed) {
            let time = frame_time(index, fps).ok_or("frame time overflow")?;
            let deadline = origin
                .checked_add(time + LOOKAHEAD)
                .ok_or("deadline overflow")?;
            while Instant::now() < deadline && !STOP.load(Ordering::Relaxed) {
                thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(2)),
                );
            }
            loop {
                let item = match producers.cursor.try_recv() {
                    Ok(item) => item?,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("cursor producer stopped".into());
                    }
                };
                match item {
                    Observation::Visibility(at, v) => {
                        motion.visibility(at.saturating_duration_since(origin), v)
                    }
                    Observation::Position(at, x, y) => {
                        motion.position(at.saturating_duration_since(origin), x, y)
                    }
                    Observation::Image(at, image) => {
                        images.push_back((at.saturating_duration_since(origin), image))
                    }
                }
            }
            while images.front().is_some_and(|(at, _)| *at <= time) {
                let (_, image) = images.pop_front().unwrap();
                gpu.cursor(image.w, image.h, &image.rgba)?;
                current_image = Some(image);
            }
            if pending_video.is_none() {
                match producers.video.try_recv() {
                    Ok(frame) => pending_video = Some(frame?),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("video producer stopped".into());
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if pending_video
                .as_ref()
                .is_some_and(|b: &Background| b.time <= origin + time)
            {
                let next = pending_video.take().unwrap();
                import(&mut gpu, &next)?;
                background = Some(next);
            }
            let position = current_image.as_ref().and_then(|image| {
                motion
                    .at(time)
                    .map(|(x, y)| (x - image.hotspot.0 as f32, y - image.hotspot.1 as f32))
            });
            gpu.draw(position)?;
            // EGL owns the import after the capture allocation is released.
            // The producing thread retains its allocation through this GPU fence.
            background.take();
            gpu.encode(pts_origin + index as i64)?;
            index += 1;
            if let Some(status) = mux.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("recording mux exited: {status}"));
            }
            // Never allocate catch-up frames or silently lower requested FPS.
            if origin.elapsed() > time + Duration::from_secs(2) {
                return Err("cursor encoder cannot sustain requested frame rate".into());
            }
        }
        gpu.finish()
    })();
    drop(pipe);
    drop(gpu);
    drop(producers);
    if result.is_err() {
        crate::process::terminate(&mut mux);
        return result;
    }
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = mux.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("recording mux failed: {status}"))
            };
        }
        if Instant::now() >= end {
            crate::process::terminate(&mut mux);
            return Err("audio/video mux finalization timed out".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn import(gpu: &mut Gpu, frame: &Background) -> Result<(), String> {
    let planes = frame
        .planes
        .iter()
        .map(|p| (p.0.as_fd(), p.1, p.2))
        .collect::<Vec<_>>();
    gpu.background(frame.format, frame.modifier, &planes)
}
