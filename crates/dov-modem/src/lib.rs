//! `dov-modem` — the software modem that has to survive a voice vocoder.
//!
//! Pure DSP, no `unsafe`, no codec knowledge. The modulator turns bits into
//! 8 kHz `i16` PCM; the demodulator turns PCM back into symbols/bits. The
//! design follows directly from what a CELP/RPE-LTP vocoder keeps and throws
//! away (see `docs/DESIGN.md`):
//!
//!   * **Frequency survives; phase and absolute amplitude do not.** So the
//!     baseline is non-coherent M-FSK with a Goertzel tone bank, never PSK/QAM.
//!   * **Symbols are carried in frequency only**, at constant amplitude, in the
//!     empirically flat 600–2400 Hz region.
//!   * **Phase-continuous** tone switching to avoid wideband splatter that the
//!     vocoder would smear.
//!
//! M1 implements fixed-tone 8-FSK (proof-of-life + the per-transition survival
//! instrument). The differential "IncDec" mode (M2) and the trained speech-like
//! symbol codebook (M6) build on the same tone-bank front end.
#![forbid(unsafe_code)]

pub mod goertzel;
pub mod mfsk;
pub mod sync;

pub use mfsk::{Decision, Demodulator, MfskConfig, Modulator};
pub use sync::{Receiver, TimingRecovery};

/// Narrowband sample rate shared with the vocoders, in Hz.
pub const SAMPLE_RATE: f64 = 8_000.0;

/// Samples per 20 ms vocoder frame.
pub const FRAME_LEN: usize = 160;

/// Pack a slice of 0/1 bits into symbol values, `bits_per_symbol` at a time,
/// most-significant bit first. A trailing partial group is zero-padded.
pub fn bits_to_symbols(bits: &[u8], bits_per_symbol: usize) -> Vec<u8> {
    bits
        .chunks(bits_per_symbol)
        .map(|chunk| {
            let mut v = 0u8;
            for i in 0..bits_per_symbol {
                v = (v << 1) | chunk.get(i).copied().unwrap_or(0);
            }
            v
        })
        .collect()
}

/// Inverse of [`bits_to_symbols`]: expand symbol values back to 0/1 bits,
/// most-significant bit first.
pub fn symbols_to_bits(symbols: &[u8], bits_per_symbol: usize) -> Vec<u8> {
    let mut bits = Vec::with_capacity(symbols.len() * bits_per_symbol);
    for &s in symbols {
        for i in (0..bits_per_symbol).rev() {
            bits.push((s >> i) & 1);
        }
    }
    bits
}

/// Hamming distance between two symbol values (number of differing bits).
pub fn symbol_bit_errors(a: u8, b: u8) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_symbols_roundtrip() {
        let bits = [1, 0, 1, 1, 0, 0, 1, 0, 1];
        let syms = bits_to_symbols(&bits, 3);
        assert_eq!(syms, vec![0b101, 0b100, 0b101]);
        assert_eq!(symbols_to_bits(&syms, 3), bits);
    }
}
