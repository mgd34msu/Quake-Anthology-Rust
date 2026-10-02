//! Shared QVM display traps for primary and component clients.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/qvm-display.ts`
//! (`qvmDisplaySyscall`). The actual renderer record and source font
//! registry answer `GETGLCONFIG` and `R_REGISTERFONT` for both ui and
//! cgame roles. Font registration is synchronous here; the seam fills
//! the 255-entry glyph table in place and reports shader handles.

use qa_client::render::q3_hardware::{q3_hardware, q3_hardware_number};
use qa_guest::qvm::abi::{QvmCgameImport, QvmUiImport};
use qa_guest::qvm::client_state::{AbiProfile, CallKind, HostCall, QvmRole, SyscallMemory};
use qa_guest::GuestError;

/// Modern `glconfig_t` record bytes.
pub const GLCONFIG_BYTES: usize = 11332;
/// Legacy `glconfig_t` record bytes.
pub const GLCONFIG_LEGACY_BYTES: usize = 4164;
/// Legacy record field shift.
const GLCONFIG_LEGACY_SHIFT: i32 = 7168;
/// Glyph table bytes (`fontInfo_t`).
pub const FONT_GLYPH_TABLE_BYTES: usize = 20548;
/// Glyph entries per table.
const FONT_GLYPHS: usize = 255;
/// Glyph entry stride.
const GLYPH_STRIDE: i32 = 80;
/// Shader-handle offset within a glyph entry.
const GLYPH_HANDLE_OFFSET: i32 = 44;
/// Shader-path offset within a glyph entry.
const GLYPH_PATH_OFFSET: i32 = 48;

/// Renderer driver strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmDisplayDriver {
    /// Renderer name.
    pub renderer: String,
    /// Vendor name.
    pub vendor: String,
    /// Version string.
    pub version: String,
}

/// Renderer GL configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmDisplayGlConfig {
    /// Maximum texture size.
    pub max_texture_size: i32,
    /// Texture units.
    pub texture_units: i32,
    /// Color bits.
    pub color_bits: i32,
    /// Depth bits.
    pub depth_bits: i32,
    /// Stereo enabled.
    pub stereo_enabled: bool,
}

/// Renderer record backing the display traps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmDisplayRenderer {
    /// Stencil bits.
    pub stencil_bits: i32,
    /// Driver strings, when known.
    pub driver: Option<QvmDisplayDriver>,
    /// GL configuration, when known.
    pub gl_config: Option<QvmDisplayGlConfig>,
}

/// Font and shader registration backing the font trap.
pub trait QvmFontServices {
    /// Register a font, filling the glyph table; `false` means missing.
    fn register_font(&mut self, name: Option<&str>, point_size: i32, glyph_table: &mut [u8]) -> bool;
    /// Register a shader without mipmaps, returning its handle.
    fn register_shader_no_mip(&mut self, path: &str) -> i32;
}

/// Display trap options.
pub struct QvmDisplayOptions<'a> {
    /// Renderer record.
    pub renderer: &'a QvmDisplayRenderer,
    /// Font services.
    pub fonts: &'a mut dyn QvmFontServices,
    /// Current viewport in pixels.
    pub viewport: &'a mut dyn FnMut() -> (i32, i32),
    /// Panic when the seat moved past this client.
    pub assert_current: &'a mut dyn FnMut(),
}

/// Dispatch a display trap, or `None` when another owner handles it.
pub fn qvm_display_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    mut options: QvmDisplayOptions<'_>,
) -> Result<Option<i32>, GuestError> {
    (options.assert_current)();
    if call.kind != CallKind::Engine || (call.role != QvmRole::Ui && call.role != QvmRole::Cgame) {
        return Ok(None);
    }
    let ui = call.role == QvmRole::Ui;
    let glconfig = if ui {
        QvmUiImport::UiGetglconfig as i32
    } else {
        QvmCgameImport::CgGetglconfig as i32
    };
    if call.code == glconfig {
        return glconfig_syscall(call, memory, &mut options).map(Some);
    }
    let register_font = if ui {
        QvmUiImport::UiRRegisterfont as i32
    } else {
        QvmCgameImport::CgRRegisterfont as i32
    };
    if call.code == register_font {
        return font_syscall(call, memory, &mut options).map(Some);
    }
    Ok(None)
}

fn glconfig_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    options: &mut QvmDisplayOptions<'_>,
) -> Result<i32, GuestError> {
    let legacy = !matches!(call.abi_profile, AbiProfile::Modern);
    let bytes = if legacy { GLCONFIG_LEGACY_BYTES } else { GLCONFIG_BYTES };
    let shift = if legacy { GLCONFIG_LEGACY_SHIFT } else { 0 };
    let pointer = call.int(1)?;
    let base = memory
        .pointer(pointer)
        .ok_or_else(|| GuestError::invalid("GETGLCONFIG requires a record pointer"))?;
    memory.span(pointer, bytes, 0)?;
    memory.fill(base, bytes, 0)?;
    let renderer = &options.renderer;
    memory.write_string(
        pointer,
        renderer
            .driver
            .as_ref()
            .map_or("Quake Anthology software renderer", |driver| driver.renderer.as_str()),
        1024,
    )?;
    memory.write_string(
        pointer + 1024,
        renderer
            .driver
            .as_ref()
            .map_or("Quake Anthology", |driver| driver.vendor.as_str()),
        1024,
    )?;
    memory.write_string(
        pointer + 2048,
        renderer
            .driver
            .as_ref()
            .map_or("software", |driver| driver.version.as_str()),
        1024,
    )?;
    let field = |offset: i32| base + (offset - shift) as usize;
    let gl = renderer.gl_config.as_ref();
    memory.write_i32(field(11264), gl.map_or(0, |gl| gl.max_texture_size))?;
    memory.write_i32(field(11268), gl.map_or(0, |gl| gl.texture_units))?;
    memory.write_i32(field(11272), gl.map_or(24, |gl| gl.color_bits))?;
    memory.write_i32(field(11276), gl.map_or(64, |gl| gl.depth_bits))?;
    memory.write_i32(field(11280), renderer.stencil_bits)?;
    memory.write_i32(
        field(11288),
        q3_hardware_number(q3_hardware(
            renderer.driver.as_ref().map_or("", |driver| driver.renderer.as_str()),
        )) as i32,
    )?;
    let (width, height) = (options.viewport)();
    memory.write_i32(field(11304), width)?;
    memory.write_i32(field(11308), height)?;
    memory.write_f32(field(11312), width as f32 / height as f32)?;
    memory.write_i32(field(11324), i32::from(gl.is_some_and(|gl| gl.stereo_enabled)))?;
    Ok(0)
}

fn font_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    options: &mut QvmDisplayOptions<'_>,
) -> Result<i32, GuestError> {
    let name_word = call.int(1)?;
    let size = call.int(2)?;
    let pointer = call.int(3)?;
    let name = if name_word == 0 {
        None
    } else {
        Some(memory.read_string(name_word)?)
    };
    (options.assert_current)();
    let base = memory
        .pointer(pointer)
        .ok_or_else(|| GuestError::invalid("R_REGISTERFONT requires a glyph table pointer"))?;
    memory.span(pointer, FONT_GLYPH_TABLE_BYTES, 0)?;
    let mut table = memory.read_bytes(base, FONT_GLYPH_TABLE_BYTES)?.to_vec();
    // Table writes land even for missing fonts: the span aliases guest
    // memory in the donor.
    let registered = options.fonts.register_font(name.as_deref(), size, &mut table);
    memory.write_bytes(base, &table)?;
    (options.assert_current)();
    if !registered {
        return Ok(0);
    }
    for index in 0..FONT_GLYPHS {
        let entry = pointer + index as i32 * GLYPH_STRIDE;
        let path = memory.read_string(entry + GLYPH_PATH_OFFSET)?;
        let handle = options.fonts.register_shader_no_mip(&path);
        (options.assert_current)();
        memory.write_i32(
            base + (index as i32 * GLYPH_STRIDE + GLYPH_HANDLE_OFFSET) as usize,
            handle,
        )?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubFonts {
        names: Vec<Option<String>>,
        shaders: Vec<String>,
        missing: bool,
    }

    impl StubFonts {
        fn new(missing: bool) -> Self {
            Self {
                names: Vec::new(),
                shaders: Vec::new(),
                missing,
            }
        }
    }

    impl QvmFontServices for StubFonts {
        fn register_font(&mut self, name: Option<&str>, _point_size: i32, glyph_table: &mut [u8]) -> bool {
            self.names.push(name.map(str::to_string));
            if self.missing {
                return false;
            }
            for index in 0..FONT_GLYPHS {
                let path = format!("fonts/glyph{index:02}.dat");
                let start = index * GLYPH_STRIDE as usize + GLYPH_PATH_OFFSET as usize;
                glyph_table[start..start + path.len()].copy_from_slice(path.as_bytes());
            }
            true
        }

        fn register_shader_no_mip(&mut self, path: &str) -> i32 {
            self.shaders.push(path.to_string());
            1000 + self.shaders.len() as i32
        }
    }

    fn renderer() -> QvmDisplayRenderer {
        QvmDisplayRenderer {
            stencil_bits: 8,
            driver: Some(QvmDisplayDriver {
                renderer: "Test Renderer".to_string(),
                vendor: "Test Vendor".to_string(),
                version: "1.0-test".to_string(),
            }),
            gl_config: Some(QvmDisplayGlConfig {
                max_texture_size: 512,
                texture_units: 2,
                color_bits: 24,
                depth_bits: 24,
                stereo_enabled: true,
            }),
        }
    }

    fn memory() -> SyscallMemory {
        SyscallMemory::new(65536).expect("memory")
    }

    #[test]
    fn glconfig_reports_renderer_and_viewport() {
        let mut memory = memory();
        let pointer = 4096;
        let call = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgGetglconfig as i32,
            &[pointer],
            AbiProfile::Modern,
        );
        let renderer = renderer();
        let mut fonts = StubFonts::new(false);
        let mut viewport = || (640, 480);
        let mut assert_current = || {};
        let result = qvm_display_syscall(
            &call,
            &mut memory,
            QvmDisplayOptions {
                renderer: &renderer,
                fonts: &mut fonts,
                viewport: &mut viewport,
                assert_current: &mut assert_current,
            },
        )
        .expect("syscall");
        assert_eq!(result, Some(0));
        let base = memory.pointer(pointer).expect("record");
        assert_eq!(memory.read_string(pointer).expect("renderer"), "Test Renderer");
        assert_eq!(memory.read_string(pointer + 1024).expect("vendor"), "Test Vendor");
        assert_eq!(memory.read_string(pointer + 2048).expect("version"), "1.0-test");
        assert_eq!(memory.read_i32(base + 11264).expect("texsize"), 512);
        assert_eq!(memory.read_i32(base + 11272).expect("color"), 24);
        assert_eq!(memory.read_i32(base + 11280).expect("stencil"), 8);
        assert_eq!(memory.read_i32(base + 11304).expect("width"), 640);
        assert_eq!(memory.read_i32(base + 11308).expect("height"), 480);
        assert_eq!(memory.read_f32(base + 11312).expect("aspect"), 640.0 / 480.0);
        assert_eq!(memory.read_i32(base + 11324).expect("stereo"), 1);
    }

    #[test]
    fn glconfig_legacy_shifts_fields() {
        let mut memory = memory();
        let pointer = 8192;
        let call = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiGetglconfig as i32,
            &[pointer],
            AbiProfile::Legacy,
        );
        let renderer = QvmDisplayRenderer {
            stencil_bits: 0,
            driver: None,
            gl_config: None,
        };
        let mut fonts = StubFonts::new(false);
        let mut viewport = || (320, 200);
        let mut assert_current = || {};
        let result = qvm_display_syscall(
            &call,
            &mut memory,
            QvmDisplayOptions {
                renderer: &renderer,
                fonts: &mut fonts,
                viewport: &mut viewport,
                assert_current: &mut assert_current,
            },
        )
        .expect("syscall");
        assert_eq!(result, Some(0));
        let base = memory.pointer(pointer).expect("record");
        assert_eq!(
            memory.read_string(pointer).expect("renderer"),
            "Quake Anthology software renderer"
        );
        assert_eq!(
            memory
                .read_i32(base + 11264 - GLCONFIG_LEGACY_SHIFT as usize)
                .expect("texsize"),
            0
        );
        assert_eq!(
            memory
                .read_i32(base + 11272 - GLCONFIG_LEGACY_SHIFT as usize)
                .expect("color"),
            24
        );
        assert_eq!(
            memory
                .read_i32(base + 11276 - GLCONFIG_LEGACY_SHIFT as usize)
                .expect("depth"),
            64
        );
    }

    #[test]
    fn foreign_calls_pass_through() {
        let mut memory = memory();
        let renderer = renderer();
        let mut fonts = StubFonts::new(false);
        let mut viewport = || (640, 480);
        let mut assert_current = || {};
        let game = HostCall::engine(
            QvmRole::Qagame,
            QvmCgameImport::CgGetglconfig as i32,
            &[64],
            AbiProfile::Modern,
        );
        let options = QvmDisplayOptions {
            renderer: &renderer,
            fonts: &mut fonts,
            viewport: &mut viewport,
            assert_current: &mut assert_current,
        };
        assert_eq!(qvm_display_syscall(&game, &mut memory, options).expect("syscall"), None);
        let ext = HostCall::extension(
            QvmRole::Ui,
            QvmUiImport::UiGetglconfig as i32,
            &[64],
            AbiProfile::Modern,
        );
        let options = QvmDisplayOptions {
            renderer: &renderer,
            fonts: &mut fonts,
            viewport: &mut viewport,
            assert_current: &mut assert_current,
        };
        assert_eq!(qvm_display_syscall(&ext, &mut memory, options).expect("syscall"), None);
        let other = HostCall::engine(QvmRole::Ui, 999, &[64], AbiProfile::Modern);
        let options = QvmDisplayOptions {
            renderer: &renderer,
            fonts: &mut fonts,
            viewport: &mut viewport,
            assert_current: &mut assert_current,
        };
        assert_eq!(
            qvm_display_syscall(&other, &mut memory, options).expect("syscall"),
            None
        );
    }

    #[test]
    fn font_registers_glyph_shaders() {
        let mut memory = memory();
        let name = 256;
        memory.write_string(name, "fonts/font1", 64).expect("name");
        let pointer = 4096;
        let call = HostCall::engine(
            QvmRole::Cgame,
            QvmCgameImport::CgRRegisterfont as i32,
            &[name, 12, pointer],
            AbiProfile::Modern,
        );
        let renderer = renderer();
        let mut fonts = StubFonts::new(false);
        let mut viewport = || (640, 480);
        let mut assert_current = || {};
        let result = qvm_display_syscall(
            &call,
            &mut memory,
            QvmDisplayOptions {
                renderer: &renderer,
                fonts: &mut fonts,
                viewport: &mut viewport,
                assert_current: &mut assert_current,
            },
        )
        .expect("syscall");
        assert_eq!(result, Some(0));
        assert_eq!(fonts.names, vec![Some("fonts/font1".to_string())]);
        assert_eq!(fonts.shaders.len(), FONT_GLYPHS);
        let base = memory.pointer(pointer).expect("table");
        assert_eq!(memory.read_i32(base + 44).expect("handle"), 1001);
        assert_eq!(
            memory.read_i32(base + 254 * 80 + 44).expect("handle"),
            1000 + FONT_GLYPHS as i32
        );
    }

    #[test]
    fn missing_font_skips_shaders() {
        let mut memory = memory();
        let pointer = 4096;
        let call = HostCall::engine(
            QvmRole::Ui,
            QvmUiImport::UiRRegisterfont as i32,
            &[0, 12, pointer],
            AbiProfile::Modern,
        );
        let renderer = renderer();
        let mut fonts = StubFonts::new(true);
        let mut viewport = || (640, 480);
        let mut assert_current = || {};
        let result = qvm_display_syscall(
            &call,
            &mut memory,
            QvmDisplayOptions {
                renderer: &renderer,
                fonts: &mut fonts,
                viewport: &mut viewport,
                assert_current: &mut assert_current,
            },
        )
        .expect("syscall");
        assert_eq!(result, Some(0));
        assert_eq!(fonts.names, vec![None]);
        assert!(fonts.shaders.is_empty());
    }
}
