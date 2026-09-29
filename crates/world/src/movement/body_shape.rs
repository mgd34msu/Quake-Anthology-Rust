//! Local hull expansion gate.
//!
//! Donor provenance: `src/movement/body-shape.ts`.

use qa_core::math::Bounds;

pub use super::movement_bounds;

/// Selected body-shape state: the accepted hull plus an optional pending
/// request. The game owner applies requests through [`movement_bounds`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementBodyShape {
    /// Currently accepted hull.
    pub current: Bounds,
    /// Pending requested hull.
    pub requested: Option<Bounds>,
}

impl MovementBodyShape {
    /// Accept or reject the pending request against a clearance probe.
    pub fn resolve(&mut self, clear: &dyn Fn(&Bounds) -> bool) {
        if let Some(requested) = self.requested {
            self.current = movement_bounds(&self.current, &requested, clear);
            self.requested = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    fn bounds(lo: f32, hi: f32) -> Bounds {
        Bounds {
            min: vec3(-16.0, -16.0, lo),
            max: vec3(16.0, 16.0, hi),
        }
    }

    #[test]
    fn expansion_requires_clearance() {
        let previous = bounds(-24.0, 32.0);
        let requested = bounds(-24.0, 48.0);
        assert_eq!(movement_bounds(&previous, &requested, &|_| false), previous);
        assert_eq!(movement_bounds(&previous, &requested, &|_| true), requested);
    }

    #[test]
    fn shrink_and_equal_apply_without_probe() {
        use std::cell::Cell;
        let previous = bounds(-24.0, 32.0);
        let shrink = bounds(-24.0, 16.0);
        let probed = Cell::new(false);
        let out = movement_bounds(&previous, &shrink, &|_| {
            probed.set(true);
            false
        });
        assert_eq!(out, shrink);
        assert!(!probed.get());
    }

    #[test]
    fn body_shape_resolve_clears_pending_request() {
        let mut shape = MovementBodyShape {
            current: bounds(-24.0, 32.0),
            requested: Some(bounds(-24.0, 48.0)),
        };
        shape.resolve(&|_| true);
        assert_eq!(shape.current, bounds(-24.0, 48.0));
        assert_eq!(shape.requested, None);
        shape.resolve(&|_| panic!("no pending request"));
    }
}
