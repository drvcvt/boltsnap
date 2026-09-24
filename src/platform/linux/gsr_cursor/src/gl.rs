//! GLES 3 drawing of the blurred arrow over the captured frame. Every GL call
//! happens inside gsr's `draw`, with its EGL context current.

use crate::cursor_motion::{Arrow, BLUR_SAMPLES};
use std::ffi::{CStr, c_char, c_void};

const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE0: u32 = 0x84C0;
const GL_RGBA: u32 = 0x1908;
const GL_RGBA8: i32 = 0x8058;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_LINEAR: i32 = 0x2601;
const GL_LINEAR_MIPMAP_NEAREST: i32 = 0x2701;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
const GL_BLEND: u32 = 0x0BE2;
const GL_ZERO: u32 = 0;
const GL_ONE: u32 = 1;
const GL_SRC_ALPHA: u32 = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
const GL_CURRENT_PROGRAM: u32 = 0x8B8D;
const GL_VERTEX_ARRAY_BINDING: u32 = 0x85B5;
const GL_ACTIVE_TEXTURE: u32 = 0x84E0;
const GL_TEXTURE_BINDING_2D: u32 = 0x8069;
const GL_BLEND_SRC_RGB: u32 = 0x80C9;
const GL_BLEND_DST_RGB: u32 = 0x80C8;
const GL_BLEND_SRC_ALPHA: u32 = 0x80CB;
const GL_BLEND_DST_ALPHA: u32 = 0x80CA;

/// A GL entry point from `libGL.so.1`, the library gsr loads its own GL
/// functions from (GLVND dispatches to the current EGL context).
pub fn symbol(name: &CStr) -> Result<*mut c_void, String> {
    let library = unsafe { libc::dlopen(c"libGL.so.1".as_ptr(), libc::RTLD_LAZY) };
    if library.is_null() {
        return Err("cannot load libGL.so.1".into());
    }
    let symbol = unsafe { libc::dlsym(library, name.as_ptr()) };
    if symbol.is_null() {
        return Err(format!("libGL.so.1 lacks {}", name.to_string_lossy()));
    }
    Ok(symbol)
}

macro_rules! gl_api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        struct Gl {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Gl {
            fn load() -> Result<Self, String> {
                Ok(Self {
                    $($name: unsafe {
                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(
                            symbol(&CStr::from_bytes_with_nul(concat!(stringify!($name), "\0").as_bytes()).unwrap())?,
                        )
                    },)*
                })
            }
        }
    };
}

gl_api! {
    glCreateShader: fn(u32) -> u32;
    glShaderSource: fn(u32, i32, *const *const c_char, *const i32);
    glCompileShader: fn(u32);
    glGetShaderiv: fn(u32, u32, *mut i32);
    glGetShaderInfoLog: fn(u32, i32, *mut i32, *mut c_char);
    glDeleteShader: fn(u32);
    glCreateProgram: fn() -> u32;
    glAttachShader: fn(u32, u32);
    glLinkProgram: fn(u32);
    glGetProgramiv: fn(u32, u32, *mut i32);
    glGetProgramInfoLog: fn(u32, i32, *mut i32, *mut c_char);
    glUseProgram: fn(u32);
    glGetUniformLocation: fn(u32, *const c_char) -> i32;
    glUniform1i: fn(i32, i32);
    glUniform2f: fn(i32, f32, f32);
    glUniform4f: fn(i32, f32, f32, f32, f32);
    glUniform2fv: fn(i32, i32, *const f32);
    glGenVertexArrays: fn(i32, *mut u32);
    glBindVertexArray: fn(u32);
    glGenTextures: fn(i32, *mut u32);
    glActiveTexture: fn(u32);
    glBindTexture: fn(u32, u32);
    glTexParameteri: fn(u32, u32, i32);
    glTexImage2D: fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    glGenerateMipmap: fn(u32);
    glGetIntegerv: fn(u32, *mut i32);
    glIsEnabled: fn(u32) -> u8;
    glEnable: fn(u32);
    glDisable: fn(u32);
    glBlendFuncSeparate: fn(u32, u32, u32, u32);
    glDrawArrays: fn(u32, i32, i32);
}

const VERTEX: &CStr = c"#version 300 es
uniform vec4 rect;
uniform vec2 size;
void main() {
    vec2 corner = vec2((gl_VertexID & 1) == 0 ? rect.x : rect.z,
                       (gl_VertexID & 2) == 0 ? rect.y : rect.w);
    gl_Position = vec4(corner / size * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Mean of the arrow under each blur tap. Row 0 of gsr's frame texture is the
/// top image row, so `gl_FragCoord` is already in image pixels. The arrow is
/// rasterized at `SUPERSAMPLE` times the video resolution, premultiplied, with
/// mipmaps: four bilinear samples of mip 1 at quarter-pixel offsets area-filter
/// each video pixel, so subpixel positions stay sharp and a resting arrow gets
/// 16x coverage antialiasing. Output is straight alpha: the averaged
/// premultiplied colour divided by coverage.
const FRAGMENT: &CStr = c"#version 300 es
precision highp float;
uniform highp sampler2D sprite;
uniform vec2 taps[8];
uniform vec2 origin;
uniform vec2 texel_scale;
out vec4 color;
vec4 box(vec2 at) {
    vec4 sum = textureLod(sprite, origin + (at + vec2(-0.25, -0.25)) * texel_scale, 1.0);
    sum += textureLod(sprite, origin + (at + vec2(0.25, -0.25)) * texel_scale, 1.0);
    sum += textureLod(sprite, origin + (at + vec2(-0.25, 0.25)) * texel_scale, 1.0);
    sum += textureLod(sprite, origin + (at + vec2(0.25, 0.25)) * texel_scale, 1.0);
    return sum * 0.25;
}
void main() {
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 8; i++) {
        sum += box(gl_FragCoord.xy - taps[i]);
    }
    if (sum.a <= 0.0) discard;
    color = vec4(sum.rgb / sum.a, sum.a / 8.0);
}
";

/// Arrow texels per video pixel.
pub const SUPERSAMPLE: u32 = 4;

/// Premultiplied texture of a `SUPERSAMPLE`d arrow with a transparent border of
/// one video pixel, sized to a multiple of `SUPERSAMPLE` so mip levels stay
/// aligned with video pixels. Returns the texels and the texture size.
pub fn texture(arrow: &Arrow) -> (Vec<u8>, u32, u32) {
    let pad = SUPERSAMPLE;
    let round_up = |v: u32| v.div_ceil(SUPERSAMPLE) * SUPERSAMPLE;
    let (width, height) = (
        round_up(arrow.width + 2 * pad),
        round_up(arrow.height + 2 * pad),
    );
    let mut texels = vec![0u8; (width * height * 4) as usize];
    for y in 0..arrow.height {
        for x in 0..arrow.width {
            let from = ((y * arrow.width + x) * 4) as usize;
            let to = (((y + pad) * width + x + pad) * 4) as usize;
            let alpha = u32::from(arrow.rgba[from + 3]);
            for c in 0..3 {
                texels[to + c] = ((u32::from(arrow.rgba[from + c]) * alpha + 127) / 255) as u8;
            }
            texels[to + 3] = alpha as u8;
        }
    }
    (texels, width, height)
}

/// Tap position that never covers a pixel.
const HIDDEN: f32 = -1.0e6;

pub struct Renderer {
    gl: Gl,
    program: u32,
    vao: u32,
    texture: u32,
    /// Arrow size in video pixels.
    sprite: (f64, f64),
    rect: i32,
    size: i32,
    taps: i32,
}

/// GL state `draw` changes, restored afterwards so gsr's own passes are unaffected.
struct Saved {
    program: i32,
    vao: i32,
    active: i32,
    texture: i32,
    blend: bool,
    func: [i32; 4],
}

impl Saved {
    unsafe fn take(gl: &Gl) -> Self {
        let get = |name| {
            let mut value = 0;
            unsafe { (gl.glGetIntegerv)(name, &mut value) };
            value
        };
        let active = get(GL_ACTIVE_TEXTURE);
        unsafe { (gl.glActiveTexture)(GL_TEXTURE0) };
        Self {
            program: get(GL_CURRENT_PROGRAM),
            vao: get(GL_VERTEX_ARRAY_BINDING),
            active,
            texture: get(GL_TEXTURE_BINDING_2D),
            blend: unsafe { (gl.glIsEnabled)(GL_BLEND) } != 0,
            func: [
                get(GL_BLEND_SRC_RGB),
                get(GL_BLEND_DST_RGB),
                get(GL_BLEND_SRC_ALPHA),
                get(GL_BLEND_DST_ALPHA),
            ],
        }
    }

    unsafe fn restore(self, gl: &Gl) {
        unsafe {
            let [src_rgb, dst_rgb, src_alpha, dst_alpha] = self.func.map(|f| f as u32);
            (gl.glBlendFuncSeparate)(src_rgb, dst_rgb, src_alpha, dst_alpha);
            if !self.blend {
                (gl.glDisable)(GL_BLEND);
            }
            (gl.glBindTexture)(GL_TEXTURE_2D, self.texture as u32);
            (gl.glActiveTexture)(self.active as u32);
            (gl.glBindVertexArray)(self.vao as u32);
            (gl.glUseProgram)(self.program as u32);
        }
    }
}

impl Renderer {
    /// Load GL, build the program and upload the arrow, which is rasterized at
    /// `SUPERSAMPLE` times the video resolution. Needs the context current.
    pub fn new(arrow: &Arrow) -> Result<Self, String> {
        let gl = Gl::load()?;
        let (texels, width, height) = texture(arrow);
        unsafe {
            let saved = Saved::take(&gl);
            let program = link(&gl);
            let mut vao = 0;
            (gl.glGenVertexArrays)(1, &mut vao);
            let mut texture = 0;
            (gl.glGenTextures)(1, &mut texture);
            (gl.glBindTexture)(GL_TEXTURE_2D, texture);
            (gl.glTexParameteri)(
                GL_TEXTURE_2D,
                GL_TEXTURE_MIN_FILTER,
                GL_LINEAR_MIPMAP_NEAREST,
            );
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (gl.glTexImage2D)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8,
                width as i32,
                height as i32,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr().cast(),
            );
            (gl.glGenerateMipmap)(GL_TEXTURE_2D);
            let uniform =
                |program: u32, name: &CStr| (gl.glGetUniformLocation)(program, name.as_ptr());
            if let Ok(program) = program {
                let pad = SUPERSAMPLE as f32;
                (gl.glUseProgram)(program);
                (gl.glUniform1i)(uniform(program, c"sprite"), 0);
                (gl.glUniform2f)(
                    uniform(program, c"origin"),
                    pad / width as f32,
                    pad / height as f32,
                );
                (gl.glUniform2f)(
                    uniform(program, c"texel_scale"),
                    pad / width as f32,
                    pad / height as f32,
                );
            }
            saved.restore(&gl);
            let program = program?;
            let ss = f64::from(SUPERSAMPLE);
            Ok(Self {
                program,
                vao,
                texture,
                sprite: (f64::from(arrow.width) / ss, f64::from(arrow.height) / ss),
                rect: uniform(program, c"rect"),
                size: uniform(program, c"size"),
                taps: uniform(program, c"taps"),
                gl,
            })
        }
    }

    /// Blend the mean of the arrow at each tap (sprite top-left, video pixels)
    /// over the bound frame of `size`.
    pub fn draw(&self, taps: &[Option<(f64, f64)>; BLUR_SAMPLES], size: (u32, u32)) {
        let visible = || taps.iter().flatten();
        if visible().next().is_none() {
            return;
        }
        let limit = |v: f64| v.clamp(f64::from(HIDDEN) / 2.0, -f64::from(HIDDEN) / 2.0) as f32;
        // One pixel of margin for the area filter's footprint.
        let left = visible().map(|(x, _)| *x).fold(f64::MAX, f64::min).floor() - 1.0;
        let top = visible().map(|(_, y)| *y).fold(f64::MAX, f64::min).floor() - 1.0;
        let right =
            visible().map(|(x, _)| *x).fold(f64::MIN, f64::max).ceil() + self.sprite.0 + 1.0;
        let bottom =
            visible().map(|(_, y)| *y).fold(f64::MIN, f64::max).ceil() + self.sprite.1 + 1.0;
        let mut positions = [HIDDEN; 2 * BLUR_SAMPLES];
        for (slot, tap) in positions.chunks_exact_mut(2).zip(taps) {
            if let Some((x, y)) = tap {
                slot.copy_from_slice(&[limit(*x), limit(*y)]);
            }
        }
        let gl = &self.gl;
        unsafe {
            let saved = Saved::take(gl);
            (gl.glUseProgram)(self.program);
            (gl.glBindVertexArray)(self.vao);
            (gl.glBindTexture)(GL_TEXTURE_2D, self.texture);
            (gl.glUniform4f)(
                self.rect,
                limit(left),
                limit(top),
                limit(right),
                limit(bottom),
            );
            (gl.glUniform2f)(self.size, size.0 as f32, size.1 as f32);
            (gl.glUniform2fv)(self.taps, BLUR_SAMPLES as i32, positions.as_ptr());
            (gl.glEnable)(GL_BLEND);
            // Colour: straight-alpha over. Alpha: keep the frame's, which gsr's
            // YUV conversion blends with; below 1 it would mix in the previous frame.
            (gl.glBlendFuncSeparate)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
            (gl.glDrawArrays)(GL_TRIANGLE_STRIP, 0, 4);
            saved.restore(gl);
        }
    }
}

unsafe fn link(gl: &Gl) -> Result<u32, String> {
    unsafe {
        let compile = |kind, source: &CStr| {
            let shader = (gl.glCreateShader)(kind);
            (gl.glShaderSource)(shader, 1, &source.as_ptr(), std::ptr::null());
            (gl.glCompileShader)(shader);
            let mut ok = 0;
            (gl.glGetShaderiv)(shader, GL_COMPILE_STATUS, &mut ok);
            if ok == 0 {
                let log = info_log(|len, out| (gl.glGetShaderInfoLog)(shader, 1024, len, out));
                return Err(format!("cursor shader: {log}"));
            }
            Ok(shader)
        };
        let vertex = compile(GL_VERTEX_SHADER, VERTEX)?;
        let fragment = compile(GL_FRAGMENT_SHADER, FRAGMENT)?;
        let program = (gl.glCreateProgram)();
        (gl.glAttachShader)(program, vertex);
        (gl.glAttachShader)(program, fragment);
        (gl.glLinkProgram)(program);
        (gl.glDeleteShader)(vertex);
        (gl.glDeleteShader)(fragment);
        let mut ok = 0;
        (gl.glGetProgramiv)(program, GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let log = info_log(|len, out| (gl.glGetProgramInfoLog)(program, 1024, len, out));
            return Err(format!("cursor program: {log}"));
        }
        Ok(program)
    }
}

fn info_log(read: impl FnOnce(*mut i32, *mut c_char)) -> String {
    let mut buffer = [0u8; 1024];
    let mut len = 0;
    read(&mut len, buffer.as_mut_ptr().cast());
    String::from_utf8_lossy(&buffer[..len.clamp(0, 1024) as usize]).into_owned()
}
