//! `dov-codec` — thin, safe wrappers around real GSM/AMR voice codecs.
//!
//! The point of the whole `dov` ("data over voice") project is to push a
//! software modem signal through a *voice* vocoder and see what survives.
//! This crate provides the vocoder side of that loop by binding the actual
//! C codec libraries the cellular world uses, so emulation on a PC exercises
//! the genuine RPE-LTP / ACELP distortion rather than an approximation.
//!
//! Everything is narrowband: 8 kHz sample rate, 20 ms frames = 160 `i16`
//! samples per frame. Beyond the cellular vocoders, [`Cvsd`] models the
//! Bluetooth HFP voice codec, and [`Chain`] composes codecs in series to model
//! tandem paths (e.g. a Bluetooth-bridged GSM call).

pub mod ffi;
mod amr_nb;
mod chain;
mod cvsd;
mod gsm_fr;

pub use amr_nb::{AmrMode, AmrNb};
pub use chain::Chain;
pub use cvsd::Cvsd;
pub use gsm_fr::GsmFr;

/// Sample rate of every narrowband GSM/AMR codec, in Hz.
pub const SAMPLE_RATE: u32 = 8_000;

/// Samples per 20 ms frame at [`SAMPLE_RATE`].
pub const FRAME_LEN: usize = 160;

/// A voice vocoder used as a (lossy, nonlinear) transmission channel.
///
/// Implementors run a single 20 ms PCM frame through a real encode→decode
/// round-trip. Encoder and decoder state are kept separate inside the
/// implementor, mirroring a real call where the encoder lives in the
/// transmitting handset and the decoder in the receiving one.
pub trait Codec {
    /// Short stable identifier, e.g. `"gsm-fr"` or `"amr-nb-12.2"`.
    fn name(&self) -> &str;

    /// Encode then decode one 20 ms frame, returning the reconstructed PCM.
    fn process_frame(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN];

    /// Process a frame whose transmitted bits were *lost*: run the codec's
    /// native packet-loss concealment instead of a clean decode. Modems must
    /// survive this — after a run of losses the decoder mutes, which both flips
    /// bits and slips symbol timing.
    ///
    /// The default mutes (no concealment); real codecs override it.
    fn process_frame_erased(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        let _ = input;
        [0i16; FRAME_LEN]
    }

    /// Run an arbitrary-length signal through the codec frame by frame.
    ///
    /// The input is zero-padded up to a whole number of frames; the returned
    /// vector therefore has length `ceil(input.len() / FRAME_LEN) * FRAME_LEN`.
    /// Callers that care about exact alignment should truncate to their own
    /// known length.
    fn process(&mut self, input: &[i16]) -> Vec<i16> {
        let frames = input.len().div_ceil(FRAME_LEN);
        let mut out = Vec::with_capacity(frames * FRAME_LEN);
        let mut frame = [0i16; FRAME_LEN];
        for chunk in input.chunks(FRAME_LEN) {
            frame[..chunk.len()].copy_from_slice(chunk);
            frame[chunk.len()..].fill(0);
            out.extend_from_slice(&self.process_frame(&frame));
        }
        out
    }
}
