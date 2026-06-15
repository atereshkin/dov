//! `coded` subcommand — M4+M2. The full realistic chain: FEC (RS errors-and-
//! erasures + interleaving) over the modem, with preamble acquisition + clock
//! tracking. Each cell is pre-FEC → post-FEC BER, per impairment.

use crate::ber::{self, CodecKind};
use crate::bridge;
use crate::scenarios;
use dov_channel::Channel;
use dov_frame::FrameCodec;
use dov_modem::{symbol_bit_errors, Decision, Demodulator, MfskConfig, Modulator, Receiver};
use std::fmt::Write as _;
use std::path::Path;

/// RS(n, k): 64-byte codewords, 40 payload + 24 parity (rate 0.625).
pub const RS_N: usize = 64;
pub const RS_K: usize = 40;
/// Interleave depth.
pub const DEPTH: usize = 8;
/// Interleave blocks per measurement.
pub const BLOCKS: usize = 6;
/// A symbol whose winning-tone margin is below this (dB) is flagged erased.
pub const ERASURE_MARGIN_DB: f64 = 6.0;

/// Run one payload through FEC + modem + receiver, where `make_rx` defines the
/// channel (clean codec, impaired channel, or a tandem). Returns (pre-FEC BER on
/// the coded symbols, post-FEC payload BER). `payload` must be block-aligned.
pub fn measure_coded<F: Fn(&[i16]) -> Vec<i16>>(
    cfg: &MfskConfig,
    fc: &FrameCodec,
    payload: &[u8],
    make_rx: F,
) -> (f64, f64) {
    let bps = cfg.bits_per_symbol();
    let m = cfg.symbol_len;
    let modu = Modulator::new(cfg.clone());
    let demod = Demodulator::new(cfg.clone());

    let coded = fc.encode(payload);
    let data_syms = bridge::coded_to_symbols(&coded, bps);
    let preamble = ber::preamble(bps, ber::PREAMBLE_LEN);
    let tx_syms: Vec<u8> = preamble.iter().copied().chain(data_syms.iter().copied()).collect();
    let tx_pcm = modu.modulate(&tx_syms);

    let rx = make_rx(&tx_pcm);

    let receiver = Receiver::new(&demod);
    let data = match receiver.acquire(&rx, &preamble, ber::MAX_DELAY) {
        Some(off) => receiver.demodulate_tracked(&rx, off + preamble.len() * m, data_syms.len()),
        None => vec![Decision { symbol: 0, margin_db: 0.0 }; data_syms.len()],
    };

    let pre_bit_err: u32 = data
        .iter()
        .zip(&data_syms)
        .map(|(d, &w)| symbol_bit_errors(w, d.symbol))
        .sum();
    let pre_ber = pre_bit_err as f64 / (data_syms.len() * bps) as f64;

    let (rx_bytes, erased) = bridge::decisions_to_coded(&data, bps, coded.len(), ERASURE_MARGIN_DB);
    let (rx_payload, _stats) = fc.decode(&rx_bytes, &erased);
    let post_bit_err: u32 = payload
        .iter()
        .zip(&rx_payload)
        .map(|(a, b)| (a ^ b).count_ones())
        .sum();
    let post_ber = post_bit_err as f64 / (payload.len() * 8) as f64;
    (pre_ber, post_ber)
}

/// PRBS payload of `n` whole bytes.
pub fn payload_bytes(n: usize) -> Vec<u8> {
    let mut prbs = crate::prbs::Prbs::new(ber::SEED);
    prbs.bits(n * 8)
        .chunks(8)
        .map(|c| c.iter().fold(0u8, |v, &x| (v << 1) | x))
        .collect()
}

pub fn run() -> std::io::Result<()> {
    let artifacts = Path::new("artifacts");
    std::fs::create_dir_all(artifacts)?;

    // Codec-agnostic higher-rate alphabet (200 bps raw) for the real link.
    let cfg = MfskConfig::fsk16();
    let fc = FrameCodec::new(RS_N, RS_K, DEPTH);
    let payload = payload_bytes(fc.block_payload() * BLOCKS);

    println!(
        "M4+M2 coded sweep — FEC + preamble/tracking — RS({RS_N},{RS_K}) depth {DEPTH}, rate {:.3}",
        fc.rate()
    );
    println!(
        "  16-FSK, raw {:.0} bps → net {:.0} bps; cells = preFEC→postFEC BER\n",
        cfg.raw_bitrate(),
        fc.rate() * cfg.raw_bitrate()
    );

    let codecs = CodecKind::standard_set();
    print!("{:>30} |", "scenario");
    for k in &codecs {
        print!(" {:>15}", k.label());
    }
    println!();
    print!("{:->31}+", "");
    for _ in &codecs {
        print!("{:->16}", "");
    }
    println!();

    let mut csv = String::from("scenario,codec,pre_fec_ber,post_fec_ber\n");
    for sc in scenarios::all() {
        print!("{:>30} |", sc.name);
        for kind in &codecs {
            let (pre, post) = measure_coded(&cfg, &fc, &payload, |tx| {
                Channel::new(kind.make(sc.dtx), (sc.build)(), ber::SEED).run(tx)
            });
            print!(" {:>7.1e}→{:<7.1e}", pre, post);
            let _ = writeln!(csv, "{},{},{:.6e},{:.6e}", sc.name, kind.label(), pre, post);
        }
        println!();
    }

    std::fs::write(artifacts.join("m4_coded.csv"), csv)?;
    println!("\nCSV: {}/m4_coded.csv", artifacts.display());
    println!("(post-FEC BER of 0 = the link is error-free after correction)");
    Ok(())
}
