//! Quake III base/game: save callbacks.
//!
//! Donor provenance: `src/content/q3/base/game/save-callbacks.ts`.

use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::*;
use crate::q3::base::game::state::{failure, Q3Driver, Q3GameError, TouchContact};

// ---------------------------------------------------------------------------
// save-callbacks.ts: callback families and catalog
// ---------------------------------------------------------------------------

/// One callback family (`Q3CallbackFamily`); identity compares handle pointers.
pub struct CallbackTable<F: ?Sized> {
    entries: Vec<(String, Rc<F>)>,
}

impl<F: ?Sized> std::fmt::Debug for CallbackTable<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self.entries.iter().map(|(id, _)| id.as_str()).collect();
        f.debug_struct("CallbackTable").field("ids", &ids).finish()
    }
}

impl<F: ?Sized> Default for CallbackTable<F> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<F: ?Sized> CallbackTable<F> {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Register an identity (`register`).
    pub fn register(&mut self, id: &str, callback: Rc<F>) -> Result<Rc<F>, Q3GameError> {
        if id.is_empty() {
            return Err(failure("Q3 callback identity is empty"));
        }
        for (known, known_callback) in &self.entries {
            if known == id && !Rc::ptr_eq(known_callback, &callback) {
                return Err(failure(format!("Duplicate Q3 callback identity {id}")));
            }
            if Rc::ptr_eq(known_callback, &callback) && known != id {
                return Err(failure(format!("Q3 callback has two identities: {known}, {id}")));
            }
        }
        if !self.entries.iter().any(|(known, _)| known == id) {
            self.entries.push((id.to_string(), callback.clone()));
        }
        Ok(callback)
    }

    /// Register unless present (`intern`).
    pub fn intern(&mut self, id: &str, callback: Rc<F>) -> Result<Rc<F>, Q3GameError> {
        for (known, known_callback) in &self.entries {
            if known == id {
                return Ok(known_callback.clone());
            }
        }
        self.register(id, callback)
    }

    /// Capture an identity (`capture`).
    pub fn capture(&self, callback: Option<&Rc<F>>) -> Result<Option<String>, Q3GameError> {
        let Some(callback) = callback else { return Ok(None) };
        for (known, known_callback) in &self.entries {
            if Rc::ptr_eq(known_callback, callback) {
                return Ok(Some(known.clone()));
            }
        }
        Err(failure("Unregistered native Q3 callback cannot be saved"))
    }

    /// Resolve an identity (`resolve`).
    pub fn resolve(&self, id: Option<&str>) -> Result<Option<Rc<F>>, Q3GameError> {
        let Some(id) = id else { return Ok(None) };
        for (known, known_callback) in &self.entries {
            if known == id {
                return Ok(Some(known_callback.clone()));
            }
        }
        Err(failure(format!("Unknown native Q3 callback {id}")))
    }
}

/// Callback catalog (`Q3CallbackCatalog`).
#[derive(Debug)]
#[allow(clippy::type_complexity)]
pub struct Q3CallbackCatalog {
    /// Think family.
    pub think: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize)>,
    /// Reached family.
    pub reached: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize)>,
    /// Blocked family.
    pub blocked: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant)>,
    /// Touch family.
    pub touch: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &TouchContact)>,
    /// Use family.
    pub use_callbacks: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, Option<&Participant>, Option<&Participant>)>,
    /// Pain family.
    pub pain: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant, i32)>,
    /// Die family.
    pub die: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &Participant, i32, i32)>,
}

impl Q3CallbackCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            think: CallbackTable::new(),
            reached: CallbackTable::new(),
            blocked: CallbackTable::new(),
            touch: CallbackTable::new(),
            use_callbacks: CallbackTable::new(),
            pain: CallbackTable::new(),
            die: CallbackTable::new(),
        }
    }
}

impl Default for Q3CallbackCatalog {
    fn default() -> Self {
        Self::new()
    }
}
