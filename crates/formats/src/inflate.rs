//! Retained raw-DEFLATE decoding into caller-owned bounded slices.
use crate::FormatError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InflateFlush {
    Continue,
    Sync,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InflateStep {
    pub consumed: usize,
    pub produced: usize,
    pub finished: bool,
}

pub struct Inflater {
    decoder: flate2::Decompress,
}
impl Inflater {
    pub fn raw() -> Self {
        Self {
            decoder: flate2::Decompress::new(false),
        }
    }
    pub fn reset(&mut self) {
        self.decoder.reset(false);
    }
    pub fn total_in(&self) -> u64 {
        self.decoder.total_in()
    }
    pub fn total_out(&self) -> u64 {
        self.decoder.total_out()
    }
    /// Exhausting either slice is a resumable boundary, not an implicit EOF.
    pub fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: InflateFlush,
    ) -> Result<InflateStep, FormatError> {
        let before_in = self.decoder.total_in();
        let before_out = self.decoder.total_out();
        let status = self
            .decoder
            .decompress(
                input,
                output,
                match flush {
                    InflateFlush::Continue => flate2::FlushDecompress::None,
                    InflateFlush::Sync => flate2::FlushDecompress::Sync,
                },
            )
            .map_err(|_| FormatError::Compression)?;
        Ok(InflateStep {
            consumed: (self.decoder.total_in() - before_in) as usize,
            produced: (self.decoder.total_out() - before_out) as usize,
            finished: status == flate2::Status::StreamEnd,
        })
    }
}
