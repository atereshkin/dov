//! CVSD — the narrowband Bluetooth HFP/SCO voice codec, as a [`Codec`].
//!
//! A Bluetooth hands-free voice call carries audio over an SCO link coded with
//! CVSD (continuously variable slope delta modulation): the 8 kHz PCM is
//! oversampled 8× to a 64 kbit/s, 1-bit delta stream with an adaptive step, then
//! reconstructed and decimated back to 8 kHz. Modelling it lets us measure how
//! much a `PC → BT → phone → GSM → …` tandem degrades the modem before any
//! hardware is plugged in.
//!
//! Parameters follow the Bluetooth-style CVSD (accumulator leak `h = 1 − 1/32`,
//! GNU Radio reference step values). This is a faithful model, not a bit-exact
//! controller implementation; the real BT measurement will calibrate it.

use crate::{Codec, FRAME_LEN};

const OVERSAMPLE: usize = 8;
const ACCUM_LEAK: f64 = 1.0 - 1.0 / 32.0; // h
const MIN_STEP: f64 = 10.0;
const MAX_STEP: f64 = 1280.0;
const STEP_DECAY: f64 = 0.9990; // β
const YMIN: f64 = -32768.0;
const YMAX: f64 = 32767.0;
/// Output reconstruction / anti-alias filter corner (Hz) before decimation.
const LPF_FC: f64 = 3600.0;
const OS_RATE: f64 = (crate::SAMPLE_RATE as usize * OVERSAMPLE) as f64;

/// One RBJ biquad low-pass section (Direct Form I).
#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn lowpass(fc: f64, fs: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * fc / fs;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - c) / 2.0 / a0,
            b1: (1.0 - c) / a0,
            b2: (1.0 - c) / 2.0 / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Bluetooth-style CVSD codec round-trip.
pub struct Cvsd {
    acc: f64,
    delta: f64,
    hist: u8, // last 4 bits
    prev_in: f64,
    lpf: [Biquad; 2], // 4th-order Butterworth
}

impl Cvsd {
    pub fn new() -> Self {
        // 4th-order Butterworth = two biquads with these Qs.
        Self {
            acc: 0.0,
            delta: MIN_STEP,
            hist: 0b1010, // alternating → no false overload at startup
            prev_in: 0.0,
            lpf: [
                Biquad::lowpass(LPF_FC, OS_RATE, 0.541_20),
                Biquad::lowpass(LPF_FC, OS_RATE, 1.306_56),
            ],
        }
    }

    #[inline]
    fn step(&mut self, x: f64) -> f64 {
        // 1-bit quantiser: is the input above the running estimate?
        let bit = x >= self.acc;
        self.hist = ((self.hist << 1) | bit as u8) & 0x0F;
        // Syllabic companding: a run of equal bits = slope overload → grow step.
        if self.hist == 0x0F || self.hist == 0x00 {
            self.delta = (self.delta + MIN_STEP).min(MAX_STEP);
        } else {
            self.delta = (self.delta * STEP_DECAY).max(MIN_STEP);
        }
        // Leaky integrator reconstruction (encoder and decoder share this, so for
        // an error-free round-trip the decoded value equals this accumulator).
        self.acc = (ACCUM_LEAK * self.acc + if bit { self.delta } else { -self.delta })
            .clamp(YMIN, YMAX);
        self.acc
    }
}

impl Default for Cvsd {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for Cvsd {
    fn name(&self) -> &str {
        "cvsd"
    }

    fn process_frame(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        let mut out = [0i16; FRAME_LEN];
        let mut out_pos = 0;
        // Walk the 8× oversampled grid; linear-interpolate the input, run CVSD,
        // low-pass the reconstruction, and decimate back by 8.
        for n in 0..FRAME_LEN * OVERSAMPLE {
            let t = n as f64 / OVERSAMPLE as f64; // position in [0, FRAME_LEN)
            let lo = t.floor() as usize;
            let frac = t - lo as f64;
            // extended[0] = prev_in, extended[k] = input[k-1]
            let s_lo = if lo == 0 { self.prev_in } else { input[lo - 1] as f64 };
            let s_hi = input[lo] as f64;
            let x = s_lo * (1.0 - frac) + s_hi * frac;

            let recon = self.step(x);
            let y0 = self.lpf[0].process(recon);
            let y = self.lpf[1].process(y0);
            if n % OVERSAMPLE == 0 {
                out[out_pos] = y.round().clamp(YMIN, YMAX) as i16;
                out_pos += 1;
            }
        }
        self.prev_in = input[FRAME_LEN - 1] as f64;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn goertzel(s: &[i16], f: f64) -> f64 {
        let w = 2.0 * PI * f / crate::SAMPLE_RATE as f64;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for &x in s {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        s1 * s1 + s2 * s2 - coeff * s1 * s2
    }

    #[test]
    fn tone_survives_cvsd() {
        let sr = crate::SAMPLE_RATE as f64;
        let input: Vec<i16> = (0..4000)
            .map(|i| (8000.0 * (2.0 * PI * 1000.0 * i as f64 / sr).sin()).round() as i16)
            .collect();
        let out = Cvsd::new().process(&input);

        // The 1 kHz tone must dominate, and survive far better than a decoy bin.
        let on = goertzel(&out, 1000.0);
        let off = goertzel(&out, 1800.0);
        assert!(on > off * 20.0, "1kHz should dominate: on={on:.3e} off={off:.3e}");

        // And it should still correlate strongly with the input once the codec's
        // group delay is accounted for (the reconstruction is shape-preserving).
        let mut best = 0.0f64;
        for lag in 0..=64usize {
            let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
            for i in 200..(out.len() - 64) {
                let (a, b) = (input[i] as f64, out[i + lag] as f64);
                sab += a * b;
                saa += a * a;
                sbb += b * b;
            }
            let corr = sab / (saa.sqrt() * sbb.sqrt());
            best = best.max(corr);
        }
        assert!(best > 0.85, "CVSD round-trip correlation too low: {best:.3}");
    }
}
