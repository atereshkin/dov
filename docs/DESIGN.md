
# dov — Data-over-Voice Modem: Emulation-on-PC Milestone Plan

A vocoder is not an additive-noise channel. It is a *memoryful, non-linear, analysis-by-synthesis* re-synthesizer that keeps the LPC spectral envelope + one dominant pitch + per-subframe gains and throws away fine waveform and absolute phase. Every architectural decision below follows from that single fact. The verified research says: **no coherent phase, no absolute amplitude, stay in ~500–2500 Hz, look like active speech to the VAD, and measure against the real codec in the loop.**

There is already a working scaffold in this repo (`dov-codec` with FFI + `GsmFr`/`AmrNb` + a `Codec` trait, and `dov-harness/dov-probe` doing tone-survival + chirp spectrograms). The probe builds and runs today. This plan **extends** that scaffold rather than replacing it.

> **A note on the existing tone-survival numbers.** The current `dov-probe` table shows single steady tones surviving almost flat (≈ −0.5 dB) everywhere in-band, even on AMR 4.75. That is *expected and misleading*: a single sustained sinusoid hits the AMR VAD tone-detector and is reproduced as a near-perfect formant. It tells us almost nothing about a modem, because the modem's enemy is **inter-symbol distortion and transition smearing**, not steady-tone attenuation. The harness's primary metric must therefore move from "tone energy survival" to **end-to-end symbol/bit error rate on a modulated stream**. This reframing is milestone 1.

---

## 1. Cargo workspace layout

Minimal but extensible. Five concerns → five crates. Keep DSP `no_std`-friendly and dependency-light; confine FFI to one crate; confine I/O and plotting to the harness.

```
dov/
├── Cargo.toml                      # [workspace], shared deps, profiles
├── crates/
│   ├── dov-codec/                  # EXISTS — FFI + Codec trait (the vocoder "channel core")
│   │   ├── build.rs                # OUT_DIR symlink → versioned .so linking (works today)
│   │   └── src/{ffi,gsm_fr,amr_nb,amr_wb,codec2,lib}.rs
│   │
│   ├── dov-channel/                # NEW — composable impairment pipeline around a Codec
│   │   └── src/{lib,stage,agc,resample,fer,bandpass,transcode,noise}.rs
│   │
│   ├── dov-modem/                  # NEW — pure DSP: modulate bits→PCM, demodulate PCM→soft bits
│   │   └── src/{lib,mfsk,goertzel,sls,preamble,timing,dsp}.rs
│   │
│   ├── dov-frame/                  # NEW — framing, sync, FEC, interleaving, CRC (codec-free)
│   │   └── src/{lib,frame,rs_fec,interleave,crc,scrambler}.rs
│   │
│   └── dov-harness/                # EXISTS — CLI + BER/throughput + spectrogram orchestration
│       └── src/{main,wav,probe,run,report,plot}.rs
└── xtask or scripts/               # optional: spectrogram/plot driving via ffmpeg
```

**Responsibilities & dependency direction** (strictly one-way, no cycles):

| Crate | Owns | Depends on | Deliberately does NOT |
|---|---|---|---|
| `dov-codec` | Hand-written FFI to libgsm / opencore-amrnb / opencore-amrwb / vo-amrwbenc / codec2; the `Codec` trait; per-frame encode→decode incl. `bfi` PLC and DTX flag | nothing (FFI only) | know about modems, impairments, or bits |
| `dov-channel` | A `Channel` that wraps a `Codec` plus an ordered `Vec<Box<dyn Stage>>`: AGC, DC, bandpass, resample/clock-drift, frame-erasure (Gilbert–Elliott→`bfi`), additive noise, transcode/tandem chains, AMR mode-switch | `dov-codec` | DSP of the modem; bit-level concerns |
| `dov-modem` | Modulator (bits→i16 PCM @ 8 kHz) and demodulator (PCM→soft symbols/LLRs). MFSK first, SLS codebook later. Timing recovery, preamble correlation, Goertzel/FFT, per-harmonic channel estimation | tiny DSP only (maybe `rustfft`) | codecs, framing, FEC |
| `dov-frame` | Preamble/unique-word, 16-bit frame counter, CRC, Reed–Solomon-with-erasures, block interleaver, run-length-limiting transcode, scrambler | nothing | DSP, codecs |
| `dov-harness` | CLI (`clap`), test vectors, BER/SER/throughput measurement, parameter sweeps, WAV dump, spectrogram driving, result tables/CSV | all of the above | being imported by anything |

**Workspace conventions**

- `resolver = "2"`, shared `[workspace.package]` (already present).
- `[profile.release] lto = "thin"`, `opt-level = 3` — DSP loops matter once sweeps get big.
- Keep external deps tiny: `clap` (harness CLI), optionally `rustfft` (demod) and `reed-solomon-erasure` or hand-rolled GF(256) RS (frame). Everything else hand-rolled (the project already hand-rolls WAV and Goertzel — keep that ethos; it makes the DSP auditable, which matters when debugging a channel this weird).
- `dov-modem` and `dov-frame` stay `#![forbid(unsafe_code)]`; all `unsafe` lives in `dov-codec`.

---

## 2. The codec "channel" abstraction

### 2.1 The `Codec` trait (already exists — keep, extend slightly)

The current trait is good. Two surgical extensions so the channel layer can model real impairments:

```rust
pub trait Codec {
    fn name(&self) -> &str;
    fn process_frame(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN];
    fn process(&mut self, input: &[i16]) -> Vec<i16> { /* default, exists */ }

    // NEW — model a Bad Frame Indicator so the decoder runs its native PLC.
    // Default impl = no erasure support (substitute/mute handled in dov-channel).
    fn process_frame_erased(&mut self, _input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        [0i16; FRAME_LEN] // codecs with native bfi override this
    }

    // NEW — sample rate so AMR-WB (16 kHz / 320-sample frame) coexists with NB.
    fn sample_rate(&self) -> u32 { SAMPLE_RATE }   // 8 kHz default
    fn frame_len(&self)   -> usize { FRAME_LEN }   // 160 default
}
```

- **`GsmFr`** (exists): `gsm_encode`/`gsm_decode`, 160 i16 → 33 bytes → 160 i16. Erasure = repeat-last / mute-and-decay per GSM 06.11 (libgsm has no `bfi`) — implement in `process_frame_erased`.
- **`AmrNb`** (exists): `Encoder_Interface_Encode(mode, …)` → `Decoder_Interface_Decode(…, bfi)`. `process_frame_erased` calls decode with `bfi=1` (verified to diverge → real native concealment). `Mr122` = EFR speech-core approximation (per verification: interoperable for speech, **not** bit-exact, and DTX/SID differs — so do NOT trust `Mr122` for EFR comfort-noise behavior).
- **`AmrWb`** (new, deferred to a later milestone): encode via `vo-amrwbenc` `E_IF_*`, decode via `opencore-amrwb` `D_IF_*`, 320 samples @ 16 kHz, `bfi` supported. Only needed once NB is solid.
- **`Codec2`** (new, optional): pure-Rust crate, a clean reference leg.

### 2.2 FFI & linking (already solved here — keep it)

- Hand-written `extern "C"` decls in `dov-codec/src/ffi.rs` (no `-dev` headers). ✔ verified ABIs.
- `build.rs` creates **unversioned `.so` symlinks in `OUT_DIR`** pointing at the versioned `libgsm.so.1` / `libopencore-amrnb.so.0`, then `rustc-link-lib=dylib=…` + `rustc-link-search=native=$OUT_DIR`. This is a clean alternative to `-l:libgsm.so.1`; both work (verification confirmed the exact-name `-l:` form, and the symlink form sidesteps the missing dev symlink the same way). **Keep the existing build.rs** — it already passes. When adding AMR-WB, register `libopencore-amrwb.so.0` and `libvo-amrwbenc.so.0` the same way, plus `-lm` if a math symbol is unresolved.
- Add a friendly existence check (already panics with an install hint — good).

### 2.3 How impairments compose: the `Channel` + `Stage` pipeline

`dov-channel` is the realism layer. A `Channel` owns a `Codec` and a pre-codec and post-codec stage list. Each stage is a toggle; the biggest realism lever (native `bfi` PLC) lives in the erasure stage, not in a stage struct but as a per-frame decision that selects `process_frame` vs `process_frame_erased`.

```rust
pub trait Stage {
    fn name(&self) -> &str;
    fn process(&mut self, pcm: &mut [i16]);          // in-place, whole-signal or per-frame
}

pub struct Channel {
    pre:  Vec<Box<dyn Stage>>,   // DC offset, AGC, telephone bandpass, resample/drift
    codec: Box<dyn Codec>,
    erasure: ErasureModel,       // Gilbert–Elliott burst → per-frame bfi / substitute-mute
    post: Vec<Box<dyn Stage>>,   // resample-back, additive/comfort noise
}
impl Channel {
    pub fn run(&mut self, tx_pcm: &[i16]) -> Vec<i16> { /* pre → frame-loop(codec|erased) → post */ }
}
```

Composable stages, each independently toggleable (default OFF for a clean baseline, ON in sweeps):

1. **DC offset / removal** — small DC, then high-pass (AMR adds an 80 Hz HPF internally; model it pre-codec).
2. **AGC/ALC** — feedback gain normalization + clamp to i16. *This is why no information may live in absolute amplitude.* Parameterize attack/release so we can study whether AGC flattens our intended VAD-defeat envelope pulses.
3. **Telephone bandpass** — 300–3400 Hz (NB) / 50–7000 Hz (WB). Kills out-of-band energy; place all symbols in 500–2500 Hz where the response is flattest.
4. **Resample / clock drift** — ±N ppm via `libsoxr.so.0` (present) or a pure-Rust sinc/Farrow. Drives the demod's continuous timing recovery; optionally explicit insert/drop slip events.
5. **Codec** (the core) — with selectable DTX.
6. **Frame erasure → PLC** — Gilbert–Elliott burst model sets `bfi=1` (AMR native PLC) or substitute/mute (GSM-FR). Default FER sweep `{0, 1, 3, 5, 10}%`.
7. **Transcode / tandem** — chain two `Channel`s (e.g. FR→AMR, or AMR mode-A→mode-B) to compound distortion; mid-call AMR **mode switching every N frames at frame boundaries**.
8. **Additive / comfort noise** — Gaussian + idle-channel noise; CNG during DTX/SID windows.

Tandem chains are just `Channel` composition: `chain(vec![fr_channel, amr_channel])`. This makes "what does FR→AMR tandem do to my modem" a one-line experiment.

---

## 3. Recommended baseline modem (build this FIRST) + iteration path

### 3.1 BASELINE: Non-coherent **constant-differential / continuous-phase M-FSK** (Hermes-style), demodulated by a Goertzel/FFT tone bank

**Why this first, for *this* channel (justified):**

- **It is the only proven *codec-agnostic* scheme.** Verified: Hermes' differential FSK ("IncDec") gets ~1.2 kbps at BER ~1e-5 over **real, unknown, heterogeneous** networks with **no training**. The SLS codebook reaches higher rate but is fragile to mid-call AMR mode switches and needs a per-codec trained dictionary — wrong place to start.
- **Frequency survives; phase and amplitude do not** (verified across all four codecs). FSK beat ASK by ~10× and PSK was "virtually impossible." Continuous-phase + differential frequency means we never depend on absolute phase, an absolute amplitude reference, or a surviving carrier-phase reference.
- **It is trivial, auditable DSP** that reuses what's already in the repo (sine gen + Goertzel power are already written). Proof-of-life in days, and it doubles as the channel-probe instrument.
- **It is naturally VAD-friendly**: a continuous, pitched, pulsing in-band tone train with bounded frequency steps reads as "active speech/tone" to the VAD's explicit tone/periodicity detector — exactly the corrected verification nuance (must look *active*, not necessarily *voiced*).

**Exact starting parameters (first PC build, through GSM-FR and AMR-NB MR122/MR795/MR475):**

- **Band / center**: `f_base ≈ 1500 Hz`, all tones confined to **600–2400 Hz** (the empirically flat region — even AMR 4.75 only collapses >3 kHz in our own probe data; band edges and >2.5 kHz are where low modes bite).
- **Stage-0 alphabet — 8-FSK proof-of-life**: tones at **{700, 900, 1100, 1300, 1500, 1700, 1900, 2100} Hz**, **20 ms/symbol** (= one full codec frame, 160 samples), 3 bits/symbol, **400 baud → 1200 bps raw**, no FEC. Phase-continuous between symbols. Goertzel bank (8 bins), pick max → hard symbol. This measures the real per-tone *transition* survival map (the thing the steady-tone table cannot show).
- **Stage-1 robust baseline — continuous-differential 2-/4-FSK ("IncDec")**: start at `f_base`, each input symbol steps frequency by `±Δ` (or one of M bounded steps), `Δ ≈ 10–25% of f_base` (≈ 200–375 Hz). Demod = compare current vs previous recovered tone frequency (≥ ⇒ 1). **Run-length-limit** the bitstream (1:2 transcode, max ~2 equal in a row) so the fundamental stays stable and voice-like for the VAD; goodput ≈ `f_base/2` ≈ 600–1200 bps. Symbol = 20 ms (frame-aligned), windowed with ~1 ms raised-cosine edges so transitions don't create wideband splatter the codec mangles.
- **Energy / envelope**: amplitude ≈ −12 dBFS (8000/32767), constant; **carry zero bits in amplitude**. Pulse the *overall* envelope on a ~0.5–1 s schedule to keep DTX from declaring silence, with the RX treating scheduled low-energy frames as known erasures.
- **Sync**: ~20 ms constant preamble (a couple of distinctive tones) for frame-boundary detection via sliding correlation; lock symbol clock to recovered 20 ms frame boundaries; continuous timing tracking for clock drift.

This is `dov-modem`'s `mfsk` module. Hard-decision first; add soft outputs (per-tone confidence margin) once it works, to feed RS erasure flags.

### 3.2 Iteration path → higher-rate **trained Speech-Like-Symbol (SLS) codebook**

Once MFSK + framing + FEC measure cleanly, graduate to the literature's highest-*rate* design (only worth it when the codec is known/fixed):

1. **Symbols**: multi-harmonic *voiced-sounding* waveforms — fundamental ~150–250 Hz with 7–10 harmonics placed in 500–2500 Hz, **5 ms/symbol** (40 samples, aligns to the ACELP subframe grid; exactly 4 symbols per 20 ms frame), raised-cosine edges so it reads as voiced speech and reproduces cleanly through the LPC synthesis filter.
2. **Codebook generation/selection (the real work)**: generate a candidate pool (LPC-synthesized voiced pulse trains, pitch-pulse trains, real voiced-speech excerpts); **pass every candidate through the actual target codec** (`dov-codec`); build the M×M *post-vocoder* distance/confusion matrix; greedily select the M symbols maximizing minimum post-vocoder distance (≈ maximizing DMC capacity). Start **M=16 (4 bits/sym)** → grow to 32/64. **Separate codebook per codec/mode**, switch on detected AMR mode. Expect the classic SER staircase as M grows.
3. **Demod**: train on a ~2–4 s preamble (per-harmonic phase shift φ_k and distortion variance σ_k²), decode each symbol by **energy-normalized, σ-weighted, phase-compensated Euclidean distance** (= max weighted correlation / matched filter) against all M post-vocoder templates; output soft confidence for erasure flags.
4. **Throughput target**: ~800 bps/stream at M=16/5 ms → run parallel streams or grow M toward **~2–3 kbps raw, ~2 kbps net** after FEC on AMR-NB/EFR; AMR-WB later for more room. Keep MFSK as the permanent fallback / unknown-codec / half-rate mode.

**Net stance:** ship the codec-agnostic MFSK modem first (robust, proven, simple), then add the trained SLS codebook as a higher-throughput mode for known codecs — *not* a replacement.

---

## 4. BER/throughput harness + iteration methodology

The harness is the primary instrument. The guiding principle: **measure what survives a modulated stream end-to-end, per codec/mode/impairment — never trust steady-tone survival as a proxy.**

### 4.1 Core measurement loop

```
random bits ─▶ dov-frame encode ─▶ dov-modem modulate ─▶ TX i16 PCM
   ─▶ dov-channel run (pre → codec(+bfi/DTX) → post)
   ─▶ RX i16 PCM ─▶ dov-modem demodulate (soft) ─▶ dov-frame decode
   ─▶ compare to known bits
```

Report, per run: **raw BER, post-FEC BER, SER, frame-success rate, goodput (bps), and effective spectral efficiency (bits/Hz)**. Use a fixed PRBS / seeded RNG so runs are reproducible and diffable. Crucially handle **insertion/deletion** (PLC mutes cause symbol slips), so alignment uses the preamble + frame counter, not naive index-for-index XOR — a Levenshtein/sync-aware comparator for raw streams.

### 4.2 Sweeps (the tuning workhorse)

A `dov-harness sweep` subcommand iterates the Cartesian product and emits CSV:

- **codec/mode**: `gsm-fr, amr-nb-{12.2,7.95,4.75}, (later) amr-wb-12.65, codec2`
- **FER**: `{0, 1, 3, 5, 10}%`, Gilbert–Elliott burst on/off
- **impairments**: AGC on/off, drift `{0, ±20, ±50} ppm`, tandem `{none, FR→AMR, AMR mode-hop}`, DTX on/off
- **modem params**: tone set / `f_base` / `Δ` / symbol length / M / codebook size

Primary tuning artifact: the **SER-vs-codec-rate staircase** plot (choose M and symbol length from it) and a **per-tone/per-transition survival heatmap** (which frequencies and which *frequency transitions* survive — the thing the current steady-tone table misses).

### 4.3 Inspection & visualization

- **Spectrograms**: keep the existing WAV dump; drive `ffmpeg -lavfi showspectrumpic` (already scripted in `main.rs`) for `tx_in`, `rx_out`, and `rx_out − tx_in` residual, per codec. Add a side-by-side input/output panel per experiment so transition smearing is visible.
- **Per-call calibration view**: dump the measured received-tone-center offsets (codec pulls tone centers, e.g. ~40 Hz) so the demod's frequency map is data-driven, not assumed.
- **Confusion matrix dump** for the SLS stage (post-vocoder symbol distances) as a heatmap PNG.
- A `report` subcommand renders CSV → markdown tables (and optionally PNG via a tiny plotter or gnuplot/ffmpeg) for milestone write-ups.

### 4.4 Methodology / discipline

1. **Codec-in-the-loop before anything fancy.** Always test against the real `.so`, never an idealized AWGN model.
2. **Worst-case-first**: validate on **AMR 4.75** (harshest) and **FR→AMR tandem with 5% FER**, not just MR122. If it survives the worst common case, mode-hopping is tolerable.
3. **One variable at a time** in sweeps; pin a seed; archive CSV + PNG under `/artifacts` (already git-ignored).
4. **Soft before FEC**: get demod soft outputs working, then size RS(n,k) to the *measured* per-codec SER, then add interleaving spanning many 20 ms frames so one PLC-muted frame spreads across codewords.
5. Treat the channel as **non-additive and memoryful**: report results as distributions over many random payloads, not single-vector numbers.

---

## 5. Critical pitfalls to handle from day one

1. **Phase is gone — do not encode in it.** No coherent PSK/QAM; no carrier-phase, no absolute symbol-edge dependence. Differential frequency only (DPSK is the *only* phase-bearing exception, and not for v1).
2. **Absolute amplitude is unreliable (AGC/ALC + gain quantization).** Never put bits in absolute or even relative amplitude across frames. Energy-normalize at the RX before any decision.
3. **VAD/DTX can delete your signal.** Make symbols read as *active* (pitched/tonal/periodic, in-band, adequate energy) — *active*, not necessarily *voiced* (verification correction: unvoiced and tonal also pass). Pulse the envelope on a ~0.5–1 s schedule; insert scheduled known-erasure "breath" frames to postpone DTX; negotiate DTX off where possible. A steady, low-energy, non-pitched tone risks being declared silence → comfort noise → data loss.
4. **Frame erasure + PLC inserts AND deletes symbols.** After several lost frames the decoder *mutes*; this is not just bit flips — it is symbol insertion/deletion. Use preamble + 16-bit frame counter resync and insertion/deletion-tolerant comparison; size FEC (RS with **erasures**, not just random-error correction) + interleaving accordingly.
5. **Clock/sample-rate drift.** Never assume a fixed sample clock; run continuous timing recovery and periodic preamble resync. Test at ±20/±50 ppm from day one.
6. **Sample format / endianness / range.** Everything is **i16, little-endian, 8 kHz mono, 160-sample frames** (16 kHz / 320 for WB). libgsm wants 13-bit-range linear PCM; clamp, don't wrap, on AGC. Keep the existing dependency-free WAV writer's contract (LE PCM-16). Zero-pad to whole frames (already done) and track the true length for BER alignment.
7. **Codec/mode changes mid-call.** Do not tune the modem to one codec or AMR mode. Assume mode hopping; validate on the harshest mode and on tandem chains.
8. **`MR122 ≠ EFR` for DTX/SID.** MR122 approximates EFR's *speech* core (interoperable, not bit-exact). Do **not** rely on it for EFR comfort-noise/SID behavior; if a milestone touches DTX/SID, treat NB-AMR CNG and EFR CNG as different paradigms.
9. **Steady-tone survival ≠ modem survival.** The flat single-tone table is a trap; gate all design decisions on *modulated-stream* SER/BER, which captures transition smearing and codec memory.
10. **Tone/echo-canceller detectors.** Avoid colliding with DTMF tone pairs; don't rely on a 2100 Hz echo-canceller-disable tone propagating through a transcoded cellular path. Keep tones in the clean mid-band.
11. **Out-of-band energy is wasted.** Bandpass removes <300 / >3400 Hz (NB); place all symbols in 500–2500 Hz and budget for ~40 Hz codec-induced tone-center pull (calibrate per call).

---

## 6. Ordered first milestones (small, runnable increments)

Each milestone is a single `cargo run` that produces an artifact you can look at. The repo is already at the end of M0.

- **M0 — Channel visibility (DONE).** `dov-codec` FFI + `Codec` trait + `dov-probe` tone-survival table + chirp spectrograms build and run. *Already in repo.* ✔
- **M1 — End-to-end BER skeleton + reframed metric.** Create `dov-modem` with **8-FSK @ 20 ms** (Goertzel bank) and a no-FEC `dov-frame` stub (preamble + raw bytes). Add `dov-harness run`: PRBS → modulate → `GsmFr`/`AmrNb` → demodulate → **raw BER + per-transition survival heatmap**. *Deliverable:* the first real BER number per codec, and proof that transition distortion ≫ steady-tone loss.
- **M2 — Codec-agnostic robust modem.** Implement continuous-differential 2/4-FSK ("IncDec") with phase continuity, run-length-limiting transcode, raised-cosine symbol edges, preamble correlation + frame-clock lock. Target Hermes-class ~1.2 kbps, low BER on MR122/FR. *Deliverable:* BER table across `{gsm-fr, amr 12.2/7.95/4.75}`.
- **M3 — Channel/impairment layer.** Stand up `dov-channel`: AGC, DC/bandpass, resample/drift (libsoxr), Gilbert–Elliott FER→`bfi` PLC, AMR mode-switch, FR→AMR tandem. Re-run M2 through impairments; produce the `{FER × mode × drift × AGC}` CSV + spectrogram residuals. *Deliverable:* robustness surface; identify what breaks first.
- **M4 — Framing + FEC + sync hardening.** Flesh out `dov-frame`: unique-word preamble, 16-bit frame counter, per-frame CRC, RS(n,k)-with-erasures sized to M3's measured SER, block interleaver across frames; scheduled VAD-defeat envelope pulse + known-erasure frames. *Deliverable:* post-FEC BER and frame-success under 5% FER + AGC + drift; insertion/deletion-tolerant goodput number.
- **M5 — Sweep + report tooling.** `dov-harness sweep` (Cartesian product → CSV) + `report` (CSV → markdown/PNG, including the SER-vs-rate staircase and confusion heatmaps). *Deliverable:* one-command reproduction of the full robustness matrix.
- **M6 — Trained SLS codebook (higher-rate mode).** `dov-modem::sls`: candidate generation → pass through real codec → post-vocoder distance matrix → greedy capacity-max selection (M=16→64, 5 ms), σ-weighted matched-filter demod with preamble training. Per-codec codebooks + mode-aware switching. *Deliverable:* SER staircase and a 2–3 kbps-raw result on AMR-NB vs the MFSK baseline.
- **M7 (stretch) — AMR-WB leg + Codec2 reference.** Add `AmrWb` (vo-amrwbenc enc + opencore-amrwb dec, 16 kHz/320) and `Codec2` to the trait; re-run M5 sweeps for the wideband path (~1.6–4 kbps target) keeping the same phase-agnostic, VAD-friendly design.

Defer entirely (out of this milestone): real radios/phones, acoustic coupling, Reticulum integration, EVS/Opus paths, encryption layering.
