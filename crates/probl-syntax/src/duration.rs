//! Unit-bearing elapsed time. Whole nanoseconds are exact integers; fractional
//! nanoseconds use finite binary64 values, as other fractional numbers do.
use probl_number::Integer;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

pub const UNITS: [(&str, u64); 8] = [
    ("w", 604_800_000_000_000),
    ("d", 86_400_000_000_000),
    ("h", 3_600_000_000_000),
    ("m", 60_000_000_000),
    ("s", 1_000_000_000),
    ("ms", 1_000_000),
    ("us", 1_000),
    ("ns", 1),
];

#[derive(Clone, Debug, PartialEq)]
enum Magnitude {
    Int(Integer),
    Fraction(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Amount(Magnitude);
impl Eq for Amount {}
impl Amount {
    pub fn integer(n: Integer) -> Self {
        Self(Magnitude::Int(n))
    }
    pub fn fractional(x: f64) -> Option<Self> {
        if !x.is_finite() {
            return None;
        }
        Some(if x.fract() == 0.0 {
            Self::integer(Integer::from_f64(x)?)
        } else {
            Self(Magnitude::Fraction(x))
        })
    }
    pub fn as_integer(&self) -> Option<&Integer> {
        match &self.0 {
            Magnitude::Int(n) => Some(n),
            _ => None,
        }
    }
    pub fn as_float(&self) -> Option<f64> {
        match &self.0 {
            Magnitude::Int(n) => n.to_f64(),
            Magnitude::Fraction(x) => Some(*x),
        }
    }
    pub fn negated(&self) -> Self {
        match &self.0 {
            Magnitude::Int(n) => Self::integer(n.negated()),
            Magnitude::Fraction(x) => Self(Magnitude::Fraction(-x)),
        }
    }
    fn exact_float(&self) -> Option<f64> {
        let f = self.as_float()?;
        self.as_integer()
            .is_none_or(|n| n.cmp_f64(f) == Some(Ordering::Equal))
            .then_some(f)
    }
    fn plus(&self, other: &Self) -> Option<Self> {
        match (self.as_integer(), other.as_integer()) {
            (Some(a), Some(b)) => a.add(b).ok().map(Self::integer),
            _ => Self::fractional(self.exact_float()? + other.exact_float()?),
        }
    }
}
impl Ord for Amount {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (Magnitude::Int(a), Magnitude::Int(b)) => a.cmp(b),
            (Magnitude::Int(a), Magnitude::Fraction(b)) => a.cmp_f64(*b).unwrap(),
            (Magnitude::Fraction(a), Magnitude::Int(b)) => b.cmp_f64(*a).unwrap().reverse(),
            (Magnitude::Fraction(a), Magnitude::Fraction(b)) => a.total_cmp(b),
        }
    }
}
impl PartialOrd for Amount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Hash for Amount {
    fn hash<H: Hasher>(&self, h: &mut H) {
        match &self.0 {
            Magnitude::Int(n) => {
                0u8.hash(h);
                n.hash(h);
            }
            Magnitude::Fraction(x) => {
                1u8.hash(h);
                x.to_bits().hash(h);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration {
    pub squared: bool,
    pub amount: Amount,
}
impl Duration {
    pub fn kind(&self) -> &'static str {
        if self.squared { "squared_duration" } else { "duration" }
    }
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() || text.trim() != text || !text.is_ascii() {
            return None;
        }
        let negative = text.starts_with('-');
        let text = text.strip_prefix('-').unwrap_or(text);
        let mut result: Option<Self> = None;
        for part in text.split(' ') {
            let squared = part.ends_with("^2");
            let part = part.strip_suffix("^2").unwrap_or(part);
            // Longer suffixes precede their common `s` suffix.
            let (unit, scale) = UNITS.iter().rev().find(|(unit, _)| part.ends_with(unit))?;
            let number = &part[..part.len() - unit.len()];
            let scale = Integer::from(*scale).pow(if squared { 2 } else { 1 }).ok()?;
            let amount = decimal(number, &scale)?;
            result = Some(match result {
                None => Self { squared, amount },
                Some(d) if d.squared == squared => Self {
                    squared,
                    amount: d.amount.plus(&amount)?,
                },
                _ => return None,
            });
        }
        let mut result = result?;
        if negative {
            result.amount = result.amount.negated();
        }
        Some(result)
    }
    pub fn format(&self) -> String {
        let suffix = if self.squared { "^2" } else { "" };
        if self.amount.as_float() == Some(0.0) {
            return format!("0s{suffix}");
        }
        for (unit, scale) in UNITS {
            let scale = Integer::from(scale).pow(if self.squared { 2 } else { 1 }).unwrap();
            let coefficient = match self.amount.as_integer() {
                Some(n) => decimal_ratio(n, &scale),
                None => self
                    .amount
                    .as_float()
                    .map(|n| (n / scale.to_f64().unwrap()).to_string()),
            };
            let Some(coefficient) = coefficient else {
                continue;
            };
            if unit != "ns" && coefficient.parse::<f64>().is_ok_and(|x| x.abs() < 0.1) {
                continue;
            }
            let candidate = format!("{coefficient}{unit}{suffix}");
            if Self::parse(&candidate).as_ref() == Some(self) {
                return candidate;
            }
        }
        // Base units always round-trip: integral amounts use their full digits;
        // fractional amounts use binary64's shortest round-tripping decimal.
        let number = self
            .amount
            .as_integer()
            .map_or_else(|| self.amount.as_float().unwrap().to_string(), ToString::to_string);
        format!("{number}ns{suffix}")
    }
}

fn decimal(text: &str, scale: &Integer) -> Option<Amount> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || (text.contains('.') && fraction.is_empty())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let fraction = fraction.trim_end_matches('0');
    let digits = format!("{whole}{fraction}");
    let numerator = digits.parse::<Integer>().ok()?.mul(scale).ok()?;
    let denominator = Integer::from(10).pow(u32::try_from(fraction.len()).ok()?).ok()?;
    let (n, r) = numerator.div_mod(&denominator).ok()?;
    if r.is_zero() {
        Some(Amount::integer(n))
    } else {
        let value = numerator.ratio(&denominator)?;
        // A nonzero literal must not silently become zero through underflow.
        (value != 0.0).then(|| Amount::fractional(value)).flatten()
    }
}

fn decimal_ratio(n: &Integer, scale: &Integer) -> Option<String> {
    let negative = n < &Integer::ZERO;
    let n = if negative { n.negated() } else { n.clone() };
    let (whole, mut remainder) = n.div_mod(scale).ok()?;
    let mut out = format!("{}{whole}", if negative { "-" } else { "" });
    if !remainder.is_zero() {
        out.push('.');
        for _ in 0..18 {
            let (digit, r) = remainder.mul(&Integer::from(10)).ok()?.div_mod(scale).ok()?;
            out.push_str(&digit.to_string());
            remainder = r;
            if remainder.is_zero() {
                return Some(out);
            }
        }
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn normalized_units_have_one_identity_and_order() {
        let values: Vec<_> = ["1m", "60s", "60000ms", "60000000000ns"]
            .map(|s| Duration::parse(s).unwrap())
            .into();
        assert_eq!(values.iter().collect::<HashSet<_>>().len(), 1);
        assert_eq!(values.iter().collect::<BTreeSet<_>>().len(), 1);
        assert!(Duration::parse("-0.5ns").unwrap() < Duration::parse("0ns").unwrap());
        assert!(Duration::parse("0ns").unwrap() < Duration::parse("0.5ns").unwrap());
        assert_eq!(Duration::parse("-0ns"), Duration::parse("0s"));
        assert_ne!(Duration::parse("1ns"), Duration::parse("1ns^2"));
        assert!(Amount::fractional(f64::INFINITY).is_none());
        assert!(Duration::parse(&format!("0.{}1ns", "0".repeat(400))).is_none());
    }

    #[test]
    fn canonical_display_round_trips_extremes_and_generated_magnitudes() {
        let mut state = 0x1234_5678_90ab_cdefu64;
        let mut magnitudes = vec![f64::from_bits(1), f64::MIN_POSITIVE, f64::MAX, -0.0, 0.5, -0.5];
        for _ in 0..512 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let x = f64::from_bits(state);
            if x.is_finite() {
                magnitudes.push(x);
            }
        }
        for x in magnitudes {
            for squared in [false, true] {
                let d = Duration {
                    squared,
                    amount: Amount::fractional(x).unwrap(),
                };
                assert_eq!(Duration::parse(&d.format()), Some(d.clone()), "{x}: {}", d.format());
            }
        }
        let huge = Integer::from(10).pow(1000).unwrap().add(&Integer::ONE).unwrap();
        let d = Duration {
            squared: true,
            amount: Amount::integer(huge),
        };
        assert_eq!(Duration::parse(&d.format()), Some(d));
    }
}
