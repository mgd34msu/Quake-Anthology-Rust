//! Inert native-image inspection. Never binds imports or runs module code.
use qa_formats::program::native::{Image, LoadRole};
use std::io::Write;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .ok_or("native_image [--dump|--dump-program] PATH [BASE]")?;
    let dump = first == "--dump" || first == "--dump-program";
    let role = if first == "--dump-program" {
        LoadRole::Program
    } else {
        LoadRole::Library
    };
    let path = if dump {
        args.next().ok_or("path")?
    } else {
        first
    };
    let base = args
        .next()
        .map(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16))
        .transpose()?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let file = std::fs::read(path)?;
    let image = Image::parse(&file, base, role).map_err(|e| format!("{e:?}"))?;
    if dump {
        let mut output = std::io::stdout().lock();
        for value in [
            image.base,
            u64::from(image.target.bits / 8),
            image.bytes.len() as u64,
            image.symbols.len() as u64,
            image.imports.len() as u64,
            image.initializers.len() as u64,
        ] {
            output.write_all(&value.to_le_bytes())?;
        }
        output.write_all(&image.bytes)?;
        return Ok(());
    }
    println!(
        "{{\"bits\":{},\"base\":{},\"preferred_base\":{},\"image_bytes\":{},\"regions\":{},\"symbols\":{},\"imports\":{},\"relocations\":{},\"initializers\":{},\"GetGameAPI\":{},\"executed\":false,\"imports_bound\":false}}",
        image.target.bits,
        image.base,
        image.preferred_base,
        image.bytes.len(),
        image.regions.len(),
        image.symbols.len(),
        image.imports.len(),
        image.relocations.len(),
        image.initializers.len(),
        image.symbol(b"GetGameAPI").map_or(0, |s| s.address)
    );
    Ok(())
}
