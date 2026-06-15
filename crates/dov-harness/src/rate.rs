//! `rate` subcommand — the throughput-vs-survival frontier.
//!
//! 150 bps comes from one symbol per 20 ms vocoder frame. To go faster we can
//! shorten symbols (sub-frame) or widen the alphabet (more tones) — but both
//! fight the codec: sub-frame symbols put two spectra in one frame the CELP/
//! RPE-LTP model can only represent as one, and shorter analysis windows blur
//! the Goertzel bins. This sweep measures raw BER per configuration through each
//! real codec (clean channel) so we can pick the next operating point: the
//! highest raw rate whose BER stays in FEC-correctable territory (≲1e-2).

use crate::ber::{self, CodecKind};
use dov_modem::{Demodulator, MfskConfig, Modulator};

fn mk(tones: Vec<f64>, symbol_len: usize) -> MfskConfig {
    // Trim a fixed ~24-sample edge per side (the vocoder's transition smear is a
    // roughly fixed duration), capped so short symbols keep a usable window.
    let decision_guard = (24.0 / symbol_len as f64).min(0.2);
    MfskConfig {
        tones,
        symbol_len,
        amplitude: 8000.0,
        edge_ramp: 20,
        decision_guard,
    }
}

pub fn run() -> std::io::Result<()> {
    let f4: Vec<f64> = vec![700.0, 1100.0, 1500.0, 1900.0];
    let f8: Vec<f64> = (0..8).map(|i| 700.0 + 200.0 * i as f64).collect();
    let f16: Vec<f64> = (0..16).map(|i| 600.0 + 120.0 * i as f64).collect();

    // (label, config). symbol_len in samples @ 8 kHz: 160=20ms, 80=10ms, 40=5ms.
    let configs: Vec<(&str, MfskConfig)> = vec![
        ("4-FSK / 20ms", mk(f4.clone(), 160)),
        ("8-FSK / 20ms", mk(f8.clone(), 160)),
        ("16-FSK / 20ms", mk(f16.clone(), 160)),
        ("4-FSK / 10ms", mk(f4.clone(), 80)),
        ("8-FSK / 10ms", mk(f8.clone(), 80)),
        ("16-FSK / 10ms", mk(f16.clone(), 80)),
        ("8-FSK / 6.7ms", mk(f8.clone(), 53)),
        ("8-FSK / 5ms", mk(f8.clone(), 40)),
    ];

    let codecs = CodecKind::standard_set();

    println!("M2b rate frontier — raw BER per config through each codec (clean channel)");
    println!("  pick the fastest config whose BER stays FEC-correctable (~1e-2)\n");

    print!("{:>16} | {:>7} |", "config", "raw bps");
    for k in &codecs {
        print!(" {:>9}", k.label());
    }
    println!();
    print!("{:->17}+{:->9}+", "", "");
    for _ in &codecs {
        print!("{:->10}", "");
    }
    println!();

    for (label, cfg) in &configs {
        let bps = cfg.bits_per_symbol();
        let modu = Modulator::new(cfg.clone());
        let demod = Demodulator::new(cfg.clone());
        let (tx_syms, tx_pcm) = ber::build_tx(&modu);

        print!("{label:>16} | {:>7.0} |", cfg.raw_bitrate());
        for kind in &codecs {
            let mut codec = kind.make(false);
            let rx = codec.process(&tx_pcm);
            let o = ber::score(kind.label(), &rx, &demod, &tx_syms, cfg.tones.len());
            print!(" {:>9.1e}", o.ber(bps));
        }
        println!();
    }
    println!("\n(20ms = frame-aligned baseline; below it, sub-frame symbols and 16 tones trade survival for speed)");
    Ok(())
}
