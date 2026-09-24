use super::*;
use std::ffi::CStr;
use std::io::Write;

const EGL_ES: InitParams = InitParams {
    width: 64,
    height: 48,
    fps: 240,
    color_depth: 0,
    graphics_api: GRAPHICS_API_EGL_ES,
};

/// A feed pipe: the read end's number for the config, the write end for the test.
fn feed() -> (i32, File) {
    let mut fds = [0; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    (fds[0], unsafe { File::from_raw_fd(fds[1]) })
}

/// Video 64 px wide showing logical x 100..132, y 50..: two pixels per unit.
fn config(fd: i32) -> String {
    format!("fd={fd} preset=quick size=8 origin=100,50 width=32")
}

#[test]
fn exported_init_and_deinit_need_no_gl_context() {
    let (fd, _writer) = feed();
    // No EGL context exists in this process: any GL call here would fail.
    unsafe { std::env::set_var(PLUGIN_ENV, config(fd)) };
    let mut ret = InitReturn {
        name: std::ptr::null(),
        version: 0,
        userdata: std::ptr::null_mut(),
        draw: None,
        is_damaged: None,
        clear_damage: None,
    };
    assert!(unsafe { gsr_plugin_init(&EGL_ES, &mut ret) });
    assert_eq!(
        unsafe { CStr::from_ptr(ret.name) }.to_str(),
        Ok("boltsnap-cursor")
    );
    assert_eq!(ret.version, 1);
    assert!(ret.draw.is_some() && ret.is_damaged.is_none());
    let plugin = unsafe { &*ret.userdata.cast::<Plugin>() };
    assert!(plugin.renderer.is_none());
    assert_eq!(plugin.mapping.scale, 2.0);
    // The feed is non-blocking and not inherited by gsr's helpers.
    assert_ne!(
        unsafe { libc::fcntl(fd, libc::F_GETFL) } & libc::O_NONBLOCK,
        0
    );
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, libc::FD_CLOEXEC);
    unsafe { gsr_plugin_deinit(ret.userdata) };
    // Deinit closed the feed.
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
    unsafe { gsr_plugin_deinit(std::ptr::null_mut()) };
}

#[test]
fn init_rejects_glx_and_bad_config() {
    let (fd, _writer) = feed();
    let glx = InitParams {
        graphics_api: 1,
        ..EGL_ES
    };
    assert!(Plugin::new(&glx, &config(fd)).is_err());
    assert!(Plugin::new(&EGL_ES, "fd=3 preset=system size=24 origin=0,0 width=1").is_err());
    assert!(Plugin::new(&EGL_ES, "").is_err());
}

#[test]
fn feed_lines_map_to_video_pixels_across_partial_writes() {
    let (fd, mut writer) = feed();
    let mut plugin = Plugin::new(&EGL_ES, &config(fd)).unwrap();
    writer.write_all(b"0 1000000 p 110 6").unwrap();
    plugin.poll();
    writer.write_all(b"0\ngarbage\n").unwrap();
    plugin.poll();
    plugin.motion.advance_to(2000);
    // (110 - 100) * 2, (60 - 50) * 2
    assert_eq!(plugin.motion.position(), Some((20.0, 20.0)));
    // A hidden cursor draws nothing and needs no GL.
    writer.write_all(b"0 2001000 l\n").unwrap();
    drop(writer);
    plugin.frame(2_010_000, (64, 48)).unwrap();
    assert!(plugin.renderer.is_none());
    plugin.frame(2_020_000, (64, 48)).unwrap();
}

/// Runs the real GL path in a surfaceless EGL (GLES 3) context: the frame is an
/// opaque grey RGBA8 texture bound as FBO, like gsr's plugin texture.
#[test]
#[ignore = "needs an EGL driver (surfaceless Mesa or an EGL device)"]
fn draws_the_arrow_over_the_frame_and_averages_blur_taps() {
    let context = egl::Context::new().expect("EGL context");
    let (width, height) = (64u32, 48u32);
    let grey = [128u8, 128, 128, 255];

    // The plugin path: config, feed, first draw creates the renderer.
    let frame = context.frame(width, height, grey);
    let (fd, mut writer) = feed();
    let mut plugin = Plugin::new(&EGL_ES, &config(fd)).unwrap();
    writer.write_all(b"0 1000000 p 110 60\n").unwrap();
    plugin.frame(3_000_000, (width, height)).unwrap();
    let pixels = context.read(&frame);
    let at = |x: u32, y: u32| &pixels[((y * width + x) * 4) as usize..][..4];
    // At rest on whole pixels each video pixel is the 4x4 area mean of the
    // supersampled arrow over the frame.
    let (texels, tex_w, _) = gl::texture(&plugin.arrow);
    let ss = gl::SUPERSAMPLE;
    let left = (20.0 - plugin.hotspot.0.round()) as u32;
    let top = (20.0 - plugin.hotspot.1.round()) as u32;
    let (out_w, out_h) = (plugin.arrow.width / ss + 1, plugin.arrow.height / ss + 1);
    let mut opaque = 0;
    for y in 0..out_h {
        for x in 0..out_w {
            let mut mean = [0f32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let texel = (((y * ss + dy + ss) * tex_w + x * ss + dx + ss) * 4) as usize;
                    for c in 0..4 {
                        mean[c] += f32::from(texels[texel + c]) / (ss * ss) as f32;
                    }
                }
            }
            let coverage = mean[3] / 255.0;
            let drawn = at(left + x, top + y);
            for c in 0..3 {
                let expected = 128.0 * (1.0 - coverage) + mean[c];
                assert!(
                    (f32::from(drawn[c]) - expected).abs() <= 2.5,
                    "({x}, {y}) channel {c}: {} vs {expected}",
                    drawn[c]
                );
            }
            if coverage > 0.99 {
                opaque += 1;
            }
        }
    }
    // Row 0 of the frame is the image top: the tip sits top-left, not flipped.
    assert!(opaque > 20, "{opaque}");
    // The tip is at (20, 20); the dark fill lies just below and right of it.
    assert!(
        at(22, 26)[0] < 60,
        "dark body near the tip: {:?}",
        at(22, 26)
    );
    assert_eq!(at(17, 26), grey, "nothing left of the white rim");
    assert!(
        pixels.chunks_exact(4).all(|p| p[3] == 255),
        "frame alpha kept"
    );
    assert_eq!(at(60, 44), grey);

    // Blur: taps average; they do not overdraw each other.
    // One video pixel of half-transparent white, at the supersampled size.
    let half = Arrow {
        rgba: [255, 255, 255, 128].repeat(16),
        width: 4,
        height: 4,
        hotspot: (0.0, 0.0),
    };
    let frame = context.frame(width, height, grey);
    let renderer = gl::Renderer::new(&half).unwrap();
    let a = Some((10.0, 10.0));
    let b = Some((30.0, 10.0));
    renderer.draw(&[a, a, a, a, b, b, b, None], (width, height));
    let pixels = context.read(&frame);
    let at = |x: u32, y: u32| &pixels[((y * width + x) * 4) as usize..][..4];
    let expect = |taps: f32| {
        let coverage = 128.0 / 255.0 * taps / 8.0;
        (128.0 * (1.0 - coverage) + 255.0 * coverage).round() as i32
    };
    assert!(
        (i32::from(at(10, 10)[0]) - expect(4.0)).abs() <= 1,
        "{:?}",
        at(10, 10)
    );
    assert!(
        (i32::from(at(30, 10)[0]) - expect(3.0)).abs() <= 1,
        "{:?}",
        at(30, 10)
    );
    assert_eq!(at(10, 10)[3], 255);
    assert_eq!(at(20, 10), grey);

    // Subpixel: half a pixel right splits the coverage over two pixels.
    let frame = context.frame(width, height, grey);
    let opaque = Arrow {
        rgba: vec![255; 64],
        ..half
    };
    let renderer = gl::Renderer::new(&opaque).unwrap();
    renderer.draw(&[Some((10.5, 20.0)); 8], (width, height));
    let pixels = context.read(&frame);
    let at = |x: u32, y: u32| &pixels[((y * width + x) * 4) as usize..][..4];
    for x in [10, 11] {
        assert!(
            (i32::from(at(x, 20)[0]) - 192).abs() <= 1,
            "{:?}",
            at(x, 20)
        );
        assert_eq!(at(x, 20)[3], 255);
    }
    assert_eq!(at(9, 20), grey);
    assert_eq!(at(12, 20), grey);
    assert_eq!(at(10, 19), grey);
}

/// Minimal surfaceless EGL + FBO harness for the GL check.
mod egl {
    use super::gl::symbol;
    use std::ffi::{CStr, c_void};

    type Display = *mut c_void;

    fn egl(name: &CStr) -> *mut c_void {
        let library = unsafe { libc::dlopen(c"libEGL.so.1".as_ptr(), libc::RTLD_LAZY) };
        assert!(!library.is_null(), "libEGL.so.1");
        let symbol = unsafe { libc::dlsym(library, name.as_ptr()) };
        assert!(!symbol.is_null(), "{name:?}");
        symbol
    }

    macro_rules! call {
        ($load:ident, $name:literal, fn($($arg:ty),*) -> $ret:ty, $($value:expr),*) => {
            unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) -> $ret>(
                    $load($name).unwrap_or_else(|e: String| panic!("{e}")),
                )($($value),*)
            }
        };
    }

    fn egl_ok(name: &CStr) -> Result<*mut c_void, String> {
        Ok(egl(name))
    }

    pub struct Context;

    pub struct Frame {
        pub width: u32,
        pub height: u32,
    }

    impl Context {
        pub fn new() -> Result<Self, String> {
            const SURFACELESS_MESA: u32 = 0x31DD;
            const ES_API: u32 = 0x30A0;
            const RENDERABLE_TYPE: i32 = 0x3040;
            const ES3_BIT: i32 = 0x40;
            const SURFACE_TYPE: i32 = 0x3033;
            const PBUFFER_BIT: i32 = 0x0001;
            const NONE: i32 = 0x3038;
            const CLIENT_VERSION: i32 = 0x3098;
            // BOLTSNAP_EGL_DEVICE=N picks EGL device N (e.g. the NVIDIA driver)
            // instead of Mesa's surfaceless platform.
            let display: Display = match std::env::var("BOLTSNAP_EGL_DEVICE") {
                Ok(index) => {
                    const DEVICE_EXT: u32 = 0x313F;
                    let query = egl(c"eglGetProcAddress");
                    let query: unsafe extern "C" fn(*const std::ffi::c_char) -> *mut c_void =
                        unsafe { std::mem::transmute(query) };
                    let devices = unsafe { query(c"eglQueryDevicesEXT".as_ptr()) };
                    let devices: unsafe extern "C" fn(i32, *mut *mut c_void, *mut i32) -> u32 =
                        unsafe { std::mem::transmute(devices) };
                    let mut list = [std::ptr::null_mut(); 8];
                    let mut count = 0;
                    unsafe { devices(8, list.as_mut_ptr(), &mut count) };
                    let index: usize = index.parse().unwrap();
                    assert!(index < count as usize, "{count} EGL devices");
                    call!(
                        egl_ok,
                        c"eglGetPlatformDisplay",
                        fn(u32, *mut c_void, *const isize) -> Display,
                        DEVICE_EXT,
                        list[index],
                        std::ptr::null()
                    )
                }
                Err(_) => call!(
                    egl_ok,
                    c"eglGetPlatformDisplay",
                    fn(u32, *mut c_void, *const isize) -> Display,
                    SURFACELESS_MESA,
                    std::ptr::null_mut(),
                    std::ptr::null()
                ),
            };
            if display.is_null() {
                return Err("no surfaceless EGL display".into());
            }
            let (mut major, mut minor) = (0, 0);
            if call!(
                egl_ok,
                c"eglInitialize",
                fn(Display, *mut i32, *mut i32) -> u32,
                display,
                &mut major,
                &mut minor
            ) == 0
            {
                return Err("eglInitialize failed".into());
            }
            call!(egl_ok, c"eglBindAPI", fn(u32) -> u32, ES_API);
            let attributes = [RENDERABLE_TYPE, ES3_BIT, SURFACE_TYPE, PBUFFER_BIT, NONE];
            let mut config: *mut c_void = std::ptr::null_mut();
            let mut count = 0;
            call!(
                egl_ok,
                c"eglChooseConfig",
                fn(Display, *const i32, *mut *mut c_void, i32, *mut i32) -> u32,
                display,
                attributes.as_ptr(),
                &mut config,
                1,
                &mut count
            );
            if count < 1 {
                return Err("no GLES 3 config".into());
            }
            let context_attributes = [CLIENT_VERSION, 3, NONE];
            let context: *mut c_void = call!(
                egl_ok,
                c"eglCreateContext",
                fn(Display, *mut c_void, *mut c_void, *const i32) -> *mut c_void,
                display,
                config,
                std::ptr::null_mut(),
                context_attributes.as_ptr()
            );
            if context.is_null()
                || call!(
                    egl_ok,
                    c"eglMakeCurrent",
                    fn(Display, *mut c_void, *mut c_void, *mut c_void) -> u32,
                    display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    context
                ) == 0
            {
                return Err("no current GLES 3 context".into());
            }
            let renderer = call!(
                symbol,
                c"glGetString",
                fn(u32) -> *const std::ffi::c_char,
                0x1F01
            );
            println!("GL renderer: {:?}", unsafe { CStr::from_ptr(renderer) });
            Ok(Self)
        }

        /// A fresh frame texture of `color`, bound as the draw target with
        /// gsr's viewport and blend state.
        pub fn frame(&self, width: u32, height: u32, color: [u8; 4]) -> Frame {
            let pixels: Vec<u8> = color.repeat((width * height) as usize);
            let (mut texture, mut framebuffer) = (0u32, 0u32);
            call!(
                symbol,
                c"glGenTextures",
                fn(i32, *mut u32) -> (),
                1,
                &mut texture
            );
            call!(
                symbol,
                c"glBindTexture",
                fn(u32, u32) -> (),
                0x0DE1,
                texture
            );
            call!(
                symbol,
                c"glTexImage2D",
                fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void) -> (),
                0x0DE1,
                0,
                0x8058,
                width as i32,
                height as i32,
                0,
                0x1908,
                0x1401,
                pixels.as_ptr().cast()
            );
            call!(
                symbol,
                c"glGenFramebuffers",
                fn(i32, *mut u32) -> (),
                1,
                &mut framebuffer
            );
            call!(
                symbol,
                c"glBindFramebuffer",
                fn(u32, u32) -> (),
                0x8D40,
                framebuffer
            );
            call!(
                symbol,
                c"glFramebufferTexture2D",
                fn(u32, u32, u32, u32, i32) -> (),
                0x8D40,
                0x8CE0,
                0x0DE1,
                texture,
                0
            );
            let status = call!(symbol, c"glCheckFramebufferStatus", fn(u32) -> u32, 0x8D40);
            assert_eq!(status, 0x8CD5, "framebuffer complete");
            call!(
                symbol,
                c"glViewport",
                fn(i32, i32, i32, i32) -> (),
                0,
                0,
                width as i32,
                height as i32
            );
            call!(symbol, c"glEnable", fn(u32) -> (), 0x0BE2);
            call!(symbol, c"glBlendFunc", fn(u32, u32) -> (), 0x0302, 0x0303);
            Frame { width, height }
        }

        pub fn finish(&self) {
            call!(symbol, c"glFinish", fn() -> (),);
        }

        /// RGBA rows of the bound frame, row 0 first.
        pub fn read(&self, frame: &Frame) -> Vec<u8> {
            let mut pixels = vec![0u8; (frame.width * frame.height * 4) as usize];
            call!(
                symbol,
                c"glReadPixels",
                fn(i32, i32, i32, i32, u32, u32, *mut c_void) -> (),
                0,
                0,
                frame.width as i32,
                frame.height as i32,
                0x1908,
                0x1401,
                pixels.as_mut_ptr().cast()
            );
            pixels
        }
    }
}

#[test]
#[ignore = "needs an EGL driver (surfaceless Mesa or an EGL device)"]
fn keeps_drawing_after_the_pointer_stops() {
    let context = egl::Context::new().expect("EGL context");
    let _frame = context.frame(64, 48, [128, 128, 128, 255]);
    let (fd, mut writer) = feed();
    let mut plugin = Plugin::new(&EGL_ES, &config(fd)).unwrap();
    let start = 1_000_000u64;
    let frame_us = 1_000_000 / 240;
    for frame in 0..240 * 20u64 {
        let now = start + frame * frame_us;
        // The pointer moves for 8 s, then rests; the feed stays open.
        if frame < 240 * 8 {
            writer
                .write_all(format!("0 {now} p {} 60\n", 100 + frame % 20).as_bytes())
                .unwrap();
        }
        let began = std::time::Instant::now();
        plugin.frame(now, (64, 48)).unwrap();
        // gsr finishes every frame on NVIDIA (`gsr_egl_swap_buffers`).
        context.finish();
        // The first frame compiles the shader, which is slow on llvmpipe.
        assert!(
            frame == 0 || began.elapsed().as_millis() < 100,
            "frame {frame} stalled"
        );
    }
}
