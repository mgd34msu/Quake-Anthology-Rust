//! Quake III base: bot debug.
//!
//! Donor provenance: `src/content/q3/base/bot-debug.ts`.

use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// bot-debug.ts
// ---------------------------------------------------------------------------

/// Bot debug polygon sink (`BotDebugPolygons`).
pub trait BotDebugPolygons {
    /// Create a debug polygon, returning its identifier.
    fn create(&mut self, color: i32, count: usize, points: &[Vec3]) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    struct FakePolygons {
        next: i32,
    }

    impl BotDebugPolygons for FakePolygons {
        fn create(&mut self, _color: i32, count: usize, points: &[Vec3]) -> i32 {
            assert_eq!(count, points.len());
            let id = self.next;
            self.next += 1;
            id
        }
    }

    #[test]
    fn bot_debug_polygons_allocate_ids() {
        let mut sink = FakePolygons { next: 4 };
        let id = sink.create(2, 1, &[vec3(1.0, 2.0, 3.0)]);
        assert_eq!(id, 4);
        assert_eq!(sink.create(2, 0, &[]), 5);
    }
}
