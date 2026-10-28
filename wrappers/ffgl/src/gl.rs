//! The few OpenGL calls the plugin makes, in the host's GL 4.1 core context. Linked
//! from macOS's OpenGL framework, which exports core-profile functions directly.
//!
//! Everything here runs on the host's GL thread with its context current, and leaves
//! the state FFGL hosts expect: no program, vertex array or texture bound, and texture
//! unit 0 active.

#![allow(clippy::upper_case_acronyms)]

use std::ffi::{CString, c_char, c_void};

pub type GLenum = u32;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLsizei = i32;

pub const TEXTURE_2D: GLenum = 0x0DE1;
pub const TEXTURE0: GLenum = 0x84C0;
pub const RGBA: GLenum = 0x1908;
pub const RGBA32F: GLint = 0x8814;
pub const FLOAT: GLenum = 0x1406;
pub const TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const TEXTURE_WRAP_S: GLenum = 0x2802;
pub const TEXTURE_WRAP_T: GLenum = 0x2803;
pub const NEAREST: GLint = 0x2600;
pub const CLAMP_TO_EDGE: GLint = 0x812F;
pub const PACK_ALIGNMENT: GLenum = 0x0D05;
pub const UNPACK_ALIGNMENT: GLenum = 0x0CF5;
pub const TRIANGLES: GLenum = 0x0004;
pub const VERTEX_SHADER: GLenum = 0x8B31;
pub const FRAGMENT_SHADER: GLenum = 0x8B30;
pub const COMPILE_STATUS: GLenum = 0x8B81;
pub const LINK_STATUS: GLenum = 0x8B82;
pub const TEXTURE_RECTANGLE: GLenum = 0x84F5;
pub const FRAMEBUFFER: GLenum = 0x8D40;
pub const COLOR_ATTACHMENT0: GLenum = 0x8CE0;
pub const FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
pub const VIEWPORT: GLenum = 0x0BA2;

#[cfg_attr(target_os = "macos", link(name = "OpenGL", kind = "framework"))]
unsafe extern "C" {
    fn glGenTextures(n: GLsizei, textures: *mut GLuint);
    fn glDeleteTextures(n: GLsizei, textures: *const GLuint);
    fn glBindTexture(target: GLenum, texture: GLuint);
    fn glActiveTexture(texture: GLenum);
    fn glTexParameteri(target: GLenum, name: GLenum, value: GLint);
    fn glTexImage2D(
        target: GLenum,
        level: GLint,
        internal: GLint,
        w: GLsizei,
        h: GLsizei,
        border: GLint,
        format: GLenum,
        kind: GLenum,
        data: *const c_void,
    );
    fn glTexSubImage2D(
        target: GLenum,
        level: GLint,
        x: GLint,
        y: GLint,
        w: GLsizei,
        h: GLsizei,
        format: GLenum,
        kind: GLenum,
        data: *const c_void,
    );
    fn glGetTexImage(target: GLenum, level: GLint, format: GLenum, kind: GLenum, data: *mut c_void);
    fn glPixelStorei(name: GLenum, value: GLint);
    fn glCreateShader(kind: GLenum) -> GLuint;
    fn glShaderSource(
        shader: GLuint,
        count: GLsizei,
        sources: *const *const c_char,
        lengths: *const GLint,
    );
    fn glCompileShader(shader: GLuint);
    fn glGetShaderiv(shader: GLuint, name: GLenum, value: *mut GLint);
    fn glDeleteShader(shader: GLuint);
    fn glCreateProgram() -> GLuint;
    fn glAttachShader(program: GLuint, shader: GLuint);
    fn glLinkProgram(program: GLuint);
    fn glGetProgramiv(program: GLuint, name: GLenum, value: *mut GLint);
    fn glDeleteProgram(program: GLuint);
    fn glUseProgram(program: GLuint);
    fn glGetUniformLocation(program: GLuint, name: *const c_char) -> GLint;
    fn glUniform1i(location: GLint, value: GLint);
    fn glUniform2f(location: GLint, x: f32, y: f32);
    fn glGenVertexArrays(n: GLsizei, arrays: *mut GLuint);
    fn glDeleteVertexArrays(n: GLsizei, arrays: *const GLuint);
    fn glBindVertexArray(array: GLuint);
    fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei);
    fn glGenFramebuffers(n: GLsizei, fbos: *mut GLuint);
    fn glDeleteFramebuffers(n: GLsizei, fbos: *const GLuint);
    fn glBindFramebuffer(target: GLenum, fbo: GLuint);
    fn glFramebufferTexture2D(
        target: GLenum,
        attachment: GLenum,
        textarget: GLenum,
        texture: GLuint,
        level: GLint,
    );
    fn glCheckFramebufferStatus(target: GLenum) -> GLenum;
    fn glGetIntegerv(name: GLenum, data: *mut GLint);
    fn glViewport(x: GLint, y: GLint, w: GLsizei, h: GLsizei);
    fn glFinish();
}

const VERTEX: &str = "#version 410 core
uniform vec2 max_uv;
out vec2 uv;
void main() {
    // One triangle covering the viewport; no vertex buffer needed.
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    uv = p * max_uv;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Draws a texture as it is.
pub const PASS: &str = "#version 410 core
uniform sampler2D image;
in vec2 uv;
out vec4 color;
void main() {
    color = texture(image, uv);
}
";

/// An FFGL texture (premultiplied, bottom-up) to a pipeline canvas (straight, top-down).
pub const TO_CANVAS: &str = "#version 410 core
uniform sampler2D image;
uniform vec2 max_uv;
in vec2 uv;
out vec4 color;
void main() {
    vec4 c = texture(image, vec2(uv.x, max_uv.y - uv.y));
    color = c.a > 0.0 ? vec4(c.rgb / c.a, c.a) : vec4(0.0);
}
";

/// A pipeline canvas in a rectangle texture back to a premultiplied, bottom-up image.
pub const FROM_CANVAS: &str = "#version 410 core
uniform sampler2DRect image;
uniform vec2 max_uv;
in vec2 uv;
out vec4 color;
void main() {
    vec4 c = texture(image, vec2(uv.x, max_uv.y - uv.y));
    color = vec4(c.rgb * c.a, c.a);
}
";

/// Draws a texture over the bound framebuffer's viewport.
pub struct Blit {
    program: GLuint,
    vao: GLuint,
    max_uv: GLint,
}

impl Blit {
    /// # Safety
    /// The host's GL context must be current.
    pub unsafe fn new(fragment: &str) -> Result<Self, String> {
        // SAFETY: the caller has the context current.
        unsafe {
            let vs = compile(VERTEX_SHADER, VERTEX)?;
            let fs = match compile(FRAGMENT_SHADER, fragment) {
                Ok(fs) => fs,
                Err(e) => {
                    glDeleteShader(vs);
                    return Err(e);
                }
            };
            let program = glCreateProgram();
            glAttachShader(program, vs);
            glAttachShader(program, fs);
            glLinkProgram(program);
            glDeleteShader(vs);
            glDeleteShader(fs);
            let mut ok = 0;
            glGetProgramiv(program, LINK_STATUS, &mut ok);
            if ok == 0 {
                glDeleteProgram(program);
                return Err("blit program failed to link".into());
            }
            let mut vao = 0;
            glGenVertexArrays(1, &mut vao);
            glUseProgram(program);
            glUniform1i(glGetUniformLocation(program, c"image".as_ptr()), 0);
            let max_uv = glGetUniformLocation(program, c"max_uv".as_ptr());
            glUseProgram(0);
            Ok(Self {
                program,
                vao,
                max_uv,
            })
        }
    }

    /// Draws `texture` (a `target` texture) over the viewport. `max_uv` is the image's
    /// far corner in texture coordinates: content / allocated size for 2D textures,
    /// pixels for rectangle textures.
    ///
    /// # Safety
    /// The host's GL context must be current and `texture` a `target` texture in it.
    pub unsafe fn draw(&self, texture: GLuint, target: GLenum, max_uv: [f32; 2]) {
        // SAFETY: the caller has the context current.
        unsafe {
            glUseProgram(self.program);
            glUniform2f(self.max_uv, max_uv[0], max_uv[1]);
            glActiveTexture(TEXTURE0);
            glBindTexture(target, texture);
            glBindVertexArray(self.vao);
            glDrawArrays(TRIANGLES, 0, 3);
            glBindVertexArray(0);
            glBindTexture(target, 0);
            glUseProgram(0);
        }
    }

    /// # Safety
    /// The context the blit was made in must be current.
    pub unsafe fn delete(&self) {
        // SAFETY: the caller has the context current.
        unsafe {
            glDeleteVertexArrays(1, &self.vao);
            glDeleteProgram(self.program);
        }
    }
}

unsafe fn compile(kind: GLenum, source: &str) -> Result<GLuint, String> {
    let source = CString::new(source).map_err(|e| e.to_string())?;
    // SAFETY: the caller has the context current; the source outlives the call.
    unsafe {
        let shader = glCreateShader(kind);
        glShaderSource(shader, 1, &source.as_ptr(), std::ptr::null());
        glCompileShader(shader);
        let mut ok = 0;
        glGetShaderiv(shader, COMPILE_STATUS, &mut ok);
        if ok == 0 {
            glDeleteShader(shader);
            return Err("blit shader failed to compile".into());
        }
        Ok(shader)
    }
}

/// Reads a whole 2D texture (its allocated size) as RGBA f32, rows bottom-up.
///
/// # Safety
/// The context must be current and `texture` a 2D texture of `size` texels.
pub unsafe fn read_texture(texture: GLuint, size: (u32, u32)) -> Vec<f32> {
    let mut pixels = vec![0.0f32; size.0 as usize * size.1 as usize * 4];
    // SAFETY: the buffer holds the whole level-0 image in RGBA f32.
    unsafe {
        glBindTexture(TEXTURE_2D, texture);
        glPixelStorei(PACK_ALIGNMENT, 1);
        glGetTexImage(TEXTURE_2D, 0, RGBA, FLOAT, pixels.as_mut_ptr().cast());
        glPixelStorei(PACK_ALIGNMENT, 4);
        glBindTexture(TEXTURE_2D, 0);
    }
    pixels
}

/// An RGBA32F texture the plugin owns, resized on demand.
pub struct Texture {
    pub handle: GLuint,
    size: (u32, u32),
}

impl Texture {
    pub fn new() -> Self {
        Self {
            handle: 0,
            size: (0, 0),
        }
    }

    /// Uploads RGBA f32 pixels, rows bottom-up, reallocating if the size changed.
    ///
    /// # Safety
    /// The context must be current; `pixels` holds `size` RGBA texels.
    pub unsafe fn upload(&mut self, size: (u32, u32), pixels: &[f32]) {
        // SAFETY: the caller has the context current and passes a full image.
        unsafe {
            if self.handle == 0 {
                glGenTextures(1, &mut self.handle);
                glBindTexture(TEXTURE_2D, self.handle);
                glTexParameteri(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST);
                glTexParameteri(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST);
                glTexParameteri(TEXTURE_2D, TEXTURE_WRAP_S, CLAMP_TO_EDGE);
                glTexParameteri(TEXTURE_2D, TEXTURE_WRAP_T, CLAMP_TO_EDGE);
            } else {
                glBindTexture(TEXTURE_2D, self.handle);
            }
            glPixelStorei(UNPACK_ALIGNMENT, 1);
            let (w, h) = (size.0 as GLsizei, size.1 as GLsizei);
            if self.size != size {
                glTexImage2D(
                    TEXTURE_2D,
                    0,
                    RGBA32F,
                    w,
                    h,
                    0,
                    RGBA,
                    FLOAT,
                    pixels.as_ptr().cast(),
                );
                self.size = size;
            } else {
                glTexSubImage2D(
                    TEXTURE_2D,
                    0,
                    0,
                    0,
                    w,
                    h,
                    RGBA,
                    FLOAT,
                    pixels.as_ptr().cast(),
                );
            }
            glPixelStorei(UNPACK_ALIGNMENT, 4);
            glBindTexture(TEXTURE_2D, 0);
        }
    }

    /// # Safety
    /// The context the texture was made in must be current.
    pub unsafe fn delete(&mut self) {
        if self.handle != 0 {
            // SAFETY: the caller has the context current.
            unsafe { glDeleteTextures(1, &self.handle) };
            self.handle = 0;
            self.size = (0, 0);
        }
    }
}

/// A framebuffer drawing into a rectangle texture, for rendering into an IOSurface.
pub struct Target {
    fbo: GLuint,
    size: (u32, u32),
}

impl Target {
    /// # Safety
    /// The context must be current and `texture` a rectangle texture of `size`.
    pub unsafe fn new(texture: GLuint, size: (u32, u32)) -> Result<Self, String> {
        // SAFETY: the caller has the context current. The host's framebuffer binding is
        // restored by the caller after drawing, as FFGL requires.
        unsafe {
            let mut previous = 0;
            glGetIntegerv(0x8CA6, &mut previous); // GL_FRAMEBUFFER_BINDING
            let mut fbo = 0;
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(FRAMEBUFFER, fbo);
            glFramebufferTexture2D(
                FRAMEBUFFER,
                COLOR_ATTACHMENT0,
                TEXTURE_RECTANGLE,
                texture,
                0,
            );
            let status = glCheckFramebufferStatus(FRAMEBUFFER);
            glBindFramebuffer(FRAMEBUFFER, previous as GLuint);
            if status != FRAMEBUFFER_COMPLETE {
                glDeleteFramebuffers(1, &fbo);
                return Err(format!("IOSurface framebuffer incomplete (0x{status:x})"));
            }
            Ok(Self { fbo, size })
        }
    }

    /// Runs `draw` with this target bound and its full size as the viewport, then puts
    /// back the host's framebuffer and viewport.
    ///
    /// # Safety
    /// The context must be current; `host_fbo` is the framebuffer to restore.
    pub unsafe fn draw_into(&self, host_fbo: GLuint, draw: impl FnOnce()) {
        // SAFETY: the caller has the context current.
        unsafe {
            let mut viewport = [0; 4];
            glGetIntegerv(VIEWPORT, viewport.as_mut_ptr());
            glBindFramebuffer(FRAMEBUFFER, self.fbo);
            glViewport(0, 0, self.size.0 as GLsizei, self.size.1 as GLsizei);
            draw();
            glBindFramebuffer(FRAMEBUFFER, host_fbo);
            glViewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        }
    }

    /// # Safety
    /// The context the target was made in must be current.
    pub unsafe fn delete(&self) {
        // SAFETY: the caller has the context current.
        unsafe { glDeleteFramebuffers(1, &self.fbo) };
    }
}

/// Waits for the context's queued GL work to finish, so another API can read its output.
///
/// # Safety
/// The context must be current.
pub unsafe fn finish() {
    // SAFETY: the caller has the context current.
    unsafe { glFinish() };
}

/// Generates a texture name.
///
/// # Safety
/// The context must be current.
pub unsafe fn gen_texture() -> GLuint {
    let mut t = 0;
    // SAFETY: the caller has the context current.
    unsafe { glGenTextures(1, &mut t) };
    t
}

/// # Safety
/// The context the texture was made in must be current.
pub unsafe fn delete_texture(texture: GLuint) {
    // SAFETY: the caller has the context current.
    unsafe { glDeleteTextures(1, &texture) };
}

/// Sets nearest filtering on a bound-and-unbound rectangle texture.
///
/// # Safety
/// The context must be current and `texture` a rectangle texture.
pub unsafe fn nearest_rectangle(texture: GLuint) {
    // SAFETY: the caller has the context current.
    unsafe {
        glBindTexture(TEXTURE_RECTANGLE, texture);
        glTexParameteri(TEXTURE_RECTANGLE, TEXTURE_MIN_FILTER, NEAREST);
        glTexParameteri(TEXTURE_RECTANGLE, TEXTURE_MAG_FILTER, NEAREST);
        glTexParameteri(TEXTURE_RECTANGLE, TEXTURE_WRAP_S, CLAMP_TO_EDGE);
        glTexParameteri(TEXTURE_RECTANGLE, TEXTURE_WRAP_T, CLAMP_TO_EDGE);
        glBindTexture(TEXTURE_RECTANGLE, 0);
    }
}

/// Binds (or with 0 unbinds) a rectangle texture on the active unit.
///
/// # Safety
/// The context must be current.
pub unsafe fn bind_rectangle(texture: GLuint) {
    // SAFETY: the caller has the context current.
    unsafe { glBindTexture(TEXTURE_RECTANGLE, texture) };
}
