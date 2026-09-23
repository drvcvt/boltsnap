//! Owns a thread-confined C compositor. No native pointers escape this module.
use std::os::unix::ffi::OsStrExt;
use std::{
    ffi::{CStr, CString, c_char, c_float, c_int, c_void},
    marker::PhantomData,
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
    path::Path,
    ptr::NonNull,
    rc::Rc,
};
unsafe extern "C" {
    fn bs_gpu_open(
        node: *const c_char,
        w: c_int,
        h: c_int,
        error: *mut c_char,
        size: c_int,
    ) -> *mut c_void;
    fn bs_gpu_close(g: *mut c_void);
    fn bs_gpu_error(g: *mut c_void) -> *const c_char;
    fn bs_gpu_rgba_background(g: *mut c_void, rgba: *const u8) -> c_int;
    fn bs_gpu_background(
        g: *mut c_void,
        w: c_int,
        h: c_int,
        format: u32,
        modifier: u64,
        planes: c_int,
        fds: *const c_int,
        strides: *const u32,
        offsets: *const u32,
    ) -> c_int;
    fn bs_gpu_cursor(g: *mut c_void, w: c_int, h: c_int, rgba: *const u8) -> c_int;
    fn bs_gpu_draw(g: *mut c_void, visible: c_int, x: c_float, y: c_float) -> c_int;
    fn bs_gpu_readback(g: *mut c_void, rgba: *mut u8, bytes: c_int) -> c_int;
    fn bs_gpu_encoder(g: *mut c_void, codec: *const c_char, fps: c_int, fd: c_int) -> c_int;
    fn bs_gpu_encode(g: *mut c_void, pts: i64) -> c_int;
    fn bs_gpu_finish(g: *mut c_void) -> c_int;
}
pub struct Gpu {
    raw: NonNull<c_void>,
    width: u32,
    height: u32,
    output: Option<OwnedFd>,
    _thread: PhantomData<Rc<()>>,
}
impl Gpu {
    pub fn new(node: &Path, width: u32, height: u32) -> Result<Self, String> {
        if !(1..=8192).contains(&width)
            || !(1..=8192).contains(&height)
            || u64::from(width) * u64::from(height) > 16_777_216
        {
            return Err("unsupported compositor size".into());
        }
        let node = CString::new(node.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let mut error = [0 as c_char; 512];
        // All strings/arrays live through the call. C returns either owned state
        // or a terminated diagnostic, and destroys partial state on failure.
        let raw = unsafe {
            bs_gpu_open(
                node.as_ptr(),
                width as i32,
                height as i32,
                error.as_mut_ptr(),
                512,
            )
        };
        let raw = NonNull::new(raw).ok_or_else(|| unsafe {
            CStr::from_ptr(error.as_ptr())
                .to_string_lossy()
                .into_owned()
        })?;
        Ok(Self {
            raw,
            width,
            height,
            output: None,
            _thread: PhantomData,
        })
    }
    fn check(&self, result: i32) -> Result<(), String> {
        if result < 0 {
            Err(unsafe {
                CStr::from_ptr(bs_gpu_error(self.raw.as_ptr()))
                    .to_string_lossy()
                    .into_owned()
            })
        } else {
            Ok(())
        }
    }
    pub fn rgba_background(&mut self, rgba: &[u8]) -> Result<(), String> {
        if rgba.len() != self.width as usize * self.height as usize * 4 {
            return Err("background byte count".into());
        }
        self.check(unsafe { bs_gpu_rgba_background(self.raw.as_ptr(), rgba.as_ptr()) })
    }
    pub fn background(
        &mut self,
        format: u32,
        modifier: u64,
        planes: &[(BorrowedFd<'_>, u32, u32)],
    ) -> Result<(), String> {
        if planes.is_empty() || planes.len() > 4 {
            return Err("invalid DMA-BUF planes".into());
        }
        let fds: Vec<_> = planes.iter().map(|p| p.0.as_raw_fd()).collect();
        let strides: Vec<_> = planes.iter().map(|p| p.1).collect();
        let offsets: Vec<_> = planes.iter().map(|p| p.2).collect();
        self.check(unsafe {
            bs_gpu_background(
                self.raw.as_ptr(),
                self.width as i32,
                self.height as i32,
                format,
                modifier,
                planes.len() as i32,
                fds.as_ptr(),
                strides.as_ptr(),
                offsets.as_ptr(),
            )
        })
    }
    pub fn cursor(&mut self, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
        if w == 0 || h == 0 || w > 1024 || h > 1024 || rgba.len() != w as usize * h as usize * 4 {
            return Err("invalid cursor image".into());
        }
        self.check(unsafe { bs_gpu_cursor(self.raw.as_ptr(), w as i32, h as i32, rgba.as_ptr()) })
    }
    pub fn draw(&mut self, position: Option<(f32, f32)>) -> Result<(), String> {
        let (x, y) = position.unwrap_or_default();
        if !x.is_finite() || !y.is_finite() {
            return Err("non-finite cursor position".into());
        }
        self.check(unsafe { bs_gpu_draw(self.raw.as_ptr(), position.is_some() as i32, x, y) })
    }
    pub fn readback(&mut self) -> Result<Vec<u8>, String> {
        let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
        self.check(unsafe {
            bs_gpu_readback(self.raw.as_ptr(), pixels.as_mut_ptr(), pixels.len() as i32)
        })?;
        Ok(pixels)
    }
    pub fn encoder(&mut self, codec: &str, fps: u32, output: BorrowedFd<'_>) -> Result<(), String> {
        if fps == 0 || fps > 240 || !matches!(codec, "libx264" | "h264_vulkan") {
            return Err("unsupported native cursor encoder/FPS".into());
        }
        let codec = CString::new(codec).map_err(|e| e.to_string())?;
        if self.output.is_some() {
            return Err("encoder already initialized".into());
        }
        self.output = Some(output.try_clone_to_owned().map_err(|e| e.to_string())?);
        self.check(unsafe {
            bs_gpu_encoder(
                self.raw.as_ptr(),
                codec.as_ptr(),
                fps as i32,
                self.output.as_ref().unwrap().as_raw_fd(),
            )
        })
    }
    pub fn encode(&mut self, pts: i64) -> Result<(), String> {
        self.check(unsafe { bs_gpu_encode(self.raw.as_ptr(), pts) })
    }
    pub fn finish(&mut self) -> Result<(), String> {
        self.check(unsafe { bs_gpu_finish(self.raw.as_ptr()) })
    }
}
impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe { bs_gpu_close(self.raw.as_ptr()) }
    }
}
