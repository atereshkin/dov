//! `adapt` subcommand — link rate adaptation.
//!
//! The `rate` sweep showed the best modem configuration depends on which codec
//! the call rides: EFR/AMR-12.2 sustains 600 bps sub-frame signalling, GSM-FR
//! tops out at frame-aligned 200 bps, AMR-4.75 needs less still. A real link
//! can't know the codec in advance — so it *sounds* the channel at setup: send a
//! short known sequence at each candidate profile through the actual codec,
//! measure the raw BER with the real receiver, and commit to the fastest profile
//! whose BER is comfortably FEC-correctable.
//!
//! This turns the frontier finding into throughput: each codec gets the highest
//! rate it can actually carry.

use crate::ber::{self, CodecKind};
use crate::prbs::Prbs;
use dov_modem::{bits_to_symbols, symbol_bit_errors, Demodulator, MfskConfig, Modulator, Receiver};

/// FEC code rate (RS(64,40)); net throughput = raw × this.
const FEC_RATE: f64 = 40.0 / 64.0;
/// Raw BER at/below which the FEC layer drives the link error-free.
const SELECT_THRESHOLD: f64 = 1.2e-2;
/// Sounding length in symbols.
const SOUND_SYMBOLS: usize = 3000;

fn mk(tones: Vec<f64>, symbol_len: usize) -> MfskConfig {
    // Trim a fixed ~24-sample (3 ms) edge per side — the vocoder's transition
    // smear is roughly a fixed duration, not a fixed fraction — capped so very
    // short symbols keep a usable analysis window.
    let decision_guard = (24.0 / symbol_len as f64).min(0.2);
    MfskConfig { tones, symbol_len, amplitude: 8000.0, edge_ramp: 20, decision_guard }
}

/// Candidate profiles, fastest first.
fn profiles() -> Vec<(&'static str, MfskConfig)> {
    let f4: Vec<f64> = vec![700.0, 1100.0, 1500.0, 1900.0];
    let f8: Vec<f64> = (0..8).map(|i| 700.0 + 200.0 * i as f64).collect();
    let f16: Vec<f64> = (0..16).map(|i| 600.0 + 120.0 * i as f64).collect();
    vec![
        ("8-FSK / 5ms", mk(f8.clone(), 40)),    // 600 bps
        ("16-FSK / 10ms", mk(f16.clone(), 80)), // 400 bps
        ("8-FSK / 10ms", mk(f8.clone(), 80)),   // 300 bps
        ("16-FSK / 20ms", mk(f16.clone(), 160)),// 200 bps
        ("8-FSK / 20ms", mk(f8.clone(), 160)),  // 150 bps
        ("4-FSK / 20ms", mk(f4.clone(), 160)),  // 100 bps
    ]
}

/// Raw BER of a sounding through one codec (clean channel) with the real receiver.
fn sound(cfg: &MfskConfig, kind: CodecKind) -> f64 {
    let bps = cfg.bits_per_symbol();
    let m = cfg.symbol_len;
    let modu = Modulator::new(cfg.clone());
    let demod = Demodulator::new(cfg.clone());

    let preamble = ber::preamble(bps, ber::PREAMBLE_LEN);
    let mut prbs = Prbs::new(ber::SEED);
    let data = bits_to_symbols(&prbs.bits(SOUND_SYMBOLS * bps), bps);
    let tx_syms: Vec<u8> = preamble.iter().copied().chain(data.iter().copied()).collect();
    let tx_pcm = modu.modulate(&tx_syms);

    let mut codec = kind.make(false);
    let rx = codec.process(&tx_pcm);

    let receiver = Receiver::new(&demod);
    let Some(off) = receiver.acquire(&rx, &preamble, ber::MAX_DELAY) else {
        return 0.5;
    };
    let dec = receiver.demodulate_tracked(&rx, off + preamble.len() * m, data.len());
    let errs: usize = dec
        .iter()
        .zip(&data)
        .map(|(d, &w)| symbol_bit_errors(w, d.symbol) as usize)
        .sum();
    errs as f64 / (data.len() * bps) as f64
}

pub fn run() -> std::io::Result<()> {
    let profiles = profiles();
    let codecs = CodecKind::standard_set();

    // Sound every (codec, profile) with the real receiver.
    let matrix: Vec<Vec<f64>> = profiles
        .iter()
        .map(|(_, cfg)| codecs.iter().map(|&k| sound(cfg, k)).collect())
        .collect();

    println!("Link rate adaptation — raw BER of a sounding (real receiver) per profile/codec");
    println!("  selection: fastest profile with raw BER ≤ {SELECT_THRESHOLD:.0e}; net = raw × {FEC_RATE:.3}\n");

    print!("{:>16} | {:>7} |", "profile", "raw bps");
    for k in &codecs {
        print!(" {:>9}", k.label());
    }
    println!();
    print!("{:->17}+{:->9}+", "", "");
    for _ in &codecs {
        print!("{:->10}", "");
    }
    println!();
    for (pi, (label, cfg)) in profiles.iter().enumerate() {
        print!("{label:>16} | {:>7.0} |", cfg.raw_bitrate());
        for ci in 0..codecs.len() {
            let ber = matrix[pi][ci];
            let mark = if ber <= SELECT_THRESHOLD { ' ' } else { '*' };
            print!(" {:>8.1e}{mark}", ber);
        }
        println!();
    }

    // Per-codec selection: fastest (profiles are ordered fastest-first).
    println!("\n{:>10} | {:>15} | {:>8} | {:>8}", "codec", "selected", "raw bps", "net bps");
    println!("{:-<10}-+-{:-<15}-+-{:-<8}-+-{:-<8}", "", "", "", "");
    for (ci, kind) in codecs.iter().enumerate() {
        let pick = profiles
            .iter()
            .enumerate()
            .find(|(pi, _)| matrix[*pi][ci] <= SELECT_THRESHOLD);
        match pick {
            Some((_, (label, cfg))) => println!(
                "{:>10} | {:>15} | {:>8.0} | {:>8.0}",
                kind.label(), label, cfg.raw_bitrate(), cfg.raw_bitrate() * FEC_RATE
            ),
            None => println!("{:>10} | {:>15} | {:>8} | {:>8}", kind.label(), "(none)", "-", "-"),
        }
    }
    println!("\n(* = above threshold. EFR/AMR-12.2 carries multiples of what coarse codecs sustain.)");
    Ok(())
}
