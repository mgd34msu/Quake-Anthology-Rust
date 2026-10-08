use crate::cvars::Cvars;
use qa_core::primitives::CvarHandle;
use std::fmt::Arguments;

pub fn error(message: &str) {
    eprintln!("{message}");
}

pub fn console(message: Arguments<'_>) {
    print!("{message}");
}

/// Module console text may contain original single-byte Quake glyphs.
pub fn console_bytes(message: &[u8]) {
    use std::io::Write;
    let _ = std::io::stdout().lock().write_all(message);
}

pub fn dev_print(cvars: &Cvars, developer: CvarHandle, level: u8, message: Arguments<'_>) {
    if cvars.value(developer) >= f32::from(level) {
        println!("{message}");
    }
}
