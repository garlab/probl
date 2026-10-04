//! One integer type with inline small values and shared, canonical large values.
//! The hard size ceiling bounds parsing and individual allocations even before
//! the runtime has a work budget. Hosts can impose a smaller runtime ceiling.

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};
use rustc_hash::FxHasher;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::Arc;

pub const MAX_INTEGER_BITS: u64 = 65_536;
// ceil(MAX_INTEGER_BITS * log10(2)); check bits after parsing the last digit.
pub const MAX_INTEGER_DIGITS: usize = 19_729;

#[derive(Clone)]
pub struct Integer(Repr);

#[derive(Clone)]
enum Repr {
    Small(i64),
    Large(Arc<Large>),
}

struct Large {
    value: BigInt,
    hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntError {
    Invalid,
    TooLarge,
    DivisionByZero,
}

impl fmt::Display for IntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => f.write_str("invalid integer"),
            Self::TooLarge => write!(f, "integer size exceeds the limit of {MAX_INTEGER_BITS} bits"),
            Self::DivisionByZero => f.write_str("division by zero"),
        }
    }
}

impl Integer {
    pub const ZERO: Self = Self(Repr::Small(0));
    pub const ONE: Self = Self(Repr::Small(1));

    /// Parse unsigned binary or hexadecimal digits (no prefix or separators).
    /// Check the significant bit count before allocating a bigint. Decimal
    /// parsing remains separate so data files keep their decimal-only contract.
    pub fn from_radix_digits(digits: &str, radix: u32) -> Result<Self, IntError> {
        let valid = match radix {
            2 => digits.bytes().all(|b| matches!(b, b'0' | b'1')),
            16 => digits.bytes().all(|b| b.is_ascii_hexdigit()),
            _ => false,
        };
        if digits.is_empty() || !valid {
            return Err(IntError::Invalid);
        }
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Ok(Self::ZERO);
        }
        let first = (digits.as_bytes()[0] as char).to_digit(radix).expect("validated digit");
        let bits = (digits.len() as u64 - 1)
            .saturating_mul(u64::from(radix.trailing_zeros()))
            .saturating_add(u64::from(32 - first.leading_zeros()));
        if bits > MAX_INTEGER_BITS {
            return Err(IntError::TooLarge);
        }
        if let Ok(n) = i64::from_str_radix(digits, radix) {
            return Ok(n.into());
        }
        Self::from_big(BigInt::parse_bytes(digits.as_bytes(), radix).ok_or(IntError::Invalid)?)
    }

    pub fn from_big(value: BigInt) -> Result<Self, IntError> {
        if let Some(n) = value.to_i64() {
            return Ok(n.into());
        }
        if value.bits() > MAX_INTEGER_BITS {
            return Err(IntError::TooLarge);
        }
        let mut h = FxHasher::default();
        value.hash(&mut h);
        Ok(Self(Repr::Large(Arc::new(Large {
            value,
            hash: h.finish(),
        }))))
    }

    pub fn big(&self) -> Cow<'_, BigInt> {
        match &self.0 {
            Repr::Small(n) => Cow::Owned(BigInt::from(*n)),
            Repr::Large(n) => Cow::Borrowed(&n.value),
        }
    }

    pub fn to_i64(&self) -> Option<i64> {
        match self.0 {
            Repr::Small(n) => Some(n),
            _ => None,
        }
    }
    pub fn to_u64(&self) -> Option<u64> {
        match &self.0 {
            Repr::Small(n) => u64::try_from(*n).ok(),
            Repr::Large(n) => n.value.to_u64(),
        }
    }
    pub fn to_u128(&self) -> Option<u128> {
        match &self.0 {
            Repr::Small(n) => u128::try_from(*n).ok(),
            Repr::Large(n) => n.value.to_u128(),
        }
    }
    pub fn to_f64(&self) -> Option<f64> {
        match &self.0 {
            Repr::Small(n) => Some(*n as f64),
            Repr::Large(n) => n.value.to_f64().filter(|x| x.is_finite()),
        }
    }
    pub fn from_f64(x: f64) -> Option<Self> {
        if x >= i64::MIN as f64 && x < -(i64::MIN as f64) {
            Some((x as i64).into())
        } else {
            BigInt::from_f64(x).and_then(|n| Self::from_big(n).ok())
        }
    }
    pub fn bits(&self) -> u64 {
        match &self.0 {
            Repr::Small(n) => 64 - u64::from(n.unsigned_abs().leading_zeros()),
            Repr::Large(n) => n.value.bits(),
        }
    }
    /// Count set bits in the magnitude, ignoring the sign.
    pub fn bit_count(&self) -> u64 {
        match &self.0 {
            Repr::Small(n) => u64::from(n.unsigned_abs().count_ones()),
            Repr::Large(n) => n.value.magnitude().count_ones(),
        }
    }
    /// Signed bit operations use infinite two's-complement sign extension.
    pub fn bit_and(&self, rhs: &Self) -> Result<Self, IntError> {
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            return Ok((a & b).into());
        }
        Self::from_big(self.big().as_ref() & rhs.big().as_ref())
    }
    pub fn bit_or(&self, rhs: &Self) -> Result<Self, IntError> {
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            return Ok((a | b).into());
        }
        Self::from_big(self.big().as_ref() | rhs.big().as_ref())
    }
    pub fn bit_xor(&self, rhs: &Self) -> Result<Self, IntError> {
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            return Ok((a ^ b).into());
        }
        Self::from_big(self.big().as_ref() ^ rhs.big().as_ref())
    }
    pub fn bit_not(&self) -> Result<Self, IntError> {
        if let Some(n) = self.to_i64() {
            return Ok((!n).into());
        }
        Self::from_big(!self.big().as_ref())
    }
    pub fn is_negative(&self) -> bool {
        match &self.0 {
            Repr::Small(n) => *n < 0,
            Repr::Large(n) => n.value.is_negative(),
        }
    }
    pub fn is_zero(&self) -> bool {
        matches!(self.0, Repr::Small(0))
    }
    pub fn is_odd(&self) -> bool {
        match &self.0 {
            Repr::Small(n) => n & 1 != 0,
            Repr::Large(n) => n.value.bit(0),
        }
    }
    pub fn magnitude_bit(&self, i: u64) -> bool {
        match &self.0 {
            Repr::Small(n) => i < 64 && (n.unsigned_abs() >> i) & 1 != 0,
            Repr::Large(n) => n.value.magnitude().bit(i),
        }
    }
    pub fn negated(&self) -> Self {
        if let Some(n) = self.to_i64().and_then(i64::checked_neg) {
            return n.into();
        }
        Self::from_big(-self.big().as_ref()).expect("negation preserves magnitude")
    }
    pub fn abs(&self) -> Self {
        if self.is_negative() {
            self.negated()
        } else {
            self.clone()
        }
    }

    pub fn add(&self, rhs: &Self) -> Result<Self, IntError> {
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            if let Some(n) = a.checked_add(b) {
                return Ok(n.into());
            }
        }
        Self::from_big(self.big().as_ref() + rhs.big().as_ref())
    }
    pub fn sub(&self, rhs: &Self) -> Result<Self, IntError> {
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            if let Some(n) = a.checked_sub(b) {
                return Ok(n.into());
            }
        }
        Self::from_big(self.big().as_ref() - rhs.big().as_ref())
    }
    pub fn mul(&self, rhs: &Self) -> Result<Self, IntError> {
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            if let Some(n) = a.checked_mul(b) {
                return Ok(n.into());
            }
        }
        if !self.is_zero() && !rhs.is_zero() && self.bits() + rhs.bits() - 1 > MAX_INTEGER_BITS {
            return Err(IntError::TooLarge);
        }
        Self::from_big(self.big().as_ref() * rhs.big().as_ref())
    }
    pub fn div_mod(&self, rhs: &Self) -> Result<(Self, Self), IntError> {
        if rhs.is_zero() {
            return Err(IntError::DivisionByZero);
        }
        if let (Some(a), Some(b)) = (self.to_i64(), rhs.to_i64()) {
            if let Some(mut q) = a.checked_div(b) {
                let mut r = a % b;
                if r != 0 && (r < 0) != (b < 0) {
                    q -= 1;
                    r += b;
                }
                return Ok((q.into(), r.into()));
            }
        }
        let (a, b) = (self.big(), rhs.big());
        let mut q = a.as_ref() / b.as_ref();
        let mut r = a.as_ref() % b.as_ref();
        if !r.is_zero() && r.is_negative() != b.is_negative() {
            q -= 1;
            r += b.as_ref();
        }
        Ok((Self::from_big(q)?, Self::from_big(r)?))
    }
    pub fn pow(&self, exponent: u32) -> Result<Self, IntError> {
        if let Some(n) = self.to_i64().and_then(|n| n.checked_pow(exponent)) {
            return Ok(n.into());
        }
        if self.bits().saturating_sub(1).saturating_mul(u64::from(exponent)) >= MAX_INTEGER_BITS {
            return Err(IntError::TooLarge);
        }
        Self::from_big(self.big().pow(exponent))
    }
    /// Convert the quotient jointly: individually converting huge operands
    /// would turn a finite ratio into infinity / infinity.
    pub fn ratio(&self, denominator: &Self) -> Option<f64> {
        if denominator.is_zero() {
            return None;
        }
        if let (Some(a), Some(b)) = (self.to_i64(), denominator.to_i64()) {
            if a.unsigned_abs() <= 1 << 53 && b.unsigned_abs() <= 1 << 53 {
                return Some(a as f64 / b as f64);
            }
        }
        BigRational::new_raw(self.big().into_owned(), denominator.big().into_owned())
            .to_f64()
            .filter(|x| x.is_finite())
    }

    /// ceil(n*p) for a nonnegative n and a finite probability. The float is
    /// interpreted exactly, so rank selection works beyond float precision.
    /// The temporary product uses at most 53 extra bits above n's size.
    pub fn probability_rank(&self, p: f64) -> Result<Self, IntError> {
        if self.is_negative() || !p.is_finite() || !(0.0..=1.0).contains(&p) {
            return Err(IntError::Invalid);
        }
        let fraction = BigRational::from_float(p).ok_or(IntError::Invalid)?;
        let product = self.big().as_ref() * fraction.numer();
        let denominator = fraction.denom();
        let q = &product / denominator;
        Self::from_big(if (&product % denominator).is_zero() { q } else { q + 1 })
    }

    /// Round an integer to a power of ten, halfway away from zero. The
    /// temporary scale is bounded to at most a few bits above the value ceiling.
    pub fn round_decimal(&self, places: u32) -> Result<Self, IntError> {
        if places as usize > MAX_INTEGER_DIGITS || u64::from(places) > self.bits() * 30103 / 100000 + 1 {
            return Ok(Self::ZERO);
        }
        let scale = BigInt::from(10).pow(places);
        let magnitude = self.big().abs();
        let rounded = (magnitude + &scale / 2u32) / &scale * &scale;
        Self::from_big(if self.is_negative() { -rounded } else { rounded })
    }
    /// Compare to the exact value represented by a float, without converting
    /// the integer to a rounded float. Finite floats need at most 1024 bits.
    pub fn cmp_f64(&self, x: f64) -> Option<Ordering> {
        if x.is_nan() {
            return None;
        }
        if x == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if x == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let truncated = Self::from_f64(x).expect("finite float fits the integer ceiling");
        let cmp = self.cmp(&truncated);
        Some(if cmp.is_eq() {
            0.0f64.partial_cmp(&x.fract()).unwrap()
        } else {
            cmp
        })
    }
}

impl FromStr for Integer {
    type Err = IntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(n) = s.parse::<i64>() {
            return Ok(n.into());
        }
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return Err(IntError::Invalid);
        }
        let digits = digits.trim_start_matches('0');
        if digits.len() > MAX_INTEGER_DIGITS {
            return Err(IntError::TooLarge);
        }
        if digits.is_empty() {
            return Ok(Self::ZERO);
        }
        let n = BigInt::from_str(digits).map_err(|_| IntError::Invalid)?;
        Self::from_big(if s.starts_with('-') { -n } else { n })
    }
}
impl From<i64> for Integer {
    fn from(n: i64) -> Self {
        Self(Repr::Small(n))
    }
}
macro_rules! from_integer {
    ($($t:ty),*) => { $(impl From<$t> for Integer { fn from(n: $t) -> Self { match i64::try_from(n) { Ok(n) => Self::from(n), Err(_) => Self::from_big(BigInt::from(n)).expect("machine integer fits") } } })* };
}
impl From<i32> for Integer {
    fn from(n: i32) -> Self {
        Self::from(i64::from(n))
    }
}
impl From<u32> for Integer {
    fn from(n: u32) -> Self {
        Self::from(i64::from(n))
    }
}
from_integer!(u64, usize, i128, u128);
impl PartialEq for Integer {
    fn eq(&self, rhs: &Self) -> bool {
        self.cmp(rhs).is_eq()
    }
}
impl Eq for Integer {}
impl PartialOrd for Integer {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}
impl Ord for Integer {
    fn cmp(&self, rhs: &Self) -> Ordering {
        match (&self.0, &rhs.0) {
            (Repr::Small(a), Repr::Small(b)) => a.cmp(b),
            (Repr::Large(a), Repr::Large(b)) => a.value.cmp(&b.value),
            (Repr::Large(a), Repr::Small(_)) => {
                if a.value.is_negative() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (Repr::Small(_), Repr::Large(b)) => {
                if b.value.is_negative() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
        }
    }
}
impl PartialEq<i64> for Integer {
    fn eq(&self, rhs: &i64) -> bool {
        self.to_i64() == Some(*rhs)
    }
}
impl PartialOrd<i64> for Integer {
    fn partial_cmp(&self, rhs: &i64) -> Option<Ordering> {
        Some(self.cmp(&Self::from(*rhs)))
    }
}
impl Hash for Integer {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match &self.0 {
            Repr::Small(n) => {
                0u8.hash(h);
                n.hash(h);
            }
            Repr::Large(n) => {
                1u8.hash(h);
                n.hash.hash(h);
            }
        }
    }
}
impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Small(n) => n.fmt(f),
            Repr::Large(n) => n.value.fmt(f),
        }
    }
}
impl fmt::Debug for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_parsing_is_exact_and_checks_limits_before_allocation() {
        for bits in [0, 1, 31, 32, 53, 63, 64, 65, 127, 1024, 65535] {
            let n: BigInt = (BigInt::from(1) << bits) - 1u32;
            let expected = Integer::from_big(n.clone()).unwrap();
            for radix in [2, 16] {
                let digits = n.to_str_radix(radix);
                assert_eq!(Integer::from_radix_digits(&digits, radix).unwrap(), expected);
                assert_eq!(
                    Integer::from_radix_digits(&format!("000{digits}"), radix).unwrap(),
                    expected
                );
            }
        }
        for (radix, digits) in [(2, "1".repeat(65536)), (16, "f".repeat(16384))] {
            assert_eq!(
                Integer::from_radix_digits(&digits, radix).unwrap().bits(),
                MAX_INTEGER_BITS
            );
            assert_eq!(
                Integer::from_radix_digits(&format!("1{digits}"), radix),
                Err(IntError::TooLarge)
            );
        }
        for radix in [2, 16] {
            assert_eq!(
                Integer::from_radix_digits(&"0".repeat(100_000), radix).unwrap(),
                Integer::ZERO
            );
            for digits in ["", "_1", "1_", "-1", "+1", "1g"] {
                assert_eq!(Integer::from_radix_digits(digits, radix), Err(IntError::Invalid));
            }
        }
        assert_eq!(Integer::from_radix_digits("1", 0), Err(IntError::Invalid));
        assert!("0xff".parse::<Integer>().is_err()); // data parsing stays decimal
    }

    #[test]
    fn bit_operations_match_signed_machine_integers_across_storage_boundaries() {
        let values = [
            i128::MIN,
            i64::MIN as i128 - 1,
            i64::MIN as i128,
            -101,
            -1,
            0,
            1,
            101,
            i64::MAX as i128,
            i64::MAX as i128 + 1,
            i128::MAX,
        ];
        for a in values {
            let x = Integer::from(a);
            assert_eq!(x.bit_not().unwrap(), Integer::from(!a));
            assert_eq!(x.bit_count(), u64::from(a.unsigned_abs().count_ones()));
            for b in values {
                let y = Integer::from(b);
                assert_eq!(x.bit_and(&y).unwrap(), Integer::from(a & b));
                assert_eq!(x.bit_or(&y).unwrap(), Integer::from(a | b));
                assert_eq!(x.bit_xor(&y).unwrap(), Integer::from(a ^ b));
            }
            // Results that fit in i64 must regain the inline representation.
            assert_eq!(x.bit_xor(&x).unwrap().to_i64(), Some(0));
            assert_eq!(x.bit_or(&(-1).into()).unwrap().to_i64(), Some(-1));
        }
    }

    #[test]
    fn bit_operations_obey_the_magnitude_ceiling() {
        let max = Integer::from_radix_digits(&"f".repeat(16384), 16).unwrap();
        assert_eq!(max.bit_count(), MAX_INTEGER_BITS);
        assert_eq!(max.negated().bit_count(), MAX_INTEGER_BITS);
        assert_eq!(max.bit_not(), Err(IntError::TooLarge));
        assert_eq!(max.bit_xor(&(-1).into()), Err(IntError::TooLarge));
        assert_eq!(max.bit_and(&(-1).into()).unwrap(), max);
        assert_eq!(max.bit_or(&(-1).into()).unwrap().to_i64(), Some(-1));
        let below = max.sub(&Integer::ONE).unwrap();
        assert_eq!(max.negated().bit_not().unwrap(), below);
        assert_eq!(below.bit_not().unwrap(), max.negated());
        // AND can also require one more magnitude bit for negative operands.
        assert_eq!(max.negated().bit_and(&(-2).into()), Err(IntError::TooLarge));
    }

    #[test]
    fn signed_arithmetic_matches_wider_machine_integers() {
        let values = [
            i64::MIN as i128 - 1,
            i64::MIN as i128,
            -101,
            -1,
            0,
            1,
            101,
            i64::MAX as i128,
            i64::MAX as i128 + 1,
        ];
        for a in values {
            for b in values {
                let (x, y) = (Integer::from(a), Integer::from(b));
                assert_eq!(x.add(&y).unwrap().to_string(), (a + b).to_string());
                assert_eq!(x.sub(&y).unwrap().to_string(), (a - b).to_string());
                assert_eq!(x.mul(&y).unwrap().to_string(), (a * b).to_string());
                if b != 0 {
                    let (mut q, mut r) = (a / b, a % b);
                    if r != 0 && (r < 0) != (b < 0) {
                        q -= 1;
                        r += b;
                    }
                    let actual = x.div_mod(&y).unwrap();
                    assert_eq!(actual, (q.into(), r.into()));
                    assert_eq!(actual.0.mul(&y).unwrap().add(&actual.1).unwrap(), x);
                }
            }
        }
    }

    #[test]
    fn comparisons_agree_with_exact_rationals() {
        let integers = [
            "-10000000000000000000000000000000001",
            "-9223372036854775809",
            "-9007199254740993",
            "-1",
            "0",
            "1",
            "9007199254740993",
            "9223372036854775807",
            "10000000000000000000000000000000001",
        ];
        let mut seed = 123456789u64;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let f = f64::from_bits(seed);
            if !f.is_finite() {
                continue;
            }
            let rational = BigRational::from_float(f).unwrap();
            for text in integers {
                let n: Integer = text.parse().unwrap();
                assert_eq!(
                    n.cmp_f64(f),
                    Some(BigRational::from_integer(n.big().into_owned()).cmp(&rational)),
                    "{n}, {f}"
                );
            }
        }
    }

    #[test]
    fn parsing_and_operations_enforce_the_bit_ceiling() {
        let max = Integer::from(2).pow(65535).unwrap();
        assert_eq!(max.bits(), MAX_INTEGER_BITS);
        assert_eq!(max.to_string().parse::<Integer>().unwrap(), max);
        assert_eq!(max.mul(&2.into()), Err(IntError::TooLarge));
        assert_eq!(Integer::from(2).pow(65536), Err(IntError::TooLarge));
        assert_eq!(
            "9".repeat(MAX_INTEGER_DIGITS + 1).parse::<Integer>(),
            Err(IntError::TooLarge)
        );
        assert_eq!("0".repeat(100_000).parse::<Integer>().unwrap(), Integer::ZERO);
        assert_eq!(
            format!("-{}12", "0".repeat(100_000)).parse::<Integer>().unwrap(),
            Integer::from(-12)
        );
        assert_eq!("+".parse::<Integer>(), Err(IntError::Invalid));
    }
}
