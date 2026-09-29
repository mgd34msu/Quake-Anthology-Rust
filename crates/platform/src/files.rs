//! Scoped user-data file storage.

pub mod contained;
pub mod writable;

pub use contained::{contained_file_parts, open_contained_parent, ContainedParent};
pub use writable::{
    parse_writable_mode, writable_mode_name, UserFileStore, WritableBinaryFile, WritableFileCheckpoint,
    WritableFileMode, WritableSeekOrigin,
};
