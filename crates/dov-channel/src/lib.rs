//! `dov-channel` — the realism layer between the modem and the raw vocoder.
//!
//! A [`Channel`] wraps a [`Codec`] and applies, in order:
//!   1. pre-codec line conditions (DC offset, level into the encoder),
//!   2. the vocoder itself, frame by frame, with **frame erasure → native PLC**,
//!   3. post-codec impairments (AGC, additive noise),
//!   4. receiver **clock drift** (resampling).
//!
//! VAD/DTX is not a stage here — it lives *inside* the codec (construct the
//! `Codec` with DTX enabled), because it is the encoder's own decision.

pub mod agc;
pub mod erasure;
pub mod resample;
pub mod rng;

pub use agc::Agc;
pub use erasure::Erasure;
pub use rng::Rng;

use dov_codec::{Codec, FRAME_LEN};

/// Which impairments to apply, and how hard.
#[derive(Default)]
pub struct ChannelConfig {
    /// Added DC offset (counts) before the encoder.
    pub dc_offset: i16,
    /// Linear gain applied before the encoder (1.0 = unchanged).
    pub pre_gain: f64,
    /// Frame-erasure model; `None` = lossless.
    pub erasure: Option<Erasure>,
    /// Receive-side AGC; `None` = off.
    pub agc: Option<Agc>,
    /// Post-codec additive white Gaussian noise at this SNR (dB); `None` = off.
    pub awgn_snr_db: Option<f64>,
    /// Receiver clock error in ppm (0 = perfect clock).
    pub clock_ppm: f64,
}

impl ChannelConfig {
    /// A clean channel (no impairments). `pre_gain` defaults to 1.0.
    pub fn clean() -> Self {
        Self {
            pre_gain: 1.0,
            ..Default::default()
        }
    }
}

pub struct Channel {
    codec: Box<dyn Codec>,
    cfg: ChannelConfig,
    rng: Rng,
    last_erasures: usize,
}

impl Channel {
    pub fn new(codec: Box<dyn Codec>, cfg: ChannelConfig, seed: u64) -> Self {
        Self {
            codec,
            cfg,
            rng: Rng::new(seed),
            last_erasures: 0,
        }
    }

    /// Number of frames erased during the most recent [`run`](Self::run).
    pub fn last_erasures(&self) -> usize {
        self.last_erasures
    }

    /// Push a transmit signal through the whole channel and return what the
    /// receiver's demodulator would see.
    pub fn run(&mut self, tx: &[i16]) -> Vec<i16> {
        // 1. Pre-codec line conditioning.
        let gain = if self.cfg.pre_gain == 0.0 { 1.0 } else { self.cfg.pre_gain };
        let dc = self.cfg.dc_offset as f64;
        let pre: Vec<i16> = tx
            .iter()
            .map(|&x| (x as f64 * gain + dc).round().clamp(-32768.0, 32767.0) as i16)
            .collect();

        // 2. Vocoder, frame by frame, with erasure → concealment.
        let mut rx = Vec::with_capacity(pre.len());
        let mut frame = [0i16; FRAME_LEN];
        let mut erasures = 0usize;
        for chunk in pre.chunks(FRAME_LEN) {
            frame[..chunk.len()].copy_from_slice(chunk);
            frame[chunk.len()..].fill(0);
            let erased = self
                .cfg
                .erasure
                .as_mut()
                .is_some_and(|e| e.tick(&mut self.rng));
            let out = if erased {
                erasures += 1;
                self.codec.process_frame_erased(&frame)
            } else {
                self.codec.process_frame(&frame)
            };
            rx.extend_from_slice(&out);
        }
        self.last_erasures = erasures;

        // 3. Post-codec impairments.
        if let Some(agc) = self.cfg.agc.as_mut() {
            agc.process(&mut rx);
        }
        if let Some(snr) = self.cfg.awgn_snr_db {
            add_awgn(&mut rx, snr, &mut self.rng);
        }

        // 4. Receiver clock drift.
        if self.cfg.clock_ppm != 0.0 {
            rx = resample::drift(&rx, self.cfg.clock_ppm);
        }
        rx
    }
}

/// Add white Gaussian noise at a given SNR (dB) relative to the signal's power.
fn add_awgn(pcm: &mut [i16], snr_db: f64, rng: &mut Rng) {
    if pcm.is_empty() {
        return;
    }
    let power: f64 = pcm.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / pcm.len() as f64;
    let noise_power = power / 10f64.powf(snr_db / 10.0);
    let std = noise_power.sqrt();
    for x in pcm.iter_mut() {
        let v = *x as f64 + rng.gaussian() * std;
        *x = v.round().clamp(-32768.0, 32767.0) as i16;
    }
}
