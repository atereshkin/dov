//! Minimal dependency-free WAV writer (mono, 16-bit PCM).

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Write `samples` as a canonical 44-byte-header mono PCM-16 WAV at `sample_rate`.
pub fn write_mono_i16(path: impl AsRef<Path>, samples: &[i16], sample_rate: u32) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);

    let bits_per_sample: u16 = 16;
    let channels: u16 = 1;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_bytes = (samples.len() * 2) as u32;

    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_bytes).to_le_bytes())?;
    w.write_all(b"WAVE")?;

    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // PCM fmt chunk size
    w.write_all(&1u16.to_le_bytes())?; // audio format = PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits_per_sample.to_le_bytes())?;

    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        w.write_all(&s.to_le_bytes())?;
    }
    w.flush()
}

/// Read a 16-bit PCM WAV as mono `i16` (stereo is downmixed to the left channel).
/// Returns the samples and the sample rate. Tolerates extra chunks and headers.
pub fn read_mono_i16(path: impl AsRef<Path>) -> io::Result<(Vec<i16>, u32)> {
    let b = std::fs::read(path)?;
    let err = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(err("not a RIFF/WAVE file"));
    }
    let u16le = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);

    let (mut channels, mut rate, mut bits) = (1u16, 8000u32, 16u16);
    let mut data: Option<(usize, usize)> = None;
    let mut p = 12;
    while p + 8 <= b.len() {
        let id = &b[p..p + 4];
        let size = u32le(p + 4) as usize;
        let body = p + 8;
        if id == b"fmt " && body + 16 <= b.len() {
            channels = u16le(body + 2);
            rate = u32le(body + 4);
            bits = u16le(body + 14);
        } else if id == b"data" {
            data = Some((body, (body + size).min(b.len())));
        }
        p = body + size + (size & 1); // chunks are word-aligned
    }
    if bits != 16 {
        return Err(err("only 16-bit PCM WAV is supported"));
    }
    let (start, end) = data.ok_or_else(|| err("no data chunk"))?;
    let ch = channels.max(1) as usize;
    let samples: Vec<i16> = b[start..end]
        .chunks_exact(2 * ch)
        .map(|frame| i16::from_le_bytes([frame[0], frame[1]])) // left channel
        .collect();
    Ok((samples, rate))
}
