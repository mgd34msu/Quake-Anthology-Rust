//! Deferred bot call steps from `src/bots/behavior/library/call-steps.ts`.
//!
//! The donor expresses multi-frame client commands as generators that
//! yield one step per frame. This port models the same queue explicitly:
//! each step is a boxed closure run once, in order, until the queue
//! drains.

/// One deferred call step.
pub struct CallStep {
    label: &'static str,
    run: Box<dyn FnMut() + Send>,
}

impl std::fmt::Debug for CallStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallStep").field("label", &self.label).finish()
    }
}

impl CallStep {
    /// New step with a diagnostic label.
    pub fn new(label: &'static str, run: impl FnMut() + Send + 'static) -> Self {
        Self {
            label,
            run: Box::new(run),
        }
    }

    /// Step label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Run the step once.
    pub fn run(&mut self) {
        (self.run)();
    }
}

/// FIFO queue of deferred call steps.
#[derive(Debug, Default)]
pub struct CallSteps {
    steps: std::collections::VecDeque<CallStep>,
}

impl CallSteps {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a step.
    pub fn push(&mut self, step: CallStep) {
        self.steps.push_back(step);
    }

    /// Run the next step; returns whether one ran.
    pub fn step(&mut self) -> bool {
        if let Some(mut step) = self.steps.pop_front() {
            step.run();
            true
        } else {
            false
        }
    }

    /// Run all queued steps.
    pub fn drain(&mut self) {
        while self.step() {}
    }

    /// Queued step count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Clear the queue.
    pub fn clear(&mut self) {
        self.steps.clear();
    }
}
