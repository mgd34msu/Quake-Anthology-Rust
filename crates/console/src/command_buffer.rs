use crate::{
    command_text::TextError,
    views::{Context, RuleSetId},
};
use qa_core::sys_events::KeySource;
use qa_core::text::FixedText;

pub const CAPACITY: usize = 65536;
#[derive(Clone, Copy)]
struct Span {
    end: usize,
    context: Context,
}
pub struct CommandBuffer {
    text: Box<[u8]>,
    spans: Vec<Span>,
    bytes: usize,
    wait: u32,
}
impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}
impl CommandBuffer {
    pub fn new() -> Self {
        Self {
            text: vec![0; CAPACITY].into_boxed_slice(),
            spans: Vec::with_capacity(CAPACITY),
            bytes: 0,
            wait: 0,
        }
    }
    pub fn append(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.append_bytes(text.as_bytes(), context)
    }
    pub fn append_line(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.append_line_bytes(text.as_bytes(), context)
    }
    pub fn append_bytes(&mut self, text: &[u8], context: Context) -> Result<(), TextError> {
        self.append_text(text, context, false)
    }
    pub fn append_line_bytes(&mut self, text: &[u8], context: Context) -> Result<(), TextError> {
        self.append_text(text, context, true)
    }
    fn append_text(
        &mut self,
        text: &[u8],
        context: Context,
        newline: bool,
    ) -> Result<(), TextError> {
        let extra = usize::from(newline);
        self.admit(text, extra)?;
        if text.is_empty() && !newline {
            return Ok(());
        }
        self.text[self.bytes..self.bytes + text.len()].copy_from_slice(text);
        self.bytes += text.len();
        if newline {
            self.text[self.bytes] = b'\n';
            self.bytes += 1;
        }
        if let Some(last) = self.spans.last_mut().filter(|s| s.context == context) {
            last.end = self.bytes;
        } else {
            self.spans.push(Span {
                end: self.bytes,
                context,
            });
        }
        Ok(())
    }
    pub fn insert(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.insert_bytes(text.as_bytes(), context)
    }
    pub fn insert_bytes(&mut self, text: &[u8], context: Context) -> Result<(), TextError> {
        let newline = usize::from(matches!(
            context.source,
            RuleSetId::QuakeWorld | RuleSetId::Quake3
        ));
        self.admit(text, newline)?;
        let size = text.len() + newline;
        if size == 0 {
            return Ok(());
        }
        self.text.copy_within(..self.bytes, size);
        self.text[..text.len()].copy_from_slice(text);
        if newline != 0 {
            self.text[text.len()] = b'\n';
        }
        self.bytes += size;
        for span in &mut self.spans {
            span.end += size;
        }
        if self.spans.first().is_none_or(|s| s.context != context) {
            self.spans.insert(0, Span { end: size, context });
        }
        Ok(())
    }
    fn admit(&self, text: &[u8], extra: usize) -> Result<(), TextError> {
        if text.contains(&0) {
            return Err(TextError::Nul);
        }
        if text.len() + extra >= CAPACITY - self.bytes {
            return Err(TextError::TooLong);
        }
        Ok(())
    }
    pub fn wait(&mut self, frames: u32) {
        self.wait = frames;
    }
    pub fn ready(&mut self) -> bool {
        if self.wait != 0 {
            self.wait -= 1;
            false
        } else {
            true
        }
    }
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
    /// Remove pending descendants of a rejected physical binding acquisition.
    /// Other command origins retain their byte order and complete Context.
    pub fn discard_source(&mut self, source: KeySource) {
        let mut read = 0;
        let mut write = 0;
        let mut kept = 0;
        for index in 0..self.spans.len() {
            let span = self.spans[index];
            if span.context.key_source != Some(source) {
                let length = span.end - read;
                self.text.copy_within(read..span.end, write);
                write += length;
                self.spans[kept] = Span {
                    end: write,
                    context: span.context,
                };
                kept += 1;
            }
            read = span.end;
        }
        self.spans.truncate(kept);
        self.bytes = write;
    }
    pub fn clear(&mut self) {
        self.spans.clear();
        self.bytes = 0;
        self.wait = 0;
    }
    pub fn next_line<const N: usize>(
        &mut self,
        line: &mut FixedText<N>,
    ) -> Option<(Context, Result<(), TextError>)> {
        let span = *self.spans.first()?;
        let end = qa_core::text::command_separator(
            &self.text[..span.end],
            true,
            span.context.source == RuleSetId::Quake3,
        );
        let result = line
            .set_bytes(&self.text[..end])
            .map_err(|_| TextError::TooLong);
        let removed = end + usize::from(end < span.end);
        self.text.copy_within(removed..self.bytes, 0);
        self.bytes -= removed;
        if removed == span.end {
            self.spans.remove(0);
        }
        for span in &mut self.spans {
            span.end -= removed;
        }
        Some((span.context, result))
    }
}
