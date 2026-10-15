#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PvsRleError {
    Truncated,
    TruncatedRun,
    InvalidRun,
}

/// Native PVS zero runs; validation and decoding use the same grammar.
pub fn pvs(
    bytes: &[u8],
    mut at: usize,
    length: usize,
    mut output: Option<&mut [u8]>,
) -> Result<(), PvsRleError> {
    if output.as_ref().is_some_and(|out| out.len() < length) {
        return Err(PvsRleError::InvalidRun);
    }
    let mut written = 0;
    while written < length {
        let byte = *bytes.get(at).ok_or(PvsRleError::Truncated)?;
        at += 1;
        let count = if byte != 0 {
            1
        } else {
            let run = *bytes.get(at).ok_or(PvsRleError::TruncatedRun)?;
            at += 1;
            usize::from(run)
        };
        if count == 0 || count > length - written {
            return Err(PvsRleError::InvalidRun);
        }
        if let Some(out) = &mut output {
            out[written..written + count].fill(byte);
        }
        written += count;
    }
    Ok(())
}

/// Append one native PVS row using nonzero literals and zero runs of up to 255.
pub fn encode_pvs(row: &[u8], encoded: &mut Vec<u8>) {
    let mut at = 0;
    while at < row.len() {
        let byte = row[at];
        at += 1;
        if byte != 0 {
            encoded.push(byte);
            continue;
        }
        let mut run = 1u8;
        while at < row.len() && row[at] == 0 && run < 255 {
            run += 1;
            at += 1;
        }
        encoded.extend_from_slice(&[0, run]);
    }
}
