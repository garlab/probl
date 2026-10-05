//! Weights: non-negative numbers with a binary mantissa and a 64-bit
//! exponent, so products of many small probabilities (long chains of
//! observations) can't underflow to zero.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Mul};

/// `mant × 2^exp`, with `mant` in [0.5, 1), or zero.
#[derive(Clone, Copy, PartialEq)]
pub struct Weight {
    mant: f64,
    exp: i64,
}

impl Weight {
    pub const ZERO: Weight = Weight { mant: 0.0, exp: 0 };
    pub const ONE: Weight = Weight { mant: 0.5, exp: 1 };

    /// A weight from a finite, non-negative number.
    pub fn new(x: f64) -> Weight {
        debug_assert!(x >= 0.0 && x.is_finite(), "invalid weight {x}");
        if x <= 0.0 || !x.is_finite() {
            return Weight::ZERO;
        }
        let (mant, exp) = frexp(x);
        Weight { mant, exp }
    }

    /// e^l: a weight from its natural logarithm, even one far outside the
    /// range of an `f64`. A logarithm of −∞ (or NaN) gives zero.
    pub fn from_ln(l: f64) -> Weight {
        if l.is_nan() || l == f64::NEG_INFINITY {
            return Weight::ZERO;
        }
        debug_assert!(l.is_finite(), "invalid log weight {l}");
        // l = k ln 2 + r with |r| ≤ ln 2 / 2, so e^l = e^r × 2^k. ln 2 is
        // split in two, the first part short enough for k ln 2 to be exact.
        const LN2_HI: f64 = 6.931_471_803_691_238e-1;
        const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
        let k = libm::round(l / std::f64::consts::LN_2);
        let r = (l - k * LN2_HI) - k * LN2_LO;
        normalize(crate::math::exp(r), k as i64)
    }

    pub fn is_zero(self) -> bool {
        self.mant == 0.0
    }

    /// The weight as an `f64`; tiny weights round to 0.
    pub fn to_f64(self) -> f64 {
        ldexp(self.mant, self.exp)
    }

    /// Multiply by a non-negative finite factor, such as a probability.
    pub fn scale(self, factor: f64) -> Weight {
        if self.is_zero() || factor <= 0.0 || !factor.is_finite() {
            return Weight::ZERO;
        }
        let (m, e) = frexp(factor);
        normalize(self.mant * m, self.exp + e)
    }

    /// `self − other`, or zero if `other` is larger.
    pub fn saturating_sub(self, other: Weight) -> Weight {
        if other >= self {
            return Weight::ZERO;
        }
        if other.is_zero() {
            return self;
        }
        let diff = self.exp - other.exp;
        if diff > 1100 {
            return self;
        }
        normalize(self.mant - ldexp(other.mant, -diff), self.exp)
    }

    /// `self / other` as an `f64` (for probabilities and shares).
    pub fn ratio(self, other: Weight) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        if other.is_zero() {
            return f64::INFINITY;
        }
        ldexp(self.mant / other.mant, self.exp - other.exp)
    }

    /// Base-10 logarithm, for printing weights too small for an `f64`.
    pub fn log10(self) -> f64 {
        if self.is_zero() {
            return f64::NEG_INFINITY;
        }
        libm::log10(self.mant) + self.exp as f64 * std::f64::consts::LOG10_2
    }

    /// The natural logarithm, even of a weight far outside the range of an
    /// `f64`: −∞ for zero.
    pub fn ln(self) -> f64 {
        if self.is_zero() {
            return f64::NEG_INFINITY;
        }
        libm::log(self.mant) + self.exp as f64 * std::f64::consts::LN_2
    }

    pub fn sum(weights: impl IntoIterator<Item = Weight>) -> Weight {
        weights.into_iter().fold(Weight::ZERO, |a, b| a + b)
    }
}

impl Default for Weight {
    fn default() -> Weight {
        Weight::ZERO
    }
}

impl Add for Weight {
    type Output = Weight;

    fn add(self, other: Weight) -> Weight {
        if self.is_zero() {
            return other;
        }
        if other.is_zero() {
            return self;
        }
        let (big, small) = if self.exp >= other.exp {
            (self, other)
        } else {
            (other, self)
        };
        let diff = big.exp - small.exp;
        if diff > 1100 {
            return big;
        }
        normalize(big.mant + ldexp(small.mant, -diff), big.exp)
    }
}

impl AddAssign for Weight {
    fn add_assign(&mut self, other: Weight) {
        *self = *self + other;
    }
}

impl Mul for Weight {
    type Output = Weight;

    fn mul(self, other: Weight) -> Weight {
        if self.is_zero() || other.is_zero() {
            return Weight::ZERO;
        }
        normalize(self.mant * other.mant, self.exp + other.exp)
    }
}

impl PartialOrd for Weight {
    fn partial_cmp(&self, other: &Weight) -> Option<Ordering> {
        Some(match (self.is_zero(), other.is_zero()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => self.exp.cmp(&other.exp).then(self.mant.total_cmp(&other.mant)),
        })
    }
}

impl fmt::Debug for Weight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let x = self.to_f64();
        if x == 0.0 && !self.is_zero() {
            let l = self.log10();
            write!(f, "{:.3}e{}", libm::pow(10.0, l - l.floor()), l.floor())
        } else {
            write!(f, "{x}")
        }
    }
}

fn normalize(m: f64, e: i64) -> Weight {
    if m == 0.0 {
        return Weight::ZERO;
    }
    let (mm, ee) = frexp(m);
    Weight { mant: mm, exp: e + ee }
}

/// Split a positive finite `x` into a mantissa in [0.5, 1) and an exponent.
fn frexp(x: f64) -> (f64, i64) {
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i64;
    if raw_exp == 0 {
        // Subnormal: scale into the normal range first.
        let (m, e) = frexp(x * 2f64.powi(64));
        return (m, e - 64);
    }
    let mant = f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52));
    (mant, raw_exp - 1022)
}

/// `m × 2^e`, saturating to 0 or infinity.
fn ldexp(m: f64, e: i64) -> f64 {
    if m == 0.0 {
        return 0.0;
    }
    if e > 2100 {
        return f64::INFINITY;
    }
    if e < -2200 {
        return 0.0;
    }
    // Two steps keep each factor representable.
    let half = e / 2;
    m * pow2(half as i32) * pow2((e - half) as i32)
}

/// 2^k, as `2f64.powi(k)` gives it, without its loop: exact from 2^-1023
/// to 2^1023, infinite above, and zero below (its intermediate 2^1024
/// overflows). A `powi` kept for the rare cases would still be called every
/// time: the compiler computes both sides of a choice between them.
fn pow2(k: i32) -> f64 {
    match k {
        1024.. => f64::INFINITY,
        -1022..=1023 => f64::from_bits(((k + 1023) as u64) << 52),
        -1023 => f64::from_bits(1 << 51),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powers_of_two_are_exact() {
        for k in -1100..=1100 {
            assert_eq!(pow2(k).to_bits(), 2f64.powi(k).to_bits(), "2^{k}");
        }
    }

    #[test]
    fn arithmetic() {
        let w = Weight::new(0.75);
        assert_eq!(w.to_f64(), 0.75);
        assert_eq!(w.scale(0.5).to_f64(), 0.375);
        assert_eq!((w + Weight::new(0.25)).to_f64(), 1.0);
        assert_eq!(Weight::ONE.to_f64(), 1.0);
        assert_eq!(Weight::new(0.3).ratio(Weight::new(0.6)), 0.5);
        assert_eq!(Weight::new(0.5).saturating_sub(Weight::new(0.25)).to_f64(), 0.25);
        assert!(Weight::new(0.5).saturating_sub(Weight::new(0.75)).is_zero());
        assert!(Weight::new(1e-300) < Weight::new(1e-200));
        assert!(Weight::ZERO < Weight::new(1e-300));
    }

    #[test]
    fn no_underflow() {
        // A thousand observations with probability 10^-5 each.
        let mut w = Weight::ONE;
        for _ in 0..1000 {
            w = w.scale(1e-5);
        }
        assert!(!w.is_zero());
        assert_eq!(w.to_f64(), 0.0);
        assert!((w.log10() + 5000.0).abs() < 1e-6);
        // Ratios of tiny weights are still exact enough.
        let a = w.scale(0.25);
        assert!((a.ratio(w) - 0.25).abs() < 1e-15);
        let b = w + w;
        assert!((b.ratio(w) - 2.0).abs() < 1e-15);
    }

    #[test]
    fn weights_from_logarithms() {
        assert_eq!(Weight::from_ln(0.0), Weight::ONE);
        assert!((Weight::from_ln(0.75f64.ln()).to_f64() - 0.75).abs() < 1e-15);
        assert!((Weight::from_ln(3.5).to_f64() - 3.5f64.exp()).abs() < 1e-13);
        assert!(Weight::from_ln(f64::NEG_INFINITY).is_zero());
        assert!(Weight::from_ln(f64::NAN).is_zero());
        // Far below an f64: e^-4234.1 = 10^-1838.8…
        let w = Weight::from_ln(-4234.102082009147);
        assert!(!w.is_zero());
        assert!((w.log10() - -4234.102082009147 / std::f64::consts::LN_10).abs() < 1e-10);
        // Multiplying adds logarithms.
        let (a, b) = (Weight::from_ln(-1000.25), Weight::from_ln(-2000.5));
        assert!(((a * b).ratio(Weight::from_ln(-3000.75)) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn subnormal_inputs() {
        let tiny = f64::MIN_POSITIVE / 1024.0;
        let w = Weight::new(tiny);
        assert!((w.to_f64() - tiny).abs() < 1e-320);
        assert!((Weight::ONE.scale(tiny).ratio(Weight::new(f64::MIN_POSITIVE)) - 1.0 / 1024.0).abs() < 1e-15);
    }
}
