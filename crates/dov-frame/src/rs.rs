//! Reed–Solomon over GF(256) with combined error-and-erasure decoding.
//!
//! Systematic encoder; Berlekamp–Massey + Forney-syndrome errata decoder. A
//! codeword of `n` bytes carries `k` message bytes and `nsym = n - k` parity
//! bytes, and corrects any `e` unknown errors plus `s` known erasures so long
//! as `2e + s ≤ nsym`. Knowing erasure positions (which the modem supplies from
//! its confidence margin) therefore doubles the correcting power per parity byte.
//!
//! Convention: first consecutive root = α^0 (fcr = 0), generator α = 2. The port
//! follows the widely used "Reed–Solomon for coders" algorithm; correctness is
//! pinned down by the randomized property tests at the bottom rather than by
//! matching any particular library byte-for-byte.

use crate::gf256 as gf;

/// Decoding failed: more errors/erasures than the code can correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsError;

// ---------------------------------------------------------------------------
// polynomial helpers (big-endian: index 0 is the highest-order coefficient)
// ---------------------------------------------------------------------------

fn poly_mul(p: &[u8], q: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; p.len() + q.len() - 1];
    for (i, &pi) in p.iter().enumerate() {
        if pi != 0 {
            for (j, &qj) in q.iter().enumerate() {
                r[i + j] ^= gf::mul(pi, qj);
            }
        }
    }
    r
}

fn poly_add(p: &[u8], q: &[u8]) -> Vec<u8> {
    let n = p.len().max(q.len());
    let mut r = vec![0u8; n];
    for (i, &pi) in p.iter().enumerate() {
        r[i + n - p.len()] = pi;
    }
    for (i, &qi) in q.iter().enumerate() {
        r[i + n - q.len()] ^= qi;
    }
    r
}

fn poly_scale(p: &[u8], x: u8) -> Vec<u8> {
    p.iter().map(|&c| gf::mul(c, x)).collect()
}

/// Horner evaluation of `p` at `x`.
fn poly_eval(p: &[u8], x: u8) -> u8 {
    let mut y = p[0];
    for &c in &p[1..] {
        y = gf::mul(y, x) ^ c;
    }
    y
}

fn rev(p: &[u8]) -> Vec<u8> {
    p.iter().rev().copied().collect()
}

// ---------------------------------------------------------------------------
// encoder
// ---------------------------------------------------------------------------

fn generator_poly(nsym: usize) -> Vec<u8> {
    let mut g = vec![1u8];
    for i in 0..nsym {
        g = poly_mul(&g, &[1, gf::pow(2, i as i32)]);
    }
    g
}

/// Systematic encode: returns the `msg.len() + nsym`-byte codeword.
pub fn encode(msg: &[u8], nsym: usize) -> Vec<u8> {
    let gen = generator_poly(nsym);
    let mut out = vec![0u8; msg.len() + nsym];
    out[..msg.len()].copy_from_slice(msg);
    for i in 0..msg.len() {
        let coef = out[i];
        if coef != 0 {
            for j in 1..gen.len() {
                out[i + j] ^= gf::mul(gen[j], coef);
            }
        }
    }
    out[..msg.len()].copy_from_slice(msg);
    out
}

// ---------------------------------------------------------------------------
// decoder
// ---------------------------------------------------------------------------

fn calc_syndromes(msg: &[u8], nsym: usize) -> Vec<u8> {
    // Leading 0 placeholder, matching the reference algorithm's indexing.
    let mut s = vec![0u8; nsym + 1];
    for i in 0..nsym {
        s[i + 1] = poly_eval(msg, gf::pow(2, i as i32));
    }
    s
}

/// Errata locator polynomial from coefficient positions (counted from the right).
fn find_errata_locator(coef_pos: &[usize]) -> Vec<u8> {
    let mut e_loc = vec![1u8];
    for &p in coef_pos {
        e_loc = poly_mul(&e_loc, &poly_add(&[1], &[gf::pow(2, p as i32), 0]));
    }
    e_loc
}

/// Error evaluator Ω(x) = S(x)·Λ(x) mod x^nsym.
fn find_error_evaluator(synd: &[u8], err_loc: &[u8], nsym: usize) -> Vec<u8> {
    let prod = poly_mul(synd, err_loc);
    let start = prod.len().saturating_sub(nsym + 1);
    prod[start..].to_vec()
}

/// Forney: compute error magnitudes at the given positions and correct.
fn correct_errata(msg: &[u8], synd: &[u8], err_pos: &[usize]) -> Vec<u8> {
    let n = msg.len();
    let coef_pos: Vec<usize> = err_pos.iter().map(|&p| n - 1 - p).collect();
    let err_loc = find_errata_locator(&coef_pos);

    let synd_rev = rev(synd);
    let err_eval_rev = find_error_evaluator(&synd_rev, &err_loc, err_loc.len() - 1);
    let err_eval = rev(&err_eval_rev);

    // X_i = α^{coef_pos_i}
    let xs: Vec<u8> = coef_pos.iter().map(|&p| gf::pow(2, p as i32)).collect();

    let mut e = vec![0u8; n];
    for (i, &xi) in xs.iter().enumerate() {
        let xi_inv = gf::inv(xi);

        // Formal derivative of the locator at X_i^{-1}, via the product form.
        let mut err_loc_prime = 1u8;
        for (j, &xj) in xs.iter().enumerate() {
            if j != i {
                err_loc_prime = gf::mul(err_loc_prime, gf::add(1, gf::mul(xi_inv, xj)));
            }
        }
        if err_loc_prime == 0 {
            // Degenerate; treat as a failure upstream via syndrome recheck.
            continue;
        }

        let mut y = poly_eval(&rev(&err_eval), xi_inv);
        y = gf::mul(gf::pow(xi, 1), y); // (1 - fcr) = 1
        e[err_pos[i]] = gf::div(y, err_loc_prime);
    }
    poly_add(msg, &e)
}

/// Berlekamp–Massey for the error locator, seeded with the erasure locator.
fn find_error_locator(synd: &[u8], nsym: usize, erase_count: usize) -> Result<Vec<u8>, RsError> {
    let mut err_loc = vec![1u8];
    let mut old_loc = vec![1u8];

    let synd_shift = synd.len().saturating_sub(nsym);
    let iters = nsym.saturating_sub(erase_count);

    for i in 0..iters {
        let k = i + synd_shift;
        let mut delta = synd[k];
        for j in 1..err_loc.len() {
            delta ^= gf::mul(err_loc[err_loc.len() - 1 - j], synd[k - j]);
        }
        old_loc.push(0);

        if delta != 0 {
            if old_loc.len() > err_loc.len() {
                let new_loc = poly_scale(&old_loc, delta);
                old_loc = poly_scale(&err_loc, gf::inv(delta));
                err_loc = new_loc;
            }
            err_loc = poly_add(&err_loc, &poly_scale(&old_loc, delta));
        }
    }

    // Strip leading zeros.
    while err_loc.first() == Some(&0) {
        err_loc.remove(0);
    }
    if err_loc.is_empty() {
        return Err(RsError);
    }
    let errs = err_loc.len() - 1;
    // Signed: `errs` can be below `erase_count` during normal operation.
    if 2 * (errs as i32 - erase_count as i32) + erase_count as i32 > nsym as i32 {
        return Err(RsError); // too many errors
    }
    Ok(err_loc)
}

/// Chien search: roots of the locator give the error positions (from the left).
fn find_errors(err_loc: &[u8], n: usize) -> Result<Vec<usize>, RsError> {
    let errs = err_loc.len() - 1;
    let mut positions = Vec::new();
    for i in 0..n {
        if poly_eval(err_loc, gf::pow(2, i as i32)) == 0 {
            positions.push(n - 1 - i);
        }
    }
    if positions.len() != errs {
        return Err(RsError);
    }
    Ok(positions)
}

/// Forney syndromes: fold the known erasures out of the syndrome sequence.
fn forney_syndromes(synd: &[u8], erase_pos: &[usize], n: usize) -> Vec<u8> {
    let mut fsynd: Vec<u8> = synd[1..].to_vec(); // drop the placeholder
    for &p in erase_pos {
        let x = gf::pow(2, (n - 1 - p) as i32);
        for j in 0..fsynd.len() - 1 {
            fsynd[j] = gf::mul(fsynd[j], x) ^ fsynd[j + 1];
        }
    }
    fsynd
}

/// Decode an `n`-byte codeword with the given erasure positions (indices into
/// the codeword). Returns the corrected `k = n - nsym` message bytes.
pub fn decode(codeword: &[u8], nsym: usize, erase_pos: &[usize]) -> Result<Vec<u8>, RsError> {
    if erase_pos.len() > nsym {
        return Err(RsError);
    }
    let n = codeword.len();
    let mut msg = codeword.to_vec();
    for &e in erase_pos {
        if e >= n {
            return Err(RsError);
        }
        msg[e] = 0;
    }

    let synd = calc_syndromes(&msg, nsym);
    if synd.iter().all(|&s| s == 0) {
        return Ok(msg[..n - nsym].to_vec()); // already clean
    }

    let fsynd = forney_syndromes(&synd, erase_pos, n);
    let err_loc = find_error_locator(&fsynd, nsym, erase_pos.len())?;
    let err_positions = find_errors(&rev(&err_loc), n)?;

    let mut all_pos = erase_pos.to_vec();
    all_pos.extend_from_slice(&err_positions);

    let corrected = correct_errata(&msg, &synd, &all_pos);

    // Verify: a correct decode has all-zero syndromes.
    let check = calc_syndromes(&corrected, nsym);
    if check.iter().any(|&s| s != 0) {
        return Err(RsError);
    }
    Ok(corrected[..n - nsym].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // tiny deterministic LCG for reproducible fuzzing
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 16
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn byte(&mut self) -> u8 {
            self.next() as u8
        }
    }

    #[test]
    fn clean_roundtrip() {
        let nsym = 16;
        let msg: Vec<u8> = (0..40).map(|i| (i * 7 + 1) as u8).collect();
        let cw = encode(&msg, nsym);
        assert_eq!(decode(&cw, nsym, &[]).unwrap(), msg);
    }

    #[test]
    fn corrects_errors_only() {
        let nsym = 16; // t = 8 random errors
        let k = 40;
        let mut rng = Lcg(1);
        for _ in 0..3000 {
            let msg: Vec<u8> = (0..k).map(|_| rng.byte()).collect();
            let mut cw = encode(&msg, nsym);
            let n = cw.len();
            let e = rng.below(nsym / 2 + 1); // 0..=8 errors
            let mut used = std::collections::HashSet::new();
            for _ in 0..e {
                let mut p = rng.below(n);
                while !used.insert(p) {
                    p = rng.below(n);
                }
                cw[p] ^= rng.byte() | 1; // ensure a real change
            }
            assert_eq!(decode(&cw, nsym, &[]).unwrap(), msg, "e={e}");
        }
    }

    #[test]
    fn corrects_errors_and_erasures() {
        let nsym = 16;
        let k = 40;
        let mut rng = Lcg(42);
        for _ in 0..5000 {
            let msg: Vec<u8> = (0..k).map(|_| rng.byte()).collect();
            let mut cw = encode(&msg, nsym);
            let n = cw.len();

            // pick s erasures and e errors with 2e + s <= nsym
            let s = rng.below(nsym + 1);
            let max_e = (nsym - s) / 2;
            let e = if max_e == 0 { 0 } else { rng.below(max_e + 1) };

            let mut used = std::collections::HashSet::new();
            let mut erase_pos = Vec::new();
            for _ in 0..s {
                let mut p = rng.below(n);
                while !used.insert(p) {
                    p = rng.below(n);
                }
                erase_pos.push(p);
                cw[p] ^= rng.byte(); // erased symbols may take any value
            }
            for _ in 0..e {
                let mut p = rng.below(n);
                while !used.insert(p) {
                    p = rng.below(n);
                }
                cw[p] ^= rng.byte() | 1;
            }
            let got = decode(&cw, nsym, &erase_pos);
            assert_eq!(got.as_deref(), Ok(&msg[..]), "s={s} e={e}");
        }
    }

    #[test]
    fn detects_beyond_capacity() {
        // With far too many errors the decoder must report failure (or, very
        // rarely, decode to a different valid codeword) — never panic.
        let nsym = 16;
        let k = 40;
        let mut rng = Lcg(7);
        let mut clean_failures = 0;
        for _ in 0..2000 {
            let msg: Vec<u8> = (0..k).map(|_| rng.byte()).collect();
            let mut cw = encode(&msg, nsym);
            let n = cw.len();
            // 12 errors >> t=8: uncorrectable
            let mut used = std::collections::HashSet::new();
            for _ in 0..12 {
                let mut p = rng.below(n);
                while !used.insert(p) {
                    p = rng.below(n);
                }
                cw[p] ^= rng.byte() | 1;
            }
            match decode(&cw, nsym, &[]) {
                Err(_) => clean_failures += 1,
                Ok(m) => assert_ne!(m, msg, "should not have recovered original"),
            }
        }
        // The vast majority must be flagged as failures, not silently wrong.
        assert!(clean_failures > 1900, "only {clean_failures}/2000 flagged");
    }
}
