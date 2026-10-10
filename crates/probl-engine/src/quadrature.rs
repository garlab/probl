//! Adaptive numerical integration on bounded intervals, for `@integrate`
//! (docs/semantics.md, section 13): the 21-point Gauss–Kronrod rule of
//! QUADPACK, bisecting the part with the largest error estimate until the
//! estimate is within the tolerance. An estimate isn't a bound: a feature
//! narrower than the rule's nodes can go unseen, which is why callers give
//! the places where the integrand has features as breakpoints.

use crate::dist::Budget;
use crate::error::{OpError, OpResult};

// The rule's constants, as QUADPACK publishes them.

/// The Kronrod nodes on [-1, 1], from the outside in; the odd ones are the
/// 10-point Gauss nodes.
#[allow(clippy::excessive_precision)]
const NODES: [f64; 11] = [
    0.995657163025808080735527280689003,
    0.973906528517171720077964012084452,
    0.930157491355708226001207180059508,
    0.865063366688984510732096688423493,
    0.780817726586416897063717578345042,
    0.679409568299024406234327365114874,
    0.562757134668604683339000099272694,
    0.433395394129247190799265943165784,
    0.294392862701460198131126603103866,
    0.148874338981631210884826001129720,
    0.0,
];

#[allow(clippy::excessive_precision)]
const KRONROD: [f64; 11] = [
    0.011694638867371874278064396062192,
    0.032558162307964727478818972459390,
    0.054755896574351996031381300244580,
    0.075039674810919952767043140916190,
    0.093125454583697605535065465083366,
    0.109387158802297641899210590325805,
    0.123491976262065851077958109831074,
    0.134709217311473325928054001771707,
    0.142775938577060080797094273138717,
    0.147739104901338491374841515972068,
    0.149445554002916905664936468389821,
];

#[allow(clippy::excessive_precision)]
const GAUSS: [f64; 5] = [
    0.066671344308688137593568809893332,
    0.149451349150580593145776339657697,
    0.219086362515982043995534934228163,
    0.269266719309996355091226921569469,
    0.295524224714752870173892994651338,
];

/// The most parts one integral is bisected into.
pub const MAX_PARTS: usize = 4_000;

/// An integral's value and its error estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub value: f64,
    pub error: f64,
}

/// The rule on `[a, b]`: the Kronrod estimate, and its difference from the
/// Gauss one as the error. Each evaluation costs a unit of work.
pub fn rule(f: &mut impl FnMut(f64) -> f64, a: f64, b: f64, budget: &mut Budget) -> OpResult<Estimate> {
    budget.work(21)?;
    let center = 0.5 * (a + b);
    let half = 0.5 * (b - a);
    let fc = f(center);
    let mut kronrod = KRONROD[10] * fc;
    let mut gauss = 0.0;
    for (i, &x) in NODES[..10].iter().enumerate() {
        let pair = f(center - half * x) + f(center + half * x);
        kronrod += KRONROD[i] * pair;
        if i % 2 == 1 {
            gauss += GAUSS[i / 2] * pair;
        }
    }
    let value = kronrod * half;
    let error = ((kronrod - gauss) * half).abs();
    Ok(Estimate { value, error })
}

/// One part of an integral: its interval and its estimate.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub a: f64,
    pub b: f64,
    pub estimate: Estimate,
}

/// `f` integrated over the intervals between consecutive `breaks`, by parts
/// bisected where the error is largest until the total error is at most
/// `tolerance` times the total, and `absolute` (the scale below which an
/// error doesn't matter). The parts, in order, or an error if the
/// tolerance can't be met, or a value isn't finite.
pub fn adaptive(
    f: &mut impl FnMut(f64) -> f64,
    breaks: &[f64],
    tolerance: f64,
    absolute: f64,
    budget: &mut Budget,
) -> OpResult<Vec<Part>> {
    let mut parts: Vec<Part> = Vec::with_capacity(breaks.len() + 16);
    for w in breaks.windows(2) {
        if w[1] > w[0] {
            parts.push(Part {
                a: w[0],
                b: w[1],
                estimate: rule(f, w[0], w[1], budget)?,
            });
        }
    }
    loop {
        let total: f64 = parts.iter().map(|p| p.estimate.value).sum();
        let error: f64 = parts.iter().map(|p| p.estimate.error).sum();
        if !total.is_finite() || !error.is_finite() {
            return Err(OpError::unsupported(
                "numerical integration didn't converge: it met a value that isn't finite",
            )
            .help("the integral may not exist; or use `@mode sample(runs: 10_000)`"));
        }
        if error <= (tolerance * total.abs()).max(absolute) {
            break;
        }
        if parts.len() >= MAX_PARTS {
            return Err(not_converged(tolerance));
        }
        let (i, _) = parts
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.estimate.error.total_cmp(&y.1.estimate.error))
            .expect("parts");
        let Part { a, b, .. } = parts[i];
        let mid = 0.5 * (a + b);
        if mid <= a || mid >= b {
            return Err(not_converged(tolerance));
        }
        let left = Part {
            a,
            b: mid,
            estimate: rule(f, a, mid, budget)?,
        };
        let right = Part {
            a: mid,
            b,
            estimate: rule(f, mid, b, budget)?,
        };
        parts[i] = left;
        parts.insert(i + 1, right);
    }
    Ok(parts)
}

pub fn not_converged(tolerance: f64) -> OpError {
    OpError::unsupported(format!(
        "numerical integration didn't converge to its relative tolerance of {tolerance:e}"
    ))
    .help("loosen it, like `@integrate 1e-6`, or use `@mode sample(runs: 10_000)`")
}

/// `f` over `[a, b]`, to `tolerance`.
pub fn integral(
    f: &mut impl FnMut(f64) -> f64,
    a: f64,
    b: f64,
    tolerance: f64,
    absolute: f64,
    budget: &mut Budget,
) -> OpResult<Estimate> {
    let parts = adaptive(f, &[a, b], tolerance, absolute, budget)?;
    Ok(Estimate {
        value: parts.iter().map(|p| p.estimate.value).sum(),
        error: parts.iter().map(|p| p.estimate.error).sum(),
    })
}

/// `f` over `[a, b]`, to `tolerance` relative to the integral of `|f|`, so
/// that an integral near zero, like a centred moment's, converges: `None` if
/// it doesn't.
pub fn moment(f: impl Fn(f64) -> f64, a: f64, b: f64, tolerance: f64) -> Option<f64> {
    let mut budget = Budget::unlimited();
    let scale = integral(&mut |u| f(u).abs(), a, b, tolerance, 0.0, &mut budget).ok()?;
    if scale.value == 0.0 {
        return Some(0.0);
    }
    integral(&mut |u| f(u), a, b, tolerance, tolerance * scale.value, &mut budget)
        .ok()
        .map(|e| e.value)
}

/// An integral over [0, 1], by parts with their running totals, to answer
/// for any interval and to invert.
#[derive(Clone, Debug)]
pub struct Table {
    parts: Vec<Part>,
    /// The total before each part.
    before: Vec<f64>,
}

impl Table {
    pub fn new(parts: Vec<Part>) -> Table {
        let mut before = Vec::with_capacity(parts.len());
        let mut sum = 0.0;
        for p in &parts {
            before.push(sum);
            sum += p.estimate.value;
        }
        Table { parts, before }
    }

    pub fn total(&self) -> f64 {
        self.parts
            .last()
            .map_or(0.0, |p| self.before[self.parts.len() - 1] + p.estimate.value)
    }

    /// The part that holds `u`.
    fn find(&self, u: f64) -> usize {
        self.parts.partition_point(|p| p.b <= u).min(self.parts.len() - 1)
    }

    /// The integral from 0 to `u`: whole parts from the totals, and the
    /// rule on the rest of the part `u` is in.
    pub fn up_to(&self, f: &mut impl FnMut(f64) -> f64, u: f64, budget: &mut Budget) -> OpResult<f64> {
        if u <= 0.0 {
            return Ok(0.0);
        }
        if u >= 1.0 {
            return Ok(self.total());
        }
        let i = self.find(u);
        let p = self.parts[i];
        let partial = if u <= p.a {
            0.0
        } else if u >= p.b {
            p.estimate.value
        } else {
            rule(f, p.a, u, budget)?.value
        };
        Ok(self.before[i] + partial)
    }

    /// The integral from `a` to `b`.
    pub fn between(&self, f: &mut impl FnMut(f64) -> f64, a: f64, b: f64, budget: &mut Budget) -> OpResult<f64> {
        if b <= a {
            return Ok(0.0);
        }
        Ok((self.up_to(f, b, budget)? - self.up_to(f, a, budget)?).max(0.0))
    }

    /// The `u` with the integral from 0 to it `target`: in its part, by
    /// Newton's steps, since `f` is the integral's derivative, kept within
    /// a bracket that bisection narrows when a step leaves it.
    pub fn inverse(&self, f: &mut impl FnMut(f64) -> f64, target: f64, budget: &mut Budget) -> OpResult<f64> {
        if target <= 0.0 {
            return Ok(self.parts.first().map_or(0.0, |p| p.a));
        }
        if target >= self.total() {
            return Ok(self.parts.last().map_or(1.0, |p| p.b));
        }
        let i = self.before.partition_point(|&b| b <= target).saturating_sub(1);
        let p = self.parts[i];
        let need = target - self.before[i];
        let mass = p.estimate.value;
        let (mut lo, mut hi) = (p.a, p.b);
        let mut u = p.a + (p.b - p.a) * (need / mass).clamp(0.0, 1.0);
        for _ in 0..60 {
            let diff = rule(f, p.a, u, budget)?.value - need;
            if diff.abs() <= 1e-12 * mass {
                break;
            }
            if diff < 0.0 {
                lo = u;
            } else {
                hi = u;
            }
            let slope = f(u);
            let step = if slope > 0.0 { u - diff / slope } else { f64::NAN };
            u = if step > lo && step < hi { step } else { 0.5 * (lo + hi) };
            if hi - lo <= f64::EPSILON * hi.abs().max(1e-300) {
                break;
            }
        }
        Ok(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Budget {
        Budget::unlimited()
    }

    #[test]
    fn smooth_integrals_are_exact_to_rounding() {
        let e = integral(&mut |x: f64| x.exp(), 0.0, 1.0, 1e-12, 0.0, &mut budget()).unwrap();
        assert!((e.value - (1f64.exp() - 1.0)).abs() < 1e-14, "{e:?}");
        // A narrow peak, found from breakpoints graded away from it.
        let mut peak = |x: f64| (-((x - 0.3) / 1e-4).powi(2)).exp();
        let mut breaks = vec![0.0, 0.3, 1.0];
        for k in 0..12 {
            let d = 1e-4 * 2f64.powi(k);
            breaks.extend([0.3 - d, 0.3 + d]);
        }
        breaks.retain(|b| (0.0..=1.0).contains(b));
        breaks.sort_by(f64::total_cmp);
        let parts = adaptive(&mut peak, &breaks, 1e-10, 0.0, &mut budget()).unwrap();
        let total: f64 = parts.iter().map(|p| p.estimate.value).sum();
        assert!((total - 1e-4 * std::f64::consts::PI.sqrt()).abs() < 1e-13, "{total}");
    }

    #[test]
    fn tables_answer_for_intervals_and_invert() {
        let mut f = |x: f64| 2.0 * x;
        let parts = adaptive(&mut f, &[0.0, 1.0], 1e-12, 0.0, &mut budget()).unwrap();
        let t = Table::new(parts);
        let mut b = budget();
        assert!((t.total() - 1.0).abs() < 1e-14);
        assert!((t.between(&mut f, 0.25, 0.5, &mut b).unwrap() - 0.1875).abs() < 1e-14);
        assert!((t.inverse(&mut f, 0.25, &mut b).unwrap() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn what_doesnt_converge_says_so() {
        let e = integral(&mut |x: f64| 1.0 / x, 0.0, 1.0, 1e-9, 0.0, &mut budget()).unwrap_err();
        assert!(e.message.contains("numerical integration didn't converge"), "{e:?}");
    }
}
