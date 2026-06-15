//! Deterministic pseudo-random bit source for reproducible BER runs.
//!
//! A 64-bit xorshift is plenty for test payloads and lets every run be diffed
//! against the last. (Not cryptographic — that lives much higher up the stack.)

pub struct Prbs {
    state: u64,
}

impl Prbs {
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero fixed point.
        Self {
            state: seed | 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Generate `n` bits, each 0 or 1.
    pub fn bits(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let word = self.next_u64();
            for i in 0..64 {
                if out.len() == n {
                    break;
                }
                out.push(((word >> i) & 1) as u8);
            }
        }
        out
    }
}
