//! gpu-screen-recorder plugin that draws Boltsnap's spring-smoothed cursor into
//! every captured frame (`gpu-screen-recorder -p libboltsnap_gsr_cursor.so`).
//! The daemon's cursor trackers feed pointer positions through an inherited
//! pipe; `BOLTSNAP_CURSOR` carries the configuration. See
//! `docs/plans/2026-09-24-live-cursor-plugin.md`.

#[allow(dead_code)]
#[path = "../../../../record/cursor_motion.rs"]
mod cursor_motion;
mod gl;

use cursor_motion::{Arrow, Mapping, Motion, PLUGIN_ENV, PluginConfig, Sources};
use std::ffi::{c_char, c_uint, c_void};
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::os::fd::FromRawFd;

/// `gsr_plugin_init_params` of gsr's `plugin/plugin.h` (interface 0.1).
#[repr(C)]
pub struct InitParams {
    pub width: c_uint,
    pub height: c_uint,
    pub fps: c_uint,
    pub color_depth: i32,
    pub graphics_api: i32,
}

/// `gsr_plugin_draw_params`.
#[repr(C)]
pub struct DrawParams {
    pub width: c_uint,
    pub height: c_uint,
}

/// `gsr_plugin_init_return`.
#[repr(C)]
pub struct InitReturn {
    pub name: *const c_char,
    pub version: c_uint,
    pub userdata: *mut c_void,
    pub draw: Option<unsafe extern "C" fn(*const DrawParams, *mut c_void)>,
    pub is_damaged: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    pub clear_damage: Option<unsafe extern "C" fn(*mut c_void)>,
}

const GRAPHICS_API_EGL_ES: i32 = 0;

struct Plugin {
    feed: File,
    partial: Vec<u8>,
    sources: Sources,
    mapping: Mapping,
    motion: Motion,
    /// Rasterized at `gl::SUPERSAMPLE` times the video resolution.
    arrow: Arrow,
    /// The arrow's tip in video pixels.
    hotspot: (f64, f64),
    /// Created on the first visible frame, inside `draw`.
    renderer: Option<gl::Renderer>,
}

impl Plugin {
    /// No GL work: gsr's context is current here, but everything GL waits for `draw`.
    fn new(params: &InitParams, config: &str) -> Result<Self, String> {
        if params.graphics_api != GRAPHICS_API_EGL_ES {
            return Err("needs gsr's EGL (GLES) context; X11/GLX capture is unsupported".into());
        }
        let config = PluginConfig::parse(config)?;
        let preset = cursor_motion::preset(&config.preset).ok_or("unknown preset")?;
        let fd = config.fd;
        // Keep the feed away from gsr's helper processes, and never block a frame.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0
            || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
        {
            return Err(format!(
                "cursor feed fd {fd}: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mapping = config.mapping(params.width);
        let ss = gl::SUPERSAMPLE as f32;
        let arrow = cursor_motion::arrow(config.size * mapping.scale as f32 * ss);
        let hotspot = (
            arrow.hotspot.0 / f64::from(ss),
            arrow.hotspot.1 / f64::from(ss),
        );
        Ok(Self {
            feed: unsafe { File::from_raw_fd(fd) },
            partial: Vec::new(),
            sources: Sources::default(),
            mapping,
            motion: Motion::new(preset),
            arrow,
            hotspot,
            renderer: None,
        })
    }

    /// Take the events the trackers wrote since the last frame.
    fn poll(&mut self) {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match self.feed.read(&mut buffer) {
                // End of feed: the trackers stopped; keep the last state.
                Ok(0) => break,
                Ok(n) => self.partial.extend_from_slice(&buffer[..n]),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let mut start = 0;
        while let Some(end) = self.partial[start..].iter().position(|b| *b == b'\n') {
            let line = &self.partial[start..start + end];
            if let Some(event) = std::str::from_utf8(line)
                .ok()
                .and_then(cursor_motion::parse_feed_line)
                && let Some(sample) = self.sources.sample(event, &self.mapping)
            {
                self.motion.push(sample);
            }
            start += end + 1;
        }
        self.partial.drain(..start);
        if self.partial.len() > 4096 {
            self.partial.clear();
        }
    }

    /// Draw the frame captured at monotonic `now_us` into the bound frame texture.
    fn frame(&mut self, now_us: u64, size: (u32, u32)) -> Result<(), String> {
        self.poll();
        let now_ms = now_us as f64 / 1000.0;
        self.motion.advance_to(now_ms.ceil() as i64);
        let taps = self.motion.taps(now_ms, self.hotspot);
        if taps.iter().all(Option::is_none) {
            return Ok(());
        }
        let renderer = match &mut self.renderer {
            Some(renderer) => renderer,
            None => self.renderer.insert(gl::Renderer::new(&self.arrow)?),
        };
        renderer.draw(&taps, size);
        Ok(())
    }
}

fn monotonic_us() -> u64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    now.tv_sec as u64 * 1_000_000 + now.tv_nsec as u64 / 1000
}

/// # Safety
/// Called by gpu-screen-recorder with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gsr_plugin_init(params: *const InitParams, ret: *mut InitReturn) -> bool {
    let (Some(params), Some(ret)) = (unsafe { params.as_ref() }, unsafe { ret.as_mut() }) else {
        return false;
    };
    let plugin = std::env::var(PLUGIN_ENV)
        .map_err(|_| format!("{PLUGIN_ENV} is not set"))
        .and_then(|config| Plugin::new(params, &config));
    match plugin {
        Ok(plugin) => {
            ret.name = c"boltsnap-cursor".as_ptr();
            ret.version = 1;
            ret.userdata = Box::into_raw(Box::new(plugin)).cast();
            ret.draw = Some(draw);
            ret.is_damaged = None;
            ret.clear_damage = None;
            true
        }
        Err(error) => {
            eprintln!("boltsnap cursor plugin: {error}");
            false
        }
    }
}

unsafe extern "C" fn draw(params: *const DrawParams, userdata: *mut c_void) {
    let (Some(params), Some(plugin)) = (unsafe { params.as_ref() }, unsafe {
        userdata.cast::<Plugin>().as_mut()
    }) else {
        return;
    };
    if let Err(error) = plugin.frame(monotonic_us(), (params.width, params.height)) {
        // Recording on without the requested cursor would lose it silently.
        eprintln!("boltsnap cursor plugin: {error}");
        unsafe { libc::_exit(1) };
    }
}

/// # Safety
/// `userdata` is the pointer `gsr_plugin_init` returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gsr_plugin_deinit(userdata: *mut c_void) {
    // GL objects go away with gsr's context; only memory and the feed are freed.
    if !userdata.is_null() {
        drop(unsafe { Box::from_raw(userdata.cast::<Plugin>()) });
    }
}

#[cfg(test)]
mod tests;
