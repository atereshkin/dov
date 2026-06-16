# dov — data over a GSM/AMR voice channel

`dov` ("data over voice") is a software modem, written in Rust, that carries
digital data through a **voice** call — i.e. through the GSM/AMR vocoders, which
are built to reproduce speech and destroy anything that isn't. The long-term aim
is a [Reticulum](https://reticulum.network/) transport over ordinary cellular
voice; this repository is the first layer of that: **the modem, plus as much
emulation on a PC as possible.** Real radios, phones, acoustic coupling, and the
Reticulum integration are deliberately out of scope here.

Everything runs against the **real** vocoder libraries (via FFI), so the
emulation exercises genuine RPE-LTP/ACELP distortion rather than an approximation
— and the codec FFI is itself cross-validated against ffmpeg (see `validate`).

## Why it's hard

A voice vocoder is not an additive-noise channel. It is a memoryful, non-linear
*analysis-by-synthesis* re-synthesiser that keeps one LPC spectral envelope, one
pitch, and per-subframe gains per 20 ms frame, and throws away fine waveform and
absolute phase. So a classic PSTN voiceband modem (V.32/V.34) does **not**
survive it. The design follows from what actually survives — see
[`docs/DESIGN.md`](docs/DESIGN.md) and the adversarially-checked
[`docs/VERIFICATION.md`](docs/VERIFICATION.md).

## What the emulation found (measured, not assumed)

- **Frequency survives; phase and absolute amplitude do not.** → non-coherent
  M-FSK with a Goertzel tone bank, never PSK/QAM; no bits in amplitude.
- **Frame alignment is decisive.** Only 20 ms-symbol (one symbol per vocoder
  frame) configs are robust on every codec. Sub-frame symbols put two spectra in
  one frame the codec can only model as one.
- **VAD/DTX is a non-issue for a continuous tone train** — it reads as *active*,
  so comfort-noise substitution never triggers. (Confirmed DTX is genuinely
  engaged.) AGC is invisible to a frequency-based modem.
- **Frame erasure is the real enemy** — a few % of lost frames raises BER by an
  order of magnitude → Reed-Solomon *with erasures* + interleaving.
- **The best rate is codec-dependent.** EFR/AMR-12.2 sustains far more than the
  coarse codecs, so the link adapts its rate to the codec it rides.

### End-to-end result (adaptive, error-free, through real codecs)

| Codec | Selected profile | Net rate (error-free after FEC) |
|---|---|---|
| AMR-12.2 (≈ EFR) | 16-FSK / 10 ms | **250 bps** |
| GSM-FR | 16-FSK / 20 ms | 125 bps |
| AMR-7.95 | 16-FSK / 20 ms | 125 bps |
| AMR-4.75 (harshest) | 4-FSK / 20 ms | 62 bps |

Error-free across clean, VAD/DTX, AGC, ±50 ppm clock drift, 20 dB AWGN, frame
erasure, and a realistic combination of all of them.

## Architecture

```
crates/
  dov-codec    FFI to the real vocoders (libgsm GSM-FR, libopencore-amrnb AMR-NB)
               behind a `Codec` trait: per-frame encode/decode, native PLC, DTX.
  dov-modem    Pure DSP: continuous-phase M-FSK modulate/demodulate, Goertzel
               tone bank, preamble acquisition + early-late clock tracking.
  dov-channel  Composable real-network impairments: VAD/DTX, AGC, Gilbert-Elliott
               frame erasure → PLC, AWGN, clock drift.
  dov-frame    FEC: Reed-Solomon errors-and-erasures over GF(256) + interleaving.
  dov-io       Live PCM audio I/O behind a trait; aplay/arecord backend (a real
               soundcard, a Bluetooth-SCO device, or a USB dongle are just
               different device strings).
  dov-harness  The emulation CLI (subcommands below) producing tables/CSV/WAVs,
               and the live-link `send`/`recv`/`selftest` (link.rs).
```

Dependencies flow one way (`harness → {codec, modem, channel, frame}`); all
`unsafe` is confined to `dov-codec`; the DSP and FEC crates are `forbid(unsafe)`.

## Build

Requires a Rust toolchain plus the codec libraries and an audio CLI:

```
# Debian/Linux:
apt install libgsm1 libopencore-amrnb0   # codecs (FFI; no -dev headers needed)
#            aplay/arecord (alsa-utils)  # live audio I/O
#            ffmpeg                       # only for the `validate` subcommand

# macOS:
brew install libgsm opencore-amr sox     # codecs + sox (play/rec) for live audio
```

```sh
cargo build --release
cargo test --workspace            # incl. ~10k RS fuzz trials, DTX/timing checks
```

The DSP/modem/FEC is portable Rust; only `dov-codec` (codec linking) and
`dov-io` (audio CLI) are platform-aware, and both Linux and macOS are supported.

## Run

```sh
cargo run --release -p dov-harness -- <subcommand>
```

| Subcommand | What it shows |
|---|---|
| `probe`    | Tone/chirp survival per codec + spectrogram WAVs (channel visibility) |
| `run`      | End-to-end 8-FSK BER + tone confusion matrix on the clean codec |
| `stress`   | The modem through each impairment — finds frame erasure is what breaks it |
| `coded`    | The full FEC + timing-recovery link, pre→post BER per impairment |
| `sync`     | Fixed-stride vs tracked demod — timing recovery fixes clock drift |
| `rate`     | Throughput-vs-survival frontier across modem configs |
| `adapt`    | Link rate adaptation: fastest error-free profile per codec |
| `validate` | Cross-check our FFI codecs against ffmpeg (GSM bit-exact; AMR 0.99+ corr) |
| `bt`       | Bluetooth-HFP tandem (CVSD around the codec): the throughput cost of bridging a call over Bluetooth |
| `selftest` | Send a text message through the whole stack (FEC + modem) and the emulated codecs — validates the live-link path with no hardware |
| `send`     | Transmit a message to a real audio device: `send "hello" [alsa-device]` |
| `recv`     | Record from a device and recover a message: `recv [seconds] [alsa-device]` |
| `loopback` | Play + record at once on one machine and recover — audio-path smoke test: `loopback [play-dev] [rec-dev]` |

Reproduce everything (writes outputs under `artifacts/`):

```sh
./scripts/reproduce.sh
```

The `stress`, `coded` subcommands also emit CSVs under `artifacts/` for plotting
with external tools (gnuplot/python).

## Sending a message over a real call

The live-link tools turn a text message into a ~16 s waveform (robust 4-FSK +
rate-½ FEC, so it survives any of the codecs) and recover it from captured audio:

```sh
# no hardware — validate the whole stack through the emulated codecs:
cargo run --release -p dov-harness -- selftest "hello over GSM"

# over a real device (a soundcard for a loopback test, a Bluetooth-SCO device
# for a bridged phone call, or the USB dongle's audio endpoint):
#   RX machine:                                  TX machine:
cargo run --release -p dov-harness -- recv 20    cargo run --release -p dov-harness -- send "hello over GSM"
# add an ALSA device as a second arg to target Bluetooth/the dongle, e.g.:
#   ... -- send "hi" bluealsa            ... -- recv 20 plughw:CARD=Dongle
```

Start `recv` first (it records for the given seconds), then fire `send` on the
other end. The receiver scans the whole capture for the preamble, so the timing
doesn't have to be exact.

To smoke-test the audio path on **one** machine, `loopback` plays and records at
the same time — you just have to close the loop somewhere:

```sh
# acoustic (zero setup): plays out the speaker, records from the mic
cargo run --release -p dov-harness -- loopback

# clean software loop (no cable, no noise) via the ALSA loopback module:
sudo modprobe snd-aloop
cargo run --release -p dov-harness -- loopback plughw:Loopback,0,0 plughw:Loopback,1,0
```

A 3.5 mm out→in cable also works (use your default devices), but `snd-aloop` is
cleaner and needs no hardware.

## Status & next

Implemented (all emulated on PC): the modem, real timing recovery, RS+interleave
FEC, the impairment layer, rate adaptation, and codec cross-validation. Open
directions: a trained speech-like-symbol codebook for higher rate on EFR; richer
sweep/report plotting; and then the deferred layers — a real audio path
(file/loopback), and Reticulum framing on top of the working link.
