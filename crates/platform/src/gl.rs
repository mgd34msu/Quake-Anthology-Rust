//! Typed system OpenGL entry points resolved through the SDL context.
//!
//! Port of donor `src/platform/gl.ts` (replacing `code/unix/linux_qgl.c`
//! loading). Each table resolves its symbols through the owning
//! [`SdlRenderContext`](crate::sdl_render_context::SdlRenderContext) and holds
//! a procedure lease until closed; the renderer selects its context before
//! raw symbol calls.

use std::ffi::c_void;

use crate::error::{Error, Result};
use crate::sdl_render_context::{ProcedureGuard, SdlRenderContext};

macro_rules! gl_table {
    ($table:ident, $symbols:ident, $($field:ident : $cname:literal : $sig:ty;)*) => {
        /// Resolved GL entry points. The caller selects the owning context
        /// before invoking any raw symbol.
        pub struct $symbols {
            $(pub $field: $sig,)*
        }

        /// Owned procedure table with a context procedure lease.
        pub struct $table {
            /// Resolved symbols.
            pub symbols: $symbols,
            guard: Option<ProcedureGuard>,
            closed: bool,
        }

        impl $table {
            /// Resolve every symbol through `context`, retaining procedures.
            pub fn load(context: &mut dyn SdlRenderContext) -> Result<Self> {
                $(let $field: $sig = {
                    let address = context.get_gl_proc_address($cname).map_err(|error| {
                        Error::unavailable("gl", format!("{}: {error}", $cname))
                    })?;
                    // SAFETY: the address is a live GL entry point with the
                    // signature declared here (Khronos gl.xml).
                    unsafe { std::mem::transmute::<*mut c_void, $sig>(address) }
                };)*
                let guard = context.retain_procedures().map_err(|error| {
                    Error::unavailable("gl", format!("retain procedures: {error}"))
                })?;
                Ok(Self {
                    symbols: $symbols { $($field,)* },
                    guard: Some(guard),
                    closed: false,
                })
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

        impl Drop for $table {
            fn drop(&mut self) {
                self.close();
            }
        }
    };
}

gl_table! {
    Gl, GlSymbols,
    gl_call_list: "glCallList": unsafe extern "C" fn(u32);
    gl_new_list: "glNewList": unsafe extern "C" fn(u32, u32);
    gl_end_list: "glEndList": unsafe extern "C" fn();
    gl_delete_lists: "glDeleteLists": unsafe extern "C" fn(u32, i32);
    gl_get_string: "glGetString": unsafe extern "C" fn(u32) -> *const u8;
    gl_get_error: "glGetError": unsafe extern "C" fn() -> u32;
    gl_hint: "glHint": unsafe extern "C" fn(u32, u32);
    gl_is_enabled: "glIsEnabled": unsafe extern "C" fn(u32) -> u8;
    gl_get_integerv: "glGetIntegerv": unsafe extern "C" fn(u32, *mut i32);
    gl_get_floatv: "glGetFloatv": unsafe extern "C" fn(u32, *mut f32);
    gl_get_tex_parameterfv: "glGetTexParameterfv": unsafe extern "C" fn(u32, u32, *mut f32);
    gl_get_tex_level_parameteriv: "glGetTexLevelParameteriv": unsafe extern "C" fn(u32, i32, u32, *mut i32);
    gl_get_tex_image: "glGetTexImage": unsafe extern "C" fn(u32, i32, u32, u32, *mut c_void);
    gl_active_texture: "glActiveTexture": unsafe extern "C" fn(u32);
    gl_client_active_texture: "glClientActiveTexture": unsafe extern "C" fn(u32);
    gl_draw_buffer: "glDrawBuffer": unsafe extern "C" fn(u32);
    gl_viewport: "glViewport": unsafe extern "C" fn(i32, i32, i32, i32);
    gl_scissor: "glScissor": unsafe extern "C" fn(i32, i32, i32, i32);
    gl_clear_color: "glClearColor": unsafe extern "C" fn(f32, f32, f32, f32);
    gl_clear_depth: "glClearDepth": unsafe extern "C" fn(f64);
    gl_clear_stencil: "glClearStencil": unsafe extern "C" fn(i32);
    gl_clear: "glClear": unsafe extern "C" fn(u32);
    gl_enable: "glEnable": unsafe extern "C" fn(u32);
    gl_disable: "glDisable": unsafe extern "C" fn(u32);
    gl_clip_plane: "glClipPlane": unsafe extern "C" fn(u32, *const f64);
    gl_depth_func: "glDepthFunc": unsafe extern "C" fn(u32);
    gl_depth_mask: "glDepthMask": unsafe extern "C" fn(u8);
    gl_color_mask: "glColorMask": unsafe extern "C" fn(u8, u8, u8, u8);
    gl_stencil_func: "glStencilFunc": unsafe extern "C" fn(u32, i32, u32);
    gl_stencil_op: "glStencilOp": unsafe extern "C" fn(u32, u32, u32);
    gl_stencil_mask: "glStencilMask": unsafe extern "C" fn(u32);
    gl_depth_range: "glDepthRange": unsafe extern "C" fn(f64, f64);
    gl_polygon_mode: "glPolygonMode": unsafe extern "C" fn(u32, u32);
    gl_shade_model: "glShadeModel": unsafe extern "C" fn(u32);
    gl_polygon_offset: "glPolygonOffset": unsafe extern "C" fn(f32, f32);
    gl_line_width: "glLineWidth": unsafe extern "C" fn(f32);
    gl_blend_func: "glBlendFunc": unsafe extern "C" fn(u32, u32);
    gl_alpha_func: "glAlphaFunc": unsafe extern "C" fn(u32, f32);
    gl_cull_face: "glCullFace": unsafe extern "C" fn(u32);
    gl_front_face: "glFrontFace": unsafe extern "C" fn(u32);
    gl_matrix_mode: "glMatrixMode": unsafe extern "C" fn(u32);
    gl_load_identity: "glLoadIdentity": unsafe extern "C" fn();
    gl_ortho: "glOrtho": unsafe extern "C" fn(f64, f64, f64, f64, f64, f64);
    gl_begin: "glBegin": unsafe extern "C" fn(u32);
    gl_end: "glEnd": unsafe extern "C" fn();
    gl_color_3f: "glColor3f": unsafe extern "C" fn(f32, f32, f32);
    gl_color_4f: "glColor4f": unsafe extern "C" fn(f32, f32, f32, f32);
    gl_color_4b: "glColor4b": unsafe extern "C" fn(i8, i8, i8, i8);
    gl_color_4ub: "glColor4ub": unsafe extern "C" fn(u8, u8, u8, u8);
    gl_tex_coord_2f: "glTexCoord2f": unsafe extern "C" fn(f32, f32);
    gl_vertex_2f: "glVertex2f": unsafe extern "C" fn(f32, f32);
    gl_vertex_4f: "glVertex4f": unsafe extern "C" fn(f32, f32, f32, f32);
    gl_enable_client_state: "glEnableClientState": unsafe extern "C" fn(u32);
    gl_disable_client_state: "glDisableClientState": unsafe extern "C" fn(u32);
    gl_push_client_attrib: "glPushClientAttrib": unsafe extern "C" fn(u32);
    gl_pop_client_attrib: "glPopClientAttrib": unsafe extern "C" fn();
    gl_bind_buffer: "glBindBuffer": unsafe extern "C" fn(u32, u32);
    gl_gen_buffers: "glGenBuffers": unsafe extern "C" fn(i32, *mut u32);
    gl_delete_buffers: "glDeleteBuffers": unsafe extern "C" fn(i32, *const u32);
    gl_get_pointerv: "glGetPointerv": unsafe extern "C" fn(u32, *mut *mut c_void);
    gl_vertex_pointer: "glVertexPointer": unsafe extern "C" fn(i32, u32, i32, *const c_void);
    gl_color_pointer: "glColorPointer": unsafe extern "C" fn(i32, u32, i32, *const c_void);
    gl_tex_coord_pointer: "glTexCoordPointer": unsafe extern "C" fn(i32, u32, i32, *const c_void);
    gl_draw_elements: "glDrawElements": unsafe extern "C" fn(u32, i32, u32, *const c_void);
    gl_array_element: "glArrayElement": unsafe extern "C" fn(i32);
    gl_gen_textures: "glGenTextures": unsafe extern "C" fn(i32, *mut u32);
    gl_delete_textures: "glDeleteTextures": unsafe extern "C" fn(i32, *const u32);
    gl_bind_texture: "glBindTexture": unsafe extern "C" fn(u32, u32);
    gl_tex_parameteri: "glTexParameteri": unsafe extern "C" fn(u32, u32, i32);
    gl_tex_parameterfv: "glTexParameterfv": unsafe extern "C" fn(u32, u32, *const f32);
    gl_tex_envi: "glTexEnvi": unsafe extern "C" fn(u32, u32, i32);
    gl_tex_envf: "glTexEnvf": unsafe extern "C" fn(u32, u32, f32);
    gl_tex_image_2d: "glTexImage2D": unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    gl_copy_tex_image_2d: "glCopyTexImage2D": unsafe extern "C" fn(u32, i32, u32, i32, i32, i32, i32, i32);
    gl_tex_sub_image_2d: "glTexSubImage2D": unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    gl_finish: "glFinish": unsafe extern "C" fn();
    gl_pixel_storei: "glPixelStorei": unsafe extern "C" fn(u32, i32);
    gl_read_pixels: "glReadPixels": unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
}

gl_table! {
    GlCompiledVertexArrays, GlCompiledVertexArraysSymbols,
    gl_lock_arrays_ext: "glLockArraysEXT": unsafe extern "C" fn(i32, i32);
    gl_unlock_arrays_ext: "glUnlockArraysEXT": unsafe extern "C" fn();
}

/// C names resolved by [`Gl::load`], in load order.
pub const GL_SYMBOL_NAMES: &[&str] = &[
    "glCallList",
    "glNewList",
    "glEndList",
    "glDeleteLists",
    "glGetString",
    "glGetError",
    "glHint",
    "glIsEnabled",
    "glGetIntegerv",
    "glGetFloatv",
    "glGetTexParameterfv",
    "glGetTexLevelParameteriv",
    "glGetTexImage",
    "glActiveTexture",
    "glClientActiveTexture",
    "glDrawBuffer",
    "glViewport",
    "glScissor",
    "glClearColor",
    "glClearDepth",
    "glClearStencil",
    "glClear",
    "glEnable",
    "glDisable",
    "glClipPlane",
    "glDepthFunc",
    "glDepthMask",
    "glColorMask",
    "glStencilFunc",
    "glStencilOp",
    "glStencilMask",
    "glDepthRange",
    "glPolygonMode",
    "glShadeModel",
    "glPolygonOffset",
    "glLineWidth",
    "glBlendFunc",
    "glAlphaFunc",
    "glCullFace",
    "glFrontFace",
    "glMatrixMode",
    "glLoadIdentity",
    "glOrtho",
    "glBegin",
    "glEnd",
    "glColor3f",
    "glColor4f",
    "glColor4b",
    "glColor4ub",
    "glTexCoord2f",
    "glVertex2f",
    "glVertex4f",
    "glEnableClientState",
    "glDisableClientState",
    "glPushClientAttrib",
    "glPopClientAttrib",
    "glBindBuffer",
    "glGenBuffers",
    "glDeleteBuffers",
    "glGetPointerv",
    "glVertexPointer",
    "glColorPointer",
    "glTexCoordPointer",
    "glDrawElements",
    "glArrayElement",
    "glGenTextures",
    "glDeleteTextures",
    "glBindTexture",
    "glTexParameteri",
    "glTexParameterfv",
    "glTexEnvi",
    "glTexEnvf",
    "glTexImage2D",
    "glCopyTexImage2D",
    "glTexSubImage2D",
    "glFinish",
    "glPixelStorei",
    "glReadPixels",
];

/// C names resolved by [`GlCompiledVertexArrays::load`].
pub const GL_COMPILED_VERTEX_ARRAY_NAMES: &[&str] = &["glLockArraysEXT", "glUnlockArraysEXT"];

/// Test fake shared by the GL module tests.
#[cfg(test)]
pub(crate) struct FakeContext {
    pub procs: std::collections::HashMap<String, *mut c_void>,
    pub leases: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub requested: Vec<String>,
}

#[cfg(test)]
impl FakeContext {
    pub(crate) fn with_names(names: &[&str]) -> Self {
        let mut procs = std::collections::HashMap::new();
        for (index, name) in names.iter().enumerate() {
            // Never invoked; only resolved and counted.
            procs.insert(
                (*name).to_string(),
                std::ptr::without_provenance_mut(0x1000 + index * 8),
            );
        }
        Self {
            procs,
            leases: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: Vec::new(),
        }
    }
}

#[cfg(test)]
impl SdlRenderContext for FakeContext {
    fn drawable_size(&mut self) -> Result<(u32, u32)> {
        Ok((64, 64))
    }
    fn rendering_enabled(&self) -> bool {
        true
    }
    fn set_rendering_enabled(&mut self, _: bool) -> Result<()> {
        Ok(())
    }
    fn make_current(&mut self) -> Result<()> {
        Ok(())
    }
    fn get_gl_proc_address(&mut self, name: &str) -> Result<*mut c_void> {
        self.requested.push(name.to_string());
        self.procs
            .get(name)
            .copied()
            .ok_or_else(|| Error::native("SDL_GL_GetProcAddress", format!("{name} is missing")))
    }
    fn retain_procedures(&mut self) -> Result<ProcedureGuard> {
        Ok(ProcedureGuard::new(std::sync::Arc::clone(&self.leases)))
    }
    fn swap(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_every_symbol_and_holds_lease() {
        let mut context = FakeContext::with_names(GL_SYMBOL_NAMES);
        let mut table = Gl::load(&mut context).unwrap();
        assert_eq!(context.requested, GL_SYMBOL_NAMES);
        assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!table.is_closed());
        // Symbols resolve to the fake addresses, in order.
        let first = context.procs["glCallList"];
        assert_eq!(table.symbols.gl_call_list as usize, first as usize);
        table.close();
        assert!(table.is_closed());
        assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
        table.close();
    }

    #[test]
    fn missing_symbol_names_gl() {
        let mut context = FakeContext::with_names(&["glCallList"]);
        let Err(error) = Gl::load(&mut context) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("glNewList"), "{error}");
        assert!(error.to_string().contains("gl"), "{error}");
        assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn compiled_vertex_arrays_reach_both_lookups() {
        let mut context = FakeContext::with_names(GL_COMPILED_VERTEX_ARRAY_NAMES);
        let mut table = GlCompiledVertexArrays::load(&mut context).unwrap();
        assert_eq!(context.requested, GL_COMPILED_VERTEX_ARRAY_NAMES);
        table.close();
        assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn drop_releases_lease() {
        let mut context = FakeContext::with_names(GL_SYMBOL_NAMES);
        {
            let _table = Gl::load(&mut context).unwrap();
            assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
        assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
