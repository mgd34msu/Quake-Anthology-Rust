//! Network foundation: protocol identities, wire scalar encodings, and the
//! shared bounded bit buffer. Per-family codecs build on this crate.

pub mod angles;
pub mod bits;
pub mod demo;
pub mod huffman;
pub mod msg;
pub mod protocol;
pub mod q1;
pub mod q2;
pub mod q3;
pub mod qw;
