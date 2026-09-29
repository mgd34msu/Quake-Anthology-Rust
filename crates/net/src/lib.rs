//! Network foundation: protocol identities, wire scalar encodings, and the
//! shared bounded bit buffer. Per-family codecs build on this crate.

pub mod angles;
pub mod bits;
pub mod common;
pub mod demo;
pub mod huffman;
pub mod msg;
pub mod protocol;
pub mod q1;
pub mod q1_checkpoint;
pub mod q1_chktbl;
pub mod q1_net;
pub mod q1_wide;
pub mod q2;
pub mod q2_adapters;
pub mod q2_kex_channel;
pub mod q2_kex_packet;
pub mod q2_net;
pub mod q2_prediction;
pub mod q2_server_demo;
pub mod q2_solid;
pub mod q2_svc;
pub mod q2_variants;
pub mod q3;
pub mod q3_net;
pub mod qw;
pub mod services;
pub mod unified;
