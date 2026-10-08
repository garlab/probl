//! Identity-preserving continuous outcomes during enumeration.
//!
//! A draw is an affine function of one latent variable. Evidence is a set of
//! intervals in that variable's CDF coordinates. Worlds own their restrictions;
//! values and closures remain immutable, and calls return restrictions alongside
//! their results. Independent latent combinations require sampling for now.

use crate::continuous::Family;
use crate::dist::Budget;
use crate::error::{Fault, OpError, OpResult};
use crate::value::{Closure, Value, family_key, float_key};
use probl_syntax::ast::BinOp;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Constraints = Arc<BTreeMap<u64, Domain>>;

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

#[derive(Clone, Debug)]
pub struct Analytic {
    pub id: u64,
    pub family: Family,
    pub domain: Domain,
    pub scale: f64,
    pub offset: f64,
}
impl Analytic {
    pub fn new(id: u64, family: Family) -> Self {
        Self {
            id,
            family,
            domain: Domain::full(),
            scale: 1.0,
            offset: 0.0,
        }
    }
    pub fn key(&self) -> impl Ord + std::hash::Hash + use<> {
        (
            self.id,
            family_key(&self.family),
            self.domain.key(),
            float_key(self.scale),
            float_key(self.offset),
        )
    }
    pub fn value(self) -> OpResult<Value> {
        if !self.scale.is_finite() || !self.offset.is_finite() {
            return Err(unsupported("non-finite affine coefficients"));
        }
        if self.scale == 0.0 {
            Ok(Value::Float(self.offset))
        } else {
            Ok(Value::Analytic(Arc::new(self)))
        }
    }
    pub fn cdf(&self, x: f64) -> f64 {
        let p = self.family.cdf((x - self.offset) / self.scale);
        let below = self.domain.intersect(&Domain::below(p)).mass() / self.domain.mass();
        (if self.scale > 0.0 { below } else { 1.0 - below }).clamp(0.0, 1.0)
    }
    pub fn pdf(&self, x: f64) -> f64 {
        let x = (x - self.offset) / self.scale;
        let u = self.family.cdf(x);
        if self.domain.0.iter().any(|(a, b)| *a <= u && u <= *b) {
            self.family.pdf(x) / (self.scale.abs() * self.domain.mass())
        } else {
            0.0
        }
    }
    pub fn quantile(&self, q: f64) -> f64 {
        let mut left = q * self.domain.mass();
        let n = self.domain.0.len();
        for i in 0..n {
            let (a, b) = self.domain.0[if self.scale > 0.0 { i } else { n - 1 - i }];
            if left <= b - a || i + 1 == n {
                let u = if self.scale > 0.0 { a + left } else { b - left };
                return self.scale * self.family.quantile(u.clamp(a, b)) + self.offset;
            }
            left -= b - a;
        }
        f64::NAN
    }
    pub fn moments(&self) -> (f64, f64) {
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
        (self.scale * mean + self.offset, self.scale * self.scale * variance)
    }

    pub fn sd(&self) -> f64 {
        if self.domain == Domain::full() {
            self.scale.abs() * self.family.sd()
        } else {
            libm::sqrt(self.moments().1)
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
        Arc::make_mut(context).insert(self.draw.id, domain);
        p.clamp(0.0, 1.0)
    }
}

pub fn unsupported(what: &str) -> OpError {
    OpError::unsupported(format!("{what} isn't supported for analytic continuous draws yet"))
        .help("use `@mode sample(runs: 10_000)` for this operation")
}

pub fn binary(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    use BinOp::*;
    let (mut x, reverse, other) = match (a, b) {
        (Value::Analytic(x), b) => ((**x).clone(), false, b),
        (a, Value::Analytic(x)) => ((**x).clone(), true, a),
        _ => return Err(unsupported("this operation")),
    };
    let (scale, offset) = match other {
        Value::Analytic(y) if x.id == y.id => (y.scale, y.offset),
        Value::Analytic(_) => return Err(unsupported("combining independent continuous draws")),
        v => (
            0.0,
            v.as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| unsupported("this operand"))?,
        ),
    };
    match op {
        Add => {
            x.scale += scale;
            x.offset += offset;
        }
        Sub => {
            x.scale -= scale;
            x.offset -= offset;
            if reverse {
                x.scale = -x.scale;
                x.offset = -x.offset;
            }
        }
        Mul if scale == 0.0 => {
            x.scale *= offset;
            x.offset *= offset;
        }
        Div if !reverse && scale == 0.0 => {
            if offset == 0.0 {
                return Err(OpError::fault(Fault::DivisionByZero, "division by zero"));
            }
            x.scale /= offset;
            x.offset /= offset;
        }
        Eq | Ne | Lt | Le | Gt | Ge => {
            x.scale -= scale;
            x.offset -= offset;
            if reverse {
                x.scale = -x.scale;
                x.offset = -x.offset;
            }
            if !x.scale.is_finite() || !x.offset.is_finite() {
                return Err(unsupported("non-finite affine coefficients"));
            }
            if x.scale == 0.0 {
                return Ok(Value::Bool(match op {
                    Eq => x.offset == 0.0,
                    Ne => x.offset != 0.0,
                    Lt => x.offset < 0.0,
                    Le => x.offset <= 0.0,
                    Gt => x.offset > 0.0,
                    _ => x.offset >= 0.0,
                }));
            }
            if op == Eq || op == Ne {
                return Ok(Value::Bool(op == Ne));
            }
            let p = x.family.cdf(-x.offset / x.scale);
            if !p.is_finite() {
                return Err(unsupported("this numerically unstable comparison"));
            }
            let below = Domain::below(p);
            let yes = if matches!(op, Lt | Le) == (x.scale > 0.0) {
                below
            } else {
                below.complement()
            };
            return Ok(Event { draw: x, yes }.value());
        }
        _ => return Err(unsupported("nonlinear arithmetic")),
    }
    x.value()
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
    let draw = |x: &Analytic| {
        let mut x = x.clone();
        if let Some(d) = c.get(&x.id) {
            x.domain = x.domain.intersect(d);
        }
        x
    };
    let mut child = |v: &Value| resolve_at(v, c, b, depth + 1);
    Ok(match v {
        Value::Analytic(x) => draw(x).value()?,
        Value::Event(e) => Event {
            draw: draw(&e.draw),
            yes: e.yes.clone(),
        }
        .value(),
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
