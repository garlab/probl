//! Finite distributions: dice, choices, counts, and the results of computing
//! with them.

use crate::continuous::Rng;
use crate::error::{OpError, OpResult};
use crate::value::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as Atomic};

/// Outcomes below this probability are dropped from infinite supports.
const TAIL: f64 = 1e-18;

#[derive(Clone, Debug)]
pub struct Dist {
    /// Sorted by value, each value once, every weight positive.
    pub outcomes: Vec<(Value, f64)>,
    /// Probability not represented by `outcomes`: a tail cut off to keep the
    /// support finite, or weight a `simulate` left unresolved. Drawing counts
    /// it as unresolved.
    pub missing: f64,
}

/// Work taken from a shared budget at a time: threads rarely touch it, and
/// between them hold back little of it.
const WORK_CHUNK: u64 = 1 << 14;

/// Limits on building distributions, shared by everything in a run.
#[derive(Clone, Debug)]
pub struct Budget {
    /// The most outcomes one distribution (or combination) may have.
    pub max_outcomes: usize,
    /// The most elements a collection built by the program may have.
    pub max_collection: usize,
    /// Units of work left: world-steps plus outcomes computed.
    pub work_left: u64,
    /// Work shared with other threads, which `work_left` is topped up from
    /// (when batches of runs are sampled in parallel).
    pub shared: Option<Arc<AtomicU64>>,
}

impl Budget {
    pub fn unlimited() -> Budget {
        Budget {
            max_outcomes: usize::MAX,
            max_collection: usize::MAX,
            work_left: u64::MAX,
            shared: None,
        }
    }

    /// Check that a distribution with `n` outcomes may be built.
    pub fn outcomes(&self, n: u128) -> OpResult<()> {
        if n > self.max_outcomes as u128 {
            let shown = if n > 1_000_000_000_000 {
                "more than 10¹²".to_string()
            } else {
                n.to_string()
            };
            return Err(OpError::limit(format!(
                "a distribution with {shown} outcomes is over the limit of {}",
                self.max_outcomes
            )));
        }
        Ok(())
    }

    /// Check that a collection with `n` elements may be built.
    pub fn collection(&self, n: u128) -> OpResult<()> {
        if n > self.max_collection as u128 {
            return Err(OpError::limit(format!(
                "a collection with {n} elements is over the limit of {}",
                self.max_collection
            )));
        }
        Ok(())
    }

    /// Spend `n` units of work.
    pub fn work(&mut self, n: u64) -> OpResult<()> {
        if n > self.work_left && !self.top_up(n - self.work_left) {
            self.work_left = 0;
            return Err(OpError::limit("the run used up its work budget"));
        }
        self.work_left -= n;
        Ok(())
    }

    /// Take at least `need` units from the shared budget, if it has them.
    fn top_up(&mut self, need: u64) -> bool {
        let Some(shared) = &self.shared else {
            return false;
        };
        let want = need.max(WORK_CHUNK);
        let taken = shared.fetch_update(Atomic::Relaxed, Atomic::Relaxed, |left| {
            (left >= need).then(|| left - want.min(left))
        });
        match taken {
            Ok(left) => {
                self.work_left += want.min(left);
                true
            }
            Err(_) => false,
        }
    }

    /// Give the work not spent back to the shared budget.
    pub fn give_back(&mut self) {
        if let Some(shared) = &self.shared {
            shared.fetch_add(std::mem::take(&mut self.work_left), Atomic::Relaxed);
        }
    }
}

impl Dist {
    pub fn point(v: Value) -> Dist {
        Dist {
            outcomes: vec![(v, 1.0)],
            missing: 0.0,
        }
    }

    /// Build from (value, weight) pairs in any order, whose weights add up
    /// to `1 - missing`; equal values are merged.
    ///
    /// The weights are rescaled to add up to exactly that, which removes the
    /// rounding of the sums that produced them: six sixths of `d6 > 0` are
    /// 0.9999999999999999, but the distribution is certainly `true`. Without
    /// this, a certain condition would leave a branch of weight 1e-16.
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
        Dist::normalized(outcomes, missing)
    }

    /// Like `from_pairs`, for pairs already sorted by value, each value once.
    fn from_sorted(mut pairs: Vec<(Value, f64)>, missing: f64) -> Dist {
        debug_assert!(pairs.windows(2).all(|p| p[0].0 < p[1].0), "unsorted outcomes");
        pairs.retain(|(_, w)| *w > 0.0);
        Dist::normalized(pairs, missing)
    }

    fn normalized(mut outcomes: Vec<(Value, f64)>, missing: f64) -> Dist {
        let target = 1.0 - missing;
        let total: f64 = outcomes.iter().map(|(_, w)| w).sum();
        if let [(_, w)] = outcomes.as_mut_slice() {
            *w = target;
        } else if target > 0.0 && total > 0.0 && total != target {
            let scale = target / total;
            for (_, w) in &mut outcomes {
                *w *= scale;
            }
        }
        Dist { outcomes, missing }
    }

    pub fn into_value(self) -> Value {
        Value::Dist(Arc::new(self))
    }

    pub fn uniform(values: Vec<Value>) -> Dist {
        let p = 1.0 / values.len() as f64;
        Dist::from_pairs(values.into_iter().map(|v| (v, p)).collect(), 0.0)
    }

    /// `true` with probability `p`.
    pub fn bernoulli(p: f64) -> Dist {
        Dist::from_sorted(vec![(Value::Bool(false), 1.0 - p), (Value::Bool(true), p)], 0.0)
    }

    /// The sum of `count` dice with `sides` sides.
    pub fn dice(count: u32, sides: u32, budget: &mut Budget) -> OpResult<Dist> {
        let support = count as u128 * (sides as u128 - 1) + 1;
        budget.outcomes(support)?;
        budget.work((count as u128 * support * sides as u128).min(u64::MAX as u128) as u64)?;
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
        Ok(Dist::from_sorted(pairs, 0.0))
    }

    pub fn binomial(n: u64, p: f64, budget: &mut Budget) -> OpResult<Dist> {
        if p <= 0.0 {
            return Ok(Dist::point(Value::Int(0)));
        }
        if p >= 1.0 {
            return Ok(Dist::point(Value::Int(n as i64)));
        }
        let odds = p / (1.0 - p);
        let mode = (((n as f64) + 1.0) * p).floor().min(n as f64) as u64;
        walk_from_mode(
            mode,
            n,
            |k| (n - k) as f64 / (k + 1) as f64 * odds,
            |k| k as f64 / (n - k + 1) as f64 / odds,
            budget,
        )
    }

    pub fn poisson(rate: f64, budget: &mut Budget) -> OpResult<Dist> {
        if rate <= 0.0 {
            return Ok(Dist::point(Value::Int(0)));
        }
        walk_from_mode(
            rate.floor() as u64,
            u64::MAX,
            |k| rate / (k + 1) as f64,
            |k| k as f64 / rate,
            budget,
        )
    }

    /// Number of tries up to and including the first success.
    pub fn geometric(p: f64, budget: &mut Budget) -> OpResult<Dist> {
        if p >= 1.0 {
            return Ok(Dist::point(Value::Int(1)));
        }
        // Outcomes until the tail (1 - p)^k drops below the threshold.
        let count = (libm::log(TAIL) / libm::log(1.0 - p)).ceil();
        budget.outcomes(if count.is_finite() { count as u128 } else { u128::MAX })?;
        budget.work(count as u64)?;
        let mut pairs = Vec::with_capacity(count as usize);
        let mut tail = 1.0;
        let mut k: i64 = 1;
        while tail >= TAIL {
            pairs.push((Value::Int(k), tail * p));
            tail *= 1.0 - p;
            k += 1;
        }
        Ok(Dist::from_sorted(pairs, tail))
    }

    /// The dice of `count` rolls of `die`, sorted from highest to lowest.
    pub fn pool(count: u32, die: &Dist, budget: &mut Budget) -> OpResult<Dist> {
        // As many pools as multisets of `count` faces.
        let faces = die.outcomes.len() as u128;
        budget.outcomes(multisets(faces, count as u128))?;
        let mut pools: BTreeMap<Vec<Value>, f64> = BTreeMap::new();
        pools.insert(Vec::new(), 1.0);
        for _ in 0..count {
            budget.work(pools.len() as u64 * faces as u64)?;
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
        // Each die independently misses `die.missing` of its probability.
        let missing = 1.0 - libm::pow(1.0 - die.missing, count as f64);
        Ok(Dist::from_pairs(pairs, missing))
    }

    pub fn total(&self) -> f64 {
        self.outcomes.iter().map(|(_, w)| w).sum()
    }

    /// For a distribution of facts: the probabilities of `true` and `false`.
    pub fn truth(&self) -> Option<(f64, f64)> {
        let (mut yes, mut no) = (0.0, 0.0);
        for (v, w) in &self.outcomes {
            match v {
                Value::Bool(true) => yes += w,
                Value::Bool(false) => no += w,
                _ => return None,
            }
        }
        Some((yes, no))
    }

    fn numbers(&self) -> Option<Vec<(f64, f64)>> {
        self.outcomes
            .iter()
            .map(|(v, w)| match v {
                Value::Bool(_) => None,
                _ => v.as_f64().map(|x| (x, *w)),
            })
            .collect()
    }

    /// The mean of the resolved outcomes.
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

    /// The smallest value whose cumulative probability (among the resolved
    /// outcomes) reaches `q`.
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

/// `binomial`, `poisson` or `geometric` by their parameters: their
/// probabilities and draws, without listing their outcomes. Sampling uses
/// them (docs/semantics.md, section 14).
#[derive(Clone, Copy, Debug)]
pub enum Counts {
    Binomial {
        n: u64,
        p: f64,
    },
    Poisson {
        rate: f64,
    },
    /// Tries up to and including the first success.
    Geometric {
        p: f64,
    },
}

impl Counts {
    /// Whether a draw can walk out from the mode: the spread is small enough
    /// for that to take a bounded number of steps. Otherwise listing the
    /// outcomes, which is budgeted, fails cleanly.
    pub fn direct(&self) -> bool {
        match *self {
            Counts::Binomial { n, p } => (n as f64) * p * (1.0 - p) <= 1e10,
            Counts::Poisson { rate } => rate <= 1e10,
            Counts::Geometric { .. } => true,
        }
    }

    /// The distribution with its outcomes listed.
    pub fn list(&self, budget: &mut Budget) -> OpResult<Dist> {
        match *self {
            Counts::Binomial { n, p } => Dist::binomial(n, p, budget),
            Counts::Poisson { rate } => Dist::poisson(rate, budget),
            Counts::Geometric { p } => Dist::geometric(p, budget),
        }
    }

    /// P(X = k).
    pub fn pmf(&self, k: f64) -> f64 {
        if k < 0.0 || k.fract() != 0.0 {
            return 0.0;
        }
        let exactly = |x: f64| if k == x { 1.0 } else { 0.0 };
        match *self {
            Counts::Binomial { n, p } => {
                let n = n as f64;
                if k > n {
                    0.0
                } else if p <= 0.0 {
                    exactly(0.0)
                } else if p >= 1.0 {
                    exactly(n)
                } else {
                    crate::math::exp(
                        libm::lgamma(n + 1.0) - libm::lgamma(k + 1.0) - libm::lgamma(n - k + 1.0)
                            + k * libm::log(p)
                            + (n - k) * libm::log1p(-p),
                    )
                }
            }
            Counts::Poisson { rate } => {
                if rate <= 0.0 {
                    exactly(0.0)
                } else {
                    crate::math::exp(k * libm::log(rate) - rate - libm::lgamma(k + 1.0))
                }
            }
            Counts::Geometric { p } => {
                if k < 1.0 {
                    0.0
                } else if p >= 1.0 {
                    exactly(1.0)
                } else {
                    p * crate::math::exp((k - 1.0) * libm::log1p(-p))
                }
            }
        }
    }

    /// One draw.
    pub fn sample(&self, rng: &mut Rng) -> i64 {
        match *self {
            Counts::Binomial { n, p } => {
                if p <= 0.0 {
                    return 0;
                }
                if p >= 1.0 {
                    return n as i64;
                }
                let odds = p / (1.0 - p);
                let mode = (((n as f64) + 1.0) * p).floor().min(n as f64) as u64;
                let up = |k: u64| (n - k) as f64 / (k + 1) as f64 * odds;
                let down = |k: u64| k as f64 / (n - k + 1) as f64 / odds;
                from_mode(rng, mode, self.pmf(mode as f64), n, up, down) as i64
            }
            Counts::Poisson { rate } => {
                if rate <= 0.0 {
                    return 0;
                }
                let mode = rate.floor() as u64;
                let up = |k: u64| rate / (k + 1) as f64;
                let down = |k: u64| k as f64 / rate;
                from_mode(rng, mode, self.pmf(mode as f64), u64::MAX, up, down) as i64
            }
            Counts::Geometric { p } => {
                if p >= 1.0 {
                    return 1;
                }
                // The inverse of P(X ≤ k) = 1 − (1 − p)^k.
                (libm::log(rng.open()) / libm::log1p(-p)).ceil().max(1.0) as i64
            }
        }
    }
}

/// Inversion from the mode outward: a uniform draw is compared with the
/// probabilities added up from the mode, taking the more likely neighbour
/// next, so that a draw usually takes a few steps. `up` and `down` are as in
/// `walk_from_mode`.
fn from_mode(
    rng: &mut Rng,
    mode: u64,
    p_mode: f64,
    max: u64,
    up: impl Fn(u64) -> f64,
    down: impl Fn(u64) -> f64,
) -> u64 {
    let u = rng.uniform();
    let mut total = p_mode;
    let (mut lo, mut hi, mut p_lo, mut p_hi) = (mode, mode, p_mode, p_mode);
    let mut last = mode;
    while u >= total {
        let below = if lo > 0 { p_lo * down(lo) } else { 0.0 };
        let above = if hi < max { p_hi * up(hi) } else { 0.0 };
        if below <= 0.0 && above <= 0.0 {
            // The probabilities ran out before reaching the draw: that is
            // only rounding.
            break;
        }
        if above >= below {
            hi += 1;
            p_hi = above;
            total += above;
            last = hi;
        } else {
            lo -= 1;
            p_lo = below;
            total += below;
            last = lo;
        }
    }
    last
}

/// The outcomes `0..=max` of a count distribution with a single mode, built
/// from the ratios between neighbouring probabilities: `up(k)` is
/// P(k + 1) / P(k) and `down(k)` is P(k − 1) / P(k). Working with ratios
/// relative to the mode stays accurate for huge parameters. The walk stops on
/// each side once an outcome is below the threshold; the rest of that tail
/// shrinks at least geometrically, which bounds the missing mass.
fn walk_from_mode(
    mode: u64,
    max: u64,
    up: impl Fn(u64) -> f64,
    down: impl Fn(u64) -> f64,
    budget: &mut Budget,
) -> OpResult<Dist> {
    let mut below: Vec<(u64, f64)> = Vec::new();
    let mut above: Vec<(u64, f64)> = Vec::new();
    let mut tails = 0.0;
    let (mut k, mut w) = (mode, 1.0);
    while k > 0 {
        let next = w * down(k);
        k -= 1;
        if next < TAIL {
            tails += geometric_tail(next, if k > 0 { down(k) } else { 0.0 });
            break;
        }
        below.push((k, next));
        w = next;
        budget.outcomes((below.len() + above.len() + 1) as u128)?;
        budget.work(1)?;
    }
    let (mut k, mut w) = (mode, 1.0);
    while k < max {
        let next = w * up(k);
        k += 1;
        if next < TAIL {
            tails += geometric_tail(next, if k < max { up(k) } else { 0.0 });
            break;
        }
        above.push((k, next));
        w = next;
        budget.outcomes((below.len() + above.len() + 1) as u128)?;
        budget.work(1)?;
    }
    let sum = 1.0 + below.iter().map(|(_, w)| w).sum::<f64>() + above.iter().map(|(_, w)| w).sum::<f64>() + tails;
    let pairs = below
        .into_iter()
        .rev()
        .chain(std::iter::once((mode, 1.0)))
        .chain(above)
        .map(|(k, w)| (Value::Int(k as i64), w / sum))
        .collect();
    Ok(Dist::from_sorted(pairs, tails / sum))
}

/// An upper bound on `first + first·r + first·r² + …` for ratios at most `r`.
fn geometric_tail(first: f64, ratio: f64) -> f64 {
    if first <= 0.0 {
        return 0.0;
    }
    first / (1.0 - ratio.clamp(0.0, 1.0 - 1e-6))
}

/// The number of multisets of size `k` drawn from `n` kinds: C(n + k − 1, k),
/// saturating.
fn multisets(n: u128, k: u128) -> u128 {
    if n == 0 {
        return if k == 0 { 1 } else { 0 };
    }
    let mut result: u128 = 1;
    for i in 1..=k {
        result = result.saturating_mul(n + i - 1) / i;
        if result > u64::MAX as u128 {
            return u128::MAX;
        }
    }
    result
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
        return libm::log(std::f64::consts::PI / libm::sin(std::f64::consts::PI * x)) - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * libm::log(2.0 * std::f64::consts::PI) + (x + 0.5) * libm::log(t) - t + libm::log(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn two_dice() {
        let d = Dist::dice(2, 6, &mut Budget::unlimited()).unwrap();
        assert_eq!(d.outcomes.len(), 11);
        assert!(close(d.outcomes[5].1, 6.0 / 36.0));
        assert!(close(d.mean().unwrap(), 7.0));
        assert!(close(d.variance().unwrap(), 35.0 / 6.0));
        assert_eq!(d.quantile(0.5), Some(Value::Int(7)));
    }

    #[test]
    fn counts_sum_to_one() {
        let b = &mut Budget::unlimited();
        for d in [
            Dist::binomial(30, 0.3, b).unwrap(),
            Dist::poisson(4.5, b).unwrap(),
            Dist::geometric(1.0 / 6.0, b).unwrap(),
            Dist::binomial(30_000, 0.03, b).unwrap(),
            Dist::poisson(1e9, b).unwrap(),
        ] {
            assert!((d.total() + d.missing - 1.0).abs() < 1e-9, "{:?}", d.outcomes.len());
            assert!(d.missing < 1e-12);
        }
        assert!((Dist::poisson(4.5, b).unwrap().mean().unwrap() - 4.5).abs() < 1e-9);
        assert!((Dist::binomial(30_000, 0.03, b).unwrap().mean().unwrap() - 900.0).abs() < 1e-6);
    }

    /// Drawing a count directly (as sampling does) follows the same
    /// distribution as listing its outcomes: a chi-square test with the
    /// cells of small expected counts pooled.
    #[test]
    fn direct_draws_follow_the_listed_distributions() {
        let b = &mut Budget::unlimited();
        let cases = [
            Counts::Binomial { n: 250, p: 0.034 },
            Counts::Binomial { n: 10, p: 0.5 },
            Counts::Binomial { n: 5, p: 0.97 },
            Counts::Binomial { n: 12_000, p: 0.03 },
            Counts::Poisson { rate: 0.3 },
            Counts::Poisson { rate: 100.5 },
            Counts::Geometric { p: 0.2 },
        ];
        let mut rng = Rng::new(3);
        for c in cases {
            let d = c.list(b).unwrap();
            for (v, p) in &d.outcomes {
                let q = c.pmf(v.as_f64().unwrap());
                assert!(
                    (q - p).abs() <= 1e-10 * p.max(1e-300) + 1e-15,
                    "{c:?} at {v}: {q} vs {p}"
                );
            }
            let n = 200_000;
            let mut seen: BTreeMap<i64, f64> = BTreeMap::new();
            for _ in 0..n {
                *seen.entry(c.sample(&mut rng)).or_default() += 1.0;
            }
            let (mut chi2, mut cells) = (0.0, 0);
            let (mut expected, mut observed) = (0.0, 0.0);
            for (v, p) in &d.outcomes {
                expected += p * n as f64;
                observed += seen.remove(&(v.as_f64().unwrap() as i64)).unwrap_or(0.0);
                if expected >= 20.0 {
                    chi2 += (observed - expected).powi(2) / expected;
                    cells += 1;
                    (expected, observed) = (0.0, 0.0);
                }
            }
            chi2 += (observed - expected).powi(2) / expected.max(1.0);
            assert!(seen.is_empty(), "{c:?} drew values outside its outcomes: {seen:?}");
            let df = cells as f64;
            assert!(
                chi2 < df + 6.0 * (2.0 * df).sqrt() + 10.0,
                "{c:?}: chi² {chi2} with {cells} cells"
            );
        }
    }

    #[test]
    fn dice_pools() {
        let b = &mut Budget::unlimited();
        let pool = Dist::pool(3, &Dist::dice(1, 6, b).unwrap(), b).unwrap();
        assert_eq!(pool.outcomes.len(), 56);
        assert!(close(pool.total(), 1.0));
        // A die with missing mass m: a pool of n misses 1 - (1 - m)^n.
        let leaky = Dist::from_pairs(vec![(Value::Int(1), 0.9)], 0.1);
        let pool = Dist::pool(2, &leaky, b).unwrap();
        assert!(close(pool.missing, 1.0 - 0.81));
        assert!(close(pool.total() + pool.missing, 1.0));
    }

    #[test]
    fn budgets_are_checked_before_building() {
        let mut small = Budget {
            max_outcomes: 1000,
            max_collection: 1000,
            work_left: u64::MAX,
            shared: None,
        };
        assert!(Dist::dice(1, 100_000, &mut small).is_err());
        assert!(Dist::pool(40, &Dist::dice(1, 6, &mut Budget::unlimited()).unwrap(), &mut small).is_err());
        assert!(Dist::geometric(1e-9, &mut small).is_err());
        assert_eq!(multisets(6, 3), 56);
        assert_eq!(multisets(u64::MAX as u128, 40), u128::MAX);
    }

    #[test]
    fn a_shared_budget_is_spent_once() {
        let shared = Arc::new(AtomicU64::new(100_000));
        let budget = Budget {
            work_left: 0,
            shared: Some(shared.clone()),
            ..Budget::unlimited()
        };
        let (mut a, mut b) = (budget.clone(), budget);
        a.work(60_000).unwrap();
        assert!(b.work(60_000).is_err(), "only 40,000 are left");
        b.work(30_000).unwrap();
        a.give_back();
        b.give_back();
        assert_eq!(shared.load(Atomic::Relaxed), 10_000);
    }

    #[test]
    fn gamma() {
        assert!((ln_gamma(1.0)).abs() < 1e-13);
        assert!((ln_gamma(5.0) - 24f64.ln()).abs() < 1e-13);
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-13);
    }
}
