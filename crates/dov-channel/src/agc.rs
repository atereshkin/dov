//! Slow automatic gain control, as a handset/network would apply.
//!
//! For a constant-amplitude FSK signal this should settle to a fixed gain and
//! barely perturb a frequency-based modem — which is precisely the property we
//! want to confirm (no bits live in amplitude). It bites only during envelope
//! transients (e.g. the VAD-defeat pulse, added later).

#[derive(Clone)]
pub struct Agc {
    target: f64,
    gain: f64,
    env: f64,
    /// One-pole smoothing coefficient for the level estimate (per sample).
    smooth: f64,
}

impl Agc {
    /// `target` is the desired RMS-ish level in i16 counts.
    pub fn new(target: f64) -> Self {
        Self {
            target,
            gain: 1.0,
            env: target,
            smooth: 0.0005, // ~ tens of ms at 8 kHz
        }
    }

    pub fn process(&mut self, pcm: &mut [i16]) {
        for x in pcm.iter_mut() {
            let a = (*x as f64).abs();
            self.env += self.smooth * (a - self.env);
            let desired = self.target / self.env.max(1.0);
            // Move gain gently toward the desired value.
            self.gain += 0.01 * (desired - self.gain);
            let v = *x as f64 * self.gain;
            *x = v.round().clamp(-32768.0, 32767.0) as i16;
        }
    }
}
