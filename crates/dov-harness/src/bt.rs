//! `bt` subcommand — emulate the Bluetooth HFP tandem (CVSD around GSM) and
//! measure what bridging the call over a BT-SCO link costs in throughput.
//!
//! A BT-bridged call passes the signal through an extra CVSD codec at each end
//! that uses Bluetooth audio: `PC → BT(CVSD) → phone → GSM → phone → BT(CVSD) →
//! PC`. We model that as `Chain([Cvsd, base, Cvsd])` and re-run the adaptive
//! error-free rate selection, comparing against the phone-only path.

use crate::adapt::{self, FEC_RATE};
use crate::ber::CodecKind;
use crate::coded;
use crate::wav;
use dov_codec::{Chain, Codec, Cvsd, GsmFr, SAMPLE_RATE};
use dov_frame::FrameCodec;
use dov_modem::MfskConfig;
use std::path::Path;

pub fn run() -> std::io::Result<()> {
    let artifacts = Path::new("artifacts");
    std::fs::create_dir_all(artifacts)?;

    let fc = FrameCodec::new(coded::RS_N, coded::RS_K, coded::DEPTH);
    // Fewer blocks than `coded`: CVSD's 8× oversampling makes the sweep heavy.
    let payload = coded::payload_bytes(fc.block_payload() * 3);

    println!("Bluetooth HFP tandem — error-free net throughput with CVSD bridging");
    println!("  plain = phone only; 1× BT = your end on a BT headset; 2× BT = both ends\n");
    println!(
        "{:>10} | {:>18} | {:>18} | {:>18}",
        "base codec", "plain", "1× BT (CVSD)", "2× BT (CVSD)"
    );
    println!("{:-<10}-+-{:-<18}-+-{:-<18}-+-{:-<18}", "", "", "", "");

    let fmt = |sel: Option<(&str, f64)>| match sel {
        Some((label, raw)) => format!("{:>3.0} bps  {label}", raw * FEC_RATE),
        None => "(none)".to_string(),
    };

    for kind in CodecKind::standard_set() {
        let plain = adapt::fastest_error_free(&fc, &payload, |tx| kind.make(false).process(tx));
        let bt1 = adapt::fastest_error_free(&fc, &payload, |tx| {
            Chain::new(vec![Box::new(Cvsd::new()), kind.make(false)]).process(tx)
        });
        let bt2 = adapt::fastest_error_free(&fc, &payload, |tx| {
            Chain::new(vec![Box::new(Cvsd::new()), kind.make(false), Box::new(Cvsd::new())])
                .process(tx)
        });
        println!(
            "{:>10} | {:>18} | {:>18} | {:>18}",
            kind.label(),
            fmt(plain),
            fmt(bt1),
            fmt(bt2)
        );
    }

    // Raw (pre-FEC) BER at a fixed robust profile — shows the noise CVSD adds,
    // which the throughput table hides because FEC absorbs it.
    let cfg = MfskConfig::fsk16();
    println!("\nRaw pre-FEC BER at 16-FSK/20ms (the degradation FEC absorbs):");
    println!("{:>10} | {:>9} | {:>9} | {:>9}", "base codec", "plain", "1× BT", "2× BT");
    println!("{:-<10}-+-{:-<9}-+-{:-<9}-+-{:-<9}", "", "", "", "");
    for kind in CodecKind::standard_set() {
        let plain = coded::measure_coded(&cfg, &fc, &payload, |tx| kind.make(false).process(tx)).0;
        let bt1 = coded::measure_coded(&cfg, &fc, &payload, |tx| {
            Chain::new(vec![Box::new(Cvsd::new()), kind.make(false)]).process(tx)
        })
        .0;
        let bt2 = coded::measure_coded(&cfg, &fc, &payload, |tx| {
            Chain::new(vec![Box::new(Cvsd::new()), kind.make(false), Box::new(Cvsd::new())])
                .process(tx)
        })
        .0;
        println!(
            "{:>10} | {:>9.1e} | {:>9.1e} | {:>9.1e}",
            kind.label(),
            plain,
            bt1,
            bt2
        );
    }

    // Spectrograms: a chirp through CVSD alone and through the full 2× BT tandem.
    let chirp = gen_chirp(300.0, 3400.0, 2.0);
    wav::write_mono_i16(artifacts.join("bt_cvsd.wav"), &Cvsd::new().process(&chirp), SAMPLE_RATE)?;
    let tandem = Chain::new(vec![
        Box::new(Cvsd::new()),
        Box::new(GsmFr::new()),
        Box::new(Cvsd::new()),
    ])
    .process(&chirp);
    wav::write_mono_i16(artifacts.join("bt_tandem.wav"), &tandem, SAMPLE_RATE)?;

    println!("\nArtifacts: bt_cvsd.wav (chirp→CVSD), bt_tandem.wav (chirp→CVSD+GSM+CVSD)");
    Ok(())
}

fn gen_chirp(f0: f64, f1: f64, dur_s: f64) -> Vec<i16> {
    let sr = SAMPLE_RATE as f64;
    let n = (dur_s * sr) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let ph = 2.0 * std::f64::consts::PI * (f0 * t + 0.5 * (f1 - f0) / dur_s * t * t);
            (8000.0 * ph.sin()).round() as i16
        })
        .collect()
}
