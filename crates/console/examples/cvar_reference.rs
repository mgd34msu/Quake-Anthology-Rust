use qa_console::{
    catalog::Scope,
    conversion::{self, Input, Text},
    cvars_generated::{BINDINGS, CONVERSIONS},
    numbers,
    views::{Context, Role, Source},
};
use std::{
    fs::File,
    io::{self, BufReader, Read, Write},
};

fn string(input: &mut impl Read) -> io::Result<String> {
    let mut length = [0; 2];
    input.read_exact(&mut length)?;
    let mut bytes = vec![0; u16::from_le_bytes(length) as usize];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| io::ErrorKind::InvalidData.into())
}
fn text(output: &mut impl Write, value: &str) -> io::Result<()> {
    output.write_all(&(value.len() as u32).to_le_bytes())?;
    output.write_all(value.as_bytes())
}
fn response(output: &mut impl Write, value: Result<Text<'_>, conversion::Error>) -> io::Result<()> {
    output.write_all(&[u8::from(value.is_ok())])?;
    if let Ok(value) = value {
        text(output, value.as_str())?;
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("mode required")?;
    let mut input = BufReader::new(File::open(args.next().ok_or("fixture path required")?)?);
    let mut output = io::stdout().lock();
    loop {
        let mut source = [0];
        match input.read_exact(&mut source) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let source = *Source::ALL.get(source[0] as usize).ok_or("source index")?;
        if mode == "numbers" {
            let value = string(&mut input)?;
            output.write_all(&numbers::number(&value, source).to_bits().to_le_bytes())?;
            output.write_all(&numbers::integer(&value).to_le_bytes())?;
        } else {
            let mut header = [0; 4];
            input.read_exact(&mut header)?;
            let role = match header[0] {
                0 => Role::Engine,
                1 => Role::Game,
                2 => Role::Cgame,
                _ => return Err("role index".into()),
            };
            let binding = &BINDINGS[u16::from_le_bytes([header[1], header[2]]) as usize];
            let c = &CONVERSIONS[binding.conversions[source as usize] as usize];
            let value = string(&mut input)?;
            let current = string(&mut input)?;
            let detail = string(&mut input)?;
            let operands = [
                string(&mut input)?,
                string(&mut input)?,
                string(&mut input)?,
                string(&mut input)?,
            ];
            let operand = |row: u16| operands[row as usize & 3].as_str();
            let context = Context {
                source,
                side: Scope::Client,
                role,
                dedicated: false,
                seat: qa_core::sys_events::SeatId::FIRST,
                event_time: None,
            };
            let detail = (header[3] != 0).then_some(detail.as_str());
            response(
                &mut output,
                conversion::read(Input {
                    context,
                    conversion: c,
                    binding,
                    value: &value,
                    current: &current,
                    detail,
                    operand: &operand,
                }),
            )?;
            let out = conversion::write(Input {
                context,
                conversion: c,
                binding,
                value: &value,
                current: &current,
                detail,
                operand: &operand,
            });
            output.write_all(&[u8::from(out.is_ok())])?;
            if let Ok(out) = out {
                text(&mut output, out.text.as_str())?;
                output.write_all(&[u8::from(out.detail), u8::from(out.detail_value.is_some())])?;
                if let Some(value) = out.detail_value {
                    text(&mut output, value)?;
                }
                output.write_all(&[out.change_count as u8])?;
                for change in out.changes.into_iter().take(out.change_count).flatten() {
                    output.write_all(&change.row.to_le_bytes())?;
                    text(&mut output, change.text.as_str())?;
                }
            }
        }
    }
    Ok(())
}
