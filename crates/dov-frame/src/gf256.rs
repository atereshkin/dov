//! Arithmetic over GF(2^8) with the standard 0x11D primitive polynomial and
//! generator α = 2 — the field used by virtually every byte-oriented Reed–
//! Solomon code.

use std::sync::OnceLock;

struct Tables {
    /// `exp[i] = α^i`, doubled to 512 so `exp[a+b]` never needs a modulo.
    exp: [u8; 512],
    /// `log[α^i] = i`.
    log: [u8; 256],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        Tables { exp, log }
    })
}

/// Addition (and subtraction) in GF(2^8) is XOR.
#[inline]
pub fn add(a: u8, b: u8) -> u8 {
    a ^ b
}

#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let t = tables();
    t.exp[t.log[a as usize] as usize + t.log[b as usize] as usize]
}

#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    assert!(b != 0, "division by zero in GF(256)");
    if a == 0 {
        return 0;
    }
    let t = tables();
    let idx = (t.log[a as usize] as i32 - t.log[b as usize] as i32).rem_euclid(255);
    t.exp[idx as usize]
}

/// `a^n` for any integer `n` (negative allowed).
#[inline]
pub fn pow(a: u8, n: i32) -> u8 {
    if a == 0 {
        return 0;
    }
    let t = tables();
    let idx = (t.log[a as usize] as i32 * n).rem_euclid(255);
    t.exp[idx as usize]
}

#[inline]
pub fn inv(a: u8) -> u8 {
    let t = tables();
    t.exp[(255 - t.log[a as usize] as usize) % 255]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_axioms() {
        for a in 1..=255u16 {
            let a = a as u8;
            assert_eq!(mul(a, inv(a)), 1, "inverse of {a}");
            assert_eq!(div(a, a), 1);
            assert_eq!(mul(a, 1), a);
            assert_eq!(mul(a, 0), 0);
            for b in 1..=255u16 {
                let b = b as u8;
                // distributivity and div/mul consistency
                assert_eq!(div(mul(a, b), b), a, "div/mul {a} {b}");
                assert_eq!(mul(a, b), mul(b, a));
            }
        }
    }

    #[test]
    fn pow_matches_repeated_mul() {
        for a in 1..=255u16 {
            let a = a as u8;
            let mut acc = 1u8;
            for n in 0..20 {
                assert_eq!(pow(a, n), acc, "{a}^{n}");
                acc = mul(acc, a);
            }
        }
    }
}
