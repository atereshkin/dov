//! `stress` subcommand — M3-lite. Push the M1 8-FSK stream through each codec
//! under each realistic impairment and tabulate raw BER, to see what breaks
//! first once the clean-channel assumptions are removed.

use crate::ber::{self, CodecKind};
use crate::scenarios;
use dov_channel::Channel;
use dov_modem::{Demodulator, MfskConfig, Modulator};
use std::fmt::Write as _;
use std::path::Path;

pub fn run() -> std::io::Result<()> {
    let artifacts = Path::new("artifacts");
    std::fs::create_dir_all(artifacts)?;

    let cfg = MfskConfig::fsk8();
    let bps = cfg.bits_per_symbol();
    let alphabet = cfg.tones.len();
    let modulator = Modulator::new(cfg.clone());
    let demod = Demodulator::new(cfg.clone());
    let (tx_syms, tx_pcm) = ber::build_tx(&modulator);
    let frames = tx_pcm.len() / cfg.symbol_len;

    let codecs = CodecKind::standard_set();

    println!(
        "M3-lite impairment sweep — raw 8-FSK BER, no FEC ({} symbols/codec)\n",
        ber::PAYLOAD_SYMBOLS
    );
    print!("{:>30} |", "scenario");
    for k in &codecs {
        print!(" {:>9}", k.label());
    }
    println!(" |  loss%");
    print!("{:->31}+", "");
    for _ in &codecs {
        print!("{:->10}", "");
    }
    println!("-+-------");

    let mut csv = String::from("scenario,codec,ber,ser,realized_loss_pct\n");

    for sc in scenarios::all() {
        print!("{:>30} |", sc.name);
        let mut realized_loss = 0.0;
        for (i, kind) in codecs.iter().enumerate() {
            let codec = kind.make(sc.dtx);
            let channel_cfg = (sc.build)();
            let mut channel = Channel::new(codec, channel_cfg, ber::SEED);
            let rx = channel.run(&tx_pcm);
            let o = ber::score(kind.label(), &rx, &demod, &tx_syms, alphabet);
            print!(" {:>9.1e}", o.ber(bps));
            if i == 0 {
                realized_loss = 100.0 * channel.last_erasures() as f64 / frames as f64;
            }
            let _ = writeln!(
                csv,
                "{},{},{:.6e},{:.6e},{:.2}",
                sc.name,
                kind.label(),
                o.ber(bps),
                o.ser(),
                realized_loss
            );
        }
        println!(" | {realized_loss:>5.1}");
    }

    std::fs::write(artifacts.join("m3_stress.csv"), csv)?;
    println!("\nCSV: {}/m3_stress.csv", artifacts.display());
    println!("(loss% = realized frame-erasure rate, codec-independent for a given scenario)");
    Ok(())
}
