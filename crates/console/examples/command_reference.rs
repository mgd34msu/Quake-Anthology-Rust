use qa_console::{
    command_buffer::CommandBuffer,
    command_text,
    views::{Context, RuleSetId},
};
use qa_core::text::FixedText;
use std::{
    io::{self, Write},
    path::Path,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("input path")?;
    let bytes = std::fs::read(Path::new(&path))?;
    let mut at = 0;
    let mut out = io::BufWriter::new(io::stdout().lock());
    while at < bytes.len() {
        let source = RuleSetId::ALL[bytes[at] as usize];
        at += 1;
        let length = u16::from_le_bytes(bytes[at..at + 2].try_into()?) as usize;
        at += 2;
        let text = std::str::from_utf8(&bytes[at..at + length])?;
        at += length;
        let mut tokens = command_text::Tokens::default();
        let args =
            command_text::tokenize(text, source, &mut tokens).map_err(|e| format!("{e:?}"))?;
        out.write_all(&(args.len() as u16).to_le_bytes())?;
        for value in args.iter() {
            out.write_all(&(value.len() as u16).to_le_bytes())?;
            out.write_all(value.as_bytes())?;
        }
        let tail = if source == RuleSetId::Quake3 {
            {
                let mut tail = FixedText::<8192>::default();
                args.join(1, &mut tail).map_err(|e| format!("{e:?}"))?;
                tail.as_str().to_owned()
            }
        } else {
            args.raw.to_owned()
        };
        out.write_all(&(tail.len() as u16).to_le_bytes())?;
        out.write_all(tail.as_bytes())?;
        let context = Context {
            source,
            ..Context::default()
        };
        let mut buffer = CommandBuffer::new();
        buffer.append(text, context).map_err(|e| format!("{e:?}"))?;
        let mut lines = Vec::new();
        let mut line = FixedText::<65536>::default();
        while let Some((_, result)) = buffer.next_line(&mut line) {
            result.map_err(|e| format!("{e:?}"))?;
            lines.push(line.as_str().to_owned());
        }
        out.write_all(&(lines.len() as u16).to_le_bytes())?;
        for line in lines {
            out.write_all(&(line.len() as u16).to_le_bytes())?;
            out.write_all(line.as_bytes())?;
        }
    }
    out.flush()?;
    Ok(())
}
