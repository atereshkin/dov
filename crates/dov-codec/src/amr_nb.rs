//! AMR narrowband (3GPP TS 26.071/26.090) via opencore-amrnb.
//!
//! AMR-NB is the dominant cellular voice codec. Its 12.2 kbit/s mode (MR122)
//! is the same ACELP coder as GSM-EFR (06.60), so emulating EFR is a matter of
//! running this codec in [`AmrMode::Mr122`].

use crate::ffi;
use crate::{Codec, FRAME_LEN};
use std::os::raw::c_void;

/// AMR-NB bit-rate mode. Discriminants match the C `enum Mode` ordinals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum AmrMode {
    /// 4.75 kbit/s — most aggressive compression, harshest on non-speech.
    Mr475 = 0,
    Mr515 = 1,
    Mr59 = 2,
    Mr67 = 3,
    Mr74 = 4,
    Mr795 = 5,
    Mr102 = 6,
    /// 12.2 kbit/s — equivalent to GSM-EFR; gentlest, most faithful.
    Mr122 = 7,
}

impl AmrMode {
    /// Rate in bits per second.
    pub fn bitrate(self) -> u32 {
        match self {
            AmrMode::Mr475 => 4_750,
            AmrMode::Mr515 => 5_150,
            AmrMode::Mr59 => 5_900,
            AmrMode::Mr67 => 6_700,
            AmrMode::Mr74 => 7_400,
            AmrMode::Mr795 => 7_950,
            AmrMode::Mr102 => 10_200,
            AmrMode::Mr122 => 12_200,
        }
    }

    fn label(self) -> &'static str {
        match self {
            AmrMode::Mr475 => "amr-nb-4.75",
            AmrMode::Mr515 => "amr-nb-5.15",
            AmrMode::Mr59 => "amr-nb-5.9",
            AmrMode::Mr67 => "amr-nb-6.7",
            AmrMode::Mr74 => "amr-nb-7.4",
            AmrMode::Mr795 => "amr-nb-7.95",
            AmrMode::Mr102 => "amr-nb-10.2",
            AmrMode::Mr122 => "amr-nb-12.2",
        }
    }
}

/// AMR narrowband codec round-trip at a fixed mode.
pub struct AmrNb {
    enc: *mut c_void,
    dec: *mut c_void,
    mode: AmrMode,
    /// Discontinuous transmission (VAD-driven). When on, the encoder will emit
    /// SID/no-data frames for input it judges to be silence — exactly the
    /// behaviour a data-over-voice modem must defeat. Off by default.
    dtx: bool,
    name: String,
}

impl AmrNb {
    /// Create an AMR-NB codec at `mode`, with DTX disabled.
    pub fn new(mode: AmrMode) -> Self {
        Self::with_dtx(mode, false)
    }

    /// Create an AMR-NB codec, choosing whether DTX/VAD is active.
    pub fn with_dtx(mode: AmrMode, dtx: bool) -> Self {
        let enc = unsafe { ffi::Encoder_Interface_init(dtx as i32) };
        let dec = unsafe { ffi::Decoder_Interface_init() };
        assert!(!enc.is_null() && !dec.is_null(), "AMR interface init failed");
        let name = if dtx {
            format!("{}-dtx", mode.label())
        } else {
            mode.label().to_string()
        };
        Self { enc, dec, mode, dtx, name }
    }

    pub fn mode(&self) -> AmrMode {
        self.mode
    }

    pub fn dtx(&self) -> bool {
        self.dtx
    }

    /// Encode one frame to its 3GPP storage-format bytes (mode header + payload),
    /// advancing the encoder. This is exactly one frame of a `.amr` file.
    pub fn encode_frame(&mut self, input: &[i16; FRAME_LEN]) -> Vec<u8> {
        let mut buf = [0u8; 64];
        let n = unsafe {
            ffi::Encoder_Interface_Encode(
                self.enc,
                self.mode as i32,
                input.as_ptr(),
                buf.as_mut_ptr(),
                0,
            )
        };
        buf[..n.max(0) as usize].to_vec()
    }

    /// Encode a whole signal to a `.amr` bytestream (magic header + frames),
    /// suitable for decoding by any conformant AMR decoder.
    pub fn encode_to_amr(&mut self, signal: &[i16]) -> Vec<u8> {
        let mut out = Vec::from(&b"#!AMR\n"[..]);
        let mut frame = [0i16; FRAME_LEN];
        for chunk in signal.chunks(FRAME_LEN) {
            frame[..chunk.len()].copy_from_slice(chunk);
            frame[chunk.len()..].fill(0);
            out.extend_from_slice(&self.encode_frame(&frame));
        }
        out
    }
}

impl Codec for AmrNb {
    fn name(&self) -> &str {
        &self.name
    }

    fn process_frame(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        // Max AMR-NB payload is 31 bytes (MR122) + 1 mode header byte.
        let mut encoded = [0u8; 64];
        let mut output = [0i16; FRAME_LEN];
        unsafe {
            let n = ffi::Encoder_Interface_Encode(
                self.enc,
                self.mode as i32,
                input.as_ptr(),
                encoded.as_mut_ptr(),
                0,
            );
            debug_assert!(n > 0 && (n as usize) <= encoded.len());
            ffi::Decoder_Interface_Decode(self.dec, encoded.as_ptr(), output.as_mut_ptr(), 0);
        }
        output
    }

    fn process_frame_erased(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        // The encoder still ran at the transmitter (advance its state), but the
        // bits never arrived, so the decoder conceals via bfi=1.
        let mut encoded = [0u8; 64];
        let mut output = [0i16; FRAME_LEN];
        unsafe {
            ffi::Encoder_Interface_Encode(
                self.enc,
                self.mode as i32,
                input.as_ptr(),
                encoded.as_mut_ptr(),
                0,
            );
            ffi::Decoder_Interface_Decode(self.dec, encoded.as_ptr(), output.as_mut_ptr(), 1);
        }
        output
    }
}

impl Drop for AmrNb {
    fn drop(&mut self) {
        unsafe {
            ffi::Encoder_Interface_exit(self.enc);
            ffi::Decoder_Interface_exit(self.dec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prove the DTX path is genuinely engaged (not a silently-ignored flag):
    /// with DTX on, sustained silence must produce short SID/NO_DATA frames,
    /// whereas without DTX every frame is a full-size speech frame. This is the
    /// premise behind testing whether a modem signal survives VAD/DTX.
    #[test]
    fn dtx_shortens_silence_frames() {
        let silence = [0i16; FRAME_LEN];
        let mut buf = [0u8; 64];
        unsafe {
            let enc_dtx = ffi::Encoder_Interface_init(1);
            let enc_off = ffi::Encoder_Interface_init(0);
            let mut min_dtx = i32::MAX;
            let mut last_off = 0i32;
            // Several frames so the VAD hangover expires and DTX actually kicks in.
            for _ in 0..30 {
                let n_dtx = ffi::Encoder_Interface_Encode(
                    enc_dtx,
                    AmrMode::Mr122 as i32,
                    silence.as_ptr(),
                    buf.as_mut_ptr(),
                    0,
                );
                min_dtx = min_dtx.min(n_dtx);
                last_off = ffi::Encoder_Interface_Encode(
                    enc_off,
                    AmrMode::Mr122 as i32,
                    silence.as_ptr(),
                    buf.as_mut_ptr(),
                    0,
                );
            }
            ffi::Encoder_Interface_exit(enc_dtx);
            ffi::Encoder_Interface_exit(enc_off);

            assert!(last_off >= 13, "non-DTX MR122 should emit full speech frames, got {last_off}");
            assert!(
                min_dtx < last_off,
                "DTX should shorten silence frames: min_dtx={min_dtx} non_dtx={last_off}"
            );
        }
    }
}

