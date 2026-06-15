//! GSM Full Rate (06.10, RPE-LTP, 13 kbit/s) via libgsm.

use crate::ffi;
use crate::{Codec, FRAME_LEN};

/// One encoded GSM-FR frame is 33 bytes (260 used bits + padding).
const GSM_FRAME_BYTES: usize = 33;

/// GSM Full Rate codec round-trip.
///
/// Uses two independent libgsm states — one acting as the transmit-side
/// encoder, one as the receive-side decoder — so the loop matches a real
/// one-way voice path.
pub struct GsmFr {
    enc: ffi::gsm,
    dec: ffi::gsm,
    /// Last clean decoder output, for substitution/muting on a lost frame
    /// (libgsm has no `bfi` input, so we model GSM 06.11 concealment here).
    last_out: [i16; FRAME_LEN],
}

impl GsmFr {
    pub fn new() -> Self {
        let enc = unsafe { ffi::gsm_create() };
        let dec = unsafe { ffi::gsm_create() };
        assert!(!enc.is_null() && !dec.is_null(), "gsm_create() returned null");
        Self {
            enc,
            dec,
            last_out: [0i16; FRAME_LEN],
        }
    }
}

impl Default for GsmFr {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for GsmFr {
    fn name(&self) -> &str {
        "gsm-fr"
    }

    fn process_frame(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        let mut encoded = [0u8; GSM_FRAME_BYTES];
        let mut output = [0i16; FRAME_LEN];
        unsafe {
            ffi::gsm_encode(self.enc, input.as_ptr(), encoded.as_mut_ptr());
            ffi::gsm_decode(self.dec, encoded.as_ptr(), output.as_mut_ptr());
        }
        self.last_out = output;
        output
    }

    fn process_frame_erased(&mut self, _input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        // GSM 06.11-style substitution: repeat the last good frame, attenuated.
        let mut output = self.last_out;
        for s in &mut output {
            *s = (*s as f64 * 0.75) as i16;
        }
        self.last_out = output;
        output
    }
}

impl Drop for GsmFr {
    fn drop(&mut self) {
        unsafe {
            ffi::gsm_destroy(self.enc);
            ffi::gsm_destroy(self.dec);
        }
    }
}
