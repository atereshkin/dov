//! Hand-written FFI declarations for the system codec libraries.
//!
//! These mirror the stable C ABIs of:
//!   * libgsm    — GSM 06.10 Full Rate (RPE-LTP), `gsm.h`
//!   * opencore-amrnb — 3GPP AMR narrowband encoder/decoder interface
//!
//! No headers are required at build time; `build.rs` handles linking.
#![allow(non_camel_case_types)]

use std::os::raw::{c_int, c_short, c_uchar, c_void};

// ---------------------------------------------------------------------------
// libgsm (GSM Full Rate, 06.10)
// ---------------------------------------------------------------------------
//
//   typedef short          gsm_signal;   // one PCM sample
//   typedef unsigned char  gsm_byte;     // one byte of the 33-byte frame
//   typedef struct gsm_state *gsm;       // opaque encoder+decoder state
//
//   gsm  gsm_create(void);
//   void gsm_destroy(gsm);
//   int  gsm_option(gsm, int opt, int *val);
//   void gsm_encode(gsm, gsm_signal *source /*160*/, gsm_byte *c /*33*/);
//   int  gsm_decode(gsm, gsm_byte *c /*33*/, gsm_signal *target /*160*/);

pub type gsm = *mut c_void;

extern "C" {
    pub fn gsm_create() -> gsm;
    pub fn gsm_destroy(s: gsm);
    pub fn gsm_option(s: gsm, opt: c_int, val: *mut c_int) -> c_int;
    pub fn gsm_encode(s: gsm, source: *const c_short, c: *mut c_uchar);
    pub fn gsm_decode(s: gsm, c: *const c_uchar, target: *mut c_short) -> c_int;
}

// ---------------------------------------------------------------------------
// opencore-amrnb (3GPP AMR narrowband)
// ---------------------------------------------------------------------------
//
//   void *Encoder_Interface_init(int dtx);
//   int   Encoder_Interface_Encode(void *st, enum Mode mode,
//                                  const short *speech /*160*/,
//                                  unsigned char *out, int forceSpeech);
//   void  Encoder_Interface_exit(void *st);
//
//   void *Decoder_Interface_init(void);
//   void  Decoder_Interface_Decode(void *st, const unsigned char *in,
//                                  short *out /*160*/, int bfi);
//   void  Decoder_Interface_exit(void *st);
//
// `Mode`: MR475=0, MR515=1, MR59=2, MR67=3, MR74=4, MR795=5, MR102=6, MR122=7.
// Encoder output is one frame in the 3GPP IF storage format (mode header byte
// followed by the speech bits); the decoder consumes the same format.

extern "C" {
    pub fn Encoder_Interface_init(dtx: c_int) -> *mut c_void;
    pub fn Encoder_Interface_Encode(
        state: *mut c_void,
        mode: c_int,
        speech: *const c_short,
        out: *mut c_uchar,
        force_speech: c_int,
    ) -> c_int;
    pub fn Encoder_Interface_exit(state: *mut c_void);

    pub fn Decoder_Interface_init() -> *mut c_void;
    pub fn Decoder_Interface_Decode(
        state: *mut c_void,
        input: *const c_uchar,
        out: *mut c_short,
        bfi: c_int,
    );
    pub fn Decoder_Interface_exit(state: *mut c_void);
}
