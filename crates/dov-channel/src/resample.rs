//! Clock-drift via fractional resampling (linear interpolation).
//!
//! The two endpoints of a real call run off independent crystals, so the RX
//! sample clock differs from the TX one by some ppm. Over a long burst this
//! accumulates into symbol-timing slip — a direct stress on synchronisation.
//! Linear interpolation is more than accurate enough at ppm-scale ratios.

/// Resample `pcm` as if the receiver's clock were off by `ppm` parts per
/// million. The number of output samples changes by the same fraction.
pub fn drift(pcm: &[i16], ppm: f64) -> Vec<i16> {
    if ppm == 0.0 || pcm.len() < 2 {
        return pcm.to_vec();
    }
    let ratio = 1.0 + ppm * 1e-6;
    let out_len = ((pcm.len() as f64) / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for j in 0..out_len {
        let pos = j as f64 * ratio;
        let i = pos.floor() as usize;
        if i + 1 >= pcm.len() {
            out.push(pcm[pcm.len() - 1]);
            continue;
        }
        let frac = pos - i as f64;
        let v = pcm[i] as f64 * (1.0 - frac) + pcm[i + 1] as f64 * frac;
        out.push(v.round() as i16);
    }
    out
}
