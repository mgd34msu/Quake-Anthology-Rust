//! Demo-file compatibility: kind detection by file suffix.
//!
//! Donor provenance: `src/network/q1/demos.ts` (WinQuake `.dem`,
//! QuakeWorld `.qwd`), `src/app/bootstrap/q2-travel.ts` (`.dm2` demo
//! travel), `src/network/q3/recording.ts` and
//! `src/network/q3/pak-references.ts` (`.dm_68`). Record framing lives
//! in `qa-net`; this module only classifies demo paths.

/// Demo file kind by family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DemoKind {
    /// WinQuake / NetQuake `.dem`.
    Netquake,
    /// QuakeWorld `.qwd`.
    Quakeworld,
    /// Quake II `.dm2`.
    Quake2,
    /// Quake III `.dm_68`.
    Quake3,
}

impl DemoKind {
    /// Canonical file suffix.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            DemoKind::Netquake => ".dem",
            DemoKind::Quakeworld => ".qwd",
            DemoKind::Quake2 => ".dm2",
            DemoKind::Quake3 => ".dm_68",
        }
    }
}

/// Classify a demo path by case-sensitive suffix, like `String.endsWith`.
#[must_use]
pub fn classify_demo_path(path: &str) -> Option<DemoKind> {
    [
        DemoKind::Netquake,
        DemoKind::Quakeworld,
        DemoKind::Quake2,
        DemoKind::Quake3,
    ]
    .into_iter()
    .find(|kind| path.ends_with(kind.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_suffixes_classify_per_family() {
        assert_eq!(classify_demo_path("e1m1.dem"), Some(DemoKind::Netquake));
        assert_eq!(classify_demo_path("match.qwd"), Some(DemoKind::Quakeworld));
        assert_eq!(classify_demo_path("cin.dm2"), Some(DemoKind::Quake2));
        assert_eq!(classify_demo_path("frag.dm_68"), Some(DemoKind::Quake3));
        assert_eq!(classify_demo_path("MAP.BSP"), None);
        assert_eq!(classify_demo_path("upper.DEM"), None);
        assert_eq!(DemoKind::Quake3.extension(), ".dm_68");
    }
}
