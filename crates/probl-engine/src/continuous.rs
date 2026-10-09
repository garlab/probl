//! Continuous distributions (docs/semantics.md, section 13): densities,
//! CDFs, quantiles and moments, and drawing from them.
//!
//! Drawing uses its own generator and `libm` rather than the platform's math
//! library, so that a seed gives the same numbers everywhere (section 14).

use crate::error::{Fault, OpError, OpResult};
use std::f64::consts::{PI, SQRT_2};
use std::fmt;

/// The 95% quantile of the standard normal: an estimate's 90% interval is
/// this many standard deviations on each side of its centre.
pub const Z95: f64 = 1.644853626951472;

/// A continuous distribution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Family {
    Normal {
        mean: f64,
        sd: f64,
    },
    Lognormal {
        mu: f64,
        sigma: f64,
    },
    Uniform {
        lo: f64,
        hi: f64,
    },
    Beta {
        a: f64,
        b: f64,
    },
    Gamma {
        shape: f64,
        scale: f64,
    },
    Exponential {
        rate: f64,
    },
    Triangular {
        lo: f64,
        mode: f64,
        hi: f64,
    },
    /// A beta distribution stretched over `lo..hi` with its mode at `mode`.
    Pert {
        lo: f64,
        mode: f64,
        hi: f64,
    },
}

fn check(ok: bool, message: impl FnOnce() -> String) -> OpResult<()> {
    if ok {
        Ok(())
    } else {
        Err(OpError::fault(Fault::DomainError, message()))
    }
}

fn finite(values: &[f64], what: &str) -> OpResult<()> {
    check(values.iter().all(|x| x.is_finite()), || {
        format!("{what} needs finite numbers")
    })
}

/// a/(a+b) for positive parameters, even when their sum overflows.
fn ratio(a: f64, b: f64) -> f64 {
    if a >= b {
        1.0 / (1.0 + b / a)
    } else {
        let r = a / b;
        r / (1.0 + r)
    }
}

fn interval_fraction(lo: f64, hi: f64, x: f64) -> f64 {
    if x <= lo {
        return 0.0;
    }
    if x >= hi {
        return 1.0;
    }
    let width = hi - lo;
    if width.is_finite() {
        (x - lo) / width
    } else {
        (x / 2.0 - lo / 2.0) / (hi / 2.0 - lo / 2.0)
    }
}

fn inverse_width(lo: f64, hi: f64) -> f64 {
    let width = hi - lo;
    if width.is_finite() {
        1.0 / width
    } else {
        0.5 / (hi / 2.0 - lo / 2.0)
    }
}

fn standardized(x: f64, mean: f64, sd: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let delta = x - mean;
    if delta.is_finite() {
        delta / sd
    } else {
        x / sd - mean / sd
    }
}

impl Family {
    pub fn normal(mean: f64, sd: f64) -> OpResult<Family> {
        finite(&[mean, sd], "normal")?;
        check(sd > 0.0, || "normal needs a standard deviation above 0".into())?;
        Ok(Family::Normal { mean, sd })
    }

    pub fn lognormal(mu: f64, sigma: f64) -> OpResult<Family> {
        finite(&[mu, sigma], "lognormal")?;
        check(sigma > 0.0, || "lognormal needs a sigma above 0".into())?;
        Ok(Family::Lognormal { mu, sigma })
    }

    pub fn uniform(lo: f64, hi: f64) -> OpResult<Family> {
        finite(&[lo, hi], "uniform")?;
        check(lo < hi, || "uniform needs its lower end below its upper end".into())?;
        Ok(Family::Uniform { lo, hi })
    }

    pub fn beta(a: f64, b: f64) -> OpResult<Family> {
        finite(&[a, b], "beta")?;
        check(a > 0.0 && b > 0.0, || "beta needs two parameters above 0".into())?;
        Ok(Family::Beta { a, b })
    }

    pub fn gamma(shape: f64, scale: f64) -> OpResult<Family> {
        finite(&[shape, scale], "gamma")?;
        check(shape > 0.0 && scale > 0.0, || {
            "gamma needs a shape and a scale above 0".into()
        })?;
        Ok(Family::Gamma { shape, scale })
    }

    pub fn exponential(rate: f64) -> OpResult<Family> {
        finite(&[rate], "exponential")?;
        check(rate > 0.0, || "exponential needs a rate above 0".into())?;
        Ok(Family::Exponential { rate })
    }

    pub fn triangular(lo: f64, mode: f64, hi: f64) -> OpResult<Family> {
        finite(&[lo, mode, hi], "triangular")?;
        check(lo < hi && lo <= mode && mode <= hi, || {
            "triangular needs lo < hi, with the mode between them".into()
        })?;
        Ok(Family::Triangular { lo, mode, hi })
    }

    pub fn pert(lo: f64, mode: f64, hi: f64) -> OpResult<Family> {
        finite(&[lo, mode, hi], "pert")?;
        check(lo < hi && lo <= mode && mode <= hi, || {
            "pert needs lo < hi, with the mode between them".into()
        })?;
        Ok(Family::Pert { lo, mode, hi })
    }

    /// `a to b`: the lognormal whose 5% and 95% quantiles are `a` and `b`.
    pub fn estimate(a: f64, b: f64) -> OpResult<Family> {
        finite(&[a, b], "`to`")?;
        if a <= 0.0 || b <= 0.0 {
            return Err(
                OpError::fault(Fault::DomainError, "`a to b` needs two positive numbers")
                    .help("for a quantity that can be zero or negative, use `normal_range(lo, hi)`"),
            );
        }
        check(a < b, || "`a to b` needs a below b".into())?;
        let (la, lb) = (libm::log(a), libm::log(b));
        Family::lognormal((la + lb) / 2.0, (lb - la) / (2.0 * Z95))
    }

    /// `normal_range(lo, hi)`: the normal whose 5% and 95% quantiles are
    /// `lo` and `hi`.
    pub fn normal_range(lo: f64, hi: f64) -> OpResult<Family> {
        finite(&[lo, hi], "normal_range")?;
        check(lo < hi, || {
            "normal_range needs its lower end below its upper end".into()
        })?;
        Family::normal(
            crate::stats::midpoint(lo, hi),
            crate::stats::scaled_difference(hi, lo, 1.0 / (2.0 * Z95)),
        )
    }

    /// The parameters of the beta distribution behind a PERT.
    fn pert_shape(lo: f64, mode: f64, hi: f64) -> (f64, f64) {
        let p = interval_fraction(lo, hi, mode);
        (1.0 + 4.0 * p, 1.0 + 4.0 * (1.0 - p))
    }

    /// The smallest and largest possible values.
    pub fn support(&self) -> (f64, f64) {
        match *self {
            Family::Normal { .. } => (f64::NEG_INFINITY, f64::INFINITY),
            Family::Lognormal { .. } | Family::Gamma { .. } | Family::Exponential { .. } => (0.0, f64::INFINITY),
            Family::Uniform { lo, hi } | Family::Triangular { lo, hi, .. } | Family::Pert { lo, hi, .. } => (lo, hi),
            Family::Beta { .. } => (0.0, 1.0),
        }
    }

    pub fn mean(&self) -> f64 {
        match *self {
            Family::Normal { mean, .. } => mean,
            Family::Lognormal { mu, sigma } => crate::math::exp(mu + sigma * sigma / 2.0),
            Family::Uniform { lo, hi } => crate::stats::midpoint(lo, hi),
            Family::Beta { a, b } => ratio(a, b),
            Family::Gamma { shape, scale } => shape * scale,
            Family::Exponential { rate } => 1.0 / rate,
            Family::Triangular { lo, mode, hi } => crate::stats::lerp(crate::stats::midpoint(lo, hi), mode, 1.0 / 3.0),
            Family::Pert { lo, mode, hi } => {
                let (a, b) = Family::pert_shape(lo, mode, hi);
                crate::stats::lerp(lo, hi, ratio(a, b))
            }
        }
    }

    pub fn variance(&self) -> f64 {
        let sd = self.sd();
        sd * sd
    }

    pub fn sd(&self) -> f64 {
        match *self {
            Family::Normal { sd, .. } => sd,
            Family::Lognormal { mu, sigma } => {
                let s2 = sigma * sigma;
                let log_sd = if s2 == 0.0 {
                    mu + libm::log(sigma)
                } else {
                    mu + s2 + 0.5 * libm::log(-libm::expm1(-s2))
                };
                crate::math::exp(log_sd)
            }
            Family::Uniform { lo, hi } => crate::stats::scaled_difference(hi, lo, 1.0 / libm::sqrt(12.0)),
            Family::Beta { a, b } => {
                let max = a.max(b);
                let inv = if max < 1.0 {
                    1.0 / (1.0 + a + b)
                } else {
                    (1.0 / max) / (1.0 + a.min(b) / max + 1.0 / max)
                };
                libm::sqrt(ratio(a, b)) * libm::sqrt(ratio(b, a)) * libm::sqrt(inv)
            }
            Family::Gamma { shape, scale } => libm::sqrt(shape) * scale,
            Family::Exponential { rate } => 1.0 / rate,
            Family::Triangular { lo, mode, hi } => {
                let d = |a, b| crate::stats::scaled_difference(a, b, 1.0 / 6.0);
                libm::hypot(libm::hypot(d(mode, lo), d(hi, mode)), d(hi, lo))
            }
            Family::Pert { lo, mode, hi } => {
                let (a, b) = Family::pert_shape(lo, mode, hi);
                crate::stats::scaled_difference(hi, lo, Family::Beta { a, b }.sd())
            }
        }
    }

    /// Conditional moments on a nonempty interval, using incomplete moments.
    /// Uniform and normal use centered formulas to avoid subtracting large
    /// location parameters when computing the variance.
    pub fn interval_moments(&self, lo: f64, hi: f64) -> (f64, f64) {
        let mass = self.cdf(hi) - self.cdf(lo);
        let raw = |first: f64, second: f64| {
            let mean = first / mass;
            (mean, (second / mass - mean * mean).max(0.0))
        };
        match *self {
            Family::Uniform { .. } => (crate::stats::midpoint(lo, hi), Family::Uniform { lo, hi }.variance()),
            Family::Normal { mean, sd } => {
                let (a, b) = ((lo - mean) / sd, (hi - mean) / sd);
                let (pa, pb) = (std_normal_pdf(a), std_normal_pdf(b));
                let shift = (pa - pb) / mass;
                let edge = |z: f64, p: f64| if z.is_finite() { z * p } else { 0.0 };
                (
                    mean + sd * shift,
                    sd * sd * (1.0 + (edge(a, pa) - edge(b, pb)) / mass - shift * shift).max(0.0),
                )
            }
            Family::Beta { a, b } => {
                let moment = |n: f64| beta_cdf(a + n, b, hi) - beta_cdf(a + n, b, lo);
                raw(
                    a / (a + b) * moment(1.0),
                    a * (a + 1.0) / ((a + b) * (a + b + 1.0)) * moment(2.0),
                )
            }
            Family::Gamma { shape, scale } => {
                let moment = |n: f64| gamma_cdf(shape + n, hi / scale) - gamma_cdf(shape + n, lo / scale);
                raw(
                    shape * scale * moment(1.0),
                    shape * (shape + 1.0) * scale * scale * moment(2.0),
                )
            }
            Family::Exponential { rate } => Family::Gamma {
                shape: 1.0,
                scale: 1.0 / rate,
            }
            .interval_moments(lo, hi),
            Family::Lognormal { mu, sigma } => {
                let moment = |n: f64| {
                    let cdf = |x| std_normal_cdf((libm::log(x) - mu - n * sigma * sigma) / sigma);
                    crate::math::exp(n * mu + n * n * sigma * sigma / 2.0) * (cdf(hi) - cdf(lo))
                };
                raw(moment(1.0), moment(2.0))
            }
            Family::Pert { lo: a, mode, hi: b } => {
                let (alpha, beta) = Self::pert_shape(a, mode, b);
                let (m, v) =
                    Family::Beta { a: alpha, b: beta }.interval_moments((lo - a) / (b - a), (hi - a) / (b - a));
                (a + (b - a) * m, (b - a).powi(2) * v)
            }
            Family::Triangular { lo: a, mode, hi: b } => {
                let width = b - a;
                let (l, h, m) = ((lo - a) / width, (hi - a) / width, (mode - a) / width);
                let moment = |n: i32| {
                    let integral = |l: f64, h: f64, k: i32| (h.powi(k + 1) - l.powi(k + 1)) / (k + 1) as f64;
                    let left = if l < m {
                        2.0 / m * integral(l, h.min(m), n + 1)
                    } else {
                        0.0
                    };
                    let right = if h > m {
                        2.0 / (1.0 - m) * (integral(l.max(m), h, n) - integral(l.max(m), h, n + 1))
                    } else {
                        0.0
                    };
                    left + right
                };
                let (m, v) = raw(moment(1), moment(2));
                (a + width * m, width * width * v)
            }
        }
    }

    pub fn pdf(&self, x: f64) -> f64 {
        match *self {
            Family::Normal { mean, sd } => std_normal_pdf(standardized(x, mean, sd)) / sd,
            Family::Lognormal { mu, sigma } => {
                if x <= 0.0 {
                    0.0
                } else {
                    std_normal_pdf((libm::log(x) - mu) / sigma) / (sigma * x)
                }
            }
            Family::Uniform { lo, hi } => {
                if (lo..=hi).contains(&x) {
                    inverse_width(lo, hi)
                } else {
                    0.0
                }
            }
            Family::Beta { a, b } => beta_pdf(a, b, x),
            Family::Gamma { shape, scale } => {
                if x < 0.0 {
                    return 0.0;
                }
                if x == 0.0 {
                    return if shape < 1.0 {
                        f64::INFINITY
                    } else if shape == 1.0 {
                        1.0 / scale
                    } else {
                        0.0
                    };
                }
                let y = x / scale;
                crate::math::exp((shape - 1.0) * libm::log(y) - y - libm::lgamma(shape)) / scale
            }
            Family::Exponential { rate } => {
                if x < 0.0 {
                    0.0
                } else {
                    rate * crate::math::exp(-rate * x)
                }
            }
            Family::Triangular { lo, mode, hi } => {
                if x < lo || x > hi {
                    0.0
                } else if x < mode {
                    2.0 * interval_fraction(lo, mode, x) * inverse_width(lo, hi)
                } else if x > mode {
                    2.0 * (1.0 - interval_fraction(mode, hi, x)) * inverse_width(lo, hi)
                } else {
                    2.0 * inverse_width(lo, hi)
                }
            }
            Family::Pert { lo, mode, hi } => {
                let (a, b) = Family::pert_shape(lo, mode, hi);
                if x < lo || x > hi {
                    0.0
                } else {
                    beta_pdf(a, b, interval_fraction(lo, hi, x)) * inverse_width(lo, hi)
                }
            }
        }
    }

    /// ln f(x), which doesn't underflow far into a tail: −∞ outside the
    /// support.
    pub fn ln_pdf(&self, x: f64) -> f64 {
        match *self {
            Family::Normal { mean, sd } => {
                let z = standardized(x, mean, sd);
                -0.5 * z * z - libm::log(sd) - LN_SQRT_2PI
            }
            Family::Lognormal { mu, sigma } if x > 0.0 => {
                let z = (libm::log(x) - mu) / sigma;
                -0.5 * z * z - libm::log(sigma) - libm::log(x) - LN_SQRT_2PI
            }
            Family::Beta { a, b } if x > 0.0 && x < 1.0 => {
                (a - 1.0) * libm::log(x) + (b - 1.0) * libm::log1p(-x) - ln_beta(a, b)
            }
            Family::Gamma { shape, scale } if x > 0.0 => {
                let y = x / scale;
                (shape - 1.0) * libm::log(y) - y - libm::lgamma(shape) - libm::log(scale)
            }
            Family::Exponential { rate } if x >= 0.0 => libm::log(rate) - rate * x,
            Family::Pert { lo, mode, hi } if x > lo && x < hi => {
                let (a, b) = Family::pert_shape(lo, mode, hi);
                Family::Beta { a, b }.ln_pdf(interval_fraction(lo, hi, x)) + libm::log(inverse_width(lo, hi))
            }
            // Ends, bounded densities and outside the support.
            _ => libm::log(self.pdf(x)),
        }
    }

    /// P(X ≤ x).
    pub fn cdf(&self, x: f64) -> f64 {
        if x.is_nan() {
            return f64::NAN;
        }
        match *self {
            Family::Normal { mean, sd } => std_normal_cdf(standardized(x, mean, sd)),
            Family::Lognormal { mu, sigma } => {
                if x <= 0.0 {
                    0.0
                } else {
                    std_normal_cdf((libm::log(x) - mu) / sigma)
                }
            }
            Family::Uniform { lo, hi } => interval_fraction(lo, hi, x).clamp(0.0, 1.0),
            Family::Beta { a, b } => beta_cdf(a, b, x),
            Family::Gamma { shape, scale } => gamma_cdf(shape, x / scale),
            Family::Exponential { rate } => {
                if x <= 0.0 {
                    0.0
                } else {
                    -libm::expm1(-rate * x)
                }
            }
            Family::Triangular { lo, mode, hi } => {
                if x <= lo {
                    0.0
                } else if x >= hi {
                    1.0
                } else if x <= mode {
                    interval_fraction(lo, hi, x) * interval_fraction(lo, mode, x)
                } else {
                    1.0 - (1.0 - interval_fraction(lo, hi, x)) * (1.0 - interval_fraction(mode, hi, x))
                }
            }
            Family::Pert { lo, mode, hi } => {
                let (a, b) = Family::pert_shape(lo, mode, hi);
                beta_cdf(a, b, interval_fraction(lo, hi, x))
            }
        }
    }

    /// The value below which a share `p` of the distribution lies.
    pub fn quantile(&self, p: f64) -> f64 {
        let (lo, hi) = self.support();
        if p <= 0.0 {
            return lo;
        }
        if p >= 1.0 {
            return hi;
        }
        match *self {
            Family::Normal { mean, sd } => mean + sd * std_normal_quantile(p),
            Family::Lognormal { mu, sigma } => crate::math::exp(mu + sigma * std_normal_quantile(p)),
            Family::Uniform { lo, hi } => crate::stats::lerp(lo, hi, p),
            Family::Exponential { rate } => -libm::log1p(-p) / rate,
            Family::Triangular { lo, mode, hi } => {
                let split = interval_fraction(lo, hi, mode);
                if p <= split {
                    crate::stats::lerp(lo, hi, libm::sqrt(p * split))
                } else {
                    crate::stats::lerp(hi, lo, libm::sqrt((1.0 - p) * (1.0 - split)))
                }
            }
            Family::Beta { .. } | Family::Pert { .. } => invert(|x| self.cdf(x), p, lo, hi),
            Family::Gamma { .. } => {
                let mut top = self.mean() + 10.0 * libm::sqrt(self.variance());
                while self.cdf(top) < p && top < f64::MAX / 4.0 {
                    top *= 2.0;
                }
                invert(|x| self.cdf(x), p, 0.0, top)
            }
        }
    }

    /// One draw.
    pub fn sample(&self, rng: &mut Rng) -> f64 {
        match *self {
            Family::Normal { mean, sd } => mean + sd * rng.normal(),
            Family::Lognormal { mu, sigma } => crate::math::exp(mu + sigma * rng.normal()),
            Family::Uniform { lo, hi } => crate::stats::lerp(lo, hi, rng.uniform()),
            Family::Beta { a, b } => rng.beta(a, b),
            Family::Gamma { shape, scale } => scale * rng.gamma(shape),
            Family::Exponential { rate } => -libm::log(rng.open()) / rate,
            Family::Triangular { .. } => self.quantile(rng.uniform()),
            Family::Pert { lo, mode, hi } => {
                let (a, b) = Family::pert_shape(lo, mode, hi);
                crate::stats::lerp(lo, hi, rng.beta(a, b))
            }
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Family::Normal { .. } => "normal",
            Family::Lognormal { .. } => "lognormal",
            Family::Uniform { .. } => "uniform",
            Family::Beta { .. } => "beta",
            Family::Gamma { .. } => "gamma",
            Family::Exponential { .. } => "exponential",
            Family::Triangular { .. } => "triangular",
            Family::Pert { .. } => "pert",
        }
    }

    /// The parameters, for hashing, ordering and display.
    pub fn params(&self) -> Vec<f64> {
        match *self {
            Family::Normal { mean, sd } => vec![mean, sd],
            Family::Lognormal { mu, sigma } => vec![mu, sigma],
            Family::Uniform { lo, hi } => vec![lo, hi],
            Family::Beta { a, b } => vec![a, b],
            Family::Gamma { shape, scale } => vec![shape, scale],
            Family::Exponential { rate } => vec![rate],
            Family::Triangular { lo, mode, hi } | Family::Pert { lo, mode, hi } => vec![lo, mode, hi],
        }
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params: Vec<String> = self
            .params()
            .iter()
            .map(|x| {
                format!("{:.4}", x)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            })
            .collect();
        write!(f, "{}({})", self.name(), params.join(", "))
    }
}

// ── Special functions ────────────────────────────────────────────────────

fn std_normal_pdf(z: f64) -> f64 {
    crate::math::exp(-z * z / 2.0) / libm::sqrt(2.0 * PI)
}

pub fn std_normal_cdf(z: f64) -> f64 {
    0.5 * libm::erfc(-z / SQRT_2)
}

/// Φ⁻¹(p): a rational approximation (Abramowitz and Stegun 26.2.23, error
/// below 4.5e-4), polished by Newton's method on the exact CDF.
pub fn std_normal_quantile(p: f64) -> f64 {
    if p == 0.5 {
        return 0.0;
    }
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let tail = |q: f64| {
        let t = libm::sqrt(-2.0 * libm::log(q));
        t - (2.515517 + 0.802853 * t + 0.010328 * t * t)
            / (1.0 + 1.432788 * t + 0.189269 * t * t + 0.001308 * t * t * t)
    };
    let mut z = if p < 0.5 { -tail(p) } else { tail(1.0 - p) };
    for _ in 0..6 {
        let density = std_normal_pdf(z);
        if density == 0.0 {
            break;
        }
        let step = (std_normal_cdf(z) - p) / density;
        z -= step;
        if step.abs() < 1e-15 * z.abs().max(1.0) {
            break;
        }
    }
    z
}

fn beta_pdf(a: f64, b: f64, x: f64) -> f64 {
    if !(0.0..=1.0).contains(&x) {
        return 0.0;
    }
    if x == 0.0 || x == 1.0 {
        // At an end, the density is infinite, 1/B(a, b), or zero, as the
        // parameter for that end is below, at or above 1.
        let edge = if x == 0.0 { a } else { b };
        return if edge < 1.0 {
            f64::INFINITY
        } else if edge == 1.0 {
            crate::math::exp(libm::lgamma(a + b) - libm::lgamma(a) - libm::lgamma(b))
        } else {
            0.0
        };
    }
    crate::math::exp(
        (a - 1.0) * libm::log(x) + (b - 1.0) * libm::log1p(-x) + libm::lgamma(a + b)
            - libm::lgamma(a)
            - libm::lgamma(b),
    )
}

/// ln √(2π).
const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_7;

/// ln B(a, b), the logarithm of the beta function. For large arguments,
/// the log gammas it's made of are huge and nearly cancel, so it's computed
/// from their Stirling series instead, as R's `lbeta` is.
pub fn ln_beta(a: f64, b: f64) -> f64 {
    let (p, q) = if a < b { (a, b) } else { (b, a) };
    let share = p / (p + q);
    if p >= 10.0 {
        let rest = stirling_rest(p) + stirling_rest(q) - stirling_rest(p + q);
        -0.5 * libm::log(q) + LN_SQRT_2PI + rest + (p - 0.5) * libm::log(share) + q * libm::log1p(-share)
    } else if q >= 10.0 {
        let rest = stirling_rest(q) - stirling_rest(p + q);
        libm::lgamma(p) + rest + p - p * libm::log(p + q) + (q - 0.5) * libm::log1p(-share)
    } else {
        libm::lgamma(p) + libm::lgamma(q) - libm::lgamma(p + q)
    }
}

/// ln Γ(x) minus Stirling's approximation (x − ½) ln x − x + ln √(2π), for
/// x ≥ 10: the first seven terms of its series, which are exact to 10⁻¹⁶
/// there.
fn stirling_rest(x: f64) -> f64 {
    let r = 1.0 / (x * x);
    let series = 1.0 / 12.0
        + r * (-1.0 / 360.0
            + r * (1.0 / 1260.0 + r * (-1.0 / 1680.0 + r * (1.0 / 1188.0 + r * (-691.0 / 360_360.0 + r / 156.0)))));
    series / x
}

/// The regularized incomplete beta function I_x(a, b), by its continued
/// fraction (Numerical Recipes, section 6.4).
pub fn beta_cdf(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front = crate::math::exp(
        libm::lgamma(a + b) - libm::lgamma(a) - libm::lgamma(b) + a * libm::log(x) + b * libm::log1p(-x),
    );
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_fraction(b, a, 1.0 - x) / b
    }
}

fn beta_fraction(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=1000 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < 1e-16 {
            break;
        }
    }
    h
}

/// The regularized lower incomplete gamma function P(a, x): its series
/// below a + 1, its continued fraction above (Numerical Recipes, 6.2).
pub fn gamma_cdf(a: f64, x: f64) -> f64 {
    if x == f64::INFINITY {
        return 1.0;
    }
    if x <= 0.0 {
        return 0.0;
    }
    let front = crate::math::exp(-x + a * libm::log(x) - libm::lgamma(a));
    if x < a + 1.0 {
        let (mut sum, mut term, mut n) = (1.0 / a, 1.0 / a, a);
        for _ in 0..10_000 {
            n += 1.0;
            term *= x / n;
            sum += term;
            if term.abs() < sum.abs() * 1e-17 {
                break;
            }
        }
        (sum * front).min(1.0)
    } else {
        const TINY: f64 = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..10_000 {
            let i = i as f64;
            let an = -i * (i - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() < 1e-16 {
                break;
            }
        }
        (1.0 - front * h).max(0.0)
    }
}

/// The x in `lo..hi` where a non-decreasing `f` reaches `p` (the smallest
/// such x), by bisection.
pub fn invert(f: impl Fn(f64) -> f64, p: f64, mut lo: f64, mut hi: f64) -> f64 {
    if !lo.is_finite() || !hi.is_finite() {
        return f64::NAN;
    }
    for _ in 0..300 {
        let mid = crate::stats::midpoint(lo, hi);
        if mid <= lo || mid >= hi {
            break;
        }
        let cumulative = f(mid);
        if !cumulative.is_finite() {
            return f64::NAN;
        }
        if cumulative >= p {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

// ── Random numbers ───────────────────────────────────────────────────────

/// xoshiro256++, seeded through SplitMix64.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

/// SplitMix64's step and output function.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

pub(crate) fn mix(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(GOLDEN);
            mix(z)
        };
        Rng {
            s: [next(), next(), next(), next()],
        }
    }

    /// The random numbers of the batch numbered `index`, for a run with this
    /// seed. Every batch has a stream of its own, so batches can run in any
    /// order and on any number of threads, and still get the same numbers
    /// (docs/semantics.md, section 14).
    pub fn stream(seed: u64, index: u64) -> Rng {
        // The index-th output of SplitMix64 from the seed: different for
        // every index.
        Rng::new(mix(seed.wrapping_add(index.wrapping_add(1).wrapping_mul(GOLDEN))))
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in (0, 1]: safe to take the logarithm of.
    pub fn open(&mut self) -> f64 {
        1.0 - self.uniform()
    }

    /// A standard normal draw (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let (u, v) = (self.open(), self.uniform());
        libm::sqrt(-2.0 * libm::log(u)) * libm::cos(2.0 * PI * v)
    }

    /// A gamma draw with scale 1 (Marsaglia and Tsang, 2000).
    pub fn gamma(&mut self, shape: f64) -> f64 {
        if shape < 1.0 {
            let u = self.open();
            return self.gamma(shape + 1.0) * libm::pow(u, 1.0 / shape);
        }
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / libm::sqrt(9.0 * d);
        loop {
            let x = self.normal();
            let v = 1.0 + c * x;
            if v <= 0.0 {
                continue;
            }
            let v = v * v * v;
            let u = self.open();
            if u < 1.0 - 0.0331 * x.powi(4) || libm::log(u) < 0.5 * x * x + d * (1.0 - v + libm::log(v)) {
                return d * v;
            }
        }
    }

    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        for _ in 0..16 {
            let x = self.gamma(a);
            let y = self.gamma(b);
            if x + y > 0.0 {
                return if x == 0.0 {
                    0.0
                } else if y == 0.0 {
                    1.0
                } else {
                    ratio(x, y)
                };
            }
        }
        // Both draws round to zero only for tiny parameters, where the
        // distribution is nearly all at 0 and 1.
        if self.uniform() < ratio(a, b) { 1.0 } else { 0.0 }
    }

    /// An index chosen with probability proportional to `weights`, or `None`
    /// if they're all zero.
    pub fn choose(&mut self, weights: impl Iterator<Item = f64> + Clone) -> Option<usize> {
        let total: f64 = weights.clone().sum();
        if total <= 0.0 {
            return None;
        }
        let target = self.uniform() * total;
        let mut acc = 0.0;
        let mut last = None;
        for (i, w) in weights.enumerate() {
            if w <= 0.0 {
                continue;
            }
            acc += w;
            last = Some(i);
            if target < acc {
                return Some(i);
            }
        }
        last
    }
}

// ── Mixtures ─────────────────────────────────────────────────────────────

/// A part of a mixture: a number, or a continuous distribution.
#[derive(Clone, Debug)]
pub enum Part {
    Point(f64),
    Continuous(Family),
    Analytic(crate::analytic::Analytic),
}

/// Moments, CDF and quantiles of a mixture of numbers and continuous
/// distributions, each with its probability.
#[derive(Clone, Debug)]
pub struct Mixture {
    pub parts: Vec<(Part, f64)>,
}

impl Mixture {
    fn total(&self) -> f64 {
        crate::stats::sum(self.parts.iter().map(|(_, p)| *p))
    }

    pub fn mean(&self) -> f64 {
        crate::stats::weighted_mean(self.parts.iter().map(|(part, p)| {
            (
                match part {
                    Part::Point(x) => *x,
                    Part::Continuous(f) => f.mean(),
                    // NaN where an outcome has no mean from formulas.
                    Part::Analytic(a) => a.moments().map_or(f64::NAN, |m| m.0),
                },
                *p,
            )
        }))
    }

    pub fn variance(&self) -> f64 {
        let sd = self.sd();
        sd * sd
    }

    pub fn sd(&self) -> f64 {
        crate::stats::weighted_sd(self.parts.iter().map(|(part, p)| {
            let (m, sd) = match part {
                Part::Point(x) => (*x, 0.0),
                Part::Continuous(f) => (f.mean(), f.sd()),
                Part::Analytic(a) => (a.moments().map_or(f64::NAN, |m| m.0), a.sd().unwrap_or(f64::NAN)),
            };
            (m, sd, *p)
        }))
    }

    pub fn cdf(&self, x: f64) -> f64 {
        let below = crate::stats::sum(self.parts.iter().map(|(part, p)| {
            p * match part {
                Part::Point(v) => {
                    if *v <= x {
                        1.0
                    } else {
                        0.0
                    }
                }
                Part::Continuous(f) => f.cdf(x),
                Part::Analytic(a) => a.cdf(x),
            }
        }));
        below / self.total()
    }

    /// Distinguish an actual gap in the support at half the mass from a CDF
    /// that merely rounds to 0.5. This also handles disconnected analytic
    /// domains in report summaries without perturbing the requested quantile.
    pub fn median_bounds(&self) -> (f64, f64) {
        let mut intervals = Vec::new();
        for (part, weight) in &self.parts {
            if *weight <= 0.0 {
                continue;
            }
            match part {
                Part::Point(x) => intervals.push((*x, *x, *weight)),
                Part::Continuous(f) => {
                    let (lo, hi) = f.support();
                    intervals.push((lo, hi, *weight));
                }
                Part::Analytic(a) => {
                    let total = a.domain.mass();
                    for (lo, hi, f) in a.segments() {
                        let (x, y) = f.ends(&a.family, lo, hi);
                        intervals.push((x.min(y), x.max(y), weight * (hi - lo) / total));
                    }
                }
            }
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total = crate::stats::sum(intervals.iter().map(|x| x.2));
        let mut acc = crate::stats::Sum::default();
        let mut end = f64::NEG_INFINITY;
        for (lo, hi, weight) in intervals {
            if lo > end && acc.value() > 0.0 && crate::stats::half_split(acc.value(), total) {
                return (end, lo);
            }
            end = end.max(hi);
            acc.add(weight);
        }
        let x = self.quantile(0.5);
        (x, x)
    }

    pub fn median(&self) -> f64 {
        let (lo, hi) = self.median_bounds();
        crate::stats::midpoint(lo, hi)
    }

    pub fn quantile(&self, q: f64) -> f64 {
        let ends = |q: f64| {
            self.parts.iter().map(move |(part, _)| match part {
                Part::Point(x) => *x,
                Part::Continuous(f) => f.quantile(q),
                Part::Analytic(a) => a.quantile(q),
            })
        };
        if self.parts.len() == 1 {
            return match &self.parts[0].0 {
                Part::Point(x) => *x,
                Part::Continuous(f) => f.quantile(q),
                Part::Analytic(a) => a.quantile(q),
            };
        }
        if q <= 0.0 {
            return ends(0.0).fold(f64::INFINITY, f64::min);
        }
        if q >= 1.0 {
            return ends(1.0).fold(f64::NEG_INFINITY, f64::max);
        }
        let lo = ends(q.min(1e-12)).fold(f64::INFINITY, f64::min).max(-f64::MAX);
        let hi = ends(q.max(1.0 - 1e-12)).fold(f64::NEG_INFINITY, f64::max).min(f64::MAX);
        if lo >= hi {
            return lo;
        }
        if self.cdf(hi) < q {
            return f64::INFINITY;
        }
        invert(|x| self.cdf(x), q, (lo - 1e-9 * lo.abs().max(1.0)).max(-f64::MAX), hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * (1.0 + b.abs()), "{a} vs {b}");
    }

    #[test]
    fn normal_quantiles() {
        close(std_normal_cdf(Z95), 0.95, 1e-15);
        close(std_normal_quantile(0.95), Z95, 1e-14);
        close(std_normal_quantile(0.5), 0.0, 1e-15);
        close(std_normal_quantile(0.025), -1.9599639845400545, 1e-13);
        close(std_normal_quantile(1e-10), -6.361340902404056, 1e-10);
        close(std_normal_cdf(-1.959963984540054), 0.025, 1e-14);
    }

    #[test]
    fn estimates_have_the_right_intervals() {
        let e = Family::estimate(3.0, 7.0).unwrap();
        close(e.quantile(0.05), 3.0, 1e-12);
        close(e.quantile(0.95), 7.0, 1e-12);
        let r = Family::normal_range(-0.08, 0.0).unwrap();
        close(r.quantile(0.05), -0.08, 1e-12);
        close(r.cdf(0.0), 0.95, 1e-12);
        assert!(Family::estimate(-1.0, 3.0).is_err());
        assert!(Family::estimate(3.0, 1.0).is_err());
    }

    #[test]
    fn incomplete_functions() {
        // I_0.5(2, 3) = 11/16; P(1, x) = 1 − e^−x; P(1/2, x) = erf(√x).
        close(beta_cdf(2.0, 3.0, 0.5), 11.0 / 16.0, 1e-14);
        close(beta_cdf(1.0, 1.0, 0.3), 0.3, 1e-14);
        close(gamma_cdf(1.0, 2.0), 1.0 - (-2.0f64).exp(), 1e-14);
        close(gamma_cdf(0.5, 3.0), libm::erf(3.0f64.sqrt()), 1e-13);
        close(gamma_cdf(10.0, 30.0), 0.9999928782491372, 1e-13);
        let b = Family::beta(2.0, 40.0).unwrap();
        close(b.quantile(b.cdf(0.05)), 0.05, 1e-10);
        let g = Family::gamma(3.0, 2.0).unwrap();
        close(g.quantile(0.5), 5.348120627447122, 1e-10);
    }

    #[test]
    fn densities_integrate_to_their_cdfs() {
        let families = [
            Family::normal(1.0, 2.0).unwrap(),
            Family::lognormal(0.5, 0.3).unwrap(),
            Family::uniform(-1.0, 3.0).unwrap(),
            Family::beta(2.5, 4.0).unwrap(),
            Family::gamma(2.0, 1.5).unwrap(),
            Family::exponential(0.7).unwrap(),
            Family::triangular(0.0, 1.0, 4.0).unwrap(),
            Family::pert(1.0, 2.0, 6.0).unwrap(),
        ];
        for f in families {
            let (a, b) = (f.quantile(0.1), f.quantile(0.8));
            let n = 20_000;
            let h = (b - a) / n as f64;
            let integral: f64 = (0..n).map(|i| f.pdf(a + (i as f64 + 0.5) * h) * h).sum();
            close(integral, 0.7, 1e-6);
            close(f.cdf(b) - f.cdf(a), 0.7, 1e-9);
        }
    }

    #[test]
    fn log_densities_agree_and_reach_far_tails() {
        let families = [
            Family::normal(1.0, 2.0).unwrap(),
            Family::lognormal(0.5, 0.3).unwrap(),
            Family::uniform(-1.0, 3.0).unwrap(),
            Family::beta(2.5, 4.0).unwrap(),
            Family::gamma(2.0, 1.5).unwrap(),
            Family::exponential(0.7).unwrap(),
            Family::triangular(0.0, 1.0, 4.0).unwrap(),
            Family::pert(1.0, 2.0, 6.0).unwrap(),
        ];
        for f in families {
            for q in [0.01, 0.3, 0.5, 0.9, 0.999] {
                let x = f.quantile(q);
                close(f.ln_pdf(x), libm::log(f.pdf(x)), 1e-12);
            }
            if f.support().0 > -10.0 {
                assert_eq!(f.ln_pdf(-10.0), f64::NEG_INFINITY, "{f}");
            }
        }
        // Where the density underflows, its logarithm doesn't.
        let n = Family::normal(0.0, 1.0).unwrap();
        assert_eq!(n.pdf(50.0), 0.0);
        close(n.ln_pdf(50.0), -1250.0 - LN_SQRT_2PI, 1e-15);
        let b = Family::beta(2000.0, 2000.0).unwrap();
        assert!(b.ln_pdf(0.01).is_finite());
    }

    /// Kolmogorov–Smirnov: the draws follow each distribution's CDF.
    #[test]
    fn draws_follow_the_distributions() {
        let families = [
            Family::normal(1.0, 2.0).unwrap(),
            Family::lognormal(0.5, 0.3).unwrap(),
            Family::uniform(-1.0, 3.0).unwrap(),
            Family::beta(2.0, 40.0).unwrap(),
            Family::beta(0.5, 0.5).unwrap(),
            Family::gamma(0.4, 1.5).unwrap(),
            Family::gamma(7.0, 0.5).unwrap(),
            Family::exponential(0.7).unwrap(),
            Family::triangular(0.0, 1.0, 4.0).unwrap(),
            Family::pert(1.0, 2.0, 6.0).unwrap(),
            Family::estimate(60.0, 150.0).unwrap(),
        ];
        let mut rng = Rng::new(42);
        let n = 20_000;
        for f in families {
            let mut xs: Vec<f64> = (0..n).map(|_| f.sample(&mut rng)).collect();
            xs.sort_by(f64::total_cmp);
            let d = xs
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    let c = f.cdf(*x);
                    (c - i as f64 / n as f64)
                        .abs()
                        .max((c - (i + 1) as f64 / n as f64).abs())
                })
                .fold(0.0, f64::max);
            // The 0.001 critical value is 1.95 / √n.
            assert!(d < 1.95 / (n as f64).sqrt(), "{f}: D = {d}");
            let mean = xs.iter().sum::<f64>() / n as f64;
            close(mean, f.mean(), 6.0 * f.variance().sqrt() / (n as f64).sqrt());
        }
    }

    #[test]
    fn batches_have_streams_of_their_own() {
        let firsts: Vec<u64> = (0..1000).map(|i| Rng::stream(11, i).next_u64()).collect();
        let mut distinct = firsts.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), firsts.len());
        assert_eq!(Rng::stream(11, 3).next_u64(), Rng::stream(11, 3).next_u64());
        assert_ne!(Rng::stream(11, 3).next_u64(), Rng::stream(12, 3).next_u64());
    }

    #[test]
    fn seeds_repeat() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(Rng::new(7).next_u64(), Rng::new(8).next_u64());
    }

    #[test]
    fn mixtures() {
        let m = Mixture {
            parts: vec![
                (Part::Point(0.0), 0.5),
                (Part::Continuous(Family::uniform(1.0, 3.0).unwrap()), 0.5),
            ],
        };
        close(m.mean(), 1.0, 1e-12);
        close(m.cdf(2.0), 0.75, 1e-12);
        close(m.quantile(0.75), 2.0, 1e-9);
        close(m.quantile(0.25), 0.0, 1e-9);
    }
}
