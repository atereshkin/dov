//! `dov-io` — live PCM audio I/O for the modem, behind a small trait so the
//! transport (a real soundcard, a Bluetooth-SCO device, or a USB GSM dongle's
//! audio endpoint) is just a swappable backend.
//!
//! It shells out to the platform's command-line audio tools — `aplay`/`arecord`
//! (ALSA) on Linux, `play`/`rec` (sox, `brew install sox`) on macOS — which
//! route through the OS audio stack. A Bluetooth headset's SCO link and a USB
//! modem's audio interface both show up as ordinary devices, so selecting
//! between them is just a different `device` string.
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

const MACOS: bool = cfg!(target_os = "macos");

/// Command-line audio backend. `device` selects the OS device:
///   * Linux: an ALSA device string (`default`, `bluealsa`, `plughw:CARD=...`)
///   * macOS: a sox `AUDIODEV` name
/// `None` uses the default device.
pub struct AudioDevice {
    pub device: Option<String>,
    pub rate: u32,
}

impl AudioDevice {
    pub fn new(device: Option<String>) -> Self {
        Self { device, rate: SAMPLE_RATE }
    }
}

impl AudioOut for AudioDevice {
    fn play(&mut self, pcm: &[i16]) -> io::Result<()> {
        let rate = self.rate.to_string();
        let mut cmd = if MACOS {
            let mut c = Command::new("play");
            c.args(["-q", "-t", "raw", "-r", &rate, "-e", "signed", "-b", "16", "-c", "1", "-"]);
            if let Some(d) = &self.device {
                c.env("AUDIODEV", d);
            }
            c
        } else {
            let mut c = Command::new("aplay");
            c.args(["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate]);
            if let Some(d) = &self.device {
                c.args(["-D", d]);
            }
            c
        };
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn()?;
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        child.stdin.take().unwrap().write_all(&bytes)?;
        if !child.wait()?.success() {
            return Err(io::Error::other("audio playback command failed"));
        }
        Ok(())
    }
}

impl AudioIn for AudioDevice {
    fn record(&mut self, seconds: f64) -> io::Result<Vec<i16>> {
        let rate = self.rate.to_string();
        let dur = seconds.ceil().max(1.0).to_string();
        let mut cmd = if MACOS {
            let mut c = Command::new("rec");
            c.args([
                "-q", "-t", "raw", "-r", &rate, "-e", "signed", "-b", "16", "-c", "1", "-",
                "trim", "0", &dur,
            ]);
            if let Some(d) = &self.device {
                c.env("AUDIODEV", d);
            }
            c
        } else {
            let mut c = Command::new("arecord");
            c.args(["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate, "-d", &dur]);
            if let Some(d) = &self.device {
                c.args(["-D", d]);
            }
            c
        };
        let out = cmd.stdout(Stdio::piped()).output()?;
        if !out.status.success() {
            return Err(io::Error::other("audio record command failed"));
        }
        Ok(out
            .stdout
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect())
    }
}
