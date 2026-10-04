//! Shared median conventions for queries and report summaries.
use crate::value::Value;

/// Compensated summation keeps half-mass boundaries stable for long populations.
#[derive(Default)]
pub(crate) struct Sum {
    sum: f64,
    correction: f64,
}
impl Sum {
    pub fn add(&mut self, x: f64) {
        let next = self.sum + x;
        self.correction += if self.sum.abs() >= x.abs() {
            (self.sum - next) + x
        } else {
            (x - next) + self.sum
        };
        self.sum = next;
    }
    pub fn value(&self) -> f64 {
        self.sum + self.correction
    }
}

pub(crate) fn sum(xs: impl Iterator<Item = f64>) -> f64 {
    let mut total = Sum::default();
    for x in xs {
        total.add(x);
    }
    total.value()
}

/// Allow only a few rounding ulps, rather than treating near-50% weights as ties.
pub(crate) fn half_split(part: f64, total: f64) -> bool {
    (part - total / 2.0).abs() <= 4.0 * f64::EPSILON * total.abs()
}

/// Lower quantile in an already ordered, resolved finite population. Work in
/// weight space with compensated sums; a fixed probability epsilon can erase
/// a genuine tail or cross a nearby mass boundary.
pub(crate) fn quantile(outcomes: &[(Value, f64)], q: f64) -> Option<&Value> {
    let mut positive = outcomes.iter().filter(|(_, w)| *w > 0.0);
    let first = &positive.next()?.0;
    let last = &outcomes.iter().rev().find(|(_, w)| *w > 0.0)?.0;
    if q <= 0.0 {
        return Some(first);
    }
    if q >= 1.0 {
        return Some(last);
    }
    let total = sum(outcomes.iter().map(|(_, w)| *w));
    let target = q * total;
    let mut acc = Sum::default();
    for (v, w) in outcomes.iter().filter(|(_, w)| *w > 0.0) {
        acc.add(*w);
        if acc.value() >= target {
            return Some(v);
        }
    }
    Some(last)
}

/// Endpoints of the median interval, in an ordered, resolved finite population.
pub(crate) fn median_bounds(outcomes: &[(Value, f64)]) -> Option<(&Value, &Value)> {
    let total = sum(outcomes.iter().map(|(_, w)| *w));
    if total <= 0.0 {
        return None;
    }
    let mut acc = Sum::default();
    let mut positive = outcomes.iter().filter(|(_, w)| *w > 0.0).peekable();
    while let Some((v, w)) = positive.next() {
        acc.add(*w);
        if half_split(acc.value(), total) {
            return Some((v, positive.peek().map_or(v, |(next, _)| next)));
        }
        if acc.value() > total / 2.0 {
            return Some((v, v));
        }
    }
    outcomes.last().map(|(v, _)| (v, v))
}

pub(crate) fn midpoint(a: f64, b: f64) -> f64 {
    if a.is_sign_negative() == b.is_sign_negative() {
        a + (b - a) / 2.0
    } else {
        (a + b) / 2.0
    }
}
