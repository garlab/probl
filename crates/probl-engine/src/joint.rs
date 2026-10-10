//! Distributions whose outcomes own continuous draws: what `simulate`
//! returns when its value holds outcomes of draws made inside it
//! (docs/semantics.md, section 8).
//!
//! Each outcome's draws are numbered apart from any world's, from `OWNED`,
//! in the order of the latents they were. Drawing from the distribution
//! gives them fresh numbers in the world, so that two draws are independent
//! and the parts of one outcome, like the fields of a record, stay
//! correlated. When sampling, an outcome is chosen, then a value for each of
//! its draws.

use crate::analytic::{self, Domain, Event};
use crate::continuous::{Family, Mixture, Part, Rng};
use crate::dist::{Budget, Dist};
use crate::error::{OpError, OpResult};
use crate::value::Value;
use probl_syntax::ast::BinOp;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Where an outcome's own draws are numbered from: above any world's.
pub const OWNED: u64 = 1 << 63;

/// The most events in one outcome whose truth values `simulate` lists.
const MAX_EVENTS: usize = 16;

/// The distribution of values that may hold outcomes of draws made where
/// they were computed, as `simulate` gives it: each outcome's draws its own,
/// and outcomes of only events as their truth values. An ordinary
/// distribution when no outcome holds an outcome of a draw.
pub fn of(pairs: Vec<(Value, f64)>, missing: f64, budget: &mut Budget) -> OpResult<Value> {
    if pairs.is_empty() {
        return Err(OpError::new("the result would be an empty distribution"));
    }
    let mut missing = missing;
    let mut flat = Vec::with_capacity(pairs.len());
    for (v, w) in pairs {
        mix_in(v, w, &mut flat, &mut missing, budget)?;
    }
    let mut out = Vec::with_capacity(flat.len());
    let mut joint = false;
    for (v, w) in flat {
        if !analytic::contains(&v) {
            out.push((v, w));
        } else if analytic::has_outcomes(&v) {
            joint = true;
            out.push((owned(&v), w));
        } else {
            for (x, p) in truths(&v, budget)? {
                out.push((x, w * p));
            }
        }
    }
    budget.outcomes(out.len() as u128)?;
    budget.work(out.len() as u64)?;
    let d = Dist::from_pairs(out, missing.min(1.0));
    Ok(if joint {
        Value::Joint(Arc::new(d))
    } else {
        d.into_value()
    })
}

/// A value's outcomes, mixed into `flat` with weight `w`: a distribution's,
/// joint or not, and the value itself otherwise.
fn mix_in(v: Value, w: f64, flat: &mut Vec<(Value, f64)>, missing: &mut f64, budget: &mut Budget) -> OpResult<()> {
    let v = match v {
        Value::Counts(_) => crate::ops::listed(&v, budget)?.into_owned(),
        v => v,
    };
    match v {
        Value::Dist(d) | Value::Joint(d) => {
            *missing += w * d.missing;
            for (x, p) in &d.outcomes {
                match x {
                    Value::Joint(inner) => {
                        *missing += w * p * inner.missing;
                        flat.extend(inner.outcomes.iter().map(|(y, q)| (y.clone(), w * p * q)));
                    }
                    x => flat.push((x.clone(), w * p)),
                }
            }
        }
        other => flat.push((other, w)),
    }
    Ok(())
}

/// A value with its latents numbered from `OWNED`, in their order.
fn owned(v: &Value) -> Value {
    let mut ids = BTreeSet::new();
    analytic::collect_ids(v, &mut ids);
    let map: BTreeMap<u64, u64> = ids.into_iter().zip(OWNED + 1..).collect();
    analytic::renamed(v, &map)
}

/// The values a value of events becomes, each event as true or false, with
/// their probabilities.
fn truths(v: &Value, budget: &mut Budget) -> OpResult<Vec<(Value, f64)>> {
    let mut events: Vec<Arc<Event>> = Vec::new();
    collect_events(v, &mut events);
    if events.len() > MAX_EVENTS {
        return Err(OpError::limit(format!(
            "a `simulate` value with more than {MAX_EVENTS} conditions on continuous draws is over the limit"
        )));
    }
    budget.work(1u64 << events.len())?;
    let mut out = Vec::new();
    for mask in 0..1u32 << events.len() {
        let mut all: Option<Event> = None;
        for (i, e) in events.iter().enumerate() {
            let side = if mask & (1 << i) != 0 {
                (**e).clone()
            } else {
                e.complement()?
            };
            all = Some(match all {
                None => side,
                Some(all) => all.combine(&side, BinOp::And)?,
            });
        }
        let p = all.map_or(1.0, |e| e.probability());
        if p > 0.0 {
            out.push((with_truths(v, &events, mask), p));
        }
    }
    Ok(out)
}

fn collect_events(v: &Value, events: &mut Vec<Arc<Event>>) {
    match v {
        Value::Event(e) => {
            let key = e.key();
            if !events.iter().any(|x| x.key() == key) {
                events.push(e.clone());
            }
        }
        Value::List(xs) => xs.iter().for_each(|x| collect_events(x, events)),
        Value::Map(xs) => xs.values().for_each(|x| collect_events(x, events)),
        Value::Record(r) => r.fields.iter().for_each(|(_, x)| collect_events(x, events)),
        Value::Dist(d) => d.outcomes.iter().for_each(|(x, _)| collect_events(x, events)),
        Value::Closure(c) => c.captured.iter().for_each(|x| collect_events(x, events)),
        _ => {}
    }
}

/// `v` with each event of `events` true where `mask` has its bit.
fn with_truths(v: &Value, events: &[Arc<Event>], mask: u32) -> Value {
    if !analytic::contains(v) {
        return v.clone();
    }
    let child = |x: &Value| with_truths(x, events, mask);
    match v {
        Value::Event(e) => {
            let key = e.key();
            let i = events.iter().position(|x| x.key() == key).expect("a collected event");
            Value::Bool(mask & (1 << i) != 0)
        }
        Value::List(xs) => Value::list(xs.iter().map(child).collect()),
        Value::Map(xs) => Value::map(xs.iter().map(|(k, x)| (k.clone(), child(x))).collect()),
        Value::Record(r) => crate::ops::make_record(
            r.ty.clone(),
            r.fields.iter().map(|(k, x)| (k.clone(), child(x))).collect(),
        ),
        Value::Dist(d) => {
            Dist::from_pairs(d.outcomes.iter().map(|(x, p)| (child(x), *p)).collect(), d.missing).into_value()
        }
        Value::Closure(c) => Value::Closure(Arc::new(crate::value::Closure {
            func: c.func,
            captured: c.captured.iter().map(child).collect(),
        })),
        v => v.clone(),
    }
}

/// An outcome drawn into a world: its draws with fresh numbers after
/// `next`.
pub fn fresh(v: &Value, next: &mut u64) -> Value {
    let mut ids = BTreeSet::new();
    analytic::collect_ids(v, &mut ids);
    if ids.is_empty() {
        return v.clone();
    }
    let map: BTreeMap<u64, u64> = ids
        .into_iter()
        .map(|id| {
            *next += 1;
            (id, *next)
        })
        .collect();
    analytic::renamed(v, &map)
}

/// A draw when sampling: an outcome chosen with its probability, with a
/// value for each of its draws, from its distribution where it can be.
#[cold]
#[inline(never)]
pub fn sample(d: &Dist, rng: &mut Rng) -> Value {
    let i = rng.choose(d.outcomes.iter().map(|(_, p)| *p)).unwrap_or(0);
    let v = &d.outcomes[i].0;
    let mut latents: BTreeMap<u64, (Family, Domain)> = BTreeMap::new();
    gather(v, &mut latents);
    let at: BTreeMap<u64, (f64, f64)> = latents
        .into_iter()
        .map(|(id, (family, domain))| {
            let c = within(&domain, rng);
            (id, (c, family.quantile(c)))
        })
        .collect();
    concrete(v, &at)
}

/// Each latent of a value, with its distribution and where it can be.
fn gather(v: &Value, latents: &mut BTreeMap<u64, (Family, Domain)>) {
    let mut add = |a: &analytic::Analytic| match &a.form {
        Some(f) => {
            for t in &f.terms {
                latents.entry(t.id).or_insert((t.family, Domain::full()));
            }
        }
        None => {
            let entry = latents.entry(a.id).or_insert((a.family, a.domain.clone()));
            entry.1 = entry.1.intersect(&a.domain);
        }
    };
    let mut pending = vec![v];
    while let Some(v) = pending.pop() {
        match v {
            Value::Analytic(a) => add(a),
            Value::Event(e) => e.draws.iter().for_each(&mut add),
            Value::List(xs) => pending.extend(xs.iter()),
            Value::Map(xs) => pending.extend(xs.values()),
            Value::Record(r) => pending.extend(r.fields.iter().map(|(_, x)| x)),
            Value::Dist(d) => pending.extend(d.outcomes.iter().map(|(x, _)| x)),
            Value::Closure(c) => pending.extend(c.captured.iter()),
            _ => {}
        }
    }
}

/// A CDF coordinate drawn uniformly from where a latent can be.
fn within(domain: &Domain, rng: &mut Rng) -> f64 {
    let mut left = rng.open() * domain.mass();
    for &(a, b) in &domain.0 {
        if left <= b - a {
            return (a + left).clamp(a, b);
        }
        left -= b - a;
    }
    domain.0.last().map_or(0.5, |&(_, b)| b)
}

/// An axis's coordinate and value, given its latents' as `at` has them.
fn axis(a: &analytic::Analytic, at: &BTreeMap<u64, (f64, f64)>) -> (f64, f64) {
    match &a.form {
        Some(f) => {
            let t = f.constant + f.terms.iter().map(|t| t.coef * at[&t.id].1).sum::<f64>();
            (a.family.cdf(t), t)
        }
        None => at[&a.id],
    }
}

/// `v` with each outcome of a draw at the values its latents have in `at`.
fn concrete(v: &Value, at: &BTreeMap<u64, (f64, f64)>) -> Value {
    if !analytic::contains(v) {
        return v.clone();
    }
    let child = |x: &Value| concrete(x, at);
    match v {
        Value::Analytic(a) => {
            let (c, t) = axis(a, at);
            a.at(c, t)
        }
        Value::Event(e) => {
            let coordinates: Vec<f64> = e.draws.iter().map(|d| axis(d, at).0).collect();
            let inside = |d: &Domain, c: f64| d.0.iter().any(|&(a, b)| a <= c && c <= b);
            Value::Bool(
                e.boxes
                    .iter()
                    .any(|part| part.iter().zip(&coordinates).all(|(d, &c)| inside(d, c))),
            )
        }
        Value::List(xs) => Value::list(xs.iter().map(child).collect()),
        Value::Map(xs) => Value::map(xs.iter().map(|(k, x)| (k.clone(), child(x))).collect()),
        Value::Record(r) => crate::ops::make_record(
            r.ty.clone(),
            r.fields.iter().map(|(k, x)| (k.clone(), child(x))).collect(),
        ),
        Value::Dist(d) => {
            Dist::from_pairs(d.outcomes.iter().map(|(x, p)| (child(x), *p)).collect(), d.missing).into_value()
        }
        Value::Closure(c) => Value::Closure(Arc::new(crate::value::Closure {
            func: c.func,
            captured: c.captured.iter().map(child).collect(),
        })),
        v => v.clone(),
    }
}

/// The marginal of a joint distribution of numbers: the mixture of its
/// outcomes'. `None` if an outcome isn't a number.
pub fn mixture(d: &Dist) -> Option<Mixture> {
    let parts = d
        .outcomes
        .iter()
        .map(|(v, p)| {
            let part = match v {
                Value::Analytic(a) => Part::Analytic((**a).clone()),
                Value::Continuous(f) => Part::Continuous(**f),
                v => Part::Point(v.as_f64()?),
            };
            Some((part, *p))
        })
        .collect::<Option<_>>()?;
    Some(Mixture { parts })
}

/// `op` between a joint distribution and a value that isn't one, outcome
/// by outcome: joint again, or an ordinary distribution of what's left,
/// like the truth values of comparisons.
#[cold]
#[inline(never)]
pub fn binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    let (d, other, flipped) = match (a, b) {
        (Value::Joint(d), other) => (d, other, false),
        (other, Value::Joint(d)) => (d, other, true),
        _ => unreachable!("an operand is joint"),
    };
    if matches!(other, Value::Joint(_)) || analytic::contains(other) {
        return Err(OpError::unsupported(format!(
            "`{}` between a `simulate` distribution of continuous outcomes and another distribution of them, or an outcome of a draw, isn't supported yet",
            op.symbol()
        ))
        .help("draw from it first, with `~`"));
    }
    let mut pairs = Vec::with_capacity(d.outcomes.len());
    for (v, p) in &d.outcomes {
        let r = if flipped {
            crate::ops::binary(op, other, v, budget)?
        } else {
            crate::ops::binary(op, v, other, budget)?
        };
        pairs.push((r, *p));
    }
    of(pairs, d.missing, budget)
}
