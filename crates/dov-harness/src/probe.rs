//! `probe` subcommand — characterise what a vocoder does to simple signals.
//!
//! Pushes pure tones and a frequency sweep through each real codec and reports
//! per-frequency energy survival, plus dumps input/output WAVs for spectrograms.
//!
//! NOTE: steady-tone survival is a deliberately *misleading* metric (a single
//! sinusoid sails through the VAD tone-detector nearly intact). It maps the
//! channel's passband but says nothing about a modulated stream — that is the
//! `run` subcommand's job. Keep this around as a channel-visibility instrument.

use crate::wav;
use dov_codec::{AmrMode, AmrNb, Codec, GsmFr, SAMPLE_RATE};
use std::f64::consts::PI;
use std::path::Path;

const SR: f64 = SAMPLE_RATE as f64;

pub fn run() -> std::io::Result<()> {
    let artifacts = Path::new("artifacts");
    std::fs::create_dir_all(artifacts)?;

    let mut codecs: Vec<Box<dyn Codec>> = vec![
        Box::new(GsmFr::new()),
        Box::new(AmrNb::new(AmrMode::Mr122)), // == GSM-EFR speech core
        Box::new(AmrNb::new(AmrMode::Mr795)),
        Box::new(AmrNb::new(AmrMode::Mr475)), // harshest
    ];

    tone_survival_sweep(&mut codecs);
    chirp_artifacts(&mut codecs, artifacts)?;

    println!("\nWrote input/output WAVs to {}/", artifacts.display());
    println!("Render spectrograms with e.g.:");
    println!("  ffmpeg -y -i artifacts/chirp_in.wav -lavfi showspectrumpic=s=1024x512 artifacts/chirp_in.png");
    Ok(())
}

fn tone_survival_sweep(codecs: &mut [Box<dyn Codec>]) {
    let freqs = [
        200.0, 300.0, 400.0, 500.0, 700.0, 900.0, 1100.0, 1300.0, 1500.0, 1800.0, 2100.0, 2400.0,
        2700.0, 3000.0, 3300.0, 3600.0,
    ];
    let dur_s = 0.5;
    let amp = 8000.0;

    println!("Tone survival — output/input energy at the tone frequency, in dB");
    println!("(0 dB = energy fully preserved; very negative = tone destroyed)\n");

    print!("{:>8} |", "freq Hz");
    for c in codecs.iter() {
        print!(" {:>13}", c.name());
    }
    println!();
    print!("{:->9}|", "");
    for _ in codecs.iter() {
        print!("{:->14}", "");
    }
    println!();

    for &f in &freqs {
        let input = gen_sine(f, dur_s, amp);
        let in_pow = goertzel_power(&input, f);
        print!("{f:>8.0} |");
        for c in codecs.iter_mut() {
            let out = c.process(&input);
            let out_pow = goertzel_power(&out, f);
            let db = 10.0 * (out_pow / in_pow.max(1e-9)).max(1e-12).log10();
            print!(" {db:>13.1}");
        }
        println!();
    }
}

fn chirp_artifacts(codecs: &mut [Box<dyn Codec>], dir: &Path) -> std::io::Result<()> {
    let chirp = gen_chirp(300.0, 3400.0, 3.0, 8000.0);
    wav::write_mono_i16(dir.join("chirp_in.wav"), &chirp, SAMPLE_RATE)?;
    for c in codecs.iter_mut() {
        let out = c.process(&chirp);
        let fname = format!("chirp_{}.wav", c.name().replace('.', "_"));
        wav::write_mono_i16(dir.join(fname), &out, SAMPLE_RATE)?;
    }
    Ok(())
}

fn gen_sine(freq: f64, dur_s: f64, amp: f64) -> Vec<i16> {
    let n = (dur_s * SR) as usize;
    (0..n)
        .map(|i| (amp * (2.0 * PI * freq * i as f64 / SR).sin()).round() as i16)
        .collect()
}

fn gen_chirp(f0: f64, f1: f64, dur_s: f64, amp: f64) -> Vec<i16> {
    let n = (dur_s * SR) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / SR;
            let phase = 2.0 * PI * (f0 * t + 0.5 * (f1 - f0) / dur_s * t * t);
            (amp * phase.sin()).round() as i16
        })
        .collect()
}

fn goertzel_power(samples: &[i16], freq: f64) -> f64 {
    let w = 2.0 * PI * freq / SR;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}
