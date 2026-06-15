//! Generalised Goertzel single-frequency power estimator.
//!
//! Cheaper than a full FFT when we only need the energy at a handful of known
//! tone frequencies, which is exactly the M-FSK demodulator's job.

use crate::SAMPLE_RATE;
use std::f64::consts::PI;

/// Relative power of `samples` at `freq` (Hz), sampled at [`SAMPLE_RATE`].
///
/// The absolute scale is arbitrary; only ratios between frequencies (which
/// tone is strongest) or between signals (input vs output) are meaningful.
pub fn power(samples: &[i16], freq: f64) -> f64 {
    power_at(samples, freq, SAMPLE_RATE)
}

/// As [`power`] but with an explicit sample rate.
pub fn power_at(samples: &[i16], freq: f64, sample_rate: f64) -> f64 {
    let w = 2.0 * PI * freq / sample_rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn tone(freq: f64, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| (8000.0 * (2.0 * PI * freq * i as f64 / SAMPLE_RATE).sin()) as i16)
            .collect()
    }

    #[test]
    fn picks_the_right_tone() {
        let s = tone(1100.0, 160);
        let on = power(&s, 1100.0);
        let off = power(&s, 1900.0);
        assert!(on > off * 50.0, "on={on} off={off}");
    }
}
