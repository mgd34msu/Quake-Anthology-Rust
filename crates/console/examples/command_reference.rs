use qa_console::{
    command_buffer::CommandBuffer,
    command_text,
    views::{Context, Source},
};
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
        let source = Source::ALL[bytes[at] as usize];
        at += 1;
        let length = u16::from_le_bytes(bytes[at..at + 2].try_into()?) as usize;
        at += 2;
        let text = std::str::from_utf8(&bytes[at..at + length])?;
        at += length;
        let args = command_text::tokenize(text, source).map_err(|e| format!("{e:?}"))?;
        out.write_all(&(args.values.len() as u16).to_le_bytes())?;
        for value in &args.values {
            out.write_all(&(value.len() as u16).to_le_bytes())?;
            out.write_all(value.as_bytes())?;
        }
        let tail = if source == Source::Quake3 {
            args.tail(1)
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
        while let Some(line) = buffer.next_line() {
            lines.push(line.text);
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
