//! Shared BER measurement: test-signal construction, codec selection, and
//! delay-aligned scoring used by both `run` and `stress`.

use crate::prbs::Prbs;
use dov_codec::{AmrMode, AmrNb, Codec, GsmFr};
use dov_modem::{bits_to_symbols, symbol_bit_errors, Demodulator, Modulator};

/// Payload symbols per measurement (≈ this many × 20 ms of audio).
pub const PAYLOAD_SYMBOLS: usize = 6000;
/// Known lead-in symbols excluded from the score (let the codec settle).
pub const GUARD_SYMBOLS: usize = 8;
/// Maximum codec algorithmic delay searched, in samples.
pub const MAX_DELAY: usize = 480;
/// Symbols used to lock alignment / measure delay.
pub const ALIGN_SYMBOLS: usize = 256;
/// Reproducible payload seed.
pub const SEED: u64 = 0xD0_F0_2026;

/// Which vocoder to instantiate.
#[derive(Clone, Copy)]
pub enum CodecKind {
    GsmFr,
    Amr(AmrMode),
}

impl CodecKind {
    /// The set we routinely sweep: GSM-FR plus the gentlest/median/harshest AMR.
    pub fn standard_set() -> Vec<CodecKind> {
        vec![
            CodecKind::GsmFr,
            CodecKind::Amr(AmrMode::Mr122),
            CodecKind::Amr(AmrMode::Mr795),
            CodecKind::Amr(AmrMode::Mr475),
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            CodecKind::GsmFr => "gsm-fr",
            CodecKind::Amr(AmrMode::Mr122) => "amr-12.2",
            CodecKind::Amr(AmrMode::Mr795) => "amr-7.95",
            CodecKind::Amr(AmrMode::Mr475) => "amr-4.75",
            CodecKind::Amr(_) => "amr",
        }
    }

    /// DTX (VAD) only applies to AMR; libgsm has no VAD, so the flag is ignored.
    pub fn make(self, dtx: bool) -> Box<dyn Codec> {
        match self {
            CodecKind::GsmFr => Box::new(GsmFr::new()),
            CodecKind::Amr(mode) => Box::new(AmrNb::with_dtx(mode, dtx)),
        }
    }
}

/// A fixed, known preamble of `len` symbols for receiver acquisition.
pub fn preamble(bits_per_symbol: usize, len: usize) -> Vec<u8> {
    let mut p = Prbs::new(0x0ACE_5EED);
    bits_to_symbols(&p.bits(len * bits_per_symbol), bits_per_symbol)
}

/// Build the transmit symbol stream (guard + PRBS payload) and its PCM.
pub fn build_tx(modulator: &Modulator) -> (Vec<u8>, Vec<i16>) {
    let cfg = modulator.config();
    let bps = cfg.bits_per_symbol();
    let alphabet = cfg.tones.len();

    let mut prbs = Prbs::new(SEED);
    let payload_bits = prbs.bits(PAYLOAD_SYMBOLS * bps);
    let payload_syms = bits_to_symbols(&payload_bits, bps);
    let guard: Vec<u8> = (0..GUARD_SYMBOLS).map(|i| (i % alphabet) as u8).collect();
    let tx_syms: Vec<u8> = guard.into_iter().chain(payload_syms).collect();

    let tx_pcm = modulator.modulate(&tx_syms);
    (tx_syms, tx_pcm)
}

/// Result of scoring one received stream.
pub struct Outcome {
    pub name: String,
    pub delay: usize,
    pub compared: usize,
    pub symbol_errors: usize,
    pub bit_errors: usize,
    pub confusion: Vec<Vec<u64>>, // [tx][rx]
}

impl Outcome {
    pub fn ser(&self) -> f64 {
        self.symbol_errors as f64 / self.compared.max(1) as f64
    }
    pub fn ber(&self, bits_per_symbol: usize) -> f64 {
        self.bit_errors as f64 / (self.compared.max(1) * bits_per_symbol) as f64
    }
}

/// Find the sample delay that best matches the head of the known `tx_syms`.
/// Recovers symbol timing and reveals the codec's algorithmic delay. (Real
/// timing recovery is M2; for a known-vector test this is the fair method.)
pub fn align(rx: &[i16], demod: &Demodulator, tx_syms: &[u8]) -> usize {
    let m = demod.config().symbol_len;
    let mut delay = 0usize;
    let mut best_matches = 0usize;
    for d in 0..=MAX_DELAY {
        let mut matches = 0usize;
        for (k, &want) in tx_syms.iter().take(ALIGN_SYMBOLS).enumerate() {
            let start = d + k * m;
            if start + m > rx.len() {
                break;
            }
            if demod.decide(&rx[start..start + m]).symbol == want {
                matches += 1;
            }
        }
        if matches > best_matches {
            best_matches = matches;
            delay = d;
        }
    }
    delay
}

/// Demodulate `count` symbols starting at sample `start`. Symbols that fall off
/// the end (e.g. after clock-drift shortening) come back as zero-margin so the
/// caller's erasure-flagging treats them as lost.
pub fn decisions(rx: &[i16], demod: &Demodulator, start: usize, count: usize) -> Vec<dov_modem::Decision> {
    let m = demod.config().symbol_len;
    (0..count)
        .map(|k| {
            let s = start + k * m;
            if s + m <= rx.len() {
                demod.decide(&rx[s..s + m])
            } else {
                dov_modem::Decision { symbol: 0, margin_db: 0.0 }
            }
        })
        .collect()
}

/// Demodulate `rx`, aligned to the known `tx_syms`, and tally errors.
pub fn score(name: &str, rx: &[i16], demod: &Demodulator, tx_syms: &[u8], alphabet: usize) -> Outcome {
    let m = demod.config().symbol_len;
    let delay = align(rx, demod, tx_syms);

    let mut confusion = vec![vec![0u64; alphabet]; alphabet];
    let mut symbol_errors = 0usize;
    let mut bit_errors = 0usize;
    let mut compared = 0usize;
    for (k, &want) in tx_syms.iter().enumerate().skip(GUARD_SYMBOLS) {
        let start = delay + k * m;
        if start + m > rx.len() {
            break;
        }
        let got = demod.decide(&rx[start..start + m]).symbol;
        confusion[want as usize][got as usize] += 1;
        compared += 1;
        if got != want {
            symbol_errors += 1;
        }
        bit_errors += symbol_bit_errors(want, got) as usize;
    }

    Outcome {
        name: name.to_string(),
        delay,
        compared,
        symbol_errors,
        bit_errors,
        confusion,
    }
}
