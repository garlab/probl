//! Stable finite-population statistics for queries and report summaries.
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

/// A convex combination without overflowing the endpoint sum or difference.
pub(crate) fn lerp(a: f64, b: f64, p: f64) -> f64 {
    if a.is_sign_negative() == b.is_sign_negative() {
        a + (b - a) * p
    } else {
        a * (1.0 - p) + b * p
    }
}

/// Preserve small offsets when possible, scale first only if subtraction overflows.
pub(crate) fn scaled_difference(a: f64, b: f64, scale: f64) -> f64 {
    let delta = a - b;
    if delta.is_finite() {
        delta * scale
    } else {
        a * scale - b * scale
    }
}

pub(crate) fn weighted_mean(xs: impl Iterator<Item = (f64, f64)> + Clone) -> f64 {
    let total = sum(xs.clone().map(|(_, w)| w));
    if xs.clone().any(|(x, w)| !x.is_finite() || !w.is_finite()) || total <= 0.0 {
        return f64::NAN;
    }
    let lo = xs.clone().map(|(x, _)| x).fold(f64::INFINITY, f64::min);
    let hi = xs.clone().map(|(x, _)| x).fold(f64::NEG_INFINITY, f64::max);
    let origin = midpoint(lo, hi);
    let scale = (hi - origin).abs().max((lo - origin).abs());
    if scale == 0.0 {
        return origin;
    }
    // Centering preserves small spreads at a large location. A mean is a
    // convex combination; clamp only rounding beyond its known endpoints.
    let offset = sum(xs.map(|(x, w)| ((x - origin) / scale) * (w / total))).clamp(-1.0, 1.0) * scale;
    (origin + offset).clamp(lo, hi)
}

/// Combine within-component and between-component spread in SD units, so a
/// representable SD does not require a representable variance first.
pub(crate) fn weighted_sd(xs: impl Iterator<Item = (f64, f64, f64)> + Clone) -> f64 {
    let mean = weighted_mean(xs.clone().map(|(m, _, w)| (m, w)));
    let total = sum(xs.clone().map(|(_, _, w)| w));
    if !mean.is_finite() || xs.clone().any(|(_, sd, _)| !sd.is_finite()) {
        return f64::NAN;
    }
    xs.fold(0.0, |acc, (m, sd, w)| {
        let root_weight = libm::sqrt(w / total);
        libm::hypot(
            libm::hypot(acc, sd * root_weight),
            scaled_difference(m, mean, root_weight),
        )
    })
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
