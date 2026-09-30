//! Q3 pinned sources (donor `tools/reference/q3/sources.ts`).
//!
//! Path and SHA-256 pins over the original Quake III Arena sources backing
//! the Q3 evaluator. The capture identifies each pinned file and rejects
//! hash drift.

use crate::json::Json;

/// A pinned original source file.
#[derive(Debug, Clone, Copy)]
pub struct Q3SourcePin {
    /// Source-relative path.
    pub path: &'static str,
    /// SHA-256 hex of the pinned bytes.
    pub sha256: &'static str,
}

impl Q3SourcePin {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(self.path)),
            ("sha256".to_owned(), Json::string(self.sha256)),
        ])
    }
}

/// Pinned Q3 sources (donor `sourcePins` order).
pub const SOURCE_PINS: [Q3SourcePin; 8] = [
    Q3SourcePin {
        path: "code/game/bg_pmove.c",
        sha256: "e4560cbd9e1bb1479e586018433fc4c2b7cc61f9798097944609b1e661aba182",
    },
    Q3SourcePin {
        path: "code/game/bg_misc.c",
        sha256: "6ac561b647882e2e5a45ceb9da86f7ae88ec9714cd54ec29ad78440b3e991bbb",
    },
    Q3SourcePin {
        path: "code/game/bg_public.h",
        sha256: "1a5a1b6c8defbfd3346a924dabc68b34a76f7ab8ced3e847cb8dc01a870c66aa",
    },
    Q3SourcePin {
        path: "code/game/q_shared.h",
        sha256: "9083a35790991b674bc58c3800b068a9a978898508c5fb08123ea52e1dc8597a",
    },
    Q3SourcePin {
        path: "code/game/g_main.c",
        sha256: "fdc9abc73283c57a27e25c15fbcac7cc7b63d0a82d6fe9ce8f8af8252548ee4a",
    },
    Q3SourcePin {
        path: "code/game/g_client.c",
        sha256: "2e5d3f526e4e82409bec32a34385cca4cb087fefdd0a5358b7895b71793b4a73",
    },
    Q3SourcePin {
        path: "code/server/sv_client.c",
        sha256: "3e9db45857b2fc56e6df573e5faa3b23839e31cd20d3f3c4e008b7262c4edbc9",
    },
    Q3SourcePin {
        path: "code/qcommon/vm.c",
        sha256: "36eb85c1ef2e7fb82bd2c79827e9d6a3a52bc867edfc7b5eec57540537169513",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        assert_eq!(SOURCE_PINS.len(), 8);
        let mut paths = std::collections::BTreeSet::new();
        for pin in SOURCE_PINS {
            assert!(paths.insert(pin.path), "duplicate {}", pin.path);
            assert_eq!(pin.sha256.len(), 64, "{}", pin.path);
        }
    }
}
