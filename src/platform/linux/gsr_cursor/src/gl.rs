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
const GL_NEAREST: i32 = 0x2600;
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
    glUniform2iv: fn(i32, i32, *const i32);
    glGenVertexArrays: fn(i32, *mut u32);
    glBindVertexArray: fn(u32);
    glGenTextures: fn(i32, *mut u32);
    glActiveTexture: fn(u32);
    glBindTexture: fn(u32, u32);
    glTexParameteri: fn(u32, u32, i32);
    glTexImage2D: fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
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
/// top image row, so `gl_FragCoord` is already in image pixels. Output is
/// straight alpha: the averaged premultiplied colour divided by coverage.
const FRAGMENT: &CStr = c"#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D sprite;
uniform ivec2 taps[8];
out vec4 color;
void main() {
    ivec2 pixel = ivec2(gl_FragCoord.xy);
    ivec2 extent = textureSize(sprite, 0);
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 8; i++) {
        ivec2 at = pixel - taps[i];
        if (all(greaterThanEqual(at, ivec2(0))) && all(lessThan(at, extent))) {
            vec4 texel = texelFetch(sprite, at, 0);
            sum += vec4(texel.rgb * texel.a, texel.a);
        }
    }
    if (sum.a <= 0.0) discard;
    color = vec4(sum.rgb / sum.a, sum.a / 8.0);
}
";

/// Tap position that never covers a pixel.
const HIDDEN: i32 = -(1 << 24);

pub struct Renderer {
    gl: Gl,
    program: u32,
    vao: u32,
    texture: u32,
    sprite: (u32, u32),
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
    /// Load GL, build the program and upload the arrow. Needs the context current.
    pub fn new(arrow: &Arrow) -> Result<Self, String> {
        let gl = Gl::load()?;
        unsafe {
            let saved = Saved::take(&gl);
            let program = link(&gl);
            let mut vao = 0;
            (gl.glGenVertexArrays)(1, &mut vao);
            let mut texture = 0;
            (gl.glGenTextures)(1, &mut texture);
            (gl.glBindTexture)(GL_TEXTURE_2D, texture);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (gl.glTexImage2D)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8,
                arrow.width as i32,
                arrow.height as i32,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                arrow.rgba.as_ptr().cast(),
            );
            let uniform =
                |program: u32, name: &CStr| (gl.glGetUniformLocation)(program, name.as_ptr());
            if let Ok(program) = program {
                (gl.glUseProgram)(program);
                (gl.glUniform1i)(uniform(program, c"sprite"), 0);
            }
            saved.restore(&gl);
            let program = program?;
            Ok(Self {
                program,
                vao,
                texture,
                sprite: (arrow.width, arrow.height),
                rect: uniform(program, c"rect"),
                size: uniform(program, c"size"),
                taps: uniform(program, c"taps"),
                gl,
            })
        }
    }

    /// Blend the mean of the arrow at each tap (sprite top-left, video pixels)
    /// over the bound frame of `size`.
    pub fn draw(&self, taps: &[Option<(i64, i64)>; BLUR_SAMPLES], size: (u32, u32)) {
        let visible = || taps.iter().flatten();
        let (Some(left), Some(top)) = (
            visible().map(|(x, _)| *x).min(),
            visible().map(|(_, y)| *y).min(),
        ) else {
            return;
        };
        let right = visible().map(|(x, _)| *x).max().unwrap_or(left) + i64::from(self.sprite.0);
        let bottom = visible().map(|(_, y)| *y).max().unwrap_or(top) + i64::from(self.sprite.1);
        let limit = |v: i64| v.clamp(i64::from(HIDDEN) / 2, -i64::from(HIDDEN) / 2) as i32;
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
                limit(left) as f32,
                limit(top) as f32,
                limit(right) as f32,
                limit(bottom) as f32,
            );
            (gl.glUniform2f)(self.size, size.0 as f32, size.1 as f32);
            (gl.glUniform2iv)(self.taps, BLUR_SAMPLES as i32, positions.as_ptr());
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
