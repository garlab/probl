//! Identity-preserving continuous outcomes during enumeration.
//!
//! An outcome is a function of one axis: a latent variable, or a linear
//! form of independent normal latents, as `x + y` is. It's affine on each
//! of a few pieces of the axis's CDF coordinates, which keeps `abs`, `min`,
//! `max` and `clamp` exact. Evidence is a set of intervals in a latent's
//! coordinates, and an event a union of boxes of independent axes' ones.
//! Worlds own their restrictions, and the latent's distribution there: a
//! posterior after an exact update. An observation of several normal
//! latents together makes each the linear form of fresh independent ones,
//! which carries their correlation. A value carries the coordinates of the
//! distribution it was made under, and moves to the world's when it's read.
//! Values and closures remain immutable, and calls return restrictions
//! alongside their results.

use crate::continuous::Family;
use crate::dist::Budget;
use crate::error::{Fault, OpError, OpResult};
use crate::value::{Closure, Value, family_key, float_key};
use probl_syntax::ast::BinOp;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Constraints = Arc<BTreeMap<u64, Latent>>;

/// What a world knows about a latent: its distribution, and where it can
/// be, in that distribution's CDF coordinates. After an observation of
/// several normal latents together, also the linear form of fresh ones that
/// it is now: its distribution is then that form's, and it's unrestricted.
#[derive(Clone, Debug, PartialEq)]
pub struct Latent {
    pub family: Family,
    pub domain: Domain,
    pub form: Option<Arc<Linear>>,
}
impl Eq for Latent {}
impl std::hash::Hash for Latent {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        family_key(&self.family).hash(h);
        self.domain.key().hash(h);
        self.form.as_ref().map(|f| f.key()).hash(h);
    }
}

/// One of the independent normal latents of a linear form: its id, its
/// coefficient, and its distribution where the form was made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Term {
    pub id: u64,
    pub coef: f64,
    pub family: Family,
}

/// `constant + Σ coef × latent` over independent normal latents, by
/// increasing id, none with a zero coefficient.
#[derive(Clone, Debug, PartialEq)]
pub struct Linear {
    pub constant: f64,
    pub terms: Vec<Term>,
}
type FormKey = (u64, Vec<(u64, u64, (&'static str, Vec<u64>))>);
impl Linear {
    fn key(&self) -> FormKey {
        (
            float_key(self.constant),
            self.terms
                .iter()
                .map(|t| (t.id, float_key(t.coef), family_key(&t.family)))
                .collect(),
        )
    }
    /// The sum of a constant and terms in any order: those of one latent
    /// add up, and zero coefficients go.
    fn from_terms(constant: f64, mut terms: Vec<Term>) -> OpResult<Linear> {
        terms.sort_by_key(|t| t.id);
        let mut out: Vec<Term> = Vec::with_capacity(terms.len());
        for t in terms {
            match out.last_mut() {
                Some(last) if last.id == t.id => {
                    if last.family != t.family {
                        return Err(OpError::internal(
                            "internal error: a latent with two distributions in one sum",
                        ));
                    }
                    last.coef += t.coef;
                }
                _ => out.push(t),
            }
        }
        out.retain(|t| t.coef != 0.0);
        Ok(Linear { constant, terms: out })
    }
    /// `a` of it: `a.scale × self + a.offset`.
    fn affine(&self, a: Affine) -> Linear {
        if a.scale == 0.0 {
            return Linear {
                constant: a.offset,
                terms: Vec::new(),
            };
        }
        Linear {
            constant: a.scale * self.constant + a.offset,
            terms: self
                .terms
                .iter()
                .map(|t| Term {
                    coef: a.scale * t.coef,
                    ..*t
                })
                .collect(),
        }
    }
    /// `self + k × other`.
    fn plus(&self, other: &Linear, k: f64) -> OpResult<Linear> {
        let terms = self
            .terms
            .iter()
            .copied()
            .chain(other.terms.iter().map(|t| Term { coef: k * t.coef, ..*t }))
            .collect();
        Linear::from_terms(self.constant + k * other.constant, terms)
    }
    /// Its mean and variance.
    fn moments(&self) -> (f64, f64) {
        let mean = self.terms.iter().map(|t| t.coef * t.family.mean()).sum::<f64>();
        let variance = self.terms.iter().map(|t| t.coef * t.coef * t.family.variance()).sum();
        (self.constant + mean, variance)
    }
    /// Its distribution, a normal one: unsupported where it isn't finite.
    fn normal(&self) -> OpResult<Family> {
        let (mean, variance) = self.moments();
        Family::normal(mean, libm::sqrt(variance))
            .map_err(|_| unsupported("a sum of normal draws this large or this small"))
    }
    /// The axis of a form of several latents, and the form as an affine
    /// function of it: the axis is the form without its constant, scaled so
    /// that its first coefficient is 1 or −1.
    fn axis(&self) -> (Arc<Linear>, Affine) {
        let k = self.terms[0].coef.abs();
        let axis = Linear {
            constant: 0.0,
            terms: self.terms.iter().map(|t| Term { coef: t.coef / k, ..*t }).collect(),
        };
        (
            Arc::new(axis),
            Affine {
                scale: k,
                offset: self.constant,
            },
        )
    }
    /// The outcome it is: a number without latents, a function of the
    /// latent with one, and otherwise a function of its axis.
    pub fn value(self) -> OpResult<Value> {
        match self.terms[..] {
            [] => Ok(Value::Float(self.constant)),
            [t] => Analytic {
                id: t.id,
                family: t.family,
                domain: Domain::full(),
                pieces: vec![(
                    1.0,
                    Fun::affine(Affine {
                        scale: t.coef,
                        offset: self.constant,
                    }),
                )],
                int: false,
                form: None,
            }
            .value(),
            _ => {
                let (axis, map) = self.axis();
                Analytic {
                    id: 0,
                    family: axis.normal()?,
                    domain: Domain::full(),
                    pieces: vec![(1.0, Fun::affine(map))],
                    int: false,
                    form: Some(axis),
                }
                .value()
            }
        }
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
    pub fn is_full(&self) -> bool {
        self.0[..] == [(0.0, 1.0)]
    }
    pub fn union(&self, other: &Self) -> Self {
        self.complement().intersect(&other.complement()).complement()
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
    pub const IDENTITY: Affine = Affine {
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
    /// The `x` where it's `y`, for a nonconstant one.
    fn inverse(self, y: f64) -> f64 {
        (y - self.offset) / self.scale
    }
    /// It of `inner`.
    fn of(self, inner: Affine) -> Affine {
        Affine {
            scale: self.scale * inner.scale,
            offset: self.scale * inner.offset + self.offset,
        }
    }
}

/// How a function of one axis becomes a function of another, when the old
/// axis is `map` of the new one, and their distributions are `from` and
/// `to`: the same values of the old axis, in the new axis's coordinates.
#[derive(Clone, Copy, Debug)]
struct Transport {
    from: Family,
    to: Family,
    map: Affine,
}
impl Transport {
    /// The same axis under another distribution.
    fn moved(from: Family, to: Family) -> Transport {
        Transport {
            from,
            to,
            map: Affine::IDENTITY,
        }
    }
    fn coordinate(&self, c: f64) -> f64 {
        self.to.cdf(self.map.inverse(self.from.quantile(c))).clamp(0.0, 1.0)
    }
    fn increasing(&self) -> bool {
        self.map.scale > 0.0
    }
    fn domain(&self, d: &Domain) -> Domain {
        let mut out = Vec::with_capacity(d.0.len());
        if self.increasing() {
            for &(a, b) in &d.0 {
                push(&mut out, self.coordinate(a), self.coordinate(b));
            }
        } else {
            for &(a, b) in d.0.iter().rev() {
                push(&mut out, self.coordinate(b), self.coordinate(a));
            }
        }
        Domain(out)
    }
    /// A piece's function of the old axis, as one of the new.
    fn fun(&self, f: Fun) -> Fun {
        match f.kernel {
            Kernel::Id => Fun::affine(f.outer.of(self.map)),
            _ => Fun {
                inner: f.inner.of(self.map),
                ..f
            },
        }
    }
    fn pieces(&self, pieces: &[(f64, Fun)]) -> Pieces {
        let n = pieces.len();
        let mut out: Pieces = Vec::with_capacity(n);
        let mut add = |end: f64, f: Fun| {
            // A piece that rounds to nothing has no probability either way.
            if end > out.last().map_or(0.0, |p| p.0) {
                out.push((end, self.fun(f)));
            }
        };
        if self.increasing() {
            for (i, &(end, f)) in pieces.iter().enumerate() {
                add(if i + 1 == n { 1.0 } else { self.coordinate(end) }, f);
            }
        } else {
            // In reverse: a piece ends where it started.
            for i in (0..n).rev() {
                add(if i == 0 { 1.0 } else { self.coordinate(pieces[i - 1].0) }, pieces[i].1);
            }
        }
        match out.last_mut() {
            Some(p) => p.0 = 1.0,
            None => out.push((1.0, self.fun(pieces[n - 1].1))),
        }
        out
    }
}

/// A monotone function of a number, between two affine ones in a piece's
/// function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    Id,
    Square,
    Sqrt,
    Exp,
    Ln,
    Recip,
}
impl Kernel {
    /// Its value at `z`, with `z` clamped into where it's defined: on a part
    /// of the draw without probability, `sqrt` may meet a negative number.
    fn apply(self, z: f64) -> f64 {
        match self {
            Kernel::Id => z,
            Kernel::Square => z * z,
            Kernel::Sqrt => z.max(0.0).sqrt(),
            Kernel::Exp => crate::math::exp(z),
            Kernel::Ln => libm::log(z.max(0.0)),
            Kernel::Recip => 1.0 / z,
        }
    }
    /// The `z` on the side of 0 that `side` is on where it's `y`, for a `y`
    /// it takes there.
    fn inverse(self, y: f64, side: f64) -> f64 {
        match self {
            Kernel::Id => y,
            Kernel::Square if side < 0.0 => -y.sqrt(),
            Kernel::Square => y.sqrt(),
            Kernel::Sqrt => y * y,
            Kernel::Exp => libm::log(y),
            Kernel::Ln => crate::math::exp(y),
            Kernel::Recip => 1.0 / y,
        }
    }
    /// Whether it increases on the side of 0 that `side` is on.
    fn increasing(self, side: f64) -> bool {
        match self {
            Kernel::Square => side >= 0.0,
            Kernel::Recip => false,
            _ => true,
        }
    }
    /// Whether it turns, or has a pole, at 0: a piece mustn't span it.
    fn turns(self) -> bool {
        matches!(self, Kernel::Square | Kernel::Recip)
    }
}

/// The function an outcome is of the draw on a piece of it: `outer`, of
/// `kernel`, of `inner`, of the draw's value. Monotone on its piece. An
/// affine one has the identity kernel and inner function, so that `outer`
/// is all of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fun {
    pub outer: Affine,
    pub kernel: Kernel,
    pub inner: Affine,
}
impl Fun {
    pub const IDENTITY: Fun = Fun {
        outer: Affine::IDENTITY,
        kernel: Kernel::Id,
        inner: Affine::IDENTITY,
    };
    pub fn affine(outer: Affine) -> Fun {
        Fun { outer, ..Fun::IDENTITY }
    }
    fn constant(c: f64) -> Fun {
        Fun::affine(Affine::constant(c))
    }
    /// `kernel` of an affine function of the draw.
    fn of(kernel: Kernel, inner: Affine) -> Fun {
        Fun {
            outer: Affine::IDENTITY,
            kernel,
            inner,
        }
    }
    /// The affine function it is, if it is one.
    pub fn as_affine(self) -> Option<Affine> {
        (self.kernel == Kernel::Id).then_some(self.outer)
    }
    /// The number it is, if it's constant.
    pub fn as_constant(self) -> Option<f64> {
        (self.outer.scale == 0.0).then_some(self.outer.offset)
    }
    /// Its value where the draw is `x`.
    pub fn at(self, x: f64) -> f64 {
        match self.kernel {
            Kernel::Id => self.outer.at(x),
            k => self.outer.at(k.apply(self.inner.at(x))),
        }
    }
    /// `scale * self + offset`.
    fn then(self, scale: f64, offset: f64) -> Fun {
        Fun {
            outer: Affine {
                scale: self.outer.scale * scale,
                offset: self.outer.offset * scale + offset,
            },
            ..self
        }
    }
    fn negated(self) -> Fun {
        Fun {
            outer: self.outer.negated(),
            ..self
        }
    }
    fn is_finite(self) -> bool {
        self.outer.is_finite() && self.inner.is_finite()
    }
    fn key(self) -> (u8, u64, u64, u64, u64) {
        (
            self.kernel as u8,
            float_key(self.outer.scale),
            float_key(self.outer.offset),
            float_key(self.inner.scale),
            float_key(self.inner.offset),
        )
    }
    /// The sign of the kernel's input on a piece, which doesn't change there.
    fn side(self, family: &Family, start: f64, end: f64) -> f64 {
        self.inner.at(family.quantile((start + end) / 2.0))
    }
    /// Its values at the ends of a piece. At a pole, the kernel's input is
    /// zero of the piece's sign: `1 / x` falls to −∞ from below 0.
    pub fn ends(self, family: &Family, start: f64, end: f64) -> (f64, f64) {
        let side = match self.kernel {
            Kernel::Recip => self.side(family, start, end),
            _ => 1.0,
        };
        let at = |u: f64| {
            let x = family.quantile(u);
            match self.kernel {
                Kernel::Id => self.outer.at(x),
                k => {
                    let z = self.inner.at(x);
                    let z = if z == 0.0 { 0.0f64.copysign(side) } else { z };
                    self.outer.at(k.apply(z))
                }
            }
        };
        (at(start), at(end))
    }
    /// Whether it increases with the draw on a piece.
    fn increasing(self, family: &Family, start: f64, end: f64) -> bool {
        match self.kernel {
            Kernel::Id => self.outer.scale > 0.0,
            k => {
                let side = self.side(family, start, end);
                (self.outer.scale > 0.0) == (k.increasing(side) == (self.inner.scale > 0.0))
            }
        }
    }
    /// The draw's coordinate on a piece where it's `t`, for a value it takes
    /// there.
    fn coordinate(self, family: &Family, start: f64, end: f64, t: f64) -> f64 {
        match self.kernel {
            Kernel::Id => family.cdf(self.outer.inverse(t)),
            k => {
                let z = k.inverse(self.outer.inverse(t), self.side(family, start, end));
                family.cdf(self.inner.inverse(z))
            }
        }
    }
    /// The part of a piece of the draw where it's below `t`, or at most `t`
    /// unless `strict`.
    fn below_on(self, family: &Family, start: f64, end: f64, t: f64, strict: bool) -> OpResult<(f64, f64)> {
        if let Some(c) = self.as_constant() {
            return Ok(if c < t || (!strict && c == t) {
                (start, end)
            } else {
                (end, end)
            });
        }
        if let Some(f) = self.as_affine() {
            let p = family.cdf(f.inverse(t));
            if !p.is_finite() {
                return Err(unsupported("this numerically unstable comparison"));
            }
            return Ok(if f.scale > 0.0 {
                (start, end.min(p))
            } else {
                (start.max(p), end)
            });
        }
        let up = self.increasing(family, start, end);
        let (a, b) = self.ends(family, start, end);
        let (low, high) = if up { (a, b) } else { (b, a) };
        if t <= low {
            return Ok((end, end));
        }
        if t >= high {
            return Ok((start, end));
        }
        let u = self.coordinate(family, start, end, t);
        if !u.is_finite() {
            return Err(unsupported("this numerically unstable comparison"));
        }
        let u = u.clamp(start, end);
        Ok(if up { (start, u) } else { (u, end) })
    }
    /// The coefficients of the polynomial in the draw it is, if it's one of
    /// degree 2 or less.
    fn polynomial(self) -> Option<[f64; 3]> {
        let Affine { scale: s, offset: o } = self.outer;
        match self.kernel {
            Kernel::Id => Some([o, s, 0.0]),
            Kernel::Square => {
                let Affine { scale: a, offset: b } = self.inner;
                Some([s * b * b + o, 2.0 * s * a * b, s * a * a])
            }
            _ => None,
        }
    }
    /// The function a polynomial of degree 2 or less is: a square about its
    /// vertex, unless it's affine.
    fn from_polynomial([c, b, a]: [f64; 3]) -> Fun {
        if a == 0.0 {
            return Fun::affine(Affine { scale: b, offset: c });
        }
        let vertex = -b / (2.0 * a);
        Fun {
            outer: Affine {
                scale: a,
                offset: c - a * vertex * vertex,
            },
            kernel: Kernel::Square,
            inner: Affine {
                scale: 1.0,
                offset: -vertex,
            },
        }
    }
}

/// A function of a latent, by where each of its pieces ends in the latent's
/// CDF coordinates: the first starts at 0, each other where the one before
/// it ends, and the last ends at 1.
pub type Pieces = Vec<(f64, Fun)>;

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
fn select(yes: &Domain, then: &[(f64, Fun)], otherwise: &[(f64, Fun)]) -> Pieces {
    overlay(&overlay(then, otherwise), &yes.mask())
        .into_iter()
        .map(|(end, ((then, otherwise), inside))| (end, if inside { then } else { otherwise }))
        .collect()
}

/// Where a function of a latent is below `t` (or at most `t`), in the
/// latent's CDF coordinates.
fn below(family: &Family, pieces: &[(f64, Fun)], t: f64, strict: bool) -> OpResult<Domain> {
    let mut out = Vec::with_capacity(pieces.len());
    let mut start = 0.0;
    for &(end, f) in pieces {
        let (lo, hi) = f.below_on(family, start, end, t, strict)?;
        push(&mut out, lo, hi);
        start = end;
    }
    Ok(Domain(out))
}

/// Splits each piece whose kernel turns, or has a pole, where it does, so
/// that every piece is monotone.
fn split_turns(family: &Family, pieces: Pieces) -> Pieces {
    if pieces.iter().all(|(_, f)| !f.kernel.turns()) {
        return pieces;
    }
    let mut out = Vec::with_capacity(pieces.len() + 1);
    let mut start = 0.0;
    for (end, f) in pieces {
        if f.kernel.turns() && f.as_constant().is_none() {
            let u = family.cdf(f.inner.inverse(0.0));
            if start < u && u < end {
                out.push((u, f));
            }
        }
        out.push((end, f));
        start = end;
    }
    out
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
    /// For a function of several normal latents, its axis: a linear form
    /// of them, whose distribution is `family`. `id` is then 0.
    pub form: Option<Arc<Linear>>,
}
impl Analytic {
    pub fn new(id: u64, family: Family) -> Self {
        Self {
            id,
            family,
            domain: Domain::full(),
            pieces: vec![(1.0, Fun::IDENTITY)],
            int: false,
            form: None,
        }
    }
    /// The axis itself: the draw, or the form.
    pub(crate) fn latent(&self) -> Self {
        self.with(vec![(1.0, Fun::IDENTITY)])
    }
    /// Whether it's a function of the same axis as `other`.
    pub fn same_axis(&self, other: &Analytic) -> bool {
        match (&self.form, &other.form) {
            (None, None) => self.id == other.id,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        }
    }
    /// Whether it depends on one of the latents that `other` does: then the
    /// two aren't independent.
    fn shares_latents(&self, other: &Analytic) -> bool {
        let mut mine = std::collections::BTreeSet::new();
        self.collect_ids(&mut mine);
        let mut theirs = std::collections::BTreeSet::new();
        other.collect_ids(&mut theirs);
        !mine.is_disjoint(&theirs)
    }
    fn collect_ids(&self, ids: &mut std::collections::BTreeSet<u64>) {
        match &self.form {
            Some(f) => ids.extend(f.terms.iter().map(|t| t.id)),
            None => {
                ids.insert(self.id);
            }
        }
    }
    /// The linear form of independent normal latents it is, if it's one:
    /// affine in a normal latent or a form, and unrestricted.
    pub fn linear(&self) -> Option<Linear> {
        let a = self.affine()?;
        if !self.domain.is_full() {
            return None;
        }
        match &self.form {
            Some(f) => Some(f.affine(a)),
            None if matches!(self.family, Family::Normal { .. }) => Some(
                Linear {
                    constant: 0.0,
                    terms: vec![Term {
                        id: self.id,
                        coef: 1.0,
                        family: self.family,
                    }],
                }
                .affine(a),
            ),
            None => None,
        }
    }
    /// The same outcome of other latents: each as `map` renames it.
    pub fn renamed(&self, map: &BTreeMap<u64, u64>) -> Self {
        let rename = |id: u64| *map.get(&id).unwrap_or(&id);
        let mut x = self.clone();
        match &self.form {
            Some(f) => {
                x.form = Some(Arc::new(Linear {
                    constant: f.constant,
                    terms: f.terms.iter().map(|t| Term { id: rename(t.id), ..*t }).collect(),
                }))
            }
            None => x.id = rename(self.id),
        }
        x
    }
    /// Its value where its axis is `value`, at the CDF coordinate `c`.
    pub fn at(&self, c: f64, value: f64) -> Value {
        let f = self
            .pieces
            .iter()
            .find(|(end, _)| c <= *end)
            .unwrap_or(&self.pieces[self.pieces.len() - 1])
            .1;
        number(f.at(value), self.int)
    }
    /// The same function of the same values, as a function of another
    /// axis, `id` or `form`, that `t` moves it to.
    fn rebased(&self, id: u64, form: Option<Arc<Linear>>, t: &Transport) -> Self {
        Self {
            id,
            family: t.to,
            domain: t.domain(&self.domain),
            pieces: t.pieces(&self.pieces),
            int: self.int,
            form,
        }
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
            form: self.form.clone(),
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
            form: self.form.clone(),
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
            [(_, f)] => f.as_affine(),
            _ => None,
        }
    }
    /// Whether it's affine on each piece: then it has a density where it
    /// isn't constant.
    pub fn piecewise_affine(&self) -> bool {
        self.pieces.iter().all(|(_, f)| f.as_affine().is_some())
    }
    pub fn key(&self) -> impl Ord + std::hash::Hash + use<> {
        (
            self.id,
            family_key(&self.family),
            self.domain.key(),
            self.pieces
                .iter()
                .map(|(end, f)| (float_key(*end), f.key()))
                .collect::<Vec<_>>(),
            self.int,
            self.form.as_ref().map(|f| f.key()),
        )
    }
    pub fn value(mut self) -> OpResult<Value> {
        if !self.pieces.iter().all(|(_, f)| f.is_finite()) {
            return Err(unsupported("non-finite affine coefficients"));
        }
        self.simplify();
        match self.pieces[..] {
            [(_, f)] if f.as_constant().is_some() => Ok(number(f.outer.offset, self.int)),
            _ => Ok(Value::Analytic(Arc::new(self))),
        }
    }
    /// Whether each of its values has positive probability: it's a constant
    /// on each part of the domain, like `floor(x)`.
    pub fn is_discrete(&self) -> bool {
        self.pieces.len() > 1 && self.segments().iter().all(|(_, _, f)| f.as_constant().is_some())
    }
    /// Each value of a discrete outcome, with where the draw is when it's
    /// that value, and that part's share of the domain's probability.
    pub fn atoms(&self) -> Vec<(Value, Domain, f64)> {
        let mass = self.domain.mass();
        let mut atoms: Vec<(f64, Vec<(f64, f64)>)> = Vec::new();
        for (lo, hi, f) in self.segments() {
            let c = f.outer.offset;
            match atoms.iter_mut().find(|(d, _)| float_key(*d) == float_key(c)) {
                Some((_, parts)) => parts.push((lo, hi)),
                None => atoms.push((c, vec![(lo, hi)])),
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
    /// are the same function. Only an affine function, monotone everywhere,
    /// stretches over another piece: one whose kernel turns stays on its
    /// side.
    fn simplify(&mut self) {
        if self.pieces.len() == 1 {
            return;
        }
        let affine = |g: Fun| g.as_affine().is_some();
        let d = &self.domain.0;
        let mut out: Pieces = Vec::with_capacity(self.pieces.len());
        let mut any_live = false;
        let (mut start, mut j) = (0.0, 0);
        for &(end, f) in &self.pieces {
            while j < d.len() && d[j].1 <= start {
                j += 1;
            }
            let live = j < d.len() && d[j].0 < end;
            start = end;
            if !live {
                match out.last_mut() {
                    Some(last) if affine(last.1) => last.0 = end,
                    _ => out.push((end, f)),
                }
                continue;
            }
            if !any_live && affine(f) {
                // A first live piece that's affine starts at 0.
                out.clear();
            }
            any_live = true;
            match out.last_mut() {
                Some(last) if last.1 == f && !f.kernel.turns() => last.0 = end,
                _ => out.push((end, f)),
            }
        }
        if any_live {
            self.pieces = out;
        }
    }
    /// Each part of the domain, with the piece of the function there, as
    /// `(lo, hi, f)`.
    pub fn segments(&self) -> Vec<(f64, f64, Fun)> {
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
        self.pieces.len() > 1 && self.segments().iter().any(|(_, _, f)| f.as_constant().is_some())
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
    /// The density at `x`, of a piecewise affine outcome: the caller rules
    /// out atoms and other functions.
    pub fn pdf(&self, x: f64) -> f64 {
        let mass = self.domain.mass();
        let Some(f) = self.affine() else {
            return self
                .segments()
                .into_iter()
                .filter_map(|(lo, hi, f)| Some((lo, hi, f.as_affine().filter(|f| f.scale != 0.0)?)))
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
        let family = &self.family;
        let segments = self.segments();
        // A segment's value `q` of the way through its probability.
        let within = |lo: f64, hi: f64, f: Fun, q: f64| {
            if let Some(c) = f.as_constant() {
                return c;
            }
            let up = f.increasing(family, lo, hi);
            if q <= 0.0 || q >= 1.0 {
                let (a, b) = f.ends(family, lo, hi);
                return if (q <= 0.0) == up { a } else { b };
            }
            let u = if up { lo + q * (hi - lo) } else { hi - q * (hi - lo) };
            f.at(family.quantile(u.clamp(lo, hi)))
        };
        let each = |q: f64| segments.iter().map(move |&(lo, hi, f)| within(lo, hi, f, q));
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
            let (a, b) = f.ends(family, lo, hi);
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
        let exact = within(lo, hi, f, (needed / (hi - lo)).clamp(0.0, 1.0));
        if (exact - t).abs() <= 1e-9 * t.abs().max(1.0) {
            exact
        } else {
            t
        }
    }
    /// The mean and variance, from formulas: `None` where a piece's function
    /// has none for the draw's family.
    pub fn moments(&self) -> Option<(f64, f64)> {
        let Some(f) = self.affine() else {
            let mass = self.domain.mass();
            let parts: Vec<_> = self
                .segments()
                .into_iter()
                .map(|(lo, hi, f)| Some((segment_moments(&self.family, f, lo, hi)?, (hi - lo) / mass)))
                .collect::<Option<_>>()?;
            let mean = parts.iter().map(|((m, _), p)| m * p).sum::<f64>();
            let variance = parts.iter().map(|((m, v), p)| p * (v + (m - mean).powi(2))).sum();
            return Some((mean, variance));
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
        Some((f.at(mean), f.scale * f.scale * variance))
    }

    pub fn sd(&self) -> Option<f64> {
        match self.affine() {
            Some(f) if self.domain == Domain::full() => Some(f.scale.abs() * self.family.sd()),
            _ => self.moments().map(|(_, v)| libm::sqrt(v)),
        }
    }
}

/// The mean and variance of a piece's function of the draw, where the draw
/// is between the coordinates `lo` and `hi`: `None` where there's no
/// formula for that function and the draw's family.
fn segment_moments(family: &Family, f: Fun, lo: f64, hi: f64) -> Option<(f64, f64)> {
    if let Some(c) = f.as_constant() {
        return Some((c, 0.0));
    }
    let (first, second) = match f.kernel {
        Kernel::Id => {
            let (m, v) = family.interval_moments(family.quantile(lo), family.quantile(hi));
            return Some((f.outer.at(m), f.outer.scale * f.outer.scale * v));
        }
        Kernel::Square => (
            moments::power(family, f.inner, 2.0, lo, hi)?,
            moments::power(family, f.inner, 4.0, lo, hi)?,
        ),
        Kernel::Sqrt => (
            moments::power(family, f.inner, 0.5, lo, hi)?,
            moments::power(family, f.inner, 1.0, lo, hi)?,
        ),
        Kernel::Recip => (
            moments::power(family, f.inner, -1.0, lo, hi)?,
            moments::power(family, f.inner, -2.0, lo, hi)?,
        ),
        Kernel::Exp => (
            moments::exp(family, f.inner, 1.0, lo, hi)?,
            moments::exp(family, f.inner, 2.0, lo, hi)?,
        ),
        Kernel::Ln => {
            return moments::ln(family, f.inner, lo, hi).map(|(m, v)| (f.outer.at(m), f.outer.scale.powi(2) * v));
        }
    };
    let variance = (second - first * first).max(0.0);
    let (mean, variance) = (f.outer.at(first), f.outer.scale * f.outer.scale * variance);
    (mean.is_finite() && variance.is_finite()).then_some((mean, variance))
}

/// Moments of functions of a draw restricted to an interval of its CDF
/// coordinates, `lo` to `hi`, from formulas. `None` where a family has none
/// for that function, or the moment isn't finite.
mod moments {
    use super::Affine;
    use crate::continuous::{Family, beta_cdf, gamma_cdf, ln_beta, std_normal_cdf, std_normal_quantile};

    fn finite(x: f64) -> Option<f64> {
        x.is_finite().then_some(x)
    }

    fn std_normal_pdf(z: f64) -> f64 {
        if z.is_infinite() {
            0.0
        } else {
            (-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt()
        }
    }

    /// E[Y^k], k = 0 to `n`, for a normal Y with mean `m` and sd `t`
    /// between `a` and `b`, whose probability there is `mass`: by the
    /// recurrence E[Y^k] = m E[Y^(k−1)] + (k−1) t² E[Y^(k−2)]
    /// − t (b^(k−1) φ(β) − a^(k−1) φ(α)) / mass.
    fn normal_powers(m: f64, t: f64, a: f64, b: f64, mass: f64, n: usize) -> Vec<f64> {
        let (alpha, beta) = ((a - m) / t, (b - m) / t);
        let (fa, fb) = (std_normal_pdf(alpha), std_normal_pdf(beta));
        // An infinite end contributes nothing: φ falls faster than any power.
        let edge = |x: f64, f: f64, k: usize| if f == 0.0 { 0.0 } else { x.powi(k as i32) * f };
        let mut out = vec![1.0];
        for k in 1..=n {
            let previous = out[k - 1];
            let before = if k >= 2 { out[k - 2] } else { 0.0 };
            let boundary = edge(b, fb, k - 1) - edge(a, fa, k - 1);
            out.push(m * previous + (k as f64 - 1.0) * t * t * before - t * boundary / mass);
        }
        out
    }

    /// E[(s X + o)^p] for the draw X.
    pub fn power(family: &Family, inner: Affine, p: f64, lo: f64, hi: f64) -> Option<f64> {
        let (a, b) = (family.quantile(lo), family.quantile(hi));
        let mass = hi - lo;
        let Affine { scale: s, offset: o } = inner;
        match *family {
            Family::Uniform { .. } => {
                // Y = s X + o is uniform between its values at the ends.
                let (ya, yb) = (inner.at(a), inner.at(b));
                let (y0, y1) = (ya.min(yb), ya.max(yb));
                uniform_power(y0, y1, p)
            }
            Family::Normal { mean, sd } if p.fract() == 0.0 && p >= 0.0 => {
                // Y = s X + o is normal; truncated where X is.
                let (ya, yb) = (inner.at(a), inner.at(b));
                let (y0, y1) = (ya.min(yb), ya.max(yb));
                let powers = normal_powers(s * mean + o, s.abs() * sd, y0, y1, mass, p as usize);
                finite(powers[p as usize])
            }
            _ if o == 0.0 && s > 0.0 => finite(s.powf(p) * positive_power(family, p, lo, hi)?),
            // A whole power of s X + o, from those of X.
            _ if p.fract() == 0.0 && p >= 0.0 => {
                let n = p as i32;
                let mut total = 0.0;
                for j in 0..=n {
                    let choose = (0..j).fold(1.0, |c, i| c * (n - i) as f64 / (i + 1) as f64);
                    let xj = if j == 0 {
                        1.0
                    } else {
                        positive_power(family, j as f64, lo, hi)?
                    };
                    total += choose * s.powi(j) * o.powi(n - j) * xj;
                }
                finite(total)
            }
            _ => None,
        }
    }

    /// E[Y^p] for Y uniform from `y0` to `y1`.
    fn uniform_power(y0: f64, y1: f64, p: f64) -> Option<f64> {
        // A real power is only taken where Y ≥ 0, which a coordinate's
        // rounding can put a hair below.
        let y0 = if p.fract() != 0.0 { y0.max(0.0) } else { y0 };
        if y1 <= y0 {
            return finite(y0.powf(p));
        }
        // Real powers need Y ≥ 0; negative ones, Y away from 0.
        if p.fract() != 0.0 && y0 < 0.0 || p < 0.0 && y0 <= 0.0 && y1 >= 0.0 {
            return None;
        }
        let integral = if p == -1.0 {
            libm::log(y1.abs()) - libm::log(y0.abs())
        } else {
            (y1.powf(p + 1.0) - y0.powf(p + 1.0)) / (p + 1.0)
        };
        finite(integral / (y1 - y0))
    }

    /// E[X^p] for the draw X of a family on positive numbers.
    fn positive_power(family: &Family, p: f64, lo: f64, hi: f64) -> Option<f64> {
        let mass = hi - lo;
        let (a, b) = (family.quantile(lo), family.quantile(hi));
        let value = match *family {
            Family::Lognormal { mu, sigma } => {
                // X^p is lognormal: e^(pμ + p²σ²/2), on the shifted interval.
                let (alpha, beta) = (std_normal_quantile(lo), std_normal_quantile(hi));
                let share = std_normal_cdf(beta - p * sigma) - std_normal_cdf(alpha - p * sigma);
                (p * mu + 0.5 * p * p * sigma * sigma).exp() * share / mass
            }
            Family::Gamma { shape, scale } if shape + p > 0.0 => {
                let ratio = (libm::lgamma(shape + p) - libm::lgamma(shape)).exp() * scale.powf(p);
                let share = gamma_cdf(shape + p, b / scale) - gamma_cdf(shape + p, a / scale);
                ratio * share / mass
            }
            Family::Exponential { rate } => {
                return positive_power(
                    &Family::Gamma {
                        shape: 1.0,
                        scale: 1.0 / rate,
                    },
                    p,
                    lo,
                    hi,
                );
            }
            Family::Beta { a: alpha, b: beta } if alpha + p > 0.0 => {
                let ratio = (ln_beta(alpha + p, beta) - ln_beta(alpha, beta)).exp();
                let share = beta_cdf(alpha + p, beta, b) - beta_cdf(alpha + p, beta, a);
                ratio * share / mass
            }
            _ => return None,
        };
        finite(value)
    }

    /// E[e^(c (s X + o))] for the draw X.
    pub fn exp(family: &Family, inner: Affine, c: f64, lo: f64, hi: f64) -> Option<f64> {
        let (a, b) = (family.quantile(lo), family.quantile(hi));
        let mass = hi - lo;
        let Affine { scale: s, offset: o } = inner;
        let k = c * s;
        let value = match *family {
            Family::Uniform { .. } => {
                let (ya, yb) = (inner.at(a), inner.at(b));
                let (y0, y1) = (ya.min(yb), ya.max(yb));
                let width = c * (y1 - y0);
                if width == 0.0 {
                    (c * y0).exp()
                } else {
                    (c * y0).exp() * libm::expm1(width) / width
                }
            }
            Family::Normal { mean, sd } => {
                // e^(kX) of a normal X, on the interval shifted by kσ.
                let (alpha, beta) = ((a - mean) / sd, (b - mean) / sd);
                let share = std_normal_cdf(beta - k * sd) - std_normal_cdf(alpha - k * sd);
                (c * o + k * mean + 0.5 * k * k * sd * sd).exp() * share / mass
            }
            Family::Gamma { shape, scale } if k * scale < 1.0 => {
                let r = 1.0 - k * scale;
                let share = gamma_cdf(shape, b * r / scale) - gamma_cdf(shape, a * r / scale);
                (c * o).exp() * r.powf(-shape) * share / mass
            }
            Family::Exponential { rate } => {
                return exp(
                    &Family::Gamma {
                        shape: 1.0,
                        scale: 1.0 / rate,
                    },
                    inner,
                    c,
                    lo,
                    hi,
                );
            }
            _ => return None,
        };
        finite(value)
    }

    /// The mean and variance of ln(s X + o) for the draw X.
    pub fn ln(family: &Family, inner: Affine, lo: f64, hi: f64) -> Option<(f64, f64)> {
        let (a, b) = (family.quantile(lo), family.quantile(hi));
        let Affine { scale: s, offset: o } = inner;
        match *family {
            Family::Uniform { .. } => {
                let (ya, yb) = (inner.at(a), inner.at(b));
                // Y > 0 where `ln` is taken, but for a coordinate's rounding.
                let (y0, y1) = (ya.min(yb).max(0.0), ya.max(yb));
                if y1 <= y0 {
                    let l = libm::log(y0);
                    return Some((l, 0.0)).filter(|(l, _)| l.is_finite());
                }
                // ∫ ln y = y ln y − y, ∫ ln² y = y (ln² y − 2 ln y + 2),
                // both 0 at y = 0.
                let first = |y: f64| if y == 0.0 { 0.0 } else { y * libm::log(y) - y };
                let second = |y: f64| {
                    if y == 0.0 {
                        return 0.0;
                    }
                    let l = libm::log(y);
                    y * (l * l - 2.0 * l + 2.0)
                };
                let m = (first(y1) - first(y0)) / (y1 - y0);
                let m2 = (second(y1) - second(y0)) / (y1 - y0);
                Some((m, (m2 - m * m).max(0.0)))
            }
            // ln(sX) = ln s + μ + σZ, with Z normal where X is.
            Family::Lognormal { mu, sigma } if o == 0.0 && s > 0.0 => {
                let z = Family::Normal { mean: 0.0, sd: 1.0 };
                let (m, v) = z.interval_moments(std_normal_quantile(lo), std_normal_quantile(hi));
                Some((libm::log(s) + mu + sigma * m, sigma * sigma * v))
            }
            _ => None,
        }
    }
}

/// Where independent axes are: a union of disjoint boxes, each a domain of
/// each axis in its CDF coordinates.
#[derive(Clone, Debug)]
pub struct Event {
    /// Each axis itself, with its domain where the event was made: no two
    /// share a latent.
    pub draws: Vec<Analytic>,
    pub boxes: Vec<Vec<Domain>>,
}

/// The most boxes an event can have: each `or` of events of different
/// draws can multiply them.
const MAX_BOXES: usize = 10_000;

/// The common parts of two unions of disjoint boxes.
fn intersect_boxes(a: &[Vec<Domain>], b: &[Vec<Domain>]) -> OpResult<Vec<Vec<Domain>>> {
    let mut out = Vec::with_capacity(a.len() * b.len());
    for x in a {
        for y in b {
            let both: Vec<Domain> = x.iter().zip(y).map(|(x, y)| x.intersect(y)).collect();
            if both.iter().all(|d| !d.0.is_empty()) {
                out.push(both);
            }
        }
        if out.len() > MAX_BOXES {
            return Err(OpError::limit(format!(
                "an event with more than {MAX_BOXES} boxes is over the limit"
            )));
        }
    }
    Ok(out)
}

/// Where none of a union of disjoint boxes of `n` axes is: the box minus
/// each box in turn, each difference as disjoint boxes (outside the first
/// axis's part; inside it and outside the second's; …).
fn complement_boxes(boxes: &[Vec<Domain>], n: usize) -> OpResult<Vec<Vec<Domain>>> {
    let mut out = vec![vec![Domain::full(); n]];
    for b in boxes {
        let mut outside = Vec::with_capacity(n);
        for i in 0..n {
            let mut part: Vec<Domain> = b[..i].to_vec();
            part.push(b[i].complement());
            part.extend(std::iter::repeat_n(Domain::full(), n - i - 1));
            if part.iter().all(|d| !d.0.is_empty()) {
                outside.push(part);
            }
        }
        out = intersect_boxes(&out, &outside)?;
    }
    Ok(out)
}

impl Event {
    pub fn single(draw: Analytic, yes: Domain) -> Self {
        Self {
            draws: vec![draw],
            boxes: vec![vec![yes]],
        }
    }
    pub fn key(&self) -> impl Ord + std::hash::Hash + use<> {
        (
            self.draws.iter().map(Analytic::key).collect::<Vec<_>>(),
            self.boxes
                .iter()
                .map(|b| b.iter().map(Domain::key).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
        )
    }
    pub fn probability(&self) -> f64 {
        let p: f64 = self
            .boxes
            .iter()
            .map(|b| {
                self.draws
                    .iter()
                    .zip(b)
                    .map(|(d, yes)| d.domain.intersect(yes).mass() / d.domain.mass())
                    .product::<f64>()
            })
            .sum();
        p.clamp(0.0, 1.0)
    }
    pub fn value(self) -> Value {
        let p = self.probability();
        if p == 0.0 || p == 1.0 {
            Value::Bool(p == 1.0)
        } else {
            Value::Event(Arc::new(self))
        }
    }
    /// Where it doesn't hold.
    pub fn complement(&self) -> OpResult<Event> {
        Ok(Event {
            draws: self.draws.clone(),
            boxes: self.side(false)?,
        })
    }
    /// The boxes where it holds, or where it doesn't. Of one axis, that's
    /// one box.
    pub fn side(&self, yes: bool) -> OpResult<Vec<Vec<Domain>>> {
        match (&self.boxes[..], self.draws.len()) {
            ([b], 1) => Ok(vec![vec![if yes { b[0].clone() } else { b[0].complement() }]]),
            _ if yes => Ok(self.boxes.clone()),
            _ => complement_boxes(&self.boxes, self.draws.len()),
        }
    }
    /// How many intervals it holds, which operations on it cost.
    pub fn size(&self) -> usize {
        let boxes: usize = self
            .boxes
            .iter()
            .map(|b| b.iter().map(|d| d.0.len()).sum::<usize>())
            .sum();
        boxes + self.draws.iter().map(|d| d.domain.0.len()).sum::<usize>()
    }
    /// Whether worlds can be restricted to it: each axis is a latent, not
    /// a form of several. Restricting a form would make its latents a
    /// truncated joint distribution.
    pub fn restrictable(&self) -> bool {
        self.draws.iter().all(|d| d.form.is_none())
    }
    /// Restricts a world to one of its boxes: the box's probability given
    /// what the world knew.
    pub fn restrict(&self, part: &[Domain], context: &mut Constraints) -> f64 {
        let mut p = 1.0;
        for (draw, yes) in self.draws.iter().zip(part) {
            let prior = &draw.domain;
            let domain = prior.intersect(yes);
            p *= domain.mass() / prior.mass();
            Arc::make_mut(context).insert(
                draw.id,
                Latent {
                    family: draw.family,
                    domain,
                    form: None,
                },
            );
        }
        p.clamp(0.0, 1.0)
    }
    /// Both events, either, or the same outcome for both, by `op`
    /// (`And`, `Or`, or `Eq`). Their axes must be the same or independent.
    pub fn combine(&self, other: &Event, op: BinOp) -> OpResult<Event> {
        let mut draws = self.draws.clone();
        let mut at = Vec::with_capacity(other.draws.len());
        for d in &other.draws {
            match draws.iter().position(|x| x.same_axis(d)) {
                Some(i) => at.push(i),
                None => {
                    if draws.iter().any(|x| x.shares_latents(d)) {
                        return Err(unsupported("combining events of draws that depend on each other"));
                    }
                    at.push(draws.len());
                    draws.push(d.clone());
                }
            }
        }
        let n = draws.len();
        let mine: Vec<Vec<Domain>> = self
            .boxes
            .iter()
            .map(|b| {
                let mut part = b.clone();
                part.resize(n, Domain::full());
                part
            })
            .collect();
        let theirs: Vec<Vec<Domain>> = other
            .boxes
            .iter()
            .map(|b| {
                let mut part = vec![Domain::full(); n];
                for (d, &i) in b.iter().zip(&at) {
                    part[i] = d.clone();
                }
                part
            })
            .collect();
        let boxes = match op {
            BinOp::And => intersect_boxes(&mine, &theirs)?,
            BinOp::Or => {
                let mut boxes = mine.clone();
                boxes.extend(intersect_boxes(&theirs, &complement_boxes(&mine, n)?)?);
                boxes
            }
            _ => {
                let mut boxes = intersect_boxes(&mine, &theirs)?;
                let (not_mine, not_theirs) = (complement_boxes(&mine, n)?, complement_boxes(&theirs, n)?);
                boxes.extend(intersect_boxes(&not_mine, &not_theirs)?);
                boxes
            }
        };
        Ok(Event { draws, boxes }.simplified())
    }
    /// Of one axis, its boxes as one.
    fn simplified(mut self) -> Event {
        if self.draws.len() == 1 && self.boxes.len() != 1 {
            let union = self.boxes.iter().fold(Domain(Vec::new()), |all, b| all.union(&b[0]));
            self.boxes = vec![vec![union]];
        }
        self
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
    if x.affine() != Some(Affine::IDENTITY) || x.form.is_some() {
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
        return Ok(Some((
            ln,
            Latent {
                family: prior,
                domain,
                form: None,
            },
        )));
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
            form: None,
        },
    )))
}

/// A draw from `normal(mean, sd)`, with `mean` an outcome of normal draws:
/// the mean plus a fresh normal latent `id` with standard deviation `sd`.
/// `None` if the mean isn't an unrestricted linear form of normal draws.
pub fn normal_draw(mean: &Analytic, sd: f64, id: u64) -> OpResult<Option<Value>> {
    let Some(m) = mean.linear() else {
        return Ok(None);
    };
    let noise = Linear {
        constant: 0.0,
        terms: vec![Term {
            id,
            coef: 1.0,
            family: Family::Normal { mean: 0.0, sd },
        }],
    };
    Ok(Some(m.plus(&noise, 1.0)?.value()?))
}

/// An observation's log density, and what the world knows about each latent
/// after it.
pub type Observed = (f64, Vec<(u64, Latent)>);

/// Observing `y` from `normal(mean, sd)`, with `mean` an outcome of normal
/// draws: the logarithm of its density, and what the world knows about
/// each latent after it. `None` if the mean isn't affine in a normal latent
/// or a linear form of several.
///
/// With one latent, it's the conjugate update, by what `y` says about the
/// latent itself. With several, which are unrestricted, their posterior is
/// normal with a covariance. Each becomes its posterior mean plus a linear
/// form of fresh standard normal latents, numbered after `next`: with
/// prior standard deviations `s`, coefficients `a` and `S` the variance of
/// `y`, the covariance is D^½ (I − c cᵀ) D^½ for D = diag(s²) and
/// c = s a / √S, and its square root D^½ (I − γ c cᵀ), γ = 1 / (1 + sd / √S).
pub fn observe_normal(mean: &Analytic, y: f64, sd: f64, next: &mut u64) -> OpResult<Option<Observed>> {
    if mean.form.is_none() && matches!(mean.family, Family::Normal { .. }) {
        let Some(a) = mean.affine() else {
            return Ok(None);
        };
        // y ~ normal(s x + o, sd) is (y − o) / s ~ normal(x, sd / |s|), with
        // the density divided by |s|.
        let seen = crate::conjugate::Seen::Normal {
            y: (y - a.offset) / a.scale,
            sd: sd / a.scale.abs(),
        };
        let updated = update(&mean.latent(), seen)?;
        return Ok(updated.map(|(ln, latent)| (ln - libm::log(a.scale.abs()), vec![(mean.id, latent)])));
    }
    let Some(form) = mean.linear() else {
        return Ok(None);
    };
    let (m, variance) = form.moments();
    let total = variance + sd * sd;
    let r = y - m;
    let ln = -0.5 * libm::log(2.0 * std::f64::consts::PI * total) - r * r / (2.0 * total);
    let root = libm::sqrt(total);
    let gamma = 1.0 / (1.0 + sd / root);
    let c: Vec<f64> = form.terms.iter().map(|t| t.family.sd() * t.coef / root).collect();
    let fresh: Vec<u64> = form
        .terms
        .iter()
        .map(|_| {
            *next += 1;
            *next
        })
        .collect();
    let standard = Family::Normal { mean: 0.0, sd: 1.0 };
    let mut known = Vec::with_capacity(form.terms.len());
    for (i, t) in form.terms.iter().enumerate() {
        let s = t.family.sd();
        let constant = t.family.mean() + s * s * t.coef * r / total;
        let terms = fresh
            .iter()
            .zip(&c)
            .enumerate()
            .map(|(j, (&id, &cj))| Term {
                id,
                coef: s * (f64::from(u8::from(i == j)) - gamma * c[i] * cj),
                family: standard,
            })
            .collect();
        let linear = Linear::from_terms(constant, terms)?;
        let latent = Latent {
            family: linear.normal()?,
            domain: Domain::full(),
            form: Some(Arc::new(linear)),
        };
        known.push((t.id, latent));
    }
    Ok(Some((ln, known)))
}

pub fn unsupported(what: &str) -> OpError {
    OpError::unsupported(format!("{what} isn't supported for analytic continuous draws yet"))
        .help("use `@mode sample(runs: 10_000)` for this operation")
}

/// Restricting worlds by an outcome of several normal draws together would
/// make them a truncated joint distribution.
pub fn joint_condition(what: &str) -> OpError {
    OpError::unsupported(format!("{what} isn't supported when enumerating yet")).help(
        "it would make the draws a truncated joint distribution: report it instead, or use `@mode sample(runs: 10_000)`",
    )
}

/// An operand of an operation on outcomes of one draw.
enum Operand<'a> {
    Pieces(&'a [(f64, Fun)]),
    Constant(f64),
}

fn operand<'a>(v: &'a Value, x: &Analytic) -> OpResult<Operand<'a>> {
    match v {
        Value::Analytic(y) if y.same_axis(x) => Ok(Operand::Pieces(&y.pieces)),
        Value::Analytic(_) => Err(different_draws()),
        v => v
            .as_f64()
            .filter(|v| v.is_finite())
            .map(Operand::Constant)
            .ok_or_else(|| unsupported("this operand")),
    }
}

/// `f(a, b)` on each part where both operands have one function.
fn zip(a: &Operand, b: &Operand, f: impl Fn(Fun, Fun) -> OpResult<Fun>) -> OpResult<Pieces> {
    match (a, b) {
        (Operand::Pieces(a), Operand::Constant(c)) => {
            a.iter().map(|&(end, a)| Ok((end, f(a, Fun::constant(*c))?))).collect()
        }
        (Operand::Constant(c), Operand::Pieces(b)) => {
            b.iter().map(|&(end, b)| Ok((end, f(Fun::constant(*c), b)?))).collect()
        }
        (Operand::Pieces(a), Operand::Pieces(b)) => overlay(a, b)
            .into_iter()
            .map(|(end, (a, b))| Ok((end, f(a, b)?)))
            .collect(),
        (Operand::Constant(_), Operand::Constant(_)) => unreachable!("an operand is analytic"),
    }
}

fn nonlinear() -> OpError {
    unsupported("nonlinear arithmetic")
}

/// `a + b`, or `a − b` with `sign` −1. Affine functions add as they did;
/// polynomials of degree 2 add as polynomials; the same kernel of the same
/// inner function adds its outer ones.
fn sum(a: Fun, b: Fun, sign: f64) -> OpResult<Fun> {
    if let (Some(x), Some(y)) = (a.as_affine(), b.as_affine()) {
        return Ok(Fun::affine(if sign > 0.0 {
            Affine {
                scale: x.scale + y.scale,
                offset: x.offset + y.offset,
            }
        } else {
            Affine {
                scale: x.scale - y.scale,
                offset: x.offset - y.offset,
            }
        }));
    }
    if let Some(c) = b.as_constant() {
        return Ok(a.then(1.0, sign * c));
    }
    if let Some(c) = a.as_constant() {
        return Ok(b.then(sign, c));
    }
    if let (Some(p), Some(q)) = (a.polynomial(), b.polynomial()) {
        return Ok(Fun::from_polynomial([0, 1, 2].map(|i| p[i] + sign * q[i])));
    }
    if a.kernel == b.kernel && a.inner == b.inner {
        let outer = Affine {
            scale: a.outer.scale + sign * b.outer.scale,
            offset: a.outer.offset + sign * b.outer.offset,
        };
        return Ok(if outer.scale == 0.0 {
            Fun::constant(outer.offset)
        } else {
            Fun { outer, ..a }
        });
    }
    Err(nonlinear())
}

fn difference(a: Fun, b: Fun) -> OpResult<Fun> {
    sum(a, b, -1.0)
}

/// `a × b`: a constant scales; affine functions multiply into a square.
fn product(a: Fun, b: Fun) -> OpResult<Fun> {
    if let (Some(x), Some(y)) = (a.as_affine(), b.as_affine()) {
        if y.scale == 0.0 {
            return Ok(Fun::affine(Affine {
                scale: x.scale * y.offset,
                offset: x.offset * y.offset,
            }));
        }
        if x.scale == 0.0 {
            return Ok(Fun::affine(Affine {
                scale: y.scale * x.offset,
                offset: y.offset * x.offset,
            }));
        }
    }
    if let Some(c) = b.as_constant() {
        return Ok(scaled(a, c));
    }
    if let Some(c) = a.as_constant() {
        return Ok(scaled(b, c));
    }
    match (a.polynomial(), b.polynomial()) {
        (Some([p0, p1, 0.0]), Some([q0, q1, 0.0])) => Ok(Fun::from_polynomial([p0 * q0, p0 * q1 + p1 * q0, p1 * q1])),
        _ => Err(nonlinear()),
    }
}

/// `c × f`, a constant again when `c` is 0.
fn scaled(f: Fun, c: f64) -> Fun {
    if c == 0.0 { Fun::constant(0.0) } else { f.then(c, 0.0) }
}

/// `a ÷ b`: by a constant, or a constant by an affine function.
fn quotient(a: Fun, b: Fun, plain: bool) -> OpResult<Fun> {
    if let Some(c) = b.as_constant() {
        if c == 0.0 {
            return Err(if plain {
                OpError::fault(Fault::DivisionByZero, "division by zero")
            } else {
                unsupported("dividing by a value that's zero on part of its domain")
            });
        }
        return Ok(match a.as_affine() {
            Some(x) => Fun::affine(Affine {
                scale: x.scale / c,
                offset: x.offset / c,
            }),
            None => Fun {
                outer: Affine {
                    scale: a.outer.scale / c,
                    offset: a.outer.offset / c,
                },
                ..a
            },
        });
    }
    // c / (s y + o) = (c / s) / (y + o / s).
    if let (Some(c), Some(y)) = (a.as_constant(), b.as_affine()) {
        return Ok(Fun {
            outer: Affine {
                scale: c / y.scale,
                offset: 0.0,
            },
            kernel: Kernel::Recip,
            inner: Affine {
                scale: 1.0,
                offset: y.offset / y.scale,
            },
        });
    }
    Err(nonlinear())
}

/// The most pieces an outcome can have. Repeated folds like
/// `y = abs(2 * y - 1)` double them each time.
const MAX_PIECES: usize = 100_000;

/// Pieces cost work, and their number is limited.
fn spend(pieces: &[(f64, Fun)], budget: &mut Budget) -> OpResult<()> {
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
    if matches!(a, Value::Joint(_)) || matches!(b, Value::Joint(_)) {
        return crate::joint::binary(op, a, b, budget);
    }
    let x = match (a, b) {
        (Value::Analytic(x), _) | (_, Value::Analytic(x)) => x,
        _ => return Err(unsupported("this operation")),
    };
    if op == Pow {
        return power(a, b, budget);
    }
    if let (Value::Analytic(p), Value::Analytic(q)) = (a, b) {
        if !p.same_axis(q) {
            return combined(op, p, q, budget);
        }
    }
    let (p, q) = (operand(a, x)?, operand(b, x)?);
    let plain = matches!(q, Operand::Constant(_));
    let pieces = match op {
        Add => zip(&p, &q, |a, b| sum(a, b, 1.0))?,
        Sub => zip(&p, &q, difference)?,
        Mul => zip(&p, &q, product)?,
        Div => zip(&p, &q, |a, b| quotient(a, b, plain))?,
        Eq | Ne | Lt | Le | Gt | Ge => {
            let d = split_turns(&x.family, zip(&p, &q, difference)?);
            spend(&d, budget)?;
            if !d.iter().all(|(_, f)| f.is_finite()) {
                return Err(unsupported("non-finite affine coefficients"));
            }
            return Ok(compare(x, op, &d)?.value());
        }
        _ => return Err(nonlinear()),
    };
    let pieces = split_turns(&x.family, pieces);
    spend(&pieces, budget)?;
    // As with numbers, sums, differences and products of ints are ints.
    let int = matches!(op, Add | Sub | Mul) && is_int(a) && is_int(b);
    x.with_type(pieces, int).value()
}

/// Combining outcomes of different axes: only sums and differences of
/// normal latents, which are linear forms of them.
fn different_draws() -> OpError {
    OpError::unsupported(
        "combining different continuous draws isn't supported when enumerating, except to add or subtract normal ones without restrictions",
    )
    .help("use `@mode sample(runs: 10_000)` for this operation")
}

/// An operation on outcomes of different axes: a sum or difference of
/// normal latents is a linear form of them, and a comparison an event of
/// the difference.
fn combined(op: BinOp, p: &Analytic, q: &Analytic, budget: &mut Budget) -> OpResult<Value> {
    use BinOp::*;
    let sign = match op {
        Add => 1.0,
        Sub | Eq | Ne | Lt | Le | Gt | Ge => -1.0,
        _ => return Err(different_draws()),
    };
    let (Some(a), Some(b)) = (p.linear(), q.linear()) else {
        return Err(different_draws());
    };
    budget.work((a.terms.len() + b.terms.len()) as u64)?;
    let d = a.plus(&b, sign)?.value()?;
    match op {
        Add | Sub => Ok(d),
        _ => crate::ops::binary(op, &d, &Value::Float(0.0), budget),
    }
}

/// `a ^ b` with an outcome of a draw: a square, a square root or a
/// reciprocal of one, or a positive number to the power of an affine one.
fn power(a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match (a, b.as_f64()) {
        (Value::Analytic(_), Some(2.0)) => binary(BinOp::Mul, a, a, budget),
        (Value::Analytic(_), Some(1.0)) => Ok(a.clone()),
        (Value::Analytic(y), Some(0.5)) => transformed(y, Kernel::Sqrt, budget),
        (Value::Analytic(_), Some(-1.0)) => binary(BinOp::Div, &Value::Float(1.0), a, budget),
        (Value::Analytic(_), _) => Err(unsupported("this power")),
        // c^y = e^(y ln c).
        (base, _) => match (base.as_f64(), b) {
            (Some(c), Value::Analytic(_)) if c > 0.0 => {
                let exponent = binary(BinOp::Mul, b, &Value::Float(libm::log(c)), budget)?;
                match exponent {
                    Value::Analytic(e) => transformed(&e, Kernel::Exp, budget),
                    v => crate::ops::binary(BinOp::Pow, base, &v, budget),
                }
            }
            _ => Err(unsupported("this power")),
        },
    }
}

/// The function a kernel makes of `f`, if it makes one this can hold.
fn compose(kernel: Kernel, f: Fun, family: &Family, start: f64, end: f64) -> OpResult<Fun> {
    if let Some(c) = f.as_constant() {
        // Outside the domain, the caller has checked, a constant can't be
        // there with any probability.
        let y = kernel.apply(c);
        return Ok(Fun::constant(if y.is_finite() { y } else { 0.0 }));
    }
    if let Some(inner) = f.as_affine() {
        return Ok(Fun::of(kernel, inner));
    }
    let Affine { scale: s, offset: o } = f.outer;
    match (kernel, f.kernel) {
        // √(s y²) = √s |y|, on the side of 0 the piece is on.
        (Kernel::Sqrt, Kernel::Square) if o == 0.0 && s > 0.0 => {
            let sign = if f.side(family, start, end) < 0.0 { -1.0 } else { 1.0 };
            let k = sign * s.sqrt();
            Ok(Fun::affine(Affine {
                scale: k * f.inner.scale,
                offset: k * f.inner.offset,
            }))
        }
        // ln(s e^y) = ln s + y.
        (Kernel::Ln, Kernel::Exp) if o == 0.0 && s > 0.0 => Ok(Fun::affine(Affine {
            scale: f.inner.scale,
            offset: f.inner.offset + libm::log(s),
        })),
        // e^(ln y + o) = e^o y.
        (Kernel::Exp, Kernel::Ln) if s == 1.0 => {
            let k = o.exp();
            Ok(Fun::affine(Affine {
                scale: k * f.inner.scale,
                offset: k * f.inner.offset,
            }))
        }
        _ => Err(unsupported("this composition of continuous transforms")),
    }
}

/// `sqrt`, `exp` or `ln` of an outcome of a draw. Where the outcome is
/// outside what the function is defined for, with positive probability, it
/// faults there: everywhere is a fault, as for a number; part of the
/// domain isn't supported yet.
pub fn transformed(x: &Analytic, kernel: Kernel, budget: &mut Budget) -> OpResult<Value> {
    let (name, outside) = match kernel {
        Kernel::Sqrt => ("sqrt", below(&x.family, &x.pieces, 0.0, true)?),
        Kernel::Ln => ("ln", below(&x.family, &x.pieces, 0.0, false)?),
        // Beyond this, e^y isn't a finite float.
        Kernel::Exp => (
            "exp",
            below(&x.family, &x.pieces, 709.782712893384, false)?.complement(),
        ),
        _ => unreachable!("a transform built in"),
    };
    let faulting = x.domain.intersect(&outside).mass();
    if faulting > 0.0 {
        return Err(if faulting >= x.domain.mass() {
            OpError::fault(Fault::DomainError, format!("`{name}` isn't defined for this outcome"))
        } else {
            unsupported(&format!(
                "`{name}` of an outcome outside its domain on part of its range"
            ))
        });
    }
    let mut pieces = Vec::with_capacity(x.pieces.len());
    let mut start = 0.0;
    for &(end, f) in &x.pieces {
        pieces.push((end, compose(kernel, f, &x.family, start, end)?));
        start = end;
    }
    spend(&pieces, budget)?;
    x.with(pieces).value()
}

/// The event that a difference `d` of outcomes of `x`'s draw compares with
/// zero by `op`.
fn compare(x: &Analytic, op: BinOp, d: &[(f64, Fun)]) -> OpResult<Event> {
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
                if f.as_constant() == Some(0.0) {
                    push(&mut equal, start, end);
                }
                start = end;
            }
            let equal = Domain(equal);
            if op == Eq { equal } else { equal.complement() }
        }
    };
    Ok(Event::single(x.latent(), yes))
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
    let family = &x.family;
    let mut pieces: Pieces = Vec::new();
    let mut at = 0.0;
    for (lo, hi, f) in x.segments() {
        if lo > at {
            // Outside the domain: anything, which `value` gives away.
            pieces.push((lo, Fun::constant(0.0)));
        }
        at = hi;
        if let Some(c) = f.as_constant() {
            pieces.push((hi, Fun::constant(how.apply(c))));
            continue;
        }
        let (a, b) = f.ends(family, lo, hi);
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
        let up = f.increasing(family, lo, hi);
        let mut bounds: Vec<(f64, f64)> = steps
            .iter()
            .map(|&t| (f.coordinate(family, lo, hi, t).clamp(lo, hi), t))
            .collect();
        if !up {
            bounds.reverse();
        }
        let mut from = if up { vmin } else { vmax };
        for (u, t) in bounds {
            pieces.push((u, Fun::constant(how.apply((from + t) / 2.0))));
            from = t;
        }
        let to = if up { vmax } else { vmin };
        pieces.push((hi, Fun::constant(how.apply((from + to) / 2.0))));
    }
    if at < 1.0 {
        pieces.push((1.0, Fun::constant(0.0)));
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
            Operand::Constant(c) => vec![(1.0, Fun::constant(c))],
        })
    };
    let mut best = tiling(&args[0])?;
    for v in &args[1..] {
        let v = tiling(v)?;
        let d = split_turns(
            &x.family,
            zip(&Operand::Pieces(&v), &Operand::Pieces(&best), difference)?,
        );
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
    let inside = select(&over, &[(1.0, Fun::constant(hi))], &x.pieces);
    let pieces = select(&under, &[(1.0, Fun::constant(lo))], &inside);
    spend(&pieces, budget)?;
    x.with_type(pieces, int).value()
}

pub fn logic(and: bool, a: &Value, b: &Value) -> OpResult<Value> {
    let (event, other) = match (a, b) {
        (Value::Event(e), b) | (b, Value::Event(e)) => (e, b),
        _ => return Err(unsupported("this logical operation")),
    };
    match other {
        Value::Bool(x) if *x != and => Ok(Value::Bool(*x)),
        Value::Bool(_) => Ok(Value::Event(event.clone())),
        Value::Event(other) => Ok(event.combine(other, if and { BinOp::And } else { BinOp::Or })?.value()),
        _ => Err(unsupported("combining this event with a probability")),
    }
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

/// Whether a value holds an outcome of a draw, other than events.
pub fn has_outcomes(v: &Value) -> bool {
    let mut pending = vec![v];
    while let Some(v) = pending.pop() {
        match v {
            Value::Analytic(_) => return true,
            Value::List(v) => pending.extend(v.iter()),
            Value::Map(v) => pending.extend(v.values()),
            Value::Record(v) => pending.extend(v.fields.iter().map(|(_, v)| v)),
            Value::Dist(d) => pending.extend(d.outcomes.iter().map(|(v, _)| v)),
            Value::Closure(c) => pending.extend(c.captured.iter()),
            _ => {}
        }
    }
    false
}

/// A value as reports keep it: each outcome of a draw by its marginal alone,
/// without the identity of its draws, so that equal laws merge.
pub fn marginal(v: &Value) -> Value {
    if !contains(v) {
        return v.clone();
    }
    let bare = |a: &Analytic| Analytic {
        id: 0,
        form: None,
        ..a.clone()
    };
    match v {
        Value::Analytic(a) => Value::Analytic(Arc::new(bare(a))),
        Value::Event(e) => Value::Event(Arc::new(Event {
            draws: e.draws.iter().map(bare).collect(),
            boxes: e.boxes.clone(),
        })),
        Value::List(xs) => Value::list(xs.iter().map(marginal).collect()),
        Value::Record(r) => crate::ops::make_record(
            r.ty.clone(),
            r.fields.iter().map(|(k, v)| (k.clone(), marginal(v))).collect(),
        ),
        Value::Map(xs) => Value::map(xs.iter().map(|(k, v)| (k.clone(), marginal(v))).collect()),
        Value::Dist(d) => {
            crate::dist::Dist::from_pairs(d.outcomes.iter().map(|(v, p)| (marginal(v), *p)).collect(), d.missing)
                .into_value()
        }
        _ => v.clone(),
    }
}

/// The same value with its latents renamed by `map`.
pub fn renamed(v: &Value, map: &BTreeMap<u64, u64>) -> Value {
    if !contains(v) {
        return v.clone();
    }
    match v {
        Value::Analytic(a) => Value::Analytic(Arc::new(a.renamed(map))),
        Value::Event(e) => Value::Event(Arc::new(Event {
            draws: e.draws.iter().map(|d| d.renamed(map)).collect(),
            boxes: e.boxes.clone(),
        })),
        Value::List(xs) => Value::list(xs.iter().map(|v| renamed(v, map)).collect()),
        Value::Record(r) => crate::ops::make_record(
            r.ty.clone(),
            r.fields.iter().map(|(k, v)| (k.clone(), renamed(v, map))).collect(),
        ),
        Value::Map(xs) => Value::map(xs.iter().map(|(k, v)| (k.clone(), renamed(v, map))).collect()),
        Value::Closure(f) => Value::Closure(Arc::new(Closure {
            func: f.func,
            captured: f.captured.iter().map(|v| renamed(v, map)).collect(),
        })),
        Value::Dist(d) => crate::dist::Dist::from_pairs(
            d.outcomes.iter().map(|(v, p)| (renamed(v, map), *p)).collect(),
            d.missing,
        )
        .into_value(),
        _ => v.clone(),
    }
}

/// Latents reachable through an immutable value (including closure captures).
pub fn collect_ids(v: &Value, ids: &mut std::collections::BTreeSet<u64>) {
    let mut pending = vec![v];
    while let Some(v) = pending.pop() {
        match v {
            Value::Analytic(a) => a.collect_ids(ids),
            Value::Event(e) => e.draws.iter().for_each(|d| d.collect_ids(ids)),
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

/// Adds the latents that the forms of those in `ids` are made of, which
/// the world must keep knowing about too.
pub fn collect_form_ids(context: &Constraints, ids: &mut std::collections::BTreeSet<u64>) {
    let mut pending: Vec<u64> = ids.iter().copied().collect();
    while let Some(id) = pending.pop() {
        if let Some(Latent { form: Some(f), .. }) = context.get(&id) {
            for t in &f.terms {
                if ids.insert(t.id) {
                    pending.push(t.id);
                }
            }
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

/// The linear form of independent latents that `form` is in a world: each
/// latent that an observation of several replaced by its form, and each
/// one's distribution there. `None` if that's `form` itself. Unsupported
/// where a latent is restricted, since the form isn't normal then.
fn expand(form: &Linear, c: &Constraints, b: &mut Budget, depth: usize) -> OpResult<Option<Linear>> {
    if depth > 64 {
        return Err(OpError::limit("analytic value nesting exceeds the limit of 64"));
    }
    b.work(form.terms.len() as u64)?;
    let mut changed = false;
    let mut constant = form.constant;
    let mut terms = Vec::with_capacity(form.terms.len());
    for t in &form.terms {
        match c.get(&t.id) {
            None => terms.push(*t),
            Some(Latent { form: Some(sub), .. }) => {
                changed = true;
                let sub = match expand(sub, c, b, depth + 1)? {
                    Some(e) => e,
                    None => (**sub).clone(),
                };
                constant += t.coef * sub.constant;
                terms.extend(sub.terms.iter().map(|u| Term {
                    coef: t.coef * u.coef,
                    ..*u
                }));
            }
            Some(l) if l.domain.is_full() && matches!(l.family, Family::Normal { .. }) => {
                changed |= l.family != t.family;
                terms.push(Term { family: l.family, ..*t });
            }
            Some(_) => return Err(different_draws()),
        }
    }
    if !changed {
        return Ok(None);
    }
    Linear::from_terms(constant, terms).map(Some)
}

/// `x`, whose axis the world now knows to be `e`, as a function of `e`'s
/// axis, and how its coordinates moved.
fn onto(x: &Analytic, e: Linear) -> OpResult<(Analytic, Transport)> {
    let (id, form, to, map) = match e.terms[..] {
        [] => return Err(unsupported("a draw that an observation made constant")),
        [t] => (
            t.id,
            None,
            t.family,
            Affine {
                scale: t.coef,
                offset: e.constant,
            },
        ),
        _ => {
            let (axis, map) = e.axis();
            let to = axis.normal()?;
            (0, Some(axis), to, map)
        }
    };
    let t = Transport {
        from: x.family,
        to,
        map,
    };
    Ok((x.rebased(id, form, &t), t))
}

/// An outcome as a world reads it, and how its coordinates moved, if they
/// did.
fn read(x: &Analytic, c: &Constraints, b: &mut Budget) -> OpResult<(Analytic, Option<Transport>)> {
    if let Some(form) = &x.form {
        return Ok(match expand(form, c, b, 0)? {
            None => (x.clone(), None),
            Some(e) => {
                let (x, t) = onto(x, e)?;
                (x, Some(t))
            }
        });
    }
    Ok(match c.get(&x.id) {
        Some(Latent { form: Some(sub), .. }) => {
            let e = expand(sub, c, b, 0)?.unwrap_or_else(|| (**sub).clone());
            let (x, t) = onto(x, e)?;
            (x, Some(t))
        }
        Some(l) => {
            let mut y = x.moved(&l.family);
            y.domain = y.domain.intersect(&l.domain);
            let t = (l.family != x.family).then(|| Transport::moved(x.family, l.family));
            (y, t)
        }
        None => (x.clone(), None),
    })
}

fn resolve_at(v: &Value, c: &Constraints, b: &mut Budget, depth: usize) -> OpResult<Value> {
    b.work(1)?;
    if depth > 64 {
        return Err(OpError::limit("analytic value nesting exceeds the limit of 64"));
    }
    Ok(match v {
        Value::Analytic(x) => read(x, c, b)?.0.value()?,
        Value::Event(e) => {
            let mut draws = Vec::with_capacity(e.draws.len());
            let mut boxes = e.boxes.clone();
            let mut moved = false;
            for (i, d) in e.draws.iter().enumerate() {
                let (d, t) = read(d, c, b)?;
                if let Some(t) = t {
                    moved = true;
                    for part in &mut boxes {
                        part[i] = t.domain(&part[i]);
                    }
                }
                draws.push(d);
            }
            // An observation can make independent draws depend on each other.
            if moved && draws.len() > 1 {
                for (i, d) in draws.iter().enumerate() {
                    if draws[..i].iter().any(|x| x.same_axis(d) || x.shares_latents(d)) {
                        return Err(unsupported(
                            "an event of draws that an observation made depend on each other",
                        ));
                    }
                }
            }
            Event { draws, boxes }.simplified().value()
        }
        Value::List(xs) => Value::list(
            xs.iter()
                .map(|v| resolve_at(v, c, b, depth + 1))
                .collect::<OpResult<_>>()?,
        ),
        Value::Record(r) => crate::ops::make_record(
            r.ty.clone(),
            r.fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), resolve_at(v, c, b, depth + 1)?)))
                .collect::<OpResult<_>>()?,
        ),
        Value::Map(xs) => Value::map(
            xs.iter()
                .map(|(k, v)| Ok((k.clone(), resolve_at(v, c, b, depth + 1)?)))
                .collect::<OpResult<_>>()?,
        ),
        Value::Closure(f) => Value::Closure(Arc::new(Closure {
            func: f.func,
            captured: f
                .captured
                .iter()
                .map(|v| resolve_at(v, c, b, depth + 1))
                .collect::<OpResult<_>>()?,
        })),
        Value::Dist(d) => {
            let pairs = d
                .outcomes
                .iter()
                .map(|(v, p)| Ok((resolve_at(v, c, b, depth + 1)?, *p)))
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
        let (mean, variance) = y.moments().unwrap();
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
        let (mean, variance) = y.moments().unwrap();
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
        close(y.moments().unwrap().0, 0.5);
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
        assert_eq!(e.boxes, vec![vec![Domain(vec![(0.25, 0.75)])]]);
        assert_eq!(e.draws[0].affine(), Some(Affine::IDENTITY));
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
        assert_eq!(y.pieces, vec![(1.0, Fun::IDENTITY)]);
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
                (0.5, Fun::constant(0.0)),
                (
                    1.0,
                    Fun::affine(Affine {
                        scale: 2.0,
                        offset: 0.0
                    })
                )
            ]
        );
        close(s.moments().unwrap().0, 0.5);
        // A constant piece times anything is affine; elsewhere it isn't.
        let product = binary(
            BinOp::Mul,
            &Value::Analytic(Arc::new(s.clone())),
            &Value::Float(3.0),
            &mut budget(),
        );
        assert!(product.is_ok());
        // Affine pieces multiply into squares: 0 below zero, 4x² above.
        let s = Value::Analytic(Arc::new(s));
        let square = analytic(binary(BinOp::Mul, &s, &s, &mut budget()).unwrap());
        close(square.moments().unwrap().0, 2.0 / 3.0);
        // A cube isn't one of the functions a piece can be.
        let cube = binary(BinOp::Mul, &Value::Analytic(Arc::new(square)), &s, &mut budget());
        assert!(cube.unwrap_err().message.contains("nonlinear arithmetic"));
    }
}
