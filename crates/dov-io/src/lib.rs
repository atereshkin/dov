//! `dov-io` — live PCM audio I/O for the modem, behind a small trait so the
//! transport is a swappable backend:
//!
//!   * an OS audio device (`aplay`/`arecord` on Linux, `play`/`rec` via sox on
//!     macOS) — a soundcard, a snd-aloop loopback, etc.;
//!   * a specific **PipeWire node** via `pw-cat` — selected with a `pw:<node>`
//!     device string. This is how we play straight into / capture straight out
//!     of a Bluetooth-HFP call's `bluez_output`/`bluez_input` nodes.
//!
//! A Bluetooth headset's SCO link and a USB modem's audio interface both show up
//! as ordinary devices/nodes, so picking between them is just the device string.
#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::{Command, Stdio};

/// Plays mono 8 kHz `i16` PCM to an output.
pub trait AudioOut {
    fn play(&mut self, pcm: &[i16]) -> io::Result<()>;
}

/// Records mono 8 kHz `i16` PCM from an input for a fixed duration.
pub trait AudioIn {
    fn record(&mut self, seconds: f64) -> io::Result<Vec<i16>>;
}

/// Sample rate every narrowband codec/modem uses.
pub const SAMPLE_RATE: u32 = 8_000;

const MACOS: bool = cfg!(target_os = "macos");

enum Backend {
    /// OS audio CLI; `Option` device string (`None` = default device).
    Os(Option<String>),
    /// A specific PipeWire node, driven by `pw-cat --target`.
    Pw(String),
}

/// Command-line audio backend. `device`:
///   * `None` / a plain string → OS device (ALSA `default`/`bluealsa`/`plughw:…`,
///     or a sox `AUDIODEV` on macOS);
///   * `"pw:<node>"` → a PipeWire node by name or serial (e.g.
///     `pw:bluez_output.D4_3A_2C_7D_C9_F3.1`).
pub struct AudioDevice {
    backend: Backend,
    rate: u32,
}

impl AudioDevice {
    pub fn new(device: Option<String>) -> Self {
        let backend = match device {
            Some(d) => match d.strip_prefix("pw:") {
                Some(node) => Backend::Pw(node.to_string()),
                None => Backend::Os(Some(d)),
            },
            None => Backend::Os(None),
        };
        Self { backend, rate: SAMPLE_RATE }
    }

    fn pw_args(&self, mode: &str, node: &str) -> Vec<String> {
        // pw-cat -p/-r --raw --target NODE --rate R --channels 1 --format s16 -
        let r = self.rate.to_string();
        [mode, "--raw", "--target", node, "--rate", &r, "--channels", "1", "--format", "s16", "-"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
}

impl AudioOut for AudioDevice {
    fn play(&mut self, pcm: &[i16]) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for &s in pcm {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let rate = self.rate.to_string();
        let mut cmd = match &self.backend {
            Backend::Pw(node) => {
                let mut c = Command::new("pw-cat");
                c.args(self.pw_args("-p", node));
                c
            }
            Backend::Os(device) if MACOS => {
                let mut c = Command::new("play");
                c.args(["-q", "-t", "raw", "-r", &rate, "-e", "signed", "-b", "16", "-c", "1", "-"]);
                if let Some(d) = device {
                    c.env("AUDIODEV", d);
                }
                c
            }
            Backend::Os(device) => {
                let mut c = Command::new("aplay");
                c.args(["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate]);
                if let Some(d) = device {
                    c.args(["-D", d]);
                }
                c
            }
        };
        let mut child = cmd.stdin(Stdio::piped()).spawn()?;
        child.stdin.take().unwrap().write_all(&bytes)?; // dropping stdin = EOF
        if !child.wait()?.success() {
            return Err(io::Error::other("audio playback command failed"));
        }
        Ok(())
    }
}

impl AudioIn for AudioDevice {
    fn record(&mut self, seconds: f64) -> io::Result<Vec<i16>> {
        let rate = self.rate.to_string();
        let secs = seconds.ceil().max(1.0);
        let out = match &self.backend {
            // `timeout` stops pw-cat after the duration; raw output stays valid.
            Backend::Pw(node) => {
                let mut c = Command::new("timeout");
                c.arg(format!("{secs}"));
                c.arg("pw-cat");
                c.args(self.pw_args("-r", node));
                c.stdout(Stdio::piped()).output()?
            }
            Backend::Os(device) if MACOS => {
                let mut c = Command::new("rec");
                c.args([
                    "-q", "-t", "raw", "-r", &rate, "-e", "signed", "-b", "16", "-c", "1", "-",
                    "trim", "0", &secs.to_string(),
                ]);
                if let Some(d) = device {
                    c.env("AUDIODEV", d);
                }
                c.stdout(Stdio::piped()).output()?
            }
            Backend::Os(device) => {
                let mut c = Command::new("arecord");
                c.args(["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", &rate, "-d", &secs.to_string()]);
                if let Some(d) = device {
                    c.args(["-D", d]);
                }
                c.stdout(Stdio::piped()).output()?
            }
        };
        // `timeout` exits 124 when it kills the recorder — that's the normal
        // path, so we take whatever was captured rather than checking status.
        Ok(out
            .stdout
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect())
    }
}
