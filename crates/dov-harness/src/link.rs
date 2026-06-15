//! `link` — message ↔ waveform, and the live-audio `send`/`recv` commands.
//!
//! Turns a text message into a transmittable waveform (lead-in tone for AGC/
//! onset + preamble + FEC-coded, length-prefixed payload) and recovers it from a
//! captured buffer (energy onset → preamble acquisition → tracked demod → FEC
//! decode). This is the bridge between the batch modem we built and a live call:
//! the same modem/FEC/timing-recovery runs on a buffer that now comes from a
//! real device instead of a WAV file.
//!
//! `selftest` validates the whole message stack through the emulated codecs (no
//! hardware); `send`/`recv` use `dov-io` to talk to a real audio device — which,
//! for the real link, is a Bluetooth-SCO device or the USB dongle.

use crate::{ber, bridge, coded};
use dov_codec::{AmrMode, AmrNb, Chain, Codec, Cvsd, GsmFr};
use dov_frame::{DecodeStats, FrameCodec};
use dov_io::{AlsaTool, AudioIn, AudioOut};
use dov_modem::{Demodulator, MfskConfig, Modulator, Receiver};
use std::io;

/// Lead-in symbols (a steady tone) before the preamble, for AGC settling and
/// energy-based onset detection on a live capture.
const LEADIN: usize = 8;

/// The modem profile for messages: the most robust (4-FSK), since a first link
/// brings up an unknown codec and short control messages favour reliability over
/// speed. (16-FSK would be ~2× faster on a known-good codec.)
fn modem() -> MfskConfig {
    MfskConfig::fsk4()
}
/// RS(48,24) depth-4 — rate 1/2, a 96-byte payload block. Heavy FEC so a short
/// message survives even the harshest codec; on-air time is set by the coded
/// byte count (n), so the extra parity is effectively free here.
fn fec() -> FrameCodec {
    FrameCodec::new(48, 24, 4)
}

/// Build the transmit waveform for `text`.
pub fn encode_message(text: &str) -> Result<Vec<i16>, String> {
    let cfg = modem();
    let bps = cfg.bits_per_symbol();
    let fc = fec();
    let cap = fc.block_payload();
    let bytes = text.as_bytes();
    if bytes.len() > cap - 2 {
        return Err(format!("message too long ({} bytes, max {})", bytes.len(), cap - 2));
    }

    // [u16 length][text][zero pad] → one FEC block.
    let mut payload = vec![0u8; cap];
    payload[0..2].copy_from_slice(&(bytes.len() as u16).to_le_bytes());
    payload[2..2 + bytes.len()].copy_from_slice(bytes);

    let mut coded = fc.encode(&payload);
    scramble(&mut coded); // dense transitions so the timing loop never coasts
    let data_syms = bridge::coded_to_symbols(&coded, bps);
    let preamble = ber::preamble(bps, ber::PREAMBLE_LEN);
    let leadin = vec![0u8; LEADIN]; // steady tone (symbol 0)

    let tx_syms: Vec<u8> = leadin
        .into_iter()
        .chain(preamble)
        .chain(data_syms)
        .collect();
    Ok(Modulator::new(cfg).modulate(&tx_syms))
}

/// Recover a message from a captured buffer. Returns the text and FEC stats.
pub fn decode_message(rx: &[i16]) -> Option<(String, DecodeStats)> {
    let cfg = modem();
    let bps = cfg.bits_per_symbol();
    let m = cfg.symbol_len;
    let fc = fec();
    let demod = Demodulator::new(cfg.clone());
    let preamble = ber::preamble(bps, ber::PREAMBLE_LEN);
    let coded_len = fc.block_coded();
    let data_count = coded_len * 8 / bps;

    let receiver = Receiver::new(&demod);
    // Scan the whole capture for the preamble (signal may start anywhere).
    let off = receiver.acquire_scan(rx, &preamble, m / 4)?;
    let data_start = off + preamble.len() * m;
    let decisions = receiver.demodulate_tracked(rx, data_start, data_count);

    let (mut bytes, erased) =
        bridge::decisions_to_coded(&decisions, bps, coded_len, coded::ERASURE_MARGIN_DB);
    scramble(&mut bytes); // de-scramble (XOR is its own inverse)
    let (payload, stats) = fc.decode(&bytes, &erased);
    if payload.len() < 2 {
        return None;
    }
    let len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    if len > payload.len() - 2 {
        return None;
    }
    let text = String::from_utf8_lossy(&payload[2..2 + len]).into_owned();
    Some((text, stats))
}

/// XOR a byte buffer with a fixed PRBS keystream (its own inverse). Scrambling
/// the coded bytes keeps the modulated symbol stream busy even where the payload
/// is zero-padded, so the early-late timing loop always has transitions to lock.
fn scramble(bytes: &mut [u8]) {
    let mut prbs = crate::prbs::Prbs::new(0x5CA3_B1ED);
    let ks = prbs.bits(bytes.len() * 8);
    for (i, b) in bytes.iter_mut().enumerate() {
        let k = ks[i * 8..i * 8 + 8].iter().fold(0u8, |v, &x| (v << 1) | x);
        *b ^= k;
    }
}

// ----------------------------------------------------------------------------
// subcommands
// ----------------------------------------------------------------------------

/// Validate the message stack end-to-end through the emulated codecs.
pub fn run_selftest(text: &str) -> io::Result<()> {
    let tx = encode_message(text).map_err(io::Error::other)?;
    println!(
        "message {:?} ({} bytes) → {} samples ({:.1}s on air, robust 4-FSK)\n",
        text,
        text.len(),
        tx.len(),
        tx.len() as f64 / 8000.0
    );

    let trials: Vec<(&str, Box<dyn Codec>)> = vec![
        ("gsm-fr", Box::new(GsmFr::new())),
        ("amr-12.2", Box::new(AmrNb::new(AmrMode::Mr122))),
        ("amr-4.75", Box::new(AmrNb::new(AmrMode::Mr475))),
        (
            "bt+gsm-fr+bt",
            Box::new(Chain::new(vec![
                Box::new(Cvsd::new()),
                Box::new(GsmFr::new()),
                Box::new(Cvsd::new()),
            ])),
        ),
    ];
    for (name, mut codec) in trials {
        let rx = codec.process(&tx);
        match decode_message(&rx) {
            Some((msg, stats)) => {
                let ok = msg == text;
                println!(
                    "{:>14}: {}  {:?}  (codewords failed {}/{})",
                    name,
                    if ok { "OK  " } else { "FAIL" },
                    msg,
                    stats.codewords_failed,
                    stats.codewords_total
                );
            }
            None => println!("{:>14}: NO-SYNC", name),
        }
    }
    Ok(())
}

/// Transmit a message to a real audio device.
pub fn run_send(text: &str, device: Option<String>) -> io::Result<()> {
    let tx = encode_message(text).map_err(io::Error::other)?;
    println!(
        "Transmitting {:?} ({:.1}s){} ...",
        text,
        tx.len() as f64 / 8000.0,
        device.as_deref().map(|d| format!(" to {d}")).unwrap_or_default()
    );
    AlsaTool::new(device).play(&tx)?;
    println!("done.");
    Ok(())
}

/// Record from a real audio device and recover a message.
pub fn run_recv(seconds: f64, device: Option<String>) -> io::Result<()> {
    println!("Recording {seconds:.0}s ...");
    let rx = AlsaTool::new(device).record(seconds)?;
    match decode_message(&rx) {
        Some((msg, stats)) => println!(
            "RX: {:?}  (codewords failed {}/{})",
            msg, stats.codewords_failed, stats.codewords_total
        ),
        None => println!("RX: no message recovered (no sync / too noisy)"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roundtrip_through_gsm() {
        let text = "DoV link check 0123456789";
        let tx = encode_message(text).unwrap();
        let rx = GsmFr::new().process(&tx);
        let (got, stats) = decode_message(&rx).expect("decode");
        assert_eq!(got, text);
        assert_eq!(stats.codewords_failed, 0);
    }
}
