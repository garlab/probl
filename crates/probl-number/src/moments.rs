//! Exact weighted moments of integer coordinates. Binary64 weights are treated
//! as their exact dyadic values, so a large origin never erases small offsets.
use crate::Integer;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};

#[derive(Default)]
pub struct IntegerMoments {
    weight: BigInt,
    first: BigInt,
    second: BigInt,
    exponent: i32,
    count: u64,
}

impl IntegerMoments {
    pub fn integral_mean(&self) -> Option<Integer> {
        if self.weight.is_zero() || !(&self.first % &self.weight).is_zero() {
            return None;
        }
        Integer::from_big(&self.first / &self.weight).ok()
    }

    pub fn fractional_mean(&self) -> Option<f64> {
        if self.weight.is_zero() {
            return None;
        }
        BigRational::new_raw(self.first.clone(), self.weight.clone()).to_f64()
    }

    /// Conservative limb work for the final products, quotient and conversion.
    pub fn finish_work(&self) -> u64 {
        let limbs = self
            .second
            .bits()
            .max(self.first.bits())
            .max(self.weight.bits())
            .div_ceil(64)
            .max(1);
        16 + 8 * limbs * limbs
    }
    /// Add an integer coordinate with finite, nonnegative weight. Zero weight
    /// does not contribute.
    pub fn push(&mut self, value: i128, weight: f64) -> bool {
        self.push_integer(&Integer::from(value), weight)
    }

    /// With at most 2^64 observations and bounded Integer coordinates, scratch
    /// integers stay below twice MAX_INTEGER_BITS plus 5,000 bits. The caller
    /// accounts for multiplication work before adding each coordinate.
    pub fn push_integer(&mut self, value: &Integer, weight: f64) -> bool {
        if !weight.is_finite() || weight < 0.0 {
            return false;
        }
        if weight == 0.0 {
            return true;
        }
        let Some(count) = self.count.checked_add(1) else {
            return false;
        };
        let bits = weight.to_bits();
        let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
        let mut mantissa = bits & ((1 << 52) - 1);
        let mut exponent = -1074;
        if exponent_bits != 0 {
            mantissa |= 1 << 52;
            exponent = exponent_bits - 1023 - 52;
        }
        let zeroes = mantissa.trailing_zeros();
        mantissa >>= zeroes;
        exponent += zeroes as i32;
        if self.weight.is_zero() {
            self.exponent = exponent;
        } else if exponent < self.exponent {
            let shift = (self.exponent - exponent) as usize;
            self.weight <<= shift;
            self.first <<= shift;
            self.second <<= shift;
            self.exponent = exponent;
        }
        let weight = BigInt::from(mantissa) << (exponent - self.exponent) as usize;
        let value = value.big();
        let first = &weight * value.as_ref();
        self.second += &first * value.as_ref();
        self.first += first;
        self.weight += weight;
        self.count = count;
        true
    }

    /// Nearest integer coordinate, choosing the lower one at an exact half tie.
    pub fn rounded_mean(&self) -> Option<i128> {
        if self.weight.is_zero() {
            return None;
        }
        let mut whole = &self.first / &self.weight;
        let mut remainder = &self.first % &self.weight;
        if remainder.is_negative() {
            whole -= 1;
            remainder += &self.weight;
        }
        if (remainder << 1usize) > self.weight {
            whole += 1;
        }
        whole.to_i128()
    }

    /// Population variance as `mantissa * 2^exponent`, with an even exponent.
    /// Computing the numerator exactly avoids cancellation. Keeping the scale
    /// separate lets callers take a square root without underflowing variance.
    pub fn variance_parts(&self) -> Option<(f64, i32)> {
        if self.weight.is_zero() {
            return None;
        }
        let mut numerator = &self.second * &self.weight - &self.first * &self.first;
        if numerator.is_zero() {
            return Some((0.0, 0));
        }
        let mut denominator = &self.weight * &self.weight;
        let exponent = (numerator.bits() as i32 - denominator.bits() as i32).div_euclid(2) * 2;
        if exponent >= 0 {
            denominator <<= exponent as usize;
        } else {
            numerator <<= (-exponent) as usize;
        }
        BigRational::new_raw(numerator, denominator)
            .to_f64()
            .map(|mantissa| (mantissa, exponent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn means_and_variances_ignore_origin_order_and_weight_scale() {
        for origin in [0, -1, i128::MIN, i128::MAX - 3] {
            for weight in [f64::from_bits(1), 0.5, 1.0, f64::MAX] {
                let mut m = IntegerMoments::default();
                assert!(m.push(origin, weight));
                assert!(m.push(origin + 3, weight));
                assert_eq!(m.rounded_mean(), Some(origin + 1));
                let (v, e) = m.variance_parts().unwrap();
                assert_eq!(v * 2.0f64.powi(e), 2.25);
                let mut reverse = IntegerMoments::default();
                assert!(reverse.push(origin + 3, weight));
                assert!(reverse.push(origin, weight));
                assert_eq!(reverse.rounded_mean(), m.rounded_mean());
                assert_eq!(reverse.variance_parts(), m.variance_parts());
            }
        }
    }

    #[test]
    fn weight_precision_and_subnormal_spread_are_preserved() {
        let mut m = IntegerMoments::default();
        m.push(0, 0.5);
        m.push(1, f64::from_bits(0.5f64.to_bits() + 1));
        assert_eq!(m.rounded_mean(), Some(1));
        let mut tiny = IntegerMoments::default();
        tiny.push(0, f64::MAX);
        tiny.push(1, f64::from_bits(1));
        let (v, e) = tiny.variance_parts().unwrap();
        assert!(v > 0.0 && e < -1074 && e % 2 == 0);
        assert_eq!(tiny.rounded_mean(), Some(0));
        assert!(!tiny.push(0, f64::NAN));
        assert!(!tiny.push(0, -1.0));
        assert_eq!(IntegerMoments::default().rounded_mean(), None);
    }
}
