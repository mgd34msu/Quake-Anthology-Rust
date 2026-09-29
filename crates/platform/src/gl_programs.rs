//! GL shader program ABI with single-string source indirection.
//!
//! Port of donor `src/platform/gl-programs.ts`: the renderer owns
//! program/shader objects and selects its context before raw symbol calls.
//! [`GlPrograms::shader_source`] validates and pins one NUL-terminated UTF-8
//! string per call with explicit buffer ownership.

use std::ffi::c_void;

use crate::error::{Error, Result};
use crate::ffi_util::c_string;
use crate::sdl_render_context::{ProcedureGuard, SdlRenderContext};

/// Resolved shader program entry points.
pub struct GlProgramSymbols {
    /// `glCreateShader`.
    pub gl_create_shader: unsafe extern "C" fn(u32) -> u32,
    /// `glShaderSource` (prefer [`GlPrograms::shader_source`]).
    pub gl_shader_source: unsafe extern "C" fn(u32, i32, *const *const u8, *const i32),
    /// `glCompileShader`.
    pub gl_compile_shader: unsafe extern "C" fn(u32),
    /// `glGetShaderiv`.
    pub gl_get_shaderiv: unsafe extern "C" fn(u32, u32, *mut i32),
    /// `glGetShaderInfoLog`.
    pub gl_get_shader_info_log: unsafe extern "C" fn(u32, i32, *mut i32, *mut u8),
    /// `glDeleteShader`.
    pub gl_delete_shader: unsafe extern "C" fn(u32),
    /// `glCreateProgram`.
    pub gl_create_program: unsafe extern "C" fn() -> u32,
    /// `glAttachShader`.
    pub gl_attach_shader: unsafe extern "C" fn(u32, u32),
    /// `glLinkProgram`.
    pub gl_link_program: unsafe extern "C" fn(u32),
    /// `glGetProgramiv`.
    pub gl_get_programiv: unsafe extern "C" fn(u32, u32, *mut i32),
    /// `glGetProgramInfoLog`.
    pub gl_get_program_info_log: unsafe extern "C" fn(u32, i32, *mut i32, *mut u8),
    /// `glDeleteProgram`.
    pub gl_delete_program: unsafe extern "C" fn(u32),
    /// `glUseProgram`.
    pub gl_use_program: unsafe extern "C" fn(u32),
    /// `glGetUniformLocation`.
    pub gl_get_uniform_location: unsafe extern "C" fn(u32, *const u8) -> i32,
    /// `glUniform1i`.
    pub gl_uniform_1i: unsafe extern "C" fn(i32, i32),
    /// `glUniform1f`.
    pub gl_uniform_1f: unsafe extern "C" fn(i32, f32),
    /// `glUniform3f`.
    pub gl_uniform_3f: unsafe extern "C" fn(i32, f32, f32, f32),
    /// `glUniform4f`.
    pub gl_uniform_4f: unsafe extern "C" fn(i32, f32, f32, f32, f32),
    /// `glUniformMatrix4fv`.
    pub gl_uniform_matrix_4fv: unsafe extern "C" fn(i32, i32, u8, *const f32),
}

/// C names resolved by [`GlPrograms::load`], in load order.
pub const GL_PROGRAM_SYMBOL_NAMES: &[&str] = &[
    "glCreateShader",
    "glShaderSource",
    "glCompileShader",
    "glGetShaderiv",
    "glGetShaderInfoLog",
    "glDeleteShader",
    "glCreateProgram",
    "glAttachShader",
    "glLinkProgram",
    "glGetProgramiv",
    "glGetProgramInfoLog",
    "glDeleteProgram",
    "glUseProgram",
    "glGetUniformLocation",
    "glUniform1i",
    "glUniform1f",
    "glUniform3f",
    "glUniform4f",
    "glUniformMatrix4fv",
];

/// Owned shader procedure table with a context procedure lease.
pub struct GlPrograms {
    /// Resolved symbols.
    pub symbols: GlProgramSymbols,
    guard: Option<ProcedureGuard>,
    closed: bool,
}

impl GlPrograms {
    /// Resolve every symbol through `context`, retaining procedures.
    /// Requires a 64-bit pointer ABI for shader source indirection.
    pub fn load(context: &mut dyn SdlRenderContext) -> Result<Self> {
        #[cfg(not(target_pointer_width = "64"))]
        {
            let _ = context;
            return Err(Error::Unsupported(
                "GL shader source requires a 64-bit pointer ABI".to_string(),
            ));
        }
        #[cfg(target_pointer_width = "64")]
        {
            macro_rules! proc {
                ($name:literal, $sig:ty) => {{
                    let address = context
                        .get_gl_proc_address($name)
                        .map_err(|error| Error::unavailable("gl", format!("{}: {error}", $name)))?;
                    // SAFETY: the address is a live GL entry point with the
                    // signature declared here (Khronos gl.xml).
                    unsafe { std::mem::transmute::<*mut c_void, $sig>(address) }
                }};
            }
            let symbols = GlProgramSymbols {
                gl_create_shader: proc!("glCreateShader", unsafe extern "C" fn(u32) -> u32),
                gl_shader_source: proc!(
                    "glShaderSource",
                    unsafe extern "C" fn(u32, i32, *const *const u8, *const i32)
                ),
                gl_compile_shader: proc!("glCompileShader", unsafe extern "C" fn(u32)),
                gl_get_shaderiv: proc!("glGetShaderiv", unsafe extern "C" fn(u32, u32, *mut i32)),
                gl_get_shader_info_log: proc!("glGetShaderInfoLog", unsafe extern "C" fn(u32, i32, *mut i32, *mut u8)),
                gl_delete_shader: proc!("glDeleteShader", unsafe extern "C" fn(u32)),
                gl_create_program: proc!("glCreateProgram", unsafe extern "C" fn() -> u32),
                gl_attach_shader: proc!("glAttachShader", unsafe extern "C" fn(u32, u32)),
                gl_link_program: proc!("glLinkProgram", unsafe extern "C" fn(u32)),
                gl_get_programiv: proc!("glGetProgramiv", unsafe extern "C" fn(u32, u32, *mut i32)),
                gl_get_program_info_log: proc!(
                    "glGetProgramInfoLog",
                    unsafe extern "C" fn(u32, i32, *mut i32, *mut u8)
                ),
                gl_delete_program: proc!("glDeleteProgram", unsafe extern "C" fn(u32)),
                gl_use_program: proc!("glUseProgram", unsafe extern "C" fn(u32)),
                gl_get_uniform_location: proc!("glGetUniformLocation", unsafe extern "C" fn(u32, *const u8) -> i32),
                gl_uniform_1i: proc!("glUniform1i", unsafe extern "C" fn(i32, i32)),
                gl_uniform_1f: proc!("glUniform1f", unsafe extern "C" fn(i32, f32)),
                gl_uniform_3f: proc!("glUniform3f", unsafe extern "C" fn(i32, f32, f32, f32)),
                gl_uniform_4f: proc!("glUniform4f", unsafe extern "C" fn(i32, f32, f32, f32, f32)),
                gl_uniform_matrix_4fv: proc!("glUniformMatrix4fv", unsafe extern "C" fn(i32, i32, u8, *const f32)),
            };
            let guard = context
                .retain_procedures()
                .map_err(|error| Error::unavailable("gl", format!("retain procedures: {error}")))?;
            Ok(Self {
                symbols,
                guard: Some(guard),
                closed: false,
            })
        }
    }

    /// Upload one shader string: validates the handle and source, selects the
    /// context, and pins the NUL-terminated bytes for exactly the call.
    pub fn shader_source(&mut self, context: &mut dyn SdlRenderContext, shader: u32, source: &str) -> Result<()> {
        if self.closed {
            return Err(Error::Closed("GL program procedure table".to_string()));
        }
        if shader == 0 {
            return Err(Error::OutOfRange("GL shader must be a nonzero uint32".to_string()));
        }
        if source.contains('\0') {
            return Err(Error::InvalidInput("GL shader source contains NUL".to_string()));
        }
        if source.len() > i32::MAX as usize {
            return Err(Error::OutOfRange(
                "GL shader source exceeds signed 32-bit length".to_string(),
            ));
        }
        let bytes = c_string(source)?;
        let pointers = [bytes.as_ptr()];
        let lengths = [source.len() as i32];
        context.make_current()?;
        // SAFETY: the context is current; the pinned buffers outlive the call.
        unsafe {
            (self.symbols.gl_shader_source)(shader, 1, pointers.as_ptr(), lengths.as_ptr());
        }
        Ok(())
    }

    /// Release the procedure lease. Idempotent.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.guard.take();
    }

    /// Whether the table is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for GlPrograms {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::FakeContext;

    #[test]
    fn loads_symbols_in_order() {
        let mut context = FakeContext::with_names(GL_PROGRAM_SYMBOL_NAMES);
        let mut table = GlPrograms::load(&mut context).unwrap();
        assert_eq!(context.requested, GL_PROGRAM_SYMBOL_NAMES);
        assert!(!table.is_closed());
        table.close();
        assert!(table.is_closed());
    }

    #[test]
    fn shader_source_validates_before_touching_gl() {
        let mut context = FakeContext::with_names(GL_PROGRAM_SYMBOL_NAMES);
        let mut table = GlPrograms::load(&mut context).unwrap();
        assert!(table.shader_source(&mut context, 0, "void main(){}").is_err());
        assert!(table.shader_source(&mut context, 1, "bad\0source").is_err());
        table.close();
        assert!(table.shader_source(&mut context, 1, "void main(){}").is_err());
    }

    #[test]
    fn missing_symbol_names_gl() {
        let mut context = FakeContext::with_names(&["glCreateShader"]);
        let Err(error) = GlPrograms::load(&mut context) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("glShaderSource"), "{error}");
    }
}
