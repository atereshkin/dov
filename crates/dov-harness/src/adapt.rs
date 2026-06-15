//! `adapt` subcommand — link rate adaptation, selected on the *real* outcome.
//!
//! The `rate` sweep showed the best modem configuration depends on which codec
//! the call rides. A real link sounds the channel at setup: run each candidate
//! profile through the actual codec *and the full FEC chain*, and commit to the
//! fastest one that comes out error-free. Selecting on raw BER is not enough —
//! errors can arrive in bursts that pass an averaged-BER threshold yet overrun
//! the code — so we select on the post-FEC result itself.

use crate::ber::CodecKind;
use crate::coded;
use dov_frame::FrameCodec;
use dov_modem::MfskConfig;

/// FEC code rate (RS(64,40)); net throughput = raw × this when error-free.
const FEC_RATE: f64 = coded::RS_K as f64 / coded::RS_N as f64;

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
        ("8-FSK / 5ms", mk(f8.clone(), 40)),     // 600 bps
        ("16-FSK / 10ms", mk(f16.clone(), 80)),  // 400 bps
        ("8-FSK / 10ms", mk(f8.clone(), 80)),    // 300 bps
        ("16-FSK / 20ms", mk(f16.clone(), 160)), // 200 bps
        ("8-FSK / 20ms", mk(f8.clone(), 160)),   // 150 bps
        ("4-FSK / 20ms", mk(f4.clone(), 160)),   // 100 bps
    ]
}

pub fn run() -> std::io::Result<()> {
    let profiles = profiles();
    let codecs = CodecKind::standard_set();
    let fc = FrameCodec::new(coded::RS_N, coded::RS_K, coded::DEPTH);
    let payload = coded::payload_bytes(fc.block_payload() * coded::BLOCKS);

    // Post-FEC BER for every (profile, codec) on the clean channel.
    let matrix: Vec<Vec<f64>> = profiles
        .iter()
        .map(|(_, cfg)| {
            codecs
                .iter()
                .map(|&k| coded::measure_coded(cfg, &fc, &payload, k, false, None).1)
                .collect()
        })
        .collect();

    println!("Link rate adaptation — post-FEC BER per profile/codec (clean channel)");
    println!("  commit to the fastest profile that decodes error-free (post-FEC BER = 0)\n");

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
            let post = matrix[pi][ci];
            if post == 0.0 {
                print!(" {:>9}", "0");
            } else {
                print!(" {:>9.1e}", post);
            }
        }
        println!();
    }

    println!("\n{:>10} | {:>15} | {:>8} | {:>8}", "codec", "selected", "raw bps", "net bps");
    println!("{:-<10}-+-{:-<15}-+-{:-<8}-+-{:-<8}", "", "", "", "");
    for (ci, kind) in codecs.iter().enumerate() {
        let pick = profiles
            .iter()
            .enumerate()
            .find(|(pi, _)| matrix[*pi][ci] == 0.0);
        match pick {
            Some((_, (label, cfg))) => println!(
                "{:>10} | {:>15} | {:>8.0} | {:>8.0}",
                kind.label(), label, cfg.raw_bitrate(), cfg.raw_bitrate() * FEC_RATE
            ),
            None => println!("{:>10} | {:>15} | {:>8} | {:>8}", kind.label(), "(none)", "-", "-"),
        }
    }
    println!("\n(every selection is end-to-end error-free; EFR carries multiples of the coarse codecs.)");
    Ok(())
}
