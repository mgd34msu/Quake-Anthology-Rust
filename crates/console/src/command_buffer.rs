use crate::{
    command_text::TextError,
    views::{CommandProducer, Context, RuleSetId},
};
use qa_core::sys_events::KeySource;
use qa_core::text::FixedText;

pub const CAPACITY: usize = 65536;
#[derive(Clone, Copy)]
struct Span {
    end: usize,
    context: Context,
}
struct Wait {
    executions: u32,
    frame: Option<(u64, Context)>,
    context: Context,
}
pub struct CommandBuffer {
    text: Box<[u8]>,
    spans: Vec<Span>,
    bytes: usize,
    wait: Option<Wait>,
    frame: u64,
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
            wait: None,
            frame: 0,
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
        self.insert_text(
            text,
            context,
            matches!(context.source, RuleSetId::QuakeWorld | RuleSetId::Quake3),
        )
    }
    pub fn insert_line_bytes(&mut self, text: &[u8], context: Context) -> Result<(), TextError> {
        self.insert_text(text, context, true)
    }
    fn insert_text(
        &mut self,
        text: &[u8],
        context: Context,
        newline: bool,
    ) -> Result<(), TextError> {
        let newline = usize::from(newline);
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
    pub fn wait(&mut self, frames: u32, context: Context) {
        if let Some(wait) = &mut self.wait
            && wait.frame.is_some_and(|(frame, _)| frame == self.frame)
        {
            // EXEC_NOW can change native wait counts, but cannot release a
            // resource-retirement barrier owned by this host frame.
            wait.executions = frames;
            wait.context = context;
            return;
        }
        self.wait = (frames != 0).then_some(Wait {
            executions: frames,
            frame: None,
            context,
        });
    }
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }
    pub fn wait_next_frame(&mut self, context: Context) {
        let wait = self.wait.get_or_insert(Wait {
            executions: 0,
            frame: None,
            context,
        });
        wait.frame = Some((self.frame, context));
    }
    pub fn wait_context(&self) -> Option<Context> {
        let wait = self.wait.as_ref()?;
        wait.frame
            .filter(|&(frame, _)| frame == self.frame)
            .map(|(_, context)| context)
            .or_else(|| (wait.executions != 0).then_some(wait.context))
    }
    pub fn ready(&mut self) -> bool {
        if let Some(wait) = &mut self.wait {
            if wait.frame.is_some_and(|(frame, _)| frame == self.frame) {
                return false;
            }
            wait.frame = None;
            if wait.executions != 0 {
                wait.executions -= 1;
                if wait.executions == 0 {
                    self.wait = None;
                }
                return false;
            }
            self.wait = None;
        }
        true
    }
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
    /// Remove pending descendants of a rejected physical binding acquisition.
    /// Other command origins retain their byte order and complete Context.
    pub fn discard_acquisition(&mut self, source: KeySource, producer: Option<CommandProducer>) {
        self.discard_contexts(|context| {
            context.key_source == Some(source) && context.producer == producer
        });
    }
    pub fn discard_producer(&mut self, producer: CommandProducer) {
        self.retain_producers(|current| current != producer);
    }
    pub fn retain_producers(&mut self, mut retained: impl FnMut(CommandProducer) -> bool) {
        self.discard_contexts(|context| {
            context.producer.is_some_and(|producer| !retained(producer))
        });
    }
    fn discard_contexts(&mut self, mut retired: impl FnMut(Context) -> bool) {
        if let Some(wait) = &mut self.wait {
            if wait.executions != 0 && retired(wait.context) {
                wait.executions = 0;
            }
            if wait.frame.is_some_and(|(_, context)| retired(context)) {
                wait.frame = None;
            }
            if wait.executions == 0 && wait.frame.is_none() {
                self.wait = None;
            }
        }
        let mut read = 0;
        let mut write = 0;
        let mut kept = 0;
        for index in 0..self.spans.len() {
            let span = self.spans[index];
            if !retired(span.context) {
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
        self.wait = None;
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
