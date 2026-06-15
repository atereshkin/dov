//! `sync` subcommand — M2. Demonstrate real timing recovery: a receiver that
//! acquires on a short known preamble and then *tracks* the symbol clock,
//! versus a fixed stride that cannot follow drift.
//!
//! Each cell is fixed-stride BER → tracked BER. The drift rows are the point:
//! tracking should collapse them to clean-channel levels, removing the sync
//! avalanche that FEC alone could not repair in M4.

use crate::ber::{self, CodecKind};
use crate::scenarios;
use dov_channel::Channel;
use dov_modem::{symbol_bit_errors, Demodulator, MfskConfig, Modulator, Receiver};

const PREAMBLE_LEN: usize = ber::PREAMBLE_LEN;
const DATA_SYMBOLS: usize = 8000;

pub fn run() -> std::io::Result<()> {
    let cfg = MfskConfig::fsk8();
    let bps = cfg.bits_per_symbol();
    let m = cfg.symbol_len;
    let modulator = Modulator::new(cfg.clone());
    let demod = Demodulator::new(cfg.clone());

    let preamble = ber::preamble(bps, PREAMBLE_LEN);
    let mut prbs = crate::prbs::Prbs::new(ber::SEED);
    let data = dov_modem::bits_to_symbols(&prbs.bits(DATA_SYMBOLS * bps), bps);
    let tx_syms: Vec<u8> = preamble.iter().copied().chain(data.iter().copied()).collect();
    let tx_pcm = modulator.modulate(&tx_syms);

    println!(
        "M2 timing recovery — {PREAMBLE_LEN}-symbol preamble acquisition + early-late tracking"
    );
    println!(
        "  {DATA_SYMBOLS} data symbols ({:.0} s); cells = fixed-stride BER → tracked BER\n",
        DATA_SYMBOLS as f64 * m as f64 / 8000.0
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

    for sc in scenarios::all() {
        print!("{:>30} |", sc.name);
        for kind in &codecs {
            let codec = kind.make(sc.dtx);
            let mut channel = Channel::new(codec, (sc.build)(), ber::SEED);
            let rx = channel.run(&tx_pcm);

            let receiver = Receiver::new(&demod);
            let Some(off) = receiver.acquire(&rx, &preamble, ber::MAX_DELAY) else {
                print!(" {:>15}", "no-sync");
                continue;
            };
            let data_start = off + preamble.len() * m;

            // fixed stride from the acquired offset (no tracking)
            let fixed_err: usize = data
                .iter()
                .enumerate()
                .map(|(k, &want)| {
                    let s = data_start + k * m;
                    if s + m <= rx.len() {
                        symbol_bit_errors(want, demod.decide(&rx[s..s + m]).symbol) as usize
                    } else {
                        bps
                    }
                })
                .sum();
            let fixed_ber = fixed_err as f64 / (data.len() * bps) as f64;

            // tracked
            let tracked = receiver.demodulate_tracked(&rx, data_start, data.len());
            let tracked_err: usize = tracked
                .iter()
                .zip(&data)
                .map(|(d, &want)| symbol_bit_errors(want, d.symbol) as usize)
                .sum();
            let tracked_ber = tracked_err as f64 / (data.len() * bps) as f64;

            print!(" {:>6.1e}→{:<6.1e}", fixed_ber, tracked_ber);
        }
        println!();
    }
    println!("\n(tracked BER ≈ clean level on the drift rows = timing recovery working)");
    Ok(())
}
