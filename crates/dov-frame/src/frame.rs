//! `FrameCodec` — the user-facing FEC layer: RS(n, k) with depth-`d` interleaving.
//!
//! Encoding consumes `depth * k` payload bytes per interleave block and emits
//! `depth * n` coded bytes. Decoding reverses it, using per-byte erasure flags
//! (from the modem's confidence margin) to drive RS erasure correction, and
//! reports how many codewords it could and couldn't recover.

use crate::interleave;
use crate::rs;

pub struct FrameCodec {
    n: usize,
    k: usize,
    depth: usize,
}

/// Outcome of decoding, for measuring how well FEC held up.
#[derive(Debug, Default, Clone, Copy)]
pub struct DecodeStats {
    pub codewords_total: usize,
    pub codewords_failed: usize,
}

impl FrameCodec {
    /// RS(`n`, `k`) with interleaving `depth`. Requires `k < n ≤ 255`.
    pub fn new(n: usize, k: usize, depth: usize) -> Self {
        assert!(k < n && n <= 255 && depth >= 1, "invalid RS/interleave params");
        Self { n, k, depth }
    }

    pub fn nsym(&self) -> usize {
        self.n - self.k
    }

    /// Payload bytes consumed per interleave block.
    pub fn block_payload(&self) -> usize {
        self.depth * self.k
    }

    /// Coded bytes produced per interleave block.
    pub fn block_coded(&self) -> usize {
        self.depth * self.n
    }

    /// Code rate (payload / coded).
    pub fn rate(&self) -> f64 {
        self.k as f64 / self.n as f64
    }

    /// Encode `payload` (must be a whole number of blocks) into coded bytes.
    pub fn encode(&self, payload: &[u8]) -> Vec<u8> {
        assert_eq!(payload.len() % self.block_payload(), 0, "payload must be block-aligned");
        let mut out = Vec::with_capacity(payload.len() / self.k * self.n);
        for block in payload.chunks(self.block_payload()) {
            // RS-encode each of the `depth` codewords, lay them out row-major.
            let mut flat = vec![0u8; self.depth * self.n];
            for row in 0..self.depth {
                let msg = &block[row * self.k..(row + 1) * self.k];
                let cw = rs::encode(msg, self.nsym());
                flat[row * self.n..(row + 1) * self.n].copy_from_slice(&cw);
            }
            out.extend_from_slice(&interleave::interleave(&flat, self.depth, self.n));
        }
        out
    }

    /// Decode `coded` (with per-byte erasure flags) back to payload bytes.
    /// Failed codewords contribute their raw (possibly wrong) message bytes so
    /// output length always matches, and are counted in [`DecodeStats`].
    pub fn decode(&self, coded: &[u8], erased: &[bool]) -> (Vec<u8>, DecodeStats) {
        assert_eq!(coded.len(), erased.len());
        assert_eq!(coded.len() % self.block_coded(), 0, "coded must be block-aligned");

        let mut payload = Vec::with_capacity(coded.len() / self.n * self.k);
        let mut stats = DecodeStats::default();

        for (block, eblock) in coded
            .chunks(self.block_coded())
            .zip(erased.chunks(self.block_coded()))
        {
            let (rows, flags) = interleave::deinterleave(block, eblock, self.depth, self.n);
            for row in 0..self.depth {
                let cw = &rows[row * self.n..(row + 1) * self.n];
                let efl = &flags[row * self.n..(row + 1) * self.n];
                let erase_pos: Vec<usize> = (0..self.n).filter(|&i| efl[i]).collect();

                stats.codewords_total += 1;
                match rs::decode(cw, self.nsym(), &erase_pos) {
                    Ok(msg) => payload.extend_from_slice(&msg),
                    Err(_) => {
                        stats.codewords_failed += 1;
                        // Hand back the raw (systematic) message bytes uncorrected.
                        payload.extend_from_slice(&cw[..self.k]);
                    }
                }
            }
        }
        (payload, stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.0 >> 16
        }
        fn byte(&mut self) -> u8 {
            self.next() as u8
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn block_burst_is_correctable_after_interleaving() {
        // RS(64,48): nsym=16 → 16 erasures/codeword. depth=8.
        let fc = FrameCodec::new(64, 48, 8);
        let mut rng = Lcg(99);
        let payload: Vec<u8> = (0..fc.block_payload() * 3).map(|_| rng.byte()).collect();
        let coded = fc.encode(&payload);

        // Erase a burst of depth*16 = 128 consecutive coded bytes → after
        // deinterleaving that is 16 erasures per codeword == exactly nsym.
        let mut erased = vec![false; coded.len()];
        let mut corrupted = coded.clone();
        let burst = fc.depth * 16;
        let start = fc.block_coded(); // within the 2nd block
        for i in start..start + burst {
            erased[i] = true;
            corrupted[i] ^= rng.byte();
        }
        let (out, stats) = fc.decode(&corrupted, &erased);
        assert_eq!(stats.codewords_failed, 0, "burst should be fully corrected");
        assert_eq!(out, payload);
    }

    #[test]
    fn random_mixed_damage() {
        let fc = FrameCodec::new(64, 40, 8); // nsym=24
        let mut rng = Lcg(2024);
        let payload: Vec<u8> = (0..fc.block_payload() * 2).map(|_| rng.byte()).collect();
        let coded = fc.encode(&payload);
        let mut corrupted = coded.clone();
        let mut erased = vec![false; coded.len()];
        // ~8% random byte damage, half of it flagged as erasures
        for i in 0..coded.len() {
            if rng.below(100) < 8 {
                corrupted[i] ^= rng.byte() | 1;
                erased[i] = rng.below(2) == 0;
            }
        }
        let (out, _stats) = fc.decode(&corrupted, &erased);
        // Not guaranteed 100% (some codewords may exceed capacity), but the bulk
        // must recover. Compare byte error rate against the corrupted input.
        let raw_errs = corrupted
            .iter()
            .zip(&coded)
            .filter(|(a, b)| a != b)
            .count();
        let out_errs = out.iter().zip(&payload).filter(|(a, b)| a != b).count();
        assert!(out_errs * 10 < raw_errs, "FEC barely helped: {out_errs} vs raw {raw_errs}");
    }
}
