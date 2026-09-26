//! Finite distributions: dice, choices, counts, and the results of computing
//! with them.

use crate::value::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// Outcomes below this probability are dropped from infinite supports.
const TAIL: f64 = 1e-18;
/// The most outcomes one distribution may have.
pub const MAX_OUTCOMES: usize = 2_000_000;

#[derive(Clone, Debug)]
pub struct Dist {
    /// Sorted by value, each value once, every weight positive.
    pub outcomes: Vec<(Value, f64)>,
    /// Probability not represented by `outcomes` (a tail cut off to keep the
    /// support finite). Drawing counts it as unresolved.
    pub missing: f64,
}

impl Dist {
    pub fn point(v: Value) -> Dist {
        Dist {
            outcomes: vec![(v, 1.0)],
            missing: 0.0,
        }
    }

    /// Build from (value, weight) pairs in any order; equal values are merged.
    pub fn from_pairs(mut pairs: Vec<(Value, f64)>, missing: f64) -> Dist {
        pairs.retain(|(_, w)| *w > 0.0);
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut outcomes: Vec<(Value, f64)> = Vec::with_capacity(pairs.len());
        for (v, w) in pairs {
            match outcomes.last_mut() {
                Some((last, total)) if *last == v => *total += w,
                _ => outcomes.push((v, w)),
            }
        }
        Dist { outcomes, missing }
    }

    pub fn uniform(values: Vec<Value>) -> Dist {
        let p = 1.0 / values.len() as f64;
        Dist::from_pairs(values.into_iter().map(|v| (v, p)).collect(), 0.0)
    }

    /// The sum of `count` dice with `sides` sides.
    pub fn dice(count: u32, sides: u32) -> Dist {
        let mut sums = vec![1.0];
        let p = 1.0 / sides as f64;
        for _ in 0..count {
            let mut next = vec![0.0; sums.len() + sides as usize];
            for (s, w) in sums.iter().enumerate() {
                if *w == 0.0 {
                    continue;
                }
                for face in 1..=sides as usize {
                    next[s + face] += w * p;
                }
            }
            sums = next;
        }
        let pairs = sums
            .into_iter()
            .enumerate()
            .filter(|(_, w)| *w > 0.0)
            .map(|(s, w)| (Value::Int(s as i64), w))
            .collect();
        Dist::from_pairs(pairs, 0.0)
    }

    pub fn binomial(n: u64, p: f64) -> Dist {
        if p <= 0.0 {
            return Dist::point(Value::Int(0));
        }
        if p >= 1.0 {
            return Dist::point(Value::Int(n as i64));
        }
        let (lp, lq) = (p.ln(), (1.0 - p).ln());
        let ln_n = ln_gamma(n as f64 + 1.0);
        let mut pairs = Vec::new();
        let mut kept = 0.0;
        // Start at the mode and walk outwards, stopping in each direction
        // once the probabilities become negligible.
        let mode = (((n + 1) as f64) * p).floor().min(n as f64) as u64;
        let pmf = |k: u64| {
            (ln_n - ln_gamma(k as f64 + 1.0) - ln_gamma((n - k) as f64 + 1.0) + k as f64 * lp + (n - k) as f64 * lq)
                .exp()
        };
        let mut k = mode as i64;
        while k >= 0 {
            let w = pmf(k as u64);
            if w < TAIL && (k as u64) < mode {
                break;
            }
            pairs.push((Value::Int(k), w));
            kept += w;
            k -= 1;
        }
        let mut k = mode + 1;
        while k <= n {
            let w = pmf(k);
            if w < TAIL {
                break;
            }
            pairs.push((Value::Int(k as i64), w));
            kept += w;
            k += 1;
        }
        Dist::from_pairs(pairs, (1.0 - kept).max(0.0))
    }

    pub fn poisson(rate: f64) -> Dist {
        if rate <= 0.0 {
            return Dist::point(Value::Int(0));
        }
        let mut pairs = Vec::new();
        let mut kept = 0.0;
        let ln_rate = rate.ln();
        let mut k: u64 = 0;
        loop {
            let w = (k as f64 * ln_rate - rate - ln_gamma(k as f64 + 1.0)).exp();
            if w >= TAIL || (k as f64) < rate {
                if w > 0.0 {
                    pairs.push((Value::Int(k as i64), w));
                    kept += w;
                }
            } else {
                break;
            }
            k += 1;
        }
        Dist::from_pairs(pairs, (1.0 - kept).max(0.0))
    }

    /// Number of tries up to and including the first success.
    pub fn geometric(p: f64) -> Dist {
        if p >= 1.0 {
            return Dist::point(Value::Int(1));
        }
        let mut pairs = Vec::new();
        let mut tail = 1.0;
        let mut k: i64 = 1;
        while tail >= TAIL && pairs.len() < MAX_OUTCOMES {
            pairs.push((Value::Int(k), tail * p));
            tail *= 1.0 - p;
            k += 1;
        }
        Dist::from_pairs(pairs, tail)
    }

    /// The dice of `count` rolls of `die`, sorted from highest to lowest.
    pub fn pool(count: u32, die: &Dist) -> Dist {
        let mut pools: BTreeMap<Vec<Value>, f64> = BTreeMap::new();
        pools.insert(Vec::new(), 1.0);
        for _ in 0..count {
            let mut next: BTreeMap<Vec<Value>, f64> = BTreeMap::new();
            for (pool, w) in &pools {
                for (face, p) in &die.outcomes {
                    let mut grown = pool.clone();
                    let at = grown.iter().position(|v| v < face).unwrap_or(grown.len());
                    grown.insert(at, face.clone());
                    *next.entry(grown).or_insert(0.0) += w * p;
                }
            }
            pools = next;
        }
        let pairs = pools.into_iter().map(|(pool, w)| (Value::list(pool), w)).collect();
        Dist::from_pairs(pairs, die.missing)
    }

    pub fn total(&self) -> f64 {
        self.outcomes.iter().map(|(_, w)| w).sum()
    }

    /// A plain value if only one outcome is possible, otherwise the distribution.
    pub fn into_value(self) -> Value {
        if self.outcomes.len() == 1 && self.missing == 0.0 && (self.outcomes[0].1 - 1.0).abs() < 1e-12 {
            return self.outcomes.into_iter().next().unwrap().0;
        }
        Value::Dist(std::sync::Arc::new(self))
    }

    /// If every outcome is a probability, the chance that a draw is true.
    pub fn chance(&self) -> Option<f64> {
        let mut p = 0.0;
        for (v, w) in &self.outcomes {
            match v {
                Value::Prob(q) => p += w * q,
                _ => return None,
            }
        }
        Some(p)
    }

    fn numbers(&self) -> Option<Vec<(f64, f64)>> {
        self.outcomes.iter().map(|(v, w)| v.as_f64().map(|x| (x, *w))).collect()
    }

    pub fn mean(&self) -> Option<f64> {
        let nums = self.numbers()?;
        let total: f64 = nums.iter().map(|(_, w)| w).sum();
        Some(nums.iter().map(|(x, w)| x * w).sum::<f64>() / total)
    }

    pub fn variance(&self) -> Option<f64> {
        let mean = self.mean()?;
        let nums = self.numbers()?;
        let total: f64 = nums.iter().map(|(_, w)| w).sum();
        Some(nums.iter().map(|(x, w)| (x - mean).powi(2) * w).sum::<f64>() / total)
    }

    /// The smallest value whose cumulative probability reaches `q`.
    pub fn quantile(&self, q: f64) -> Option<Value> {
        let total = self.total();
        let mut acc = 0.0;
        for (v, w) in &self.outcomes {
            acc += w / total;
            if acc >= q - 1e-12 {
                return Some(v.clone());
            }
        }
        self.outcomes.last().map(|(v, _)| v.clone())
    }
}

impl PartialEq for Dist {
    fn eq(&self, other: &Dist) -> bool {
        self.missing.to_bits() == other.missing.to_bits()
            && self.outcomes.len() == other.outcomes.len()
            && self
                .outcomes
                .iter()
                .zip(&other.outcomes)
                .all(|((a, p), (b, q))| a == b && p.to_bits() == q.to_bits())
    }
}

impl Eq for Dist {}

impl Hash for Dist {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.outcomes.len().hash(state);
        for (v, w) in &self.outcomes {
            v.hash(state);
            w.to_bits().hash(state);
        }
        self.missing.to_bits().hash(state);
    }
}

impl Ord for Dist {
    fn cmp(&self, other: &Dist) -> Ordering {
        for ((a, p), (b, q)) in self.outcomes.iter().zip(&other.outcomes) {
            let c = a.cmp(b).then_with(|| p.total_cmp(q));
            if c != Ordering::Equal {
                return c;
            }
        }
        self.outcomes
            .len()
            .cmp(&other.outcomes.len())
            .then_with(|| self.missing.total_cmp(&other.missing))
    }
}

impl PartialOrd for Dist {
    fn partial_cmp(&self, other: &Dist) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Natural log of the gamma function (Lanczos approximation, g = 7).
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection formula.
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn two_dice() {
        let d = Dist::dice(2, 6);
        assert_eq!(d.outcomes.len(), 11);
        assert!(close(d.outcomes[5].1, 6.0 / 36.0));
        assert!(close(d.mean().unwrap(), 7.0));
        assert!(close(d.variance().unwrap(), 35.0 / 6.0));
        assert_eq!(d.quantile(0.5), Some(Value::Int(7)));
    }

    #[test]
    fn counts_sum_to_one() {
        for d in [
            Dist::binomial(30, 0.3),
            Dist::poisson(4.5),
            Dist::geometric(1.0 / 6.0),
            Dist::binomial(30_000, 0.03),
        ] {
            assert!((d.total() + d.missing - 1.0).abs() < 1e-9, "{d:?}");
            assert!(d.missing < 1e-12);
        }
        assert!((Dist::poisson(4.5).mean().unwrap() - 4.5).abs() < 1e-9);
        assert!((Dist::binomial(30_000, 0.03).mean().unwrap() - 900.0).abs() < 1e-6);
    }

    #[test]
    fn dice_pools() {
        let pool = Dist::pool(3, &Dist::dice(1, 6));
        assert_eq!(pool.outcomes.len(), 56);
        assert!(close(pool.total(), 1.0));
    }

    #[test]
    fn gamma() {
        assert!((ln_gamma(1.0)).abs() < 1e-13);
        assert!((ln_gamma(5.0) - 24f64.ln()).abs() < 1e-13);
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-13);
    }
}
