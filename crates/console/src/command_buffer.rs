use crate::{
    command_text::TextError,
    views::{Context, Source},
};
use std::collections::VecDeque;

const CAPACITY: usize = 65536;
struct Chunk {
    text: String,
    context: Context,
}
pub struct CommandBuffer {
    chunks: VecDeque<Chunk>,
    bytes: usize,
    wait: u32,
}
pub struct Line {
    pub text: String,
    pub context: Context,
}
impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}
impl CommandBuffer {
    pub fn new() -> Self {
        Self {
            chunks: VecDeque::with_capacity(16),
            bytes: 0,
            wait: 0,
        }
    }
    pub fn append(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.admit(text, 0)?;
        if text.is_empty() {
            return Ok(());
        }
        if let Some(last) = self.chunks.back_mut().filter(|c| c.context == context) {
            last.text.push_str(text);
        } else {
            self.chunks.push_back(Chunk {
                text: text.to_owned(),
                context,
            });
        }
        self.bytes += text.len();
        Ok(())
    }
    pub fn insert(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        let newline = matches!(context.source, Source::QuakeWorld | Source::Quake3);
        self.admit(text, usize::from(newline))?;
        let mut owned = String::with_capacity(text.len() + usize::from(newline));
        owned.push_str(text);
        if newline {
            owned.push('\n');
        }
        self.bytes += owned.len();
        if !newline && let Some(front) = self.chunks.front_mut().filter(|c| c.context == context) {
            front.text.insert_str(0, &owned);
        } else if !owned.is_empty() {
            self.chunks.push_front(Chunk {
                text: owned,
                context,
            });
        }
        Ok(())
    }
    fn admit(&self, text: &str, extra: usize) -> Result<(), TextError> {
        if text.contains('\0') {
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
        self.chunks.is_empty()
    }
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.bytes = 0;
        self.wait = 0;
    }
    pub fn next_line(&mut self) -> Option<Line> {
        let chunk = self.chunks.front_mut()?;
        let mut quoted = false;
        let end = chunk
            .text
            .bytes()
            .position(|b| {
                if b == b'"' {
                    quoted = !quoted;
                }
                (!quoted && b == b';')
                    || b == b'\n'
                    || (chunk.context.source == Source::Quake3 && b == b'\r')
            })
            .unwrap_or(chunk.text.len());
        let text = chunk.text[..end].to_owned();
        let removed = end + usize::from(end < chunk.text.len());
        let line = Line {
            text,
            context: chunk.context,
        };
        chunk.text.drain(..removed);
        self.bytes -= removed;
        if chunk.text.is_empty() {
            self.chunks.pop_front();
        }
        Some(line)
    }
}
