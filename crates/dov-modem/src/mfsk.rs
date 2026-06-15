//! Non-coherent M-FSK over a vocoder channel.
//!
//! Fixed-tone alphabet, phase-continuous modulation, Goertzel-bank demod with
//! a centred analysis window (the symbol edges are where the vocoder smears
//! frequency transitions the most, so we ignore them when deciding).

use crate::goertzel;
use crate::{FRAME_LEN, SAMPLE_RATE};
use std::f64::consts::PI;

/// Configuration of an M-FSK alphabet.
#[derive(Clone, Debug)]
pub struct MfskConfig {
    /// Tone frequencies in Hz; `len()` must be a power of two.
    pub tones: Vec<f64>,
    /// Samples per symbol (20 ms = 160 keeps symbols frame-aligned).
    pub symbol_len: usize,
    /// Peak amplitude in `i16` counts (constant — no bits live in amplitude).
    pub amplitude: f64,
    /// Raised-cosine fade length, in samples, applied at the very start and end
    /// of a transmission to avoid a hard click. (Not per-symbol — that would be
    /// amplitude modulation.)
    pub edge_ramp: usize,
    /// Fraction of the symbol trimmed from each edge before the Goertzel
    /// decision, to dodge vocoder transition smearing. 0.0–0.49.
    pub decision_guard: f64,
}

impl MfskConfig {
    /// M1 proof-of-life alphabet: 8 tones, 700–2100 Hz, 200 Hz spacing, in the
    /// empirically flat mid-band; 20 ms/symbol (frame-aligned) → 3 bits/symbol,
    /// 50 baud = 150 bps raw. (The plan's "1200 bps" conflated 20 ms symbols
    /// with 400 baud; raising the rate is M2's differential sub-frame scheme.)
    pub fn fsk8() -> Self {
        Self {
            tones: (0..8).map(|i| 700.0 + 200.0 * i as f64).collect(),
            symbol_len: FRAME_LEN,
            amplitude: 8000.0,
            edge_ramp: 40, // 5 ms
            decision_guard: 0.15,
        }
    }

    /// Codec-agnostic higher-rate alphabet: 16 tones, 600–2400 Hz, 120 Hz
    /// spacing, still frame-aligned (20 ms) so it survives every codec's
    /// per-frame model. 4 bits/symbol → 200 bps raw, a +33% on [`Self::fsk8`]
    /// at the same robustness (per the `rate` frontier sweep).
    pub fn fsk16() -> Self {
        Self {
            tones: (0..16).map(|i| 600.0 + 120.0 * i as f64).collect(),
            symbol_len: FRAME_LEN,
            amplitude: 8000.0,
            edge_ramp: 40,
            decision_guard: 0.1,
        }
    }

    /// Bits carried per symbol = log2(alphabet size).
    pub fn bits_per_symbol(&self) -> usize {
        debug_assert!(self.tones.len().is_power_of_two());
        self.tones.len().trailing_zeros() as usize
    }

    /// Raw bit rate in bits per second.
    pub fn raw_bitrate(&self) -> f64 {
        self.bits_per_symbol() as f64 * SAMPLE_RATE / self.symbol_len as f64
    }
}

/// Phase-continuous M-FSK modulator. Stateless across calls.
pub struct Modulator {
    cfg: MfskConfig,
}

impl Modulator {
    pub fn new(cfg: MfskConfig) -> Self {
        Self { cfg }
    }

    pub fn config(&self) -> &MfskConfig {
        &self.cfg
    }

    /// Modulate a sequence of symbol values into 8 kHz `i16` PCM.
    pub fn modulate(&self, symbols: &[u8]) -> Vec<i16> {
        let mut out = Vec::with_capacity(symbols.len() * self.cfg.symbol_len);
        let mut phase = 0.0f64;
        for &s in symbols {
            let freq = self.cfg.tones[s as usize];
            let dphi = 2.0 * PI * freq / SAMPLE_RATE;
            for _ in 0..self.cfg.symbol_len {
                let sample = self.cfg.amplitude * phase.sin();
                out.push(sample.round().clamp(-32768.0, 32767.0) as i16);
                phase += dphi;
                if phase >= 2.0 * PI {
                    phase -= 2.0 * PI;
                }
            }
        }
        apply_edge_fades(&mut out, self.cfg.edge_ramp);
        out
    }
}

/// Result of demodulating one symbol window.
#[derive(Clone, Copy, Debug)]
pub struct Decision {
    /// Most likely symbol value.
    pub symbol: u8,
    /// Confidence margin in dB: how far the winning tone's power is above the
    /// runner-up. Small margins mark frames worth flagging as erasures later.
    pub margin_db: f64,
}

/// Goertzel-bank M-FSK demodulator.
pub struct Demodulator {
    cfg: MfskConfig,
}

impl Demodulator {
    pub fn new(cfg: MfskConfig) -> Self {
        Self { cfg }
    }

    pub fn config(&self) -> &MfskConfig {
        &self.cfg
    }

    /// Decide one symbol from exactly `symbol_len` samples.
    pub fn decide(&self, window: &[i16]) -> Decision {
        debug_assert_eq!(window.len(), self.cfg.symbol_len);
        let trim = (self.cfg.symbol_len as f64 * self.cfg.decision_guard) as usize;
        let sub = &window[trim..self.cfg.symbol_len - trim];

        let mut best = (0usize, f64::MIN);
        let mut second = f64::MIN;
        for (i, &freq) in self.cfg.tones.iter().enumerate() {
            let p = goertzel::power(sub, freq);
            if p > best.1 {
                second = best.1;
                best = (i, p);
            } else if p > second {
                second = p;
            }
        }
        let margin_db = 10.0 * (best.1 / second.max(1e-12)).max(1e-12).log10();
        Decision {
            symbol: best.0 as u8,
            margin_db,
        }
    }

    /// Demodulate a contiguous run of symbols starting at sample `offset`.
    /// Stops when fewer than `symbol_len` samples remain.
    pub fn demodulate(&self, pcm: &[i16], offset: usize) -> Vec<Decision> {
        let m = self.cfg.symbol_len;
        let mut out = Vec::new();
        let mut pos = offset;
        while pos + m <= pcm.len() {
            out.push(self.decide(&pcm[pos..pos + m]));
            pos += m;
        }
        out
    }
}

/// Apply a raised-cosine fade-in/out over `ramp` samples at each end.
fn apply_edge_fades(pcm: &mut [i16], ramp: usize) {
    let ramp = ramp.min(pcm.len() / 2);
    if ramp == 0 {
        return;
    }
    let n = pcm.len();
    for i in 0..ramp {
        let w = 0.5 * (1.0 - (PI * i as f64 / ramp as f64).cos());
        pcm[i] = (pcm[i] as f64 * w).round() as i16;
        pcm[n - 1 - i] = (pcm[n - 1 - i] as f64 * w).round() as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clean-channel sanity: modulate then demodulate must round-trip exactly.
    #[test]
    fn clean_roundtrip() {
        let cfg = MfskConfig::fsk8();
        let m = Modulator::new(cfg.clone());
        let d = Demodulator::new(cfg.clone());
        let syms: Vec<u8> = (0..200).map(|i| (i % 8) as u8).collect();
        let pcm = m.modulate(&syms);
        let got: Vec<u8> = d.demodulate(&pcm, 0).iter().map(|x| x.symbol).collect();
        // Edge fades can disturb only the first/last symbol; check the interior.
        assert_eq!(&got[1..199], &syms[1..199]);
    }

    #[test]
    fn rates_are_sane() {
        let cfg = MfskConfig::fsk8();
        assert_eq!(cfg.bits_per_symbol(), 3);
        assert_eq!(cfg.raw_bitrate(), 150.0); // 50 baud × 3 bits

    }
}
