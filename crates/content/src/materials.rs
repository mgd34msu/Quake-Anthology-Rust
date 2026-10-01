//! Quake I surface-kind classification.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/materials/legacy.ts`
//! (`q1SurfaceKind`).

/// Texture-name surface classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1SurfaceKind {
    /// Sky surface.
    Sky,
    /// Fence (masked) surface.
    Fence,
    /// Ordinary surface.
    Ordinary,
    /// Lava surface.
    Lava,
    /// Slime surface.
    Slime,
    /// Teleport surface.
    Teleport,
    /// Water surface.
    Water,
}

/// Classify a Quake texture name, exactly like the donor.
#[must_use]
pub fn q1_surface_kind(name: &str) -> Q1SurfaceKind {
    if name.starts_with("sky") {
        Q1SurfaceKind::Sky
    } else if name.starts_with('{') {
        Q1SurfaceKind::Fence
    } else if !name.starts_with('*') {
        Q1SurfaceKind::Ordinary
    } else if name.starts_with("*lava") {
        Q1SurfaceKind::Lava
    } else if name.starts_with("*slime") {
        Q1SurfaceKind::Slime
    } else if name.starts_with("*tele") {
        Q1SurfaceKind::Teleport
    } else {
        Q1SurfaceKind::Water
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_kinds_match_donor() {
        for (name, kind) in [
            ("sky1", Q1SurfaceKind::Sky),
            ("{fence", Q1SurfaceKind::Fence),
            ("base_wall", Q1SurfaceKind::Ordinary),
            ("", Q1SurfaceKind::Ordinary),
            ("*lava", Q1SurfaceKind::Lava),
            ("*lava1", Q1SurfaceKind::Lava),
            ("*slime", Q1SurfaceKind::Slime),
            ("*teleport", Q1SurfaceKind::Teleport),
            ("*water", Q1SurfaceKind::Water),
            ("*rift", Q1SurfaceKind::Water),
        ] {
            assert_eq!(q1_surface_kind(name), kind, "{name}");
        }
    }
}
