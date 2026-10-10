//! Likelihoods of one latent, for `@integrate` (docs/semantics.md, section
//! 13): what observations whose parameters are outcomes of the latent say
//! about it, as a weight on its distribution. The latent keeps its
//! distribution and its CDF coordinates `u`, and its posterior there has
//! the density w(u) ∝ Π L(x(u)) for x(u) its quantile. Probabilities,
//! quantiles and moments come from integrating w numerically, to a relative
//! tolerance, with a table of its integral shared by every question.

use crate::analytic::{Domain, Pieces};
use crate::continuous::Family;
use crate::dist::Budget;
use crate::error::OpResult;
use crate::quadrature::{self, Table};
use crate::value::{family_key, float_key};
use std::sync::Mutex;

/// The distribution an observation is from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Law {
    Normal,
    Lognormal,
    Uniform,
    Beta,
    Gamma,
    Exponential,
    Binomial,
    Bernoulli,
    Poisson,
    Geometric,
}

impl Law {
    pub fn name(self) -> &'static str {
        match self {
            Law::Normal => "normal",
            Law::Lognormal => "lognormal",
            Law::Uniform => "uniform",
            Law::Beta => "beta",
            Law::Gamma => "gamma",
            Law::Exponential => "exponential",
            Law::Binomial => "binomial",
            Law::Bernoulli => "bernoulli",
            Law::Poisson => "poisson",
            Law::Geometric => "geometric",
        }
    }

    /// Whether what it gives is a density.
    pub fn is_density(self) -> bool {
        !matches!(self, Law::Binomial | Law::Bernoulli | Law::Poisson | Law::Geometric)
    }

    /// The logarithm of the probability, or density, of `v` under the law
    /// with these parameters: −∞ for what it can't give, and NaN for
    /// parameters it doesn't take.
    pub fn ln_likelihood(self, params: &[f64], v: f64) -> f64 {
        let continuous = |family: OpResult<Family>| family.map_or(f64::NAN, |f| f.ln_pdf(v));
        let count = v >= 0.0 && v.fract() == 0.0;
        match (self, params) {
            (Law::Normal, &[m, s]) => continuous(Family::normal(m, s)),
            (Law::Lognormal, &[m, s]) => continuous(Family::lognormal(m, s)),
            (Law::Uniform, &[a, b]) => continuous(Family::uniform(a, b)),
            (Law::Beta, &[a, b]) => continuous(Family::beta(a, b)),
            (Law::Gamma, &[k, s]) => continuous(Family::gamma(k, s)),
            (Law::Exponential, &[r]) => continuous(Family::exponential(r)),
            (_, &[.., p])
                if matches!(self, Law::Binomial | Law::Bernoulli | Law::Geometric) && !(0.0..=1.0).contains(&p) =>
            {
                f64::NAN
            }
            (Law::Bernoulli, &[p]) => match v {
                1.0 => libm::log(p),
                0.0 => libm::log1p(-p),
                _ => f64::NEG_INFINITY,
            },
            (Law::Binomial, &[n, p]) => {
                if !count || v > n {
                    return f64::NEG_INFINITY;
                }
                let ln_choose = libm::lgamma(n + 1.0) - libm::lgamma(v + 1.0) - libm::lgamma(n - v + 1.0);
                let success = if v == 0.0 { 0.0 } else { v * libm::log(p) };
                let failure = if v == n { 0.0 } else { (n - v) * libm::log1p(-p) };
                ln_choose + success + failure
            }
            (Law::Poisson, &[rate]) if rate > 0.0 => {
                if !count {
                    return f64::NEG_INFINITY;
                }
                v * libm::log(rate) - rate - libm::lgamma(v + 1.0)
            }
            (Law::Poisson, &[0.0]) => {
                if v == 0.0 {
                    0.0
                } else {
                    f64::NEG_INFINITY
                }
            }
            (Law::Poisson, _) => f64::NAN,
            // Tries up to and including the first success.
            (Law::Geometric, &[p]) => {
                if !count || v < 1.0 {
                    return f64::NEG_INFINITY;
                }
                let failures = if v == 1.0 { 0.0 } else { (v - 1.0) * libm::log1p(-p) };
                failures + libm::log(p)
            }
            _ => f64::NAN,
        }
    }
}

/// A parameter of an observation: a number, or an outcome of the latent,
/// by its pieces in the latent's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    Plain(f64),
    Outcome(Pieces),
}

impl Param {
    fn at(&self, u: f64, x: f64) -> f64 {
        match self {
            Param::Plain(v) => *v,
            Param::Outcome(pieces) => {
                let i = pieces.partition_point(|(end, _)| *end < u).min(pieces.len() - 1);
                pieces[i].1.at(x)
            }
        }
    }
}

/// One observation of `value` from `law`, with its parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Factor {
    pub law: Law,
    pub params: Vec<Param>,
    pub value: f64,
}

type FactorKey = (
    &'static str,
    Vec<Result<u64, Vec<(u64, (u8, u64, u64, u64, u64))>>>,
    u64,
);

impl Factor {
    fn key(&self) -> FactorKey {
        (
            self.law.name(),
            self.params
                .iter()
                .map(|p| match p {
                    Param::Plain(v) => Ok(float_key(*v)),
                    Param::Outcome(pieces) => Err(pieces.iter().map(|(end, f)| (float_key(*end), f.key())).collect()),
                })
                .collect(),
            float_key(self.value),
        )
    }

    fn ln_at(&self, u: f64, x: f64) -> f64 {
        let mut params = [0.0; 3];
        for (i, p) in self.params.iter().enumerate() {
            params[i] = p.at(u, x);
        }
        self.law.ln_likelihood(&params[..self.params.len()], self.value)
    }

    /// Where its outcome parameters change function, in the coordinates.
    fn breaks(&self, out: &mut Vec<f64>) {
        for p in &self.params {
            if let Param::Outcome(pieces) = p {
                out.extend(pieces.iter().map(|(end, _)| *end));
            }
        }
    }
}

/// The product of what observations of a latent say about it, as a weight
/// on its distribution, with the table of its integral.
#[derive(Debug)]
pub struct Likelihood {
    /// The latent's distribution, in whose coordinates it's a function.
    pub family: Family,
    pub factors: Vec<Factor>,
    /// The relative tolerance its integrals are computed to.
    pub tolerance: f64,
    /// The logarithm of the weight where it's largest, about: the weight is
    /// computed relative to it, so that it neither overflows nor underflows.
    pub shift: f64,
    table: Table,
    /// Moments already integrated, by what they're of: a value drawn again
    /// and again, when sampling, asks for the same ones.
    moments: Mutex<Vec<(MomentKey, Moments)>>,
}

/// A mean and variance, if integrating them converged.
type Moments = Option<(f64, f64)>;

/// An outcome's pieces and domain, which its moments are of.
pub type MomentKey = (Vec<(u64, (u8, u64, u64, u64, u64))>, Vec<(u64, u64)>);

impl Clone for Likelihood {
    fn clone(&self) -> Likelihood {
        Likelihood {
            family: self.family,
            factors: self.factors.clone(),
            tolerance: self.tolerance,
            shift: self.shift,
            table: self.table.clone(),
            moments: Mutex::new(Vec::new()),
        }
    }
}

/// How many points of the coordinates are tried for where the weight is
/// largest, before refining around the best.
const GRID: usize = 4096;

impl Likelihood {
    /// The likelihood of these factors, for a latent distributed by
    /// `family`; `None` where every value of the latent rules out what was
    /// observed.
    pub fn new(
        family: Family,
        factors: Vec<Factor>,
        tolerance: f64,
        budget: &mut Budget,
    ) -> OpResult<Option<Likelihood>> {
        let ln_at = |u: f64| {
            let x = family.quantile(u);
            factors.iter().map(|f| f.ln_at(u, x)).sum::<f64>()
        };
        budget.work((GRID * factors.len()) as u64)?;
        // The largest weights, on a grid, then refined around the best ones.
        let mut grid: Vec<(f64, f64)> = (0..GRID)
            .map(|i| {
                let u = (i as f64 + 0.5) / GRID as f64;
                (u, nan_to_impossible(ln_at(u)))
            })
            .collect();
        let shift = grid.iter().map(|g| g.1).fold(f64::NEG_INFINITY, f64::max);
        if shift == f64::NEG_INFINITY {
            return Ok(None);
        }
        let mut peaks: Vec<(f64, f64)> = local_peaks(&grid);
        peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
        peaks.truncate(4);
        let mut shift = shift;
        let mut breaks = vec![0.0, 1.0];
        for f in &factors {
            f.breaks(&mut breaks);
        }
        for &(peak, _) in &peaks {
            let (u, ln, width) = refine(&ln_at, peak, 1.0 / GRID as f64, budget, factors.len())?;
            shift = shift.max(ln);
            // Breakpoints graded away from the peak, so that the rule sees
            // its flanks whatever its width.
            breaks.push(u);
            let mut d = width;
            while d < 1.0 {
                breaks.extend([u - d, u + d]);
                d *= 2.0;
            }
        }
        grid.clear();
        breaks.retain(|b| (0.0..=1.0).contains(b));
        breaks.sort_by(f64::total_cmp);
        breaks.dedup();
        let mut f = |u: f64| relative(ln_at(u), shift);
        let parts = quadrature::adaptive(&mut f, &breaks, tolerance, 0.0, budget)?;
        let table = Table::new(parts);
        if table.total() <= 0.0 {
            return Ok(None);
        }
        Ok(Some(Likelihood {
            family,
            factors,
            tolerance,
            shift,
            table,
            moments: Mutex::new(Vec::new()),
        }))
    }

    pub fn key(&self) -> (Vec<FactorKey>, (&'static str, Vec<u64>), u64) {
        (
            self.factors.iter().map(Factor::key).collect(),
            family_key(&self.family),
            float_key(self.tolerance),
        )
    }

    /// The logarithm of the weight at `u`.
    pub fn ln_at(&self, u: f64) -> f64 {
        let x = self.family.quantile(u);
        nan_to_impossible(self.factors.iter().map(|f| f.ln_at(u, x)).sum())
    }

    /// The weight at `u`, relative to `exp(shift)`.
    pub fn at(&self, u: f64) -> f64 {
        relative(self.ln_at(u), self.shift)
    }

    /// The weight's integral over the intervals of `d`, relative to
    /// `exp(shift)`.
    pub fn mass(&self, d: &Domain) -> f64 {
        let mut budget = Budget::unlimited();
        let mut f = |u: f64| self.at(u);
        d.0.iter()
            .map(|&(a, b)| self.table.between(&mut f, a, b, &mut budget).unwrap_or(0.0))
            .sum()
    }

    /// The coordinate where the weight's integral over `d`, from its start,
    /// is `share` of the whole.
    pub fn inverse(&self, d: &Domain, share: f64) -> f64 {
        let mut budget = Budget::unlimited();
        let mut f = |u: f64| self.at(u);
        let total = self.mass(d);
        let mut left = share.clamp(0.0, 1.0) * total;
        for &(a, b) in &d.0 {
            let here = self.table.between(&mut f, a, b, &mut budget).unwrap_or(0.0);
            if left <= here {
                let start = self.table.up_to(&mut f, a, &mut budget).unwrap_or(0.0);
                let u = self.table.inverse(&mut f, start + left, &mut budget).unwrap_or(a);
                return u.clamp(a, b);
            }
            left -= here;
        }
        d.0.last().map_or(0.5, |&(_, b)| b)
    }

    /// The moments of an outcome, by its `key`, as `compute` gives them the
    /// first time.
    pub fn moments(&self, key: MomentKey, compute: impl FnOnce() -> Option<(f64, f64)>) -> Option<(f64, f64)> {
        if let Some((_, m)) = self.moments.lock().expect("moments").iter().find(|(k, _)| *k == key) {
            return *m;
        }
        let m = compute();
        let mut cache = self.moments.lock().expect("moments");
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.push((key, m));
        m
    }

    /// ∫ g(u) w(u) du over `[a, b]`, relative to `exp(shift)`, to the
    /// tolerance: `None` if it doesn't converge.
    pub fn integral(&self, g: impl Fn(f64) -> f64, a: f64, b: f64) -> Option<f64> {
        quadrature::moment(
            |u| {
                let w = self.at(u);
                if w == 0.0 { 0.0 } else { g(u) * w }
            },
            a,
            b,
            self.tolerance,
        )
    }
}

impl PartialEq for Likelihood {
    fn eq(&self, other: &Likelihood) -> bool {
        self.key() == other.key()
    }
}

fn nan_to_impossible(ln: f64) -> f64 {
    if ln.is_nan() { f64::NEG_INFINITY } else { ln }
}

fn relative(ln: f64, shift: f64) -> f64 {
    if ln == f64::NEG_INFINITY {
        0.0
    } else {
        crate::math::exp(ln - shift)
    }
}

/// The grid points at least as high as the one before them, and higher than
/// the one after, with their values.
fn local_peaks(grid: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = grid.len();
    (0..n)
        .filter(|&i| {
            let v = grid[i].1;
            v > f64::NEG_INFINITY && (i == 0 || grid[i - 1].1 <= v) && (i + 1 == n || grid[i + 1].1 < v)
        })
        .map(|i| grid[i])
        .collect()
}

/// Around a grid peak at `u` with spacing `h`: a better peak, its log
/// weight, and how far from it the weight falls by e⁻¹⁰, at least.
fn refine(
    ln_at: &impl Fn(f64) -> f64,
    u: f64,
    h: f64,
    budget: &mut Budget,
    factors: usize,
) -> OpResult<(f64, f64, f64)> {
    let (mut best, mut ln, mut h) = (u, nan_to_impossible(ln_at(u)), h);
    for _ in 0..6 {
        budget.work((65 * factors) as u64)?;
        for i in 0..=64 {
            let v = (best - h + 2.0 * h * i as f64 / 64.0).clamp(1e-300, 1.0 - 1e-16);
            let l = nan_to_impossible(ln_at(v));
            if l > ln {
                ln = l;
                best = v;
            }
        }
        h /= 16.0;
    }
    // How wide the peak is: the nearest distance at which it has fallen by
    // e⁻¹⁰ on either side, among distances that double.
    let mut width = h;
    while width < 0.5 {
        let lower = nan_to_impossible(ln_at((best - width).max(0.0)));
        let upper = nan_to_impossible(ln_at((best + width).min(1.0)));
        if lower < ln - 10.0 || upper < ln - 10.0 {
            break;
        }
        width *= 2.0;
    }
    Ok((best, ln, (width / 8.0).max(1e-15)))
}
