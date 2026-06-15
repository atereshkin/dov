//! Block interleaving to break loss bursts across codewords.
//!
//! `depth` codewords of `n` bytes are written as the rows of a `depth × n`
//! matrix and transmitted column by column. Consecutive transmitted bytes then
//! belong to *different* codewords, so a burst of `B` consecutive losses lands
//! at most `⌈B / depth⌉` erasures on any single codeword.

/// Interleave `depth` codewords (each `n` bytes) into one `depth*n` stream,
/// column-major. `flat` must be `depth * n` bytes laid out codeword-by-codeword.
pub fn interleave(flat: &[u8], depth: usize, n: usize) -> Vec<u8> {
    debug_assert_eq!(flat.len(), depth * n);
    let mut out = vec![0u8; depth * n];
    for row in 0..depth {
        for col in 0..n {
            out[col * depth + row] = flat[row * n + col];
        }
    }
    out
}

/// Inverse of [`interleave`] for both the bytes and their per-byte erasure flags.
pub fn deinterleave(stream: &[u8], erased: &[bool], depth: usize, n: usize) -> (Vec<u8>, Vec<bool>) {
    debug_assert_eq!(stream.len(), depth * n);
    debug_assert_eq!(erased.len(), depth * n);
    let mut bytes = vec![0u8; depth * n];
    let mut flags = vec![false; depth * n];
    for row in 0..depth {
        for col in 0..n {
            bytes[row * n + col] = stream[col * depth + row];
            flags[row * n + col] = erased[col * depth + row];
        }
    }
    (bytes, flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let depth = 4;
        let n = 6;
        let flat: Vec<u8> = (0..(depth * n) as u8).collect();
        let inter = interleave(&flat, depth, n);
        let flags = vec![false; depth * n];
        let (back, _) = deinterleave(&inter, &flags, depth, n);
        assert_eq!(back, flat);
    }

    #[test]
    fn burst_spreads_across_codewords() {
        let depth = 8;
        let n = 16;
        let flat: Vec<u8> = vec![0u8; depth * n];
        let inter = interleave(&flat, depth, n);
        let _ = inter;
        // A burst of `depth` consecutive losses hits one byte per codeword.
        let mut erased = vec![false; depth * n];
        for e in erased.iter_mut().take(depth) {
            *e = true;
        }
        let (_, flags) = deinterleave(&vec![0u8; depth * n], &erased, depth, n);
        // each codeword (row) should have exactly one erased byte
        for row in 0..depth {
            let cnt = (0..n).filter(|&col| flags[row * n + col]).count();
            assert_eq!(cnt, 1, "row {row}");
        }
    }
}
