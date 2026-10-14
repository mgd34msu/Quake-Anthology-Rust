// Vorbis decoder written in Rust
// Copyright (c) 2016 est31 <MTest31@outlook.com> and contributors.
// Licensed under MIT or Apache-2.0; see LICENSE.
#![forbid(unsafe_code)]
extern crate byteorder;
extern crate tinyvec;

macro_rules! try {
    ($expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(error) => return Err(::std::convert::From::from(error)),
        }
    };
}

pub mod audio;
mod bitpacking;
pub mod header;
mod header_cached;
mod huffman_tree;
mod imdct;
#[cfg(test)]
mod imdct_test;
mod samples;

fn ilog(value: u64) -> u8 {
    64 - value.leading_zeros() as u8
}
fn bit_reverse(value: u32) -> u32 {
    value.reverse_bits()
}
