use qa_console::{catalog::ValueType, cvars_generated::DEFINITIONS};
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(&(DEFINITIONS.len() as u32).to_le_bytes())?;
    for row in DEFINITIONS {
        let kind = match row.value_type {
            ValueType::Float => "float",
            ValueType::Int => "int",
            ValueType::Bool => "bool",
            ValueType::Enum => "enum",
            ValueType::String => "string",
            ValueType::Bitmask => "bitmask",
        };
        let mut cells = vec![row.name, row.aliases, kind, row.range_hint];
        cells.extend(row.defaults.iter().map(|s| s.raw_default));
        cells.extend([row.raw_flags, row.owner]);
        cells.extend(row.defaults.iter().map(|s| s.effect));
        cells.push(row.conversion);
        cells.extend(row.audit_status);
        cells.push(row.sources);
        for cell in cells {
            output.write_all(&(cell.len() as u32).to_le_bytes())?;
            output.write_all(cell.as_bytes())?;
        }
    }
    Ok(())
}
