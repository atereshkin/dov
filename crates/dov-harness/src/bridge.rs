//! Bridge between RS-coded *bytes* and FSK *symbols*, carrying erasure flags.
//!
//! The modem speaks 3-bit symbols; the FEC speaks bytes. Serialising bits
//! MSB-first in both directions keeps them aligned. On receive, a byte is
//! marked erased if *any* of the symbols covering its bits was low-confidence —
//! conservative, which is the safe direction for erasure decoding.

use dov_modem::Decision;

/// Pack coded bytes into FSK symbols (`bits_per_symbol` bits each, MSB-first).
pub fn coded_to_symbols(coded: &[u8], bits_per_symbol: usize) -> Vec<u8> {
    let mut bits = Vec::with_capacity(coded.len() * 8);
    for &b in coded {
        for i in (0..8).rev() {
            bits.push((b >> i) & 1);
        }
    }
    while bits.len() % bits_per_symbol != 0 {
        bits.push(0);
    }
    bits.chunks(bits_per_symbol)
        .map(|c| c.iter().fold(0u8, |v, &x| (v << 1) | x))
        .collect()
}

/// Reconstruct coded bytes + per-byte erasure flags from demod decisions.
///
/// A symbol is treated as erased when its confidence margin is below
/// `margin_db`. Output is truncated/padded to exactly `n_coded_bytes`.
pub fn decisions_to_coded(
    decisions: &[Decision],
    bits_per_symbol: usize,
    n_coded_bytes: usize,
    margin_db: f64,
) -> (Vec<u8>, Vec<bool>) {
    let mut bits = Vec::with_capacity(decisions.len() * bits_per_symbol);
    let mut bit_erased = Vec::with_capacity(decisions.len() * bits_per_symbol);
    for d in decisions {
        let erased = d.margin_db < margin_db;
        for i in (0..bits_per_symbol).rev() {
            bits.push((d.symbol >> i) & 1);
            bit_erased.push(erased);
        }
    }
    let need = n_coded_bytes * 8;
    bits.resize(need, 0);
    bit_erased.resize(need, true); // missing bits = erased

    let mut bytes = vec![0u8; n_coded_bytes];
    let mut erased = vec![false; n_coded_bytes];
    for bi in 0..n_coded_bytes {
        let mut v = 0u8;
        let mut er = false;
        for j in 0..8 {
            v = (v << 1) | bits[bi * 8 + j];
            er |= bit_erased[bi * 8 + j];
        }
        bytes[bi] = v;
        erased[bi] = er;
    }
    (bytes, erased)
}
