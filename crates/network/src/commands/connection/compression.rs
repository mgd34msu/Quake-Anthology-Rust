use qa_formats::inflate::{InflateFlush, Inflater};

/// q2repro's MAX_DEFLATED_SIZE is independent of its 32768-byte wire limit.
const EXPANDED_BYTES: usize = 65536;

pub(super) struct Compression {
    pub packet: Stream,
    pub download: Stream,
}
impl Compression {
    pub fn load() -> Self {
        Self {
            packet: Stream::load(),
            download: Stream::load(),
        }
    }
}

pub(super) struct Stream {
    decoder: Inflater,
    pub bytes: Box<[u8]>,
    active: bool,
}
impl Stream {
    fn load() -> Self {
        Self {
            decoder: Inflater::raw(),
            bytes: vec![0; EXPANDED_BYTES].into_boxed_slice(),
            active: false,
        }
    }
    pub fn reset(&mut self) {
        self.decoder.reset();
        self.active = false;
    }
    pub fn packet(&mut self, input: &[u8]) -> Result<usize, ()> {
        self.reset();
        expand(&mut self.decoder, input, &mut self.bytes).map(|(length, _)| length)
    }
    pub fn download(&mut self, input: &[u8]) -> Result<&[u8], ()> {
        if !self.active {
            self.decoder.reset();
        }
        match expand(&mut self.decoder, input, &mut self.bytes) {
            Ok((length, finished)) => {
                self.active = !finished;
                Ok(&self.bytes[..length])
            }
            Err(()) => {
                self.reset();
                Err(())
            }
        }
    }
}

/// A full output slice still permits consuming a terminal DEFLATE marker.
/// A one-byte probe distinguishes that marker from output beyond the bound.
fn expand(decoder: &mut Inflater, input: &[u8], output: &mut [u8]) -> Result<(usize, bool), ()> {
    let mut consumed = 0;
    let mut produced = 0;
    loop {
        let mut overflow = [0];
        let full = produced == output.len();
        let destination = if full {
            &mut overflow[..]
        } else {
            &mut output[produced..]
        };
        let step = decoder
            .step(&input[consumed..], destination, InflateFlush::Sync)
            .map_err(|_| ())?;
        consumed += step.consumed;
        if full && step.produced != 0 {
            return Err(());
        }
        produced += step.produced;
        if step.finished {
            return Ok((produced, true));
        }
        if step.consumed == 0 && step.produced == 0 {
            return if consumed == input.len() {
                Ok((produced, false))
            } else {
                Err(())
            };
        }
    }
}
