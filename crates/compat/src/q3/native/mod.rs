//! Q3 native-module bridge over guest and world.
//!
//! Donor provenance: `src/compat/q3/native/index.ts` (re-export root)
//! plus `src/compat/q3/native/*.ts` per submodule.

pub mod imports;
pub mod module;
pub mod syscall;
