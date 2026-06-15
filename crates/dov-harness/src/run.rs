//! `run` subcommand — the M1 end-to-end BER skeleton on the *clean* channel.
//!
//! random bits → 8-FSK modulate → real vocoder → demodulate → compare.
//! Produces the first true per-codec BER plus an 8×8 tone confusion matrix.

use crate::ber::{self, CodecKind, Outcome};
use crate::wav;
use dov_codec::SAMPLE_RATE;
use dov_modem::{Demodulator, MfskConfig, Modulator};
use std::path::Path;

pub fn run() -> std::io::Result<()> {
    let artifacts = Path::new("artifacts");
    std::fs::create_dir_all(artifacts)?;

    let cfg = MfskConfig::fsk8();
    let alphabet = cfg.tones.len();
    let bps = cfg.bits_per_symbol();
    let modulator = Modulator::new(cfg.clone());
    let demod = Demodulator::new(cfg.clone());

    let (tx_syms, tx_pcm) = ber::build_tx(&modulator);
    wav::write_mono_i16(artifacts.join("m1_tx.wav"), &tx_pcm, SAMPLE_RATE)?;

    println!(
        "M1 8-FSK BER (clean channel): tones {:?} Hz, {bps} bits/symbol, {:.0} bps raw, {} payload symbols ({:.1} s)\n",
        cfg.tones.iter().map(|f| *f as u32).collect::<Vec<_>>(),
        cfg.raw_bitrate(),
        ber::PAYLOAD_SYMBOLS,
        ber::PAYLOAD_SYMBOLS as f64 * cfg.symbol_len as f64 / SAMPLE_RATE as f64,
    );

    let mut outcomes = Vec::new();
    for kind in CodecKind::standard_set() {
        let mut codec = kind.make(false);
        let rx_pcm = codec.process(&tx_pcm);
        let fname = format!("m1_rx_{}.wav", kind.label().replace('.', "_"));
        wav::write_mono_i16(artifacts.join(fname), &rx_pcm, SAMPLE_RATE)?;
        outcomes.push(ber::score(kind.label(), &rx_pcm, &demod, &tx_syms, alphabet));
    }

    print_summary(&outcomes, bps);
    print_confusion(&outcomes, &cfg.tones);

    println!("\nArtifacts in {}/: m1_tx.wav, m1_rx_*.wav", artifacts.display());
    Ok(())
}

fn print_summary(outcomes: &[Outcome], bps: usize) {
    println!("Per-codec error rates (8-FSK, no FEC):\n");
    println!("{:>10} | {:>7} | {:>10} | {:>10}", "codec", "delay", "SER", "BER");
    println!("{:-<10}-+-{:-<7}-+-{:-<10}-+-{:-<10}", "", "", "", "");
    for o in outcomes {
        println!(
            "{:>10} | {:>3} smp | {:>10.2e} | {:>10.2e}",
            o.name,
            o.delay,
            o.ser(),
            o.ber(bps)
        );
    }
}

fn print_confusion(outcomes: &[Outcome], tones: &[f64]) {
    for o in outcomes {
        println!("\nConfusion % for {} (row=TX tone Hz, col=RX tone Hz; diagonal=correct):", o.name);
        print!("{:>7}", "");
        for &f in tones {
            print!(" {:>5.0}", f);
        }
        println!();
        for (i, &f) in tones.iter().enumerate() {
            let row_total: u64 = o.confusion[i].iter().sum();
            print!("{f:>7.0}");
            for j in 0..tones.len() {
                let pct = if row_total > 0 {
                    100.0 * o.confusion[i][j] as f64 / row_total as f64
                } else {
                    0.0
                };
                if pct >= 0.05 {
                    print!(" {pct:>5.1}");
                } else {
                    print!("     ·");
                }
            }
            println!();
        }
    }
}
