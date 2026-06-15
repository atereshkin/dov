//! `dov-frame` — framing and forward error correction for the DoV link.
//!
//! The M3 stress sweep showed frame erasure is the dominant impairment, so this
//! layer is built around it: Reed–Solomon *errors-and-erasures* coding
//! (`rs`) over GF(256) (`gf256`), plus a block `interleave`r that spreads each
//! codeword across the stream so a burst of lost vocoder frames becomes a few
//! correctable erasures in many codewords instead of a wipe-out in one.
#![forbid(unsafe_code)]

pub mod gf256;
pub mod interleave;
pub mod rs;

mod frame;
pub use frame::{DecodeStats, FrameCodec};
