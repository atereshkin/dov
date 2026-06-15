//! `dov-io` — live PCM audio I/O for the modem, behind a small trait so the
//! transport (a real soundcard, a Bluetooth-SCO device, or a USB GSM dongle's
//! audio endpoint) is just a swappable backend.
//!
//! The first backend shells out to ALSA's `aplay`/`arecord`, which route through
//! PipeWire/PulseAudio via the ALSA bridge on a normal desktop. Crucially, a
//! Bluetooth headset's SCO link and a USB modem's audio interface both show up
//! as ordinary ALSA/PCM devices, so selecting between them is just a different
//! `device` string — exactly the "two backends behind one interface" we want.
#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::{Command, Stdio};

/// Plays mono 8 kHz `i16` PCM to an output device.
pub trait AudioOut {
    fn play(&mut self, pcm: &[i16]) -> io::Result<()>;
}

/// Records mono 8 kHz `i16` PCM from an input device for a fixed duration.
pub trait AudioIn {
    fn record(&mut self, seconds: f64) -> io::Result<Vec<i16>>;
}

/// Sample rate every narrowband codec/modem uses.
pub const SAMPLE_RATE: u32 = 8_000;

/// ALSA `aplay`/`arecord` backend. `device` is the ALSA device string (e.g.
/// `default`, a `bluez`/`bluealsa` SCO device, or `plughw:CARD=<dongle>`);
/// `None` uses the default device.
pub struct AlsaTool {
    pub device: Option<String>,
    pub rate: u32,
}

impl AlsaTool {
    pub fn new(device: Option<String>) -> Self {
        Self { device, rate: SAMPLE_RATE }
    }
}

impl AudioOut for AlsaTool {
    fn play(&mut self, pcm: &[i16]) -> io::Result<()> {
        let rate = self.rate.to_string();
        let mut cmd = Command::new("aplay");
        cmd.args(["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate]);
        if let Some(d) = &self.device {
            cmd.args(["-D", d]);
        }
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn()?;
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        // aplay drains stdin at playback rate, so this blocks ~ real time.
        child.stdin.take().unwrap().write_all(&bytes)?;
        if !child.wait()?.success() {
            return Err(io::Error::other("aplay failed"));
        }
        Ok(())
    }
}

impl AudioIn for AlsaTool {
    fn record(&mut self, seconds: f64) -> io::Result<Vec<i16>> {
        let rate = self.rate.to_string();
        let dur = seconds.ceil().max(1.0).to_string();
        let mut cmd = Command::new("arecord");
        cmd.args(["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate, "-d", &dur]);
        if let Some(d) = &self.device {
            cmd.args(["-D", d]);
        }
        let out = cmd.stdout(Stdio::piped()).output()?;
        if !out.status.success() {
            return Err(io::Error::other("arecord failed"));
        }
        Ok(out
            .stdout
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect())
    }
}
