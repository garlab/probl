//! Identity-preserving continuous outcomes during enumeration.
//!
//! A draw is a function of one latent variable: affine on each of a few
//! pieces of the latent's CDF coordinates, which keeps `abs`, `min`, `max`
//! and `clamp` exact. Evidence is a set of intervals in those coordinates.
//! Worlds own their restrictions, and the latent's distribution there: a
//! posterior after an exact update. A value carries the coordinates of the
//! distribution it was made under, and moves to the world's when it's read.
//! Values and closures remain immutable, and calls return restrictions
//! alongside their results. Independent latent combinations require
//! sampling for now.

use crate::continuous::Family;
use crate::dist::Budget;
use crate::error::{Fault, OpError, OpResult};
use crate::value::{Closure, Value, family_key, float_key};
use probl_syntax::ast::BinOp;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Constraints = Arc<BTreeMap<u64, Latent>>;

/// What a world knows about a latent: its distribution, and where it can
/// be, in that distribution's CDF coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Latent {
    pub family: Family,
    pub domain: Domain,
}
impl Eq for Latent {}
impl std::hash::Hash for Latent {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        family_key(&self.family).hash(h);
        self.domain.key().hash(h);
    }
}

/// The CDF coordinate under `to` of the value at `c` under `from`.
fn moved(c: f64, from: &Family, to: &Family) -> f64 {
    to.cdf(from.quantile(c)).clamp(0.0, 1.0)
}

/// Sorted, disjoint CDF intervals. Endpoints have zero probability.
#[derive(Clone, Debug, PartialEq)]
pub struct Domain(pub Vec<(f64, f64)>);
impl Domain {
    pub fn full() -> Self {
        Self(vec![(0.0, 1.0)])
    }
    pub fn mass(&self) -> f64 {
        self.0.iter().map(|(a, b)| b - a).sum()
    }
    pub fn intersect(&self, other: &Self) -> Self {
        let (mut i, mut j) = (0, 0);
        let mut out = Vec::new();
        while i < self.0.len() && j < other.0.len() {
            let (a, b) = self.0[i];
            let (c, d) = other.0[j];
            if a.max(c) < b.min(d) {
                out.push((a.max(c), b.min(d)));
            }
            if b < d {
                i += 1;
            } else {
                j += 1;
            }
        }
        Self(out)
    }
    pub fn complement(&self) -> Self {
        let mut out = Vec::new();
        let mut lo = 0.0;
        for &(a, b) in &self.0 {
            if lo < a {
                out.push((lo, a));
            }
            lo = b;
        }
        if lo < 1.0 {
            out.push((lo, 1.0));
        }
        Self(out)
    }
    fn below(p: f64) -> Self {
        if p > 0.0 { Self(vec![(0.0, p)]) } else { Self(vec![]) }
    }
    fn key(&self) -> Vec<(u64, u64)> {
        self.0.iter().map(|(a, b)| (float_key(*a), float_key(*b))).collect()
    }
    /// The same values of a latent, in `to`'s coordinates instead of
    /// `from`'s.
    pub fn moved(&self, from: &Family, to: &Family) -> Self {
        let mut out = Vec::with_capacity(self.0.len());
        for &(a, b) in &self.0 {
            push(&mut out, moved(a, from, to), moved(b, from, to));
        }
        Self(out)
    }
    /// The domain as a tiling of the CDF coordinates: whether each part is
    /// in it.
    fn mask(&self) -> Vec<(f64, bool)> {
        let mut out = Vec::with_capacity(2 * self.0.len() + 1);
        let mut at = 0.0;
        for &(a, b) in &self.0 {
            if a > at {
                out.push((a, false));
            }
            out.push((b, true));
            at = b;
        }
        if at < 1.0 {
            out.push((1.0, false));
        }
        out
    }
}
impl Eq for Domain {}
impl std::hash::Hash for Domain {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.key().hash(h);
    }
}
impl Ord for Domain {
    fn cmp(&self, b: &Self) -> std::cmp::Ordering {
        self.key().cmp(&b.key())
    }
}
impl PartialOrd for Domain {
    fn partial_cmp(&self, b: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(b))
    }
}

/// Adds an interval to sorted ones, joining it to the last if they touch.
fn push(out: &mut Vec<(f64, f64)>, a: f64, b: f64) {
    if a < b {
        match out.last_mut() {
            Some(last) if last.1 >= a => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
}

/// `scale * x + offset`, for the latent `x`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub scale: f64,
    pub offset: f64,
}
impl Affine {
    const IDENTITY: Affine = Affine {
        scale: 1.0,
        offset: 0.0,
    };
    fn constant(c: f64) -> Self {
        Affine { scale: 0.0, offset: c }
    }
    /// Its value where the latent is `x`. A constant is that constant even
    /// at an infinite end of the latent's range.
    pub fn at(self, x: f64) -> f64 {
        if self.scale == 0.0 {
            self.offset
        } else {
            self.scale * x + self.offset
        }
    }
    fn negated(self) -> Self {
        Affine {
            scale: -self.scale,
            offset: -self.offset,
        }
    }
    fn is_finite(self) -> bool {
        self.scale.is_finite() && self.offset.is_finite()
    }
}

/// A function of a latent, by where each of its pieces ends in the latent's
/// CDF coordinates: the first starts at 0, each other where the one before
/// it ends, and the last ends at 1.
pub type Pieces = Vec<(f64, Affine)>;

/// The common refinement of two tilings of the CDF coordinates.
fn overlay<A: Copy, B: Copy>(a: &[(f64, A)], b: &[(f64, B)]) -> Vec<(f64, (A, B))> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len() + b.len() - 1);
    loop {
        let end = a[i].0.min(b[j].0);
        out.push((end, (a[i].1, b[j].1)));
        // Both tilings end at 1, so neither runs out first.
        if end >= 1.0 {
            return out;
        }
        i += usize::from(a[i].0 == end);
        j += usize::from(b[j].0 == end);
    }
}

/// `then` where `yes` holds and `otherwise` elsewhere: the partition that
/// piecewise operations share.
fn select(yes: &Domain, then: &[(f64, Affine)], otherwise: &[(f64, Affine)]) -> Pieces {
    overlay(&overlay(then, otherwise), &yes.mask())
        .into_iter()
        .map(|(end, ((then, otherwise), inside))| (end, if inside { then } else { otherwise }))
        .collect()
}

/// Where a function of a latent is below `t` (or at most `t`), in the
/// latent's CDF coordinates.
fn below(family: &Family, pieces: &[(f64, Affine)], t: f64, strict: bool) -> OpResult<Domain> {
    let mut out = Vec::with_capacity(pieces.len());
    let mut start = 0.0;
    for &(end, f) in pieces {
        let (lo, hi) = if f.scale == 0.0 {
            if f.offset < t || (!strict && f.offset == t) {
                (start, end)
            } else {
                (end, end)
            }
        } else {
            let p = family.cdf((t - f.offset) / f.scale);
            if !p.is_finite() {
                return Err(unsupported("this numerically unstable comparison"));
            }
            if f.scale > 0.0 {
                (start, end.min(p))
            } else {
                (start.max(p), end)
            }
        };
        push(&mut out, lo, hi);
        start = end;
    }
    Ok(Domain(out))
}

#[derive(Clone, Debug)]
pub struct Analytic {
    pub id: u64,
    pub family: Family,
    pub domain: Domain,
    pub pieces: Pieces,
    /// Whether it's an int, as `floor(x)` is: then every piece is a whole
    /// number.
    pub int: bool,
}
impl Analytic {
    pub fn new(id: u64, family: Family) -> Self {
        Self {
            id,
            family,
            domain: Domain::full(),
            pieces: vec![(1.0, Affine::IDENTITY)],
            int: false,
        }
    }
    /// The draw itself.
    fn latent(&self) -> Self {
        self.with(vec![(1.0, Affine::IDENTITY)])
    }
    /// The same function of the same values of the draw, in the coordinates
    /// of `to`, a distribution of the draw with the same support or less.
    pub fn moved(&self, to: &Family) -> Self {
        if self.family == *to {
            return self.clone();
        }
        let last = self.pieces.len() - 1;
        let mut pieces: Pieces = Vec::with_capacity(self.pieces.len());
        for (i, &(end, f)) in self.pieces.iter().enumerate() {
            let end = if i == last { 1.0 } else { moved(end, &self.family, to) };
            // A piece that rounds to nothing has no probability either way.
            if end > pieces.last().map_or(0.0, |p| p.0) {
                pieces.push((end, f));
            }
        }
        match pieces.last_mut() {
            Some(p) => p.0 = 1.0,
            None => pieces.push((1.0, self.pieces[last].1)),
        }
        Self {
            id: self.id,
            family: *to,
            domain: self.domain.moved(&self.family, to),
            pieces,
            int: self.int,
        }
    }
    /// Another function of the same draw, a float.
    fn with(&self, pieces: Pieces) -> Self {
        Self {
            id: self.id,
            family: self.family,
            domain: self.domain.clone(),
            pieces,
            int: false,
        }
    }
    /// Another function of the same draw, an int if `int`.
    fn with_type(&self, pieces: Pieces, int: bool) -> Self {
        Self {
            int,
            ..self.with(pieces)
        }
    }
    /// The function, when it's affine everywhere.
    pub fn affine(&self) -> Option<Affine> {
        match self.pieces[..] {
            [(_, f)] => Some(f),
            _ => None,
        }
    }
    pub fn key(&self) -> impl Ord + std::hash::Hash + use<> {
        (
            self.id,
            family_key(&self.family),
            self.domain.key(),
            self.pieces
                .iter()
                .map(|(end, f)| (float_key(*end), float_key(f.scale), float_key(f.offset)))
                .collect::<Vec<_>>(),
            self.int,
        )
    }
    pub fn value(mut self) -> OpResult<Value> {
        if !self.pieces.iter().all(|(_, f)| f.is_finite()) {
            return Err(unsupported("non-finite affine coefficients"));
        }
        self.simplify();
        match self.affine() {
            Some(f) if f.scale == 0.0 => Ok(number(f.offset, self.int)),
            _ => Ok(Value::Analytic(Arc::new(self))),
        }
    }
    /// Whether each of its values has positive probability: it's a constant
    /// on each part of the domain, like `floor(x)`.
    pub fn is_discrete(&self) -> bool {
        self.pieces.len() > 1 && self.segments().iter().all(|(_, _, f)| f.scale == 0.0)
    }
    /// Each value of a discrete outcome, with where the draw is when it's
    /// that value, and that part's share of the domain's probability.
    pub fn atoms(&self) -> Vec<(Value, Domain, f64)> {
        let mass = self.domain.mass();
        let mut atoms: Vec<(f64, Vec<(f64, f64)>)> = Vec::new();
        for (lo, hi, f) in self.segments() {
            match atoms.iter_mut().find(|(c, _)| float_key(*c) == float_key(f.offset)) {
                Some((_, parts)) => parts.push((lo, hi)),
                None => atoms.push((f.offset, vec![(lo, hi)])),
            }
        }
        atoms.sort_by(|a, b| a.0.total_cmp(&b.0));
        atoms
            .into_iter()
            .map(|(c, parts)| {
                let mut region = Vec::with_capacity(parts.len());
                for (lo, hi) in parts {
                    push(&mut region, lo, hi);
                }
                let region = Domain(region);
                let share = region.mass() / mass;
                (number(c, self.int), region, share)
            })
            .collect()
    }
    /// Gives each piece with no probability in the domain to a neighbour,
    /// since nothing outside the domain is read, then joins neighbours that
    /// are the same function.
    fn simplify(&mut self) {
        if self.pieces.len() == 1 {
            return;
        }
        let d = &self.domain.0;
        let mut out: Pieces = Vec::with_capacity(self.pieces.len());
        let (mut start, mut j) = (0.0, 0);
        for &(end, f) in &self.pieces {
            while j < d.len() && d[j].1 <= start {
                j += 1;
            }
            let live = j < d.len() && d[j].0 < end;
            start = end;
            match out.last_mut() {
                Some(last) if !live || last.1 == f => last.0 = end,
                _ if live => out.push((end, f)),
                // Before the first live piece, which then starts at 0.
                _ => {}
            }
        }
        if !out.is_empty() {
            self.pieces = out;
        }
    }
    /// Each part of the domain, with the piece of the function there, as
    /// `(lo, hi, f)`.
    pub fn segments(&self) -> Vec<(f64, f64, Affine)> {
        let mut out = Vec::with_capacity(self.domain.0.len() + self.pieces.len() - 1);
        let (mut k, mut start) = (0, 0.0);
        for &(a, b) in &self.domain.0 {
            loop {
                let (end, f) = self.pieces[k];
                let (lo, hi) = (a.max(start), b.min(end));
                if lo < hi {
                    out.push((lo, hi, f));
                }
                if end >= b || k + 1 == self.pieces.len() {
                    break;
                }
                start = end;
                k += 1;
            }
        }
        out
    }
    /// Whether a single value has positive probability: a constant piece.
    pub fn has_atoms(&self) -> bool {
        self.pieces.len() > 1 && self.segments().iter().any(|(_, _, f)| f.scale == 0.0)
    }
    pub fn cdf(&self, x: f64) -> f64 {
        let Some(f) = self.affine() else {
            return match below(&self.family, &self.pieces, x, false) {
                Ok(at_most) => (self.domain.intersect(&at_most).mass() / self.domain.mass()).clamp(0.0, 1.0),
                Err(_) => f64::NAN,
            };
        };
        let p = self.family.cdf((x - f.offset) / f.scale);
        let below = self.domain.intersect(&Domain::below(p)).mass() / self.domain.mass();
        (if f.scale > 0.0 { below } else { 1.0 - below }).clamp(0.0, 1.0)
    }
    pub fn pdf(&self, x: f64) -> f64 {
        let mass = self.domain.mass();
        let Some(f) = self.affine() else {
            // Constant pieces are atoms, which the caller rules out.
            return self
                .segments()
                .into_iter()
                .filter(|(_, _, f)| f.scale != 0.0)
                .map(|(lo, hi, f)| {
                    let x = (x - f.offset) / f.scale;
                    let u = self.family.cdf(x);
                    if lo <= u && u <= hi {
                        self.family.pdf(x) / (f.scale.abs() * mass)
                    } else {
                        0.0
                    }
                })
                .sum();
        };
        let x = (x - f.offset) / f.scale;
        let u = self.family.cdf(x);
        if self.domain.0.iter().any(|(a, b)| *a <= u && u <= *b) {
            self.family.pdf(x) / (f.scale.abs() * mass)
        } else {
            0.0
        }
    }
    pub fn quantile(&self, q: f64) -> f64 {
        let Some(f) = self.affine() else {
            return self.piecewise_quantile(q);
        };
        let mut left = q * self.domain.mass();
        let n = self.domain.0.len();
        for i in 0..n {
            let (a, b) = self.domain.0[if f.scale > 0.0 { i } else { n - 1 - i }];
            if left <= b - a || i + 1 == n {
                let u = if f.scale > 0.0 { a + left } else { b - left };
                return f.at(self.family.quantile(u.clamp(a, b)));
            }
            left -= b - a;
        }
        f64::NAN
    }
    /// The quantile of the mixture of the segments lies between the lowest
    /// and the highest of theirs.
    fn piecewise_quantile(&self, q: f64) -> f64 {
        let segments = self.segments();
        let each = |q: f64| {
            segments.iter().map(move |&(lo, hi, f)| {
                if f.scale == 0.0 {
                    return f.offset;
                }
                let u = if f.scale > 0.0 {
                    lo + q * (hi - lo)
                } else {
                    hi - q * (hi - lo)
                };
                f.at(self.family.quantile(u.clamp(lo, hi)))
            })
        };
        let lo = each(q).fold(f64::INFINITY, f64::min);
        let hi = each(q).fold(f64::NEG_INFINITY, f64::max);
        if q <= 0.0 {
            return lo;
        }
        if q >= 1.0 {
            return hi;
        }
        let (lo, hi) = (lo.max(-f64::MAX), hi.min(f64::MAX));
        if lo >= hi || self.cdf(lo) >= q {
            return lo;
        }
        let t = crate::continuous::invert(|t| self.cdf(t), q, lo, hi);
        // Where a single segment takes values around `t`, its own quantile
        // is exact: bisection only gets as close as the CDF rounds.
        let mut needed = q * self.domain.mass();
        let mut around = None;
        for &(lo, hi, f) in &segments {
            let (a, b) = (f.at(self.family.quantile(lo)), f.at(self.family.quantile(hi)));
            if a.max(b) <= t {
                needed -= hi - lo;
            } else if a.min(b) < t {
                if around.is_some() {
                    return t;
                }
                around = Some((lo, hi, f));
            }
        }
        let Some((lo, hi, f)) = around else { return t };
        let u = if f.scale > 0.0 { lo + needed } else { hi - needed };
        let exact = f.at(self.family.quantile(u.clamp(lo, hi)));
        if (exact - t).abs() <= 1e-9 * t.abs().max(1.0) {
            exact
        } else {
            t
        }
    }
    pub fn moments(&self) -> (f64, f64) {
        let Some(f) = self.affine() else {
            let mass = self.domain.mass();
            let parts: Vec<_> = self
                .segments()
                .into_iter()
                .map(|(lo, hi, f)| {
                    let (m, v) = if f.scale == 0.0 {
                        (f.offset, 0.0)
                    } else {
                        let (m, v) = self
                            .family
                            .interval_moments(self.family.quantile(lo), self.family.quantile(hi));
                        (f.at(m), f.scale * f.scale * v)
                    };
                    (m, v, (hi - lo) / mass)
                })
                .collect();
            let mean = parts.iter().map(|(m, _, p)| m * p).sum::<f64>();
            let variance = parts.iter().map(|(m, v, p)| p * (v + (m - mean).powi(2))).sum();
            return (mean, variance);
        };
        let (mean, variance) = if self.domain == Domain::full() {
            (self.family.mean(), self.family.variance())
        } else {
            let parts: Vec<_> = self
                .domain
                .0
                .iter()
                .map(|&(a, b)| {
                    let (m, v) = self
                        .family
                        .interval_moments(self.family.quantile(a), self.family.quantile(b));
                    (m, v, (b - a) / self.domain.mass())
                })
                .collect();
            let mean = parts.iter().map(|(m, _, p)| m * p).sum::<f64>();
            let variance = parts.iter().map(|(m, v, p)| p * (v + (m - mean).powi(2))).sum();
            (mean, variance)
        };
        (f.at(mean), f.scale * f.scale * variance)
    }

    pub fn sd(&self) -> f64 {
        match self.affine() {
            Some(f) if self.domain == Domain::full() => f.scale.abs() * self.family.sd(),
            _ => libm::sqrt(self.moments().1),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub draw: Analytic,
    pub yes: Domain,
}
impl Event {
    pub fn key(&self) -> impl Ord + std::hash::Hash + use<> {
        (self.draw.key(), self.yes.key())
    }
    pub fn probability(&self) -> f64 {
        (self.draw.domain.intersect(&self.yes).mass() / self.draw.domain.mass()).clamp(0.0, 1.0)
    }
    pub fn value(self) -> Value {
        let p = self.probability();
        if p == 0.0 || p == 1.0 {
            Value::Bool(p == 1.0)
        } else {
            Value::Event(Arc::new(self))
        }
    }
    pub fn restrict(&self, yes: bool, context: &mut Constraints) -> f64 {
        let prior = &self.draw.domain;
        let domain = prior.intersect(&if yes { self.yes.clone() } else { self.yes.complement() });
        let p = domain.mass() / prior.mass();
        Arc::make_mut(context).insert(
            self.draw.id,
            Latent {
                family: self.draw.family,
                domain,
            },
        );
        p.clamp(0.0, 1.0)
    }
}

/// An exact update of a draw `x`, read in the world it's updated in, from
/// observing `seen` with `x` itself as the parameter: the logarithm of the
/// observation's probability (a density for a normal), and what the world
/// knows about the draw after it. `None` if the draw's distribution isn't
/// the prior of a conjugate pair with this observation.
///
/// A draw restricted to part of its range stays restricted: its posterior
/// is the updated family on the same values, and the probability includes
/// the ratio of the updated and the current family's mass there.
pub fn update(x: &Analytic, seen: crate::conjugate::Seen) -> OpResult<Option<(f64, Latent)>> {
    use crate::conjugate::Seen;
    if x.affine() != Some(Affine::IDENTITY) {
        return Ok(None);
    }
    // A uniform probability is a beta(1, 1) on part of its range, and an
    // exponential rate a gamma with shape 1.
    let prior = match (x.family, seen) {
        (Family::Uniform { lo, hi }, Seen::Binomial { .. } | Seen::Bernoulli(_)) if lo >= 0.0 && hi <= 1.0 => {
            Family::Beta { a: 1.0, b: 1.0 }
        }
        (Family::Exponential { rate }, Seen::Poisson { .. }) => Family::Gamma {
            shape: 1.0,
            scale: 1.0 / rate,
        },
        (family, _) => family,
    };
    let domain = x.domain.moved(&x.family, &prior);
    let Some((ln, posterior)) = crate::conjugate::update(&prior, seen) else {
        return Ok(None);
    };
    if ln == f64::NEG_INFINITY {
        return Ok(Some((ln, Latent { family: prior, domain })));
    }
    let after = domain.moved(&prior, &posterior);
    if after.mass() == 0.0 {
        // Not impossible: beyond what CDF coordinates can tell apart.
        return Err(OpError::unsupported(
            "this update leaves the draw where its updated distribution has too little probability to represent",
        )
        .help("the draw is restricted to a region far into the tail of its distribution after this observation"));
    }
    let ln = if domain == Domain::full() && after == Domain::full() {
        ln
    } else {
        ln + libm::log(after.mass()) - libm::log(domain.mass())
    };
    Ok(Some((
        ln,
        Latent {
            family: posterior,
            domain: after,
        },
    )))
}

pub fn unsupported(what: &str) -> OpError {
    OpError::unsupported(format!("{what} isn't supported for analytic continuous draws yet"))
        .help("use `@mode sample(runs: 10_000)` for this operation")
}

/// An operand of an operation on outcomes of one draw.
enum Operand<'a> {
    Pieces(&'a [(f64, Affine)]),
    Constant(f64),
}

fn operand<'a>(v: &'a Value, x: &Analytic) -> OpResult<Operand<'a>> {
    match v {
        Value::Analytic(y) if y.id == x.id => Ok(Operand::Pieces(&y.pieces)),
        Value::Analytic(_) => Err(unsupported("combining independent continuous draws")),
        v => v
            .as_f64()
            .filter(|v| v.is_finite())
            .map(Operand::Constant)
            .ok_or_else(|| unsupported("this operand")),
    }
}

/// `f(a, b)` on each part where both operands are affine.
fn zip(a: &Operand, b: &Operand, f: impl Fn(Affine, Affine) -> OpResult<Affine>) -> OpResult<Pieces> {
    match (a, b) {
        (Operand::Pieces(a), Operand::Constant(c)) => a
            .iter()
            .map(|&(end, a)| Ok((end, f(a, Affine::constant(*c))?)))
            .collect(),
        (Operand::Constant(c), Operand::Pieces(b)) => b
            .iter()
            .map(|&(end, b)| Ok((end, f(Affine::constant(*c), b)?)))
            .collect(),
        (Operand::Pieces(a), Operand::Pieces(b)) => overlay(a, b)
            .into_iter()
            .map(|(end, (a, b))| Ok((end, f(a, b)?)))
            .collect(),
        (Operand::Constant(_), Operand::Constant(_)) => unreachable!("an operand is analytic"),
    }
}

fn difference(a: Affine, b: Affine) -> OpResult<Affine> {
    Ok(Affine {
        scale: a.scale - b.scale,
        offset: a.offset - b.offset,
    })
}

/// The most pieces an outcome can have. Repeated folds like
/// `y = abs(2 * y - 1)` double them each time.
const MAX_PIECES: usize = 100_000;

/// Pieces cost work, and their number is limited.
fn spend(pieces: &[(f64, Affine)], budget: &mut Budget) -> OpResult<()> {
    if pieces.len() > MAX_PIECES {
        return Err(OpError::limit(format!(
            "a continuous outcome with more than {MAX_PIECES} pieces is over the limit"
        )));
    }
    if pieces.len() > 1 {
        budget.work(pieces.len() as u64)?;
    }
    Ok(())
}

pub fn binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    use BinOp::*;
    let x = match (a, b) {
        (Value::Analytic(x), _) | (_, Value::Analytic(x)) => x,
        _ => return Err(unsupported("this operation")),
    };
    let (p, q) = (operand(a, x)?, operand(b, x)?);
    let pieces = match op {
        Add => zip(&p, &q, |a, b| {
            Ok(Affine {
                scale: a.scale + b.scale,
                offset: a.offset + b.offset,
            })
        })?,
        Sub => zip(&p, &q, difference)?,
        Mul => zip(&p, &q, |a, b| match (a.scale == 0.0, b.scale == 0.0) {
            (_, true) => Ok(Affine {
                scale: a.scale * b.offset,
                offset: a.offset * b.offset,
            }),
            (true, false) => Ok(Affine {
                scale: b.scale * a.offset,
                offset: b.offset * a.offset,
            }),
            (false, false) => Err(unsupported("nonlinear arithmetic")),
        })?,
        Div => zip(&p, &q, |a, b| {
            if b.scale != 0.0 {
                Err(unsupported("nonlinear arithmetic"))
            } else if b.offset == 0.0 {
                Err(match q {
                    Operand::Constant(_) => OpError::fault(Fault::DivisionByZero, "division by zero"),
                    Operand::Pieces(_) => unsupported("dividing by a value that's zero on part of its domain"),
                })
            } else {
                Ok(Affine {
                    scale: a.scale / b.offset,
                    offset: a.offset / b.offset,
                })
            }
        })?,
        Eq | Ne | Lt | Le | Gt | Ge => {
            let d = zip(&p, &q, difference)?;
            spend(&d, budget)?;
            if !d.iter().all(|(_, f)| f.is_finite()) {
                return Err(unsupported("non-finite affine coefficients"));
            }
            return Ok(compare(x, op, &d)?.value());
        }
        _ => return Err(unsupported("nonlinear arithmetic")),
    };
    spend(&pieces, budget)?;
    // As with numbers, sums, differences and products of ints are ints.
    let int = matches!(op, Add | Sub | Mul) && is_int(a) && is_int(b);
    x.with_type(pieces, int).value()
}

/// The event that a difference `d` of outcomes of `x`'s draw compares with
/// zero by `op`.
fn compare(x: &Analytic, op: BinOp, d: &[(f64, Affine)]) -> OpResult<Event> {
    use BinOp::*;
    let yes = match op {
        Lt => below(&x.family, d, 0.0, true)?,
        Le => below(&x.family, d, 0.0, false)?,
        Gt => below(&x.family, d, 0.0, false)?.complement(),
        Ge => below(&x.family, d, 0.0, true)?.complement(),
        _ => {
            // Only a constant piece can equal zero with positive probability.
            let mut equal = Vec::new();
            let mut start = 0.0;
            for &(end, f) in d {
                if f.scale == 0.0 && f.offset == 0.0 {
                    push(&mut equal, start, end);
                }
                start = end;
            }
            let equal = Domain(equal);
            if op == Eq { equal } else { equal.complement() }
        }
    };
    Ok(Event { draw: x.latent(), yes })
}

pub fn negate(x: &Analytic) -> OpResult<Value> {
    x.with_type(x.pieces.iter().map(|&(end, f)| (end, f.negated())).collect(), x.int)
        .value()
}

pub fn abs(x: &Analytic, budget: &mut Budget) -> OpResult<Value> {
    let negative = below(&x.family, &x.pieces, 0.0, true)?;
    let negated: Pieces = x.pieces.iter().map(|&(end, f)| (end, f.negated())).collect();
    let pieces = select(&negative, &negated, &x.pieces);
    spend(&pieces, budget)?;
    x.with_type(pieces, x.int).value()
}

/// A number as the language types it: an int, or a float.
fn number(c: f64, int: bool) -> Value {
    match probl_number::Integer::from_f64(c).filter(|_| int) {
        Some(n) => Value::Int(n),
        None => Value::Float(c),
    }
}

/// Whether a value is an int, or an outcome that is one.
fn is_int(v: &Value) -> bool {
    match v {
        Value::Int(_) => true,
        Value::Analytic(x) => x.int,
        _ => false,
    }
}

/// How `floor`, `ceil`, `trunc` and `round` make an int of a number.
#[derive(Clone, Copy, Debug)]
pub enum Rounding {
    Floor,
    Ceil,
    Trunc,
    Round,
}
impl Rounding {
    fn apply(self, v: f64) -> f64 {
        match self {
            Rounding::Floor => v.floor(),
            Rounding::Ceil => v.ceil(),
            Rounding::Trunc => libm::trunc(v),
            // Half away from zero, as `round` does.
            Rounding::Round => v.round(),
        }
    }
    /// Where it can change, strictly between `lo` and `hi`: the whole
    /// numbers, or the halves for `round`.
    fn steps(self, lo: f64, hi: f64) -> impl Iterator<Item = f64> {
        let half = if matches!(self, Rounding::Round) { 0.5 } else { 0.0 };
        let first = (lo - half).floor() + 1.0;
        let last = (hi - half).ceil() - 1.0;
        (0..)
            .map(move |i| first + i as f64)
            .take_while(move |k| *k <= last)
            .map(move |k| k + half)
    }
}

/// `floor(x)`, `ceil(x)`, `trunc(x)` or `round(x)`: an int that is constant
/// on each part of the draw where `x` is between two of the rounding's
/// steps, with that part's probability. Where `x` is constant, it rounds as
/// a number does, ties included; elsewhere a step has no probability.
pub fn rounded(x: &Analytic, how: Rounding, budget: &mut Budget) -> OpResult<Value> {
    let mut pieces: Pieces = Vec::new();
    let mut at = 0.0;
    for (lo, hi, f) in x.segments() {
        if lo > at {
            // Outside the domain: anything, which `value` gives away.
            pieces.push((lo, Affine::constant(0.0)));
        }
        at = hi;
        if f.scale == 0.0 {
            pieces.push((hi, Affine::constant(how.apply(f.offset))));
            continue;
        }
        let (a, b) = (f.at(x.family.quantile(lo)), f.at(x.family.quantile(hi)));
        if !a.is_finite() || !b.is_finite() {
            return Err(unsupported("rounding an outcome whose values have no bound"));
        }
        let (vmin, vmax) = (a.min(b), a.max(b));
        let steps: Vec<f64> = how.steps(vmin, vmax).take(MAX_PIECES + 1).collect();
        if pieces.len() + steps.len() > MAX_PIECES {
            return Err(OpError::limit(format!(
                "a continuous outcome with more than {MAX_PIECES} pieces is over the limit"
            )));
        }
        budget.work(steps.len() as u64 + 1)?;
        // Each step, as a coordinate of the draw, in increasing order.
        let mut bounds: Vec<(f64, f64)> = steps
            .iter()
            .map(|&t| (x.family.cdf((t - f.offset) / f.scale).clamp(lo, hi), t))
            .collect();
        if f.scale < 0.0 {
            bounds.reverse();
        }
        let mut from = if f.scale > 0.0 { vmin } else { vmax };
        for (u, t) in bounds {
            pieces.push((u, Affine::constant(how.apply((from + t) / 2.0))));
            from = t;
        }
        let to = if f.scale > 0.0 { vmax } else { vmin };
        pieces.push((hi, Affine::constant(how.apply((from + to) / 2.0))));
    }
    if at < 1.0 {
        pieces.push((1.0, Affine::constant(0.0)));
    }
    // Ends that rounded together leave parts with no probability.
    let mut tiling: Pieces = Vec::with_capacity(pieces.len());
    for (end, f) in pieces {
        match tiling.last_mut() {
            Some(last) if end <= last.0 => {}
            _ => tiling.push((end, f)),
        }
    }
    if let Some(last) = tiling.last_mut() {
        last.0 = 1.0;
    }
    spend(&tiling, budget)?;
    x.with_type(tiling, true).value()
}

/// `min` or `max` of numbers and outcomes of one draw. As with numbers, each
/// argument replaces the best so far where it's strictly better.
pub fn extreme(args: &[Value], want_max: bool, budget: &mut Budget) -> OpResult<Value> {
    let x = args
        .iter()
        .find_map(|v| match v {
            Value::Analytic(x) => Some(x),
            _ => None,
        })
        .expect("an analytic argument");
    let tiling = |v: &Value| -> OpResult<Pieces> {
        Ok(match operand(v, x)? {
            Operand::Pieces(p) => p.to_vec(),
            Operand::Constant(c) => vec![(1.0, Affine::constant(c))],
        })
    };
    let mut best = tiling(&args[0])?;
    for v in &args[1..] {
        let v = tiling(v)?;
        let d = zip(&Operand::Pieces(&v), &Operand::Pieces(&best), difference)?;
        let better = if want_max {
            below(&x.family, &d, 0.0, false)?.complement()
        } else {
            below(&x.family, &d, 0.0, true)?
        };
        best = select(&better, &v, &best);
        spend(&best, budget)?;
    }
    x.with_type(best, args.iter().all(is_int)).value()
}

/// `clamp(x, lo, hi)` with numbers for bounds, which the caller has checked
/// are in order. It's an int if `x` and the bounds are.
pub fn clamp(x: &Analytic, lo: &Value, hi: &Value, budget: &mut Budget) -> OpResult<Value> {
    let int = x.int && is_int(lo) && is_int(hi);
    let (Some(lo), Some(hi)) = (lo.as_f64(), hi.as_f64()) else {
        return Err(unsupported("`clamp` with these bounds"));
    };
    let under = below(&x.family, &x.pieces, lo, true)?;
    let over = below(&x.family, &x.pieces, hi, false)?.complement();
    let inside = select(&over, &[(1.0, Affine::constant(hi))], &x.pieces);
    let pieces = select(&under, &[(1.0, Affine::constant(lo))], &inside);
    spend(&pieces, budget)?;
    x.with_type(pieces, int).value()
}

pub fn logic(and: bool, a: &Value, b: &Value) -> OpResult<Value> {
    let (event, other) = match (a, b) {
        (Value::Event(e), b) | (b, Value::Event(e)) => (e, b),
        _ => return Err(unsupported("this logical operation")),
    };
    let mut e = (**event).clone();
    match other {
        Value::Bool(x) => {
            if *x != and {
                return Ok(Value::Bool(*x));
            }
        }
        Value::Event(other) if e.draw.id == other.draw.id => {
            e.yes = if and {
                e.yes.intersect(&other.yes)
            } else {
                e.yes.complement().intersect(&other.yes.complement()).complement()
            };
        }
        _ => {
            return Err(unsupported(
                "combining this event with another probability or independent draw",
            ));
        }
    }
    Ok(e.value())
}

/// Scan through aggregate values too: symbolic numbers must never become
/// categorical keys or be compared by their internal identity as user data.
pub fn contains(v: &Value) -> bool {
    match v {
        Value::Analytic(_) | Value::Event(_) => return true,
        Value::List(_) | Value::Map(_) | Value::Bag(_) | Value::Record(_) | Value::Dist(_) | Value::Closure(_) => {}
        _ => return false,
    }
    let mut pending = vec![v];
    while let Some(v) = pending.pop() {
        match v {
            Value::Analytic(_) | Value::Event(_) => return true,
            Value::List(v) => pending.extend(v.iter()),
            Value::Map(v) => pending.extend(v.iter().flat_map(|(k, v)| [k, v])),
            Value::Bag(v) => pending.extend(v.keys()),
            Value::Record(v) => pending.extend(v.fields.iter().map(|(_, v)| v)),
            Value::Dist(d) => pending.extend(d.outcomes.iter().map(|(v, _)| v)),
            Value::Closure(c) => pending.extend(c.captured.iter()),
            _ => {}
        }
    }
    false
}

/// Latents reachable through an immutable value (including closure captures).
pub fn collect_ids(v: &Value, ids: &mut std::collections::BTreeSet<u64>) {
    let mut pending = vec![v];
    while let Some(v) = pending.pop() {
        match v {
            Value::Analytic(a) => {
                ids.insert(a.id);
            }
            Value::Event(e) => {
                ids.insert(e.draw.id);
            }
            Value::List(v) => pending.extend(v.iter()),
            Value::Map(v) => pending.extend(v.iter().flat_map(|(k, v)| [k, v])),
            Value::Bag(v) => pending.extend(v.keys()),
            Value::Record(v) => pending.extend(v.fields.iter().map(|(_, v)| v)),
            Value::Dist(d) => pending.extend(d.outcomes.iter().map(|(v, _)| v)),
            Value::Closure(c) => pending.extend(c.captured.iter()),
            _ => {}
        }
    }
}

/// Snapshot a value under this world's posterior without mutating aliases.
pub fn resolve(v: &Value, context: &Constraints, budget: &mut Budget) -> OpResult<Value> {
    if context.is_empty() || !contains(v) {
        return Ok(v.clone());
    }
    resolve_at(v, context, budget, 0)
}
fn resolve_at(v: &Value, c: &Constraints, b: &mut Budget, depth: usize) -> OpResult<Value> {
    b.work(1)?;
    if depth > 64 {
        return Err(OpError::limit("analytic value nesting exceeds the limit of 64"));
    }
    let draw = |x: &Analytic| match c.get(&x.id) {
        Some(l) => {
            let mut x = x.moved(&l.family);
            x.domain = x.domain.intersect(&l.domain);
            x
        }
        None => x.clone(),
    };
    let mut child = |v: &Value| resolve_at(v, c, b, depth + 1);
    Ok(match v {
        Value::Analytic(x) => draw(x).value()?,
        Value::Event(e) => {
            let draw = draw(&e.draw);
            let yes = if draw.family == e.draw.family {
                e.yes.clone()
            } else {
                e.yes.moved(&e.draw.family, &draw.family)
            };
            Event { draw, yes }.value()
        }
        Value::List(xs) => Value::list(xs.iter().map(&mut child).collect::<OpResult<_>>()?),
        Value::Record(r) => crate::ops::make_record(
            r.ty.clone(),
            r.fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), child(v)?)))
                .collect::<OpResult<_>>()?,
        ),
        Value::Map(xs) => Value::map(
            xs.iter()
                .map(|(k, v)| Ok((k.clone(), child(v)?)))
                .collect::<OpResult<_>>()?,
        ),
        Value::Closure(f) => Value::Closure(Arc::new(Closure {
            func: f.func,
            captured: f.captured.iter().map(&mut child).collect::<OpResult<_>>()?,
        })),
        Value::Dist(d) => {
            let pairs = d
                .outcomes
                .iter()
                .map(|(v, p)| Ok((child(v)?, *p)))
                .collect::<OpResult<_>>()?;
            crate::ops::combine(pairs, d.missing, b)?
        }
        // Analytic keys and bags are rejected when constructed.
        _ => v.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(lo: f64, hi: f64) -> Analytic {
        Analytic::new(1, Family::Uniform { lo, hi })
    }
    fn budget() -> Budget {
        Budget::unlimited()
    }
    fn analytic(v: Value) -> Analytic {
        match v {
            Value::Analytic(a) => (*a).clone(),
            v => panic!("expected an analytic value, found {v:?}"),
        }
    }
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} vs {b}");
    }

    #[test]
    fn overlay_refines_both_tilings() {
        let a = [(0.25, 'a'), (1.0, 'b')];
        let b = [(0.5, 1), (0.75, 2), (1.0, 3)];
        assert_eq!(
            overlay(&a, &b),
            vec![(0.25, ('a', 1)), (0.5, ('b', 1)), (0.75, ('b', 2)), (1.0, ('b', 3))]
        );
    }

    #[test]
    fn abs_of_a_symmetric_uniform_is_uniform() {
        // x ~ uniform(-1, 1): |x| ~ uniform(0, 1).
        let y = analytic(abs(&uniform(-1.0, 1.0), &mut budget()).unwrap());
        assert_eq!(y.pieces.len(), 2);
        close(y.cdf(0.3), 0.3);
        close(y.quantile(0.05), 0.05);
        close(y.quantile(0.5), 0.5);
        let (mean, variance) = y.moments();
        close(mean, 0.5);
        close(variance, 1.0 / 12.0);
        close(y.pdf(0.4), 1.0);
        assert!(!y.has_atoms());
    }

    #[test]
    fn min_with_a_number_has_an_atom() {
        // x ~ uniform(0, 2): min(x, 1) is x below 1, and 1 with probability 1/2.
        let x = Value::Analytic(Arc::new(uniform(0.0, 2.0)));
        let y = analytic(extreme(&[x, Value::Float(1.0)], false, &mut budget()).unwrap());
        assert!(y.has_atoms());
        close(y.cdf(0.5), 0.25);
        close(y.cdf(1.0), 1.0);
        close(y.quantile(0.25), 0.5);
        assert_eq!(y.quantile(0.5), 1.0);
        assert_eq!(y.quantile(0.95), 1.0);
        let (mean, variance) = y.moments();
        close(mean, 0.75);
        close(variance, 2.0 / 3.0 - 0.75 * 0.75);
    }

    #[test]
    fn clamp_has_two_atoms() {
        // x ~ uniform(-1, 2): clamp(x, 0, 1) is 0 and 1 with 1/3 each.
        let y = analytic(
            clamp(
                &uniform(-1.0, 2.0),
                &Value::Float(0.0),
                &Value::Float(1.0),
                &mut budget(),
            )
            .unwrap(),
        );
        assert_eq!(y.pieces.len(), 3);
        close(y.cdf(0.0), 1.0 / 3.0);
        close(y.cdf(0.5), 0.5);
        close(y.moments().0, 0.5);
        assert_eq!(y.quantile(0.2), 0.0);
        assert_eq!(y.quantile(0.9), 1.0);
    }

    #[test]
    fn comparisons_of_pieces_are_events_of_the_latent() {
        // |x| < 0.5 for x ~ uniform(-1, 1) is -0.5 < x < 0.5.
        let x = uniform(-1.0, 1.0);
        let y = abs(&x, &mut budget()).unwrap();
        let Value::Event(e) = binary(BinOp::Lt, &y, &Value::Float(0.5), &mut budget()).unwrap() else {
            panic!("expected an event");
        };
        assert_eq!(e.yes, Domain(vec![(0.25, 0.75)]));
        assert_eq!(e.draw.affine(), Some(Affine::IDENTITY));
        // An atom equals its value with its probability.
        let m = extreme(&[Value::Analytic(Arc::new(x)), Value::Float(0.0)], true, &mut budget()).unwrap();
        let Value::Event(e) = binary(BinOp::Eq, &m, &Value::Float(0.0), &mut budget()).unwrap() else {
            panic!("expected an event");
        };
        close(e.probability(), 0.5);
    }

    #[test]
    fn pieces_outside_the_domain_go() {
        // Once x > 0, |x| is x.
        let mut x = uniform(-1.0, 1.0);
        x.domain = Domain(vec![(0.5, 1.0)]);
        let y = analytic(abs(&x, &mut budget()).unwrap());
        assert_eq!(y.pieces, vec![(1.0, Affine::IDENTITY)]);
        // And |x| - x is 0.
        let d = binary(
            BinOp::Sub,
            &Value::Analytic(Arc::new(y)),
            &Value::Analytic(Arc::new(x)),
            &mut budget(),
        )
        .unwrap();
        assert!(matches!(d, Value::Float(z) if z == 0.0), "{d:?}");
    }

    #[test]
    fn the_same_draw_combines_piece_by_piece() {
        // |x| + x is 0 below zero and 2x above.
        let x = uniform(-1.0, 1.0);
        let y = abs(&x, &mut budget()).unwrap();
        let s = analytic(binary(BinOp::Add, &y, &Value::Analytic(Arc::new(x)), &mut budget()).unwrap());
        assert_eq!(
            s.pieces,
            vec![
                (0.5, Affine::constant(0.0)),
                (
                    1.0,
                    Affine {
                        scale: 2.0,
                        offset: 0.0
                    }
                )
            ]
        );
        close(s.moments().0, 0.5);
        // A constant piece times anything is affine; elsewhere it isn't.
        let product = binary(
            BinOp::Mul,
            &Value::Analytic(Arc::new(s.clone())),
            &Value::Float(3.0),
            &mut budget(),
        );
        assert!(product.is_ok());
        let square = binary(
            BinOp::Mul,
            &Value::Analytic(Arc::new(s.clone())),
            &Value::Analytic(Arc::new(s)),
            &mut budget(),
        );
        assert!(square.unwrap_err().message.contains("nonlinear arithmetic"));
    }
}
