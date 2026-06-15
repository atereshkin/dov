//! `coded` subcommand — M4. Same impairment sweep as `stress`, but now with the
//! full FEC chain (RS errors-and-erasures + interleaving, erasures flagged from
//! the demod confidence margin). Each cell shows pre-FEC → post-FEC bit error
//! rate, so the value FEC adds is visible at a glance.

use crate::ber;
use crate::bridge;
use crate::scenarios;
use dov_channel::Channel;
use dov_frame::FrameCodec;
use dov_modem::{symbol_bit_errors, Decision, Demodulator, MfskConfig, Modulator, Receiver};
use std::fmt::Write as _;
use std::path::Path;

/// RS(n, k): 64-byte codewords, 40 payload + 24 parity → corrects up to 24
/// erasures or 12 errors per codeword (rate 0.625).
const RS_N: usize = 64;
const RS_K: usize = 40;
/// Interleave depth: spreads a burst across this many codewords.
const DEPTH: usize = 8;
/// Number of interleave blocks to transmit.
const BLOCKS: usize = 6;
/// A symbol whose winning-tone margin is below this (dB) is flagged as erased.
const ERASURE_MARGIN_DB: f64 = 6.0;

pub fn run() -> std::io::Result<()> {
    let artifacts = Path::new("artifacts");
    std::fs::create_dir_all(artifacts)?;

    // Use the codec-agnostic higher-rate alphabet (200 bps raw) for the real link.
    let cfg = MfskConfig::fsk16();
    let bps = cfg.bits_per_symbol();
    let modulator = Modulator::new(cfg.clone());
    let demod = Demodulator::new(cfg.clone());
    let m = cfg.symbol_len;

    let fc = FrameCodec::new(RS_N, RS_K, DEPTH);

    // Payload → RS+interleave → coded bytes → FSK symbols (after a guard lead-in).
    let payload_len = fc.block_payload() * BLOCKS;
    let mut prbs = crate::prbs::Prbs::new(ber::SEED);
    let payload = prbs.bits(payload_len * 8);
    let payload: Vec<u8> = payload.chunks(8).map(|c| c.iter().fold(0u8, |v, &x| (v << 1) | x)).collect();
    let coded = fc.encode(&payload);
    let data_syms = bridge::coded_to_symbols(&coded, bps);
    // Real receiver: a known preamble for acquisition, then clock tracking.
    let preamble = ber::preamble(bps, ber::PREAMBLE_LEN);
    let tx_syms: Vec<u8> = preamble.iter().copied().chain(data_syms.iter().copied()).collect();
    let tx_pcm = modulator.modulate(&tx_syms);

    let goodput = fc.rate() * cfg.raw_bitrate();
    println!(
        "M4+M2 coded sweep — FEC + preamble acquisition/tracking — RS({RS_N},{RS_K}) depth {DEPTH}, rate {:.3}, erasure flag < {ERASURE_MARGIN_DB} dB",
        fc.rate()
    );
    println!(
        "  payload {} B over {} blocks; raw {:.0} bps → net {:.0} bps; cells = preFEC→postFEC BER\n",
        payload.len(),
        BLOCKS,
        cfg.raw_bitrate(),
        goodput
    );

    let codecs = ber::CodecKind::standard_set();
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

    let mut csv = String::from("scenario,codec,pre_fec_ber,post_fec_ber,codewords_failed,codewords_total\n");

    for sc in scenarios::all() {
        print!("{:>30} |", sc.name);
        for kind in &codecs {
            let codec = kind.make(sc.dtx);
            let mut channel = Channel::new(codec, (sc.build)(), ber::SEED);
            let rx = channel.run(&tx_pcm);

            let receiver = Receiver::new(&demod);
            let data = match receiver.acquire(&rx, &preamble, ber::MAX_DELAY) {
                Some(off) => receiver.demodulate_tracked(&rx, off + preamble.len() * m, data_syms.len()),
                // Lost the preamble: hand the FEC all-erased symbols (worst case).
                None => vec![Decision { symbol: 0, margin_db: 0.0 }; data_syms.len()],
            };

            // pre-FEC bit error rate on the coded symbol stream
            let pre_bit_err: u32 = data
                .iter()
                .zip(&data_syms)
                .map(|(d, &want)| symbol_bit_errors(d.symbol, want))
                .sum();
            let pre_ber = pre_bit_err as f64 / (data_syms.len() * bps) as f64;

            // post-FEC payload bit error rate
            let (rx_bytes, erased) = bridge::decisions_to_coded(&data, bps, coded.len(), ERASURE_MARGIN_DB);
            let (rx_payload, stats) = fc.decode(&rx_bytes, &erased);
            let post_bit_err: u32 = payload
                .iter()
                .zip(&rx_payload)
                .map(|(a, b)| (a ^ b).count_ones())
                .sum();
            let post_ber = post_bit_err as f64 / (payload.len() * 8) as f64;

            print!(" {:>7.1e}→{:<7.1e}", pre_ber, post_ber);
            let _ = writeln!(
                csv,
                "{},{},{:.6e},{:.6e},{},{}",
                sc.name,
                kind.label(),
                pre_ber,
                post_ber,
                stats.codewords_failed,
                stats.codewords_total
            );
        }
        println!();
    }

    std::fs::write(artifacts.join("m4_coded.csv"), csv)?;
    println!("\nCSV: {}/m4_coded.csv", artifacts.display());
    println!("(post-FEC BER of 0 = the link is error-free after correction)");
    Ok(())
}
