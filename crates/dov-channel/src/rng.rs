//! Deterministic RNG with a Gaussian draw, for reproducible impairments.

pub struct Rng {
    state: u64,
    spare: Option<f64>,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed | 0x9E37_79B9_7F4A_7C15,
            spare: None,
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

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        // top 53 bits → f64 mantissa
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal via Box–Muller (cached spare).
    pub fn gaussian(&mut self) -> f64 {
        if let Some(z) = self.spare.take() {
            return z;
        }
        // Avoid log(0).
        let u1 = self.uniform().max(1e-12);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f64::consts::PI * u2;
        self.spare = Some(r * theta.sin());
        r * theta.cos()
    }
}
