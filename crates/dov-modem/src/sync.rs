//! Receiver-side synchronisation: preamble acquisition + symbol-clock tracking.
//!
//! The M1 harness "cheated" by aligning against the known data sequence, which
//! cannot follow a drifting sample clock — over a long burst the symbol window
//! walks off, and (as M4 showed) FEC can't repair the resulting contiguous error
//! run. A real receiver instead:
//!
//!   1. **acquires** on a short *known preamble* (correlation over candidate
//!      offsets), recovering the initial symbol boundary + codec delay, then
//!   2. **tracks** the symbol clock through the unknown data with an early-late
//!      energy gate, so ±ppm drift is followed instead of accumulated.
//!
//! Both endpoints run off independent crystals, so tracking — not a fixed
//! offset — is what makes the link survive a real call.

use crate::goertzel;
use crate::mfsk::{Decision, Demodulator};

/// Early-late timing-recovery parameters.
#[derive(Clone, Copy, Debug)]
pub struct TimingRecovery {
    /// Half-spacing of the early/late energy probes, in samples.
    pub early_late: usize,
    /// Loop gain: samples of clock correction per unit normalised error.
    pub gain: f64,
}

impl Default for TimingRecovery {
    fn default() -> Self {
        // δ=10 samples gives a clean gradient at 160 samples/symbol; a small
        // gain tracks tens of ppm without jittering on codec noise.
        Self { early_late: 10, gain: 0.2 }
    }
}

pub struct Receiver<'a> {
    demod: &'a Demodulator,
    timing: TimingRecovery,
}

impl<'a> Receiver<'a> {
    pub fn new(demod: &'a Demodulator) -> Self {
        Self { demod, timing: TimingRecovery::default() }
    }

    pub fn with_timing(demod: &'a Demodulator, timing: TimingRecovery) -> Self {
        Self { demod, timing }
    }

    /// Find the sample offset of the first preamble symbol by best match over
    /// `0..=max_delay`. Returns `None` if fewer than half the preamble symbols
    /// match (no confident acquisition).
    pub fn acquire(&self, rx: &[i16], preamble: &[u8], max_delay: usize) -> Option<usize> {
        let m = self.demod.config().symbol_len;
        let mut best_offset = 0usize;
        let mut best_matches = 0usize;
        for d in 0..=max_delay {
            if d + preamble.len() * m > rx.len() {
                break;
            }
            let mut matches = 0usize;
            for (k, &want) in preamble.iter().enumerate() {
                let s = d + k * m;
                if self.demod.decide(&rx[s..s + m]).symbol == want {
                    matches += 1;
                }
            }
            if matches > best_matches {
                best_matches = matches;
                best_offset = d;
            }
        }
        (best_matches * 2 >= preamble.len()).then_some(best_offset)
    }

    /// Demodulate `n` symbols beginning at sample `start`, tracking the symbol
    /// clock so a drifting sample rate does not accumulate timing slip.
    pub fn demodulate_tracked(&self, rx: &[i16], start: usize, n: usize) -> Vec<Decision> {
        let cfg = self.demod.config();
        let m = cfg.symbol_len;
        let di = self.timing.early_late;

        let mut pos = start as f64;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let i = pos.round() as usize;
            if i + m > rx.len() {
                out.push(Decision { symbol: 0, margin_db: 0.0 });
                pos += m as f64;
                continue;
            }
            let dec = self.demod.decide(&rx[i..i + m]);

            // Early-late gate on the winning tone: if the symbol's energy is
            // stronger in the later-shifted window we are sampling early, so
            // advance the clock a touch (and vice versa).
            let f = cfg.tones[dec.symbol as usize];
            let e_early = if i >= di {
                goertzel::power(&rx[i - di..i - di + m], f)
            } else {
                0.0
            };
            let e_late = if i + di + m <= rx.len() {
                goertzel::power(&rx[i + di..i + di + m], f)
            } else {
                0.0
            };
            let err = (e_late - e_early) / (e_late + e_early + 1.0);
            pos += m as f64 + self.timing.gain * err;

            out.push(dec);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mfsk::{MfskConfig, Modulator};

    #[test]
    fn acquire_and_track_clean() {
        let cfg = MfskConfig::fsk8();
        let m = cfg.symbol_len;
        let modulator = Modulator::new(cfg.clone());
        let demod = Demodulator::new(cfg.clone());

        let preamble: Vec<u8> = [0u8, 5, 2, 7, 1, 6, 3, 4, 0, 7].to_vec();
        let data: Vec<u8> = (0..500).map(|i| (i % 8) as u8).collect();
        let tx: Vec<u8> = preamble.iter().copied().chain(data.iter().copied()).collect();
        let pcm = modulator.modulate(&tx);

        let rx = Receiver::new(&demod);
        let off = rx.acquire(&pcm, &preamble, 400).expect("acquire");
        assert_eq!(off, 0, "clean signal starts at 0");

        let got = rx.demodulate_tracked(&pcm, preamble.len() * m, data.len());
        let syms: Vec<u8> = got.iter().map(|d| d.symbol).collect();
        // interior must match (edge fade can disturb only the last symbol)
        assert_eq!(&syms[..data.len() - 1], &data[..data.len() - 1]);
    }

    #[test]
    fn tracking_follows_drift() {
        // Emulate a heavy clock offset by resampling, then check tracked demod
        // beats a fixed stride. Pure tones are very timing-tolerant, so we push
        // the drift hard to exercise the loop unambiguously; the realistic
        // ±50 ppm case is validated against real codecs in the harness.
        let cfg = MfskConfig::fsk8();
        let m = cfg.symbol_len;
        let modulator = Modulator::new(cfg.clone());
        let demod = Demodulator::new(cfg.clone());

        let preamble: Vec<u8> = [0u8, 5, 2, 7, 1, 6, 3, 4, 0, 7].to_vec();
        let data: Vec<u8> = (0..4000).map(|i| ((i * 3 + 1) % 8) as u8).collect();
        let tx: Vec<u8> = preamble.iter().copied().chain(data.iter().copied()).collect();
        let pcm = modulator.modulate(&tx);

        // resample to ~ +300 ppm (cumulative slip > one full symbol over the run)
        let ratio = 1.0 + 300e-6;
        let out_len = (pcm.len() as f64 / ratio) as usize;
        let drifted: Vec<i16> = (0..out_len)
            .map(|j| {
                let p = j as f64 * ratio;
                let a = p.floor() as usize;
                let frac = p - a as f64;
                let s0 = pcm[a] as f64;
                let s1 = pcm[(a + 1).min(pcm.len() - 1)] as f64;
                (s0 * (1.0 - frac) + s1 * frac).round() as i16
            })
            .collect();

        let rx = Receiver::new(&demod);
        let off = rx.acquire(&drifted, &preamble, 400).expect("acquire");

        // tracked
        let tracked: Vec<u8> = rx
            .demodulate_tracked(&drifted, off + preamble.len() * m, data.len())
            .iter()
            .map(|d| d.symbol)
            .collect();
        let tracked_err = tracked.iter().zip(&data).filter(|(a, b)| a != b).count();

        // fixed stride (no tracking)
        let mut fixed_err = 0usize;
        for (k, &want) in data.iter().enumerate() {
            let s = off + (preamble.len() + k) * m;
            if s + m <= drifted.len() {
                if demod.decide(&drifted[s..s + m]).symbol != want {
                    fixed_err += 1;
                }
            } else {
                fixed_err += 1;
            }
        }

        assert!(
            tracked_err * 5 < fixed_err.max(1),
            "tracking should crush fixed: tracked={tracked_err} fixed={fixed_err}"
        );
    }
}
