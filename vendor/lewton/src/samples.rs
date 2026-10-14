// Vorbis decoder written in Rust
// Copyright (c) 2016 est31 <MTest31@outlook.com> and contributors.
// Licensed under MIT or Apache-2.0; see LICENSE.
pub trait Sample {
    fn from_float(value: f32) -> Self;
}
impl Sample for i16 {
    fn from_float(value: f32) -> Self {
        let value = value * 32768.0;
        if value > 32767.0 {
            32767
        } else if value < -32768.0 {
            -32768
        } else {
            value as i16
        }
    }
}
