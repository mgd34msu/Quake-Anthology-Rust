//! One load-sized history of produced commands, independent of wire protocol.
use crate::primitives::UserCmd;

pub const DEFAULT_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryError {
    Capacity,
    Future,
}

pub struct CommandHistory {
    commands: Box<[UserCmd]>,
    number: u64,
    head: usize,
    written: usize,
}

impl CommandHistory {
    /// A supplied starting number supports restored recordings without
    /// importing any protocol's sequence width into the common history.
    pub fn load(capacity: usize, number: u64) -> Result<Self, HistoryError> {
        if capacity == 0 {
            return Err(HistoryError::Capacity);
        }
        let mut commands = Vec::new();
        commands
            .try_reserve_exact(capacity)
            .map_err(|_| HistoryError::Capacity)?;
        commands.resize(capacity, UserCmd::default());
        Ok(Self {
            commands: commands.into_boxed_slice(),
            number,
            head: (number % capacity as u64) as usize,
            written: 0,
        })
    }

    pub fn capacity(&self) -> usize {
        self.commands.len()
    }
    pub fn number(&self) -> u64 {
        self.number
    }
    pub fn len(&self) -> usize {
        self.written
    }
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }

    pub fn append(&mut self, command: UserCmd) -> u64 {
        self.number = self.number.wrapping_add(1);
        self.head += 1;
        if self.head == self.commands.len() {
            self.head = 0;
        }
        self.commands[self.head] = command;
        self.written = (self.written + 1).min(self.commands.len());
        self.number
    }

    pub fn get(&self, age: usize) -> Option<&UserCmd> {
        if age >= self.written {
            return None;
        }
        let index = if age > self.head {
            self.commands.len() - (age - self.head)
        } else {
            self.head - age
        };
        self.commands.get(index)
    }

    pub fn read(&self, number: u64) -> Result<Option<&UserCmd>, HistoryError> {
        let age = self.number.wrapping_sub(number);
        if age > i64::MAX as u64 {
            return Err(HistoryError::Future);
        }
        Ok(usize::try_from(age).ok().and_then(|age| self.get(age)))
    }

    /// Capacity changes happen at a load/connection boundary. Retain the
    /// newest commands and their numbers when a configured window changes.
    pub fn resize(&mut self, capacity: usize) -> Result<(), HistoryError> {
        if capacity == self.capacity() {
            return Ok(());
        }
        let keep = self.written.min(capacity);
        let mut next = Self::load(capacity, self.number.wrapping_sub(keep as u64))?;
        for age in (0..keep).rev() {
            let command = self.get(age).ok_or(HistoryError::Capacity)?;
            next.append(*command);
        }
        *self = next;
        Ok(())
    }

    /// Restart can retain the command number; disconnect/new state uses zero.
    pub fn reset(&mut self, number: u64) {
        self.commands.fill(UserCmd::default());
        self.number = number;
        self.head = (number % self.commands.len() as u64) as usize;
        self.written = 0;
    }
}
