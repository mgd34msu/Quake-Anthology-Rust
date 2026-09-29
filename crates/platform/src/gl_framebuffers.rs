//! GL framebuffer object ABI resolved through the owning SDL context.
//!
//! Port of donor `src/platform/gl-framebuffers.ts`. The renderer owns
//! framebuffer objects and selects its context before symbol calls.

use std::ffi::c_void;

use crate::error::{Error, Result};
use crate::sdl_render_context::{ProcedureGuard, SdlRenderContext};

/// Resolved framebuffer entry points.
pub struct GlFramebufferSymbols {
    /// `glGenFramebuffers`.
    pub gl_gen_framebuffers: unsafe extern "C" fn(i32, *mut u32),
    /// `glDeleteFramebuffers`.
    pub gl_delete_framebuffers: unsafe extern "C" fn(i32, *const u32),
    /// `glBindFramebuffer`.
    pub gl_bind_framebuffer: unsafe extern "C" fn(u32, u32),
    /// `glFramebufferTexture2D`.
    pub gl_framebuffer_texture_2d: unsafe extern "C" fn(u32, u32, u32, u32, i32),
    /// `glCheckFramebufferStatus`.
    pub gl_check_framebuffer_status: unsafe extern "C" fn(u32) -> u32,
    /// `glTexImage2D` (null-data form for renderbuffer storage).
    pub gl_tex_image_2d: unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void),
    /// `glBlitFramebuffer`.
    pub gl_blit_framebuffer: unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32),
    /// `glGetFramebufferAttachmentParameteriv`.
    pub gl_get_framebuffer_attachment_parameteriv: unsafe extern "C" fn(u32, u32, u32, *mut i32),
    /// `glReadBuffer`.
    pub gl_read_buffer: unsafe extern "C" fn(u32),
}

/// C names resolved by [`GlFramebuffers::load`], in load order.
pub const GL_FRAMEBUFFER_SYMBOL_NAMES: &[&str] = &[
    "glGenFramebuffers",
    "glDeleteFramebuffers",
    "glBindFramebuffer",
    "glFramebufferTexture2D",
    "glCheckFramebufferStatus",
    "glTexImage2D",
    "glBlitFramebuffer",
    "glGetFramebufferAttachmentParameteriv",
    "glReadBuffer",
];

/// Owned framebuffer procedure table with a context procedure lease.
pub struct GlFramebuffers {
    /// Resolved symbols.
    pub symbols: GlFramebufferSymbols,
    guard: Option<ProcedureGuard>,
    closed: bool,
}

impl GlFramebuffers {
    /// Resolve every symbol through `context`, retaining procedures.
    pub fn load(context: &mut dyn SdlRenderContext) -> Result<Self> {
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
        let symbols = GlFramebufferSymbols {
            gl_gen_framebuffers: proc!("glGenFramebuffers", unsafe extern "C" fn(i32, *mut u32)),
            gl_delete_framebuffers: proc!("glDeleteFramebuffers", unsafe extern "C" fn(i32, *const u32)),
            gl_bind_framebuffer: proc!("glBindFramebuffer", unsafe extern "C" fn(u32, u32)),
            gl_framebuffer_texture_2d: proc!("glFramebufferTexture2D", unsafe extern "C" fn(u32, u32, u32, u32, i32)),
            gl_check_framebuffer_status: proc!("glCheckFramebufferStatus", unsafe extern "C" fn(u32) -> u32),
            gl_tex_image_2d: proc!(
                "glTexImage2D",
                unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void)
            ),
            gl_blit_framebuffer: proc!(
                "glBlitFramebuffer",
                unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32)
            ),
            gl_get_framebuffer_attachment_parameteriv: proc!(
                "glGetFramebufferAttachmentParameteriv",
                unsafe extern "C" fn(u32, u32, u32, *mut i32)
            ),
            gl_read_buffer: proc!("glReadBuffer", unsafe extern "C" fn(u32)),
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

impl Drop for GlFramebuffers {
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
        let mut context = FakeContext::with_names(GL_FRAMEBUFFER_SYMBOL_NAMES);
        let mut table = GlFramebuffers::load(&mut context).unwrap();
        assert_eq!(context.requested, GL_FRAMEBUFFER_SYMBOL_NAMES);
        assert!(!table.is_closed());
        table.close();
        assert!(table.is_closed());
        assert_eq!(context.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn missing_symbol_names_gl() {
        let mut context = FakeContext::with_names(&["glGenFramebuffers"]);
        let Err(error) = GlFramebuffers::load(&mut context) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("glDeleteFramebuffers"), "{error}");
    }
}
