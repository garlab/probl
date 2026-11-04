//! Time dimensions on analytic outcomes. Datetimes use an exact epoch and a
//! rounded relative offset, so calendar magnitude never consumes clock precision.
use crate::{
    analytic::{self, Analytic},
    continuous::{Mixture, Part},
    dates::DateTime,
    dist::Budget,
    duration,
    error::{Fault, OpError, OpResult},
    ops,
    value::Value,
};
use probl_syntax::ast::BinOp;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Duration(bool),
    DateTime(i128),
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Duration(false) => "duration",
            Self::Duration(true) => "squared_duration",
            Self::DateTime(_) => "datetime",
        }
    }
    pub fn value(self, n: f64) -> OpResult<Value> {
        match self {
            Self::Duration(squared) => duration::from_number(Value::Float(n), squared),
            Self::DateTime(origin) => probl_number::Integer::from_f64(duration::round_nanos(n))
                .and_then(|n| n.to_i128())
                .and_then(|n| origin.checked_add(n))
                .and_then(DateTime::from_total_nanos)
                .map(Value::DateTime)
                .ok_or_else(|| OpError::fault(Fault::NumericOverflow, "datetime out of range (years 1..9999)")),
        }
    }
}
pub fn kind(v: &Value) -> Option<Kind> {
    match v {
        Value::DateTime(t) => Some(Kind::DateTime(t.total_nanos())),
        Value::Duration(d) => Some(Kind::Duration(d.squared)),
        Value::Analytic(a) => a.time,
        _ => None,
    }
}
pub fn has_time(v: &Value) -> bool {
    kind(v).is_some()
}

/// Strip only the time dimension. Datetime coordinates are relative to `origin`.
fn coordinate(v: &Value, origin: i128, budget: &mut Budget) -> OpResult<Value> {
    match v {
        Value::Analytic(a) => {
            let shift = match a.time {
                Some(Kind::DateTime(t)) => t - origin,
                _ => 0,
            };
            let n = Value::Analytic(Arc::new(Analytic {
                time: None,
                ..(**a).clone()
            }));
            if shift == 0 {
                Ok(n)
            } else {
                ops::binary(BinOp::Add, &n, &Value::Int(shift.into()), budget)
            }
        }
        Value::DateTime(t) => Ok(Value::Int((t.total_nanos() - origin).into())),
        Value::Duration(d) => Ok(duration::number(d)),
        _ => Ok(v.clone()),
    }
}
pub(crate) fn wrap(v: Value, kind: Kind) -> OpResult<Value> {
    match v {
        Value::Analytic(a) => Analytic {
            time: Some(kind),
            int: false,
            ..(*a).clone()
        }
        .value(),
        Value::Int(_) | Value::Float(_) | Value::Prob(_) => match kind {
            Kind::Duration(squared) => duration::from_number(v, squared),
            _ => kind.value(v.as_f64().ok_or_else(|| OpError::new("datetime delay out of range"))?),
        },
        _ => Err(OpError::new("time arithmetic requires a real number")),
    }
}
fn bad() -> OpError {
    OpError::new("incompatible time dimensions for this operation")
}
fn compare(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    )
}

pub(crate) fn binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    use BinOp::*;
    use Kind::*;
    let (ka, kb) = (kind(a), kind(b));
    // A fixed whole-nanosecond shift changes only the exact epoch.
    if let (Value::Analytic(t), Value::Duration(d)) = (a, b)
        && let Some(DateTime(origin)) = t.time
        && !d.squared
        && matches!(op, Add | Sub)
        && let Some(n) = d.amount.as_integer().and_then(|n| n.to_i128())
    {
        let origin = if op == Sub {
            origin.checked_sub(n)
        } else {
            origin.checked_add(n)
        }
        .ok_or_else(|| OpError::fault(Fault::NumericOverflow, "datetime delay out of range"))?;
        return datetime(
            coordinate(
                a,
                match t.time.unwrap() {
                    DateTime(o) => o,
                    _ => unreachable!(),
                },
                budget,
            )?,
            origin,
            budget,
        );
    }
    if op == Add && matches!(ka, Some(Duration(false))) && matches!(kb, Some(DateTime(_))) {
        return binary(op, b, a, budget);
    }
    let origin = match (ka, kb) {
        (Some(DateTime(t)), _) | (_, Some(DateTime(t))) => t,
        _ => 0,
    };
    let result =
        match (ka, kb) {
            (Some(DateTime(_)), Some(Duration(false))) if matches!(op, Add | Sub) => Some(DateTime(origin)),
            (Some(DateTime(_)), Some(DateTime(_))) if op == Sub => Some(Duration(false)),
            (Some(DateTime(_)), Some(DateTime(_))) if compare(op) => None,
            (Some(Duration(x)), Some(Duration(y))) if x == y && (matches!(op, Add | Sub) || compare(op)) => {
                if compare(op) { None } else { Some(Duration(x)) }
            }
            (Some(Duration(false)), Some(Duration(false))) if op == Mul => Some(Duration(true)),
            (Some(Duration(x)), Some(Duration(y))) if x == y && op == Div => None,
            (Some(Duration(true)), Some(Duration(false))) if op == Div => Some(Duration(false)),
            (Some(Duration(x)), None) if matches!(op, Mul | Div) => Some(Duration(x)),
            (None, Some(Duration(x))) if op == Mul => Some(Duration(x)),
            (Some(Duration(x)), None) if op == Pow => {
                return match b.as_f64() {
                    Some(0.0) => Ok(Value::Int(1.into())),
                    Some(1.0) => Ok(a.clone()),
                    Some(2.0) if !x => binary(Mul, a, a, budget),
                    _ => Err(bad()),
                };
            }
            _ => return Err(bad()),
        };
    let (x, y) = (coordinate(a, origin, budget)?, coordinate(b, origin, budget)?);
    let v = ops::binary(op, &x, &y, budget)?;
    match result {
        Some(DateTime(t)) => datetime(v, t, budget),
        Some(k) => wrap(v, k),
        None => Ok(v),
    }
}
fn datetime(v: Value, origin: i128, budget: &mut Budget) -> OpResult<Value> {
    let v = match v {
        Value::Analytic(a) => analytic::nanoseconds(&a, budget)?,
        v => v,
    };
    if let Value::Analytic(a) = &v {
        // Match other analytic domain checks: do not silently clamp or discard
        // probability outside the calendar. Extremely remote normal tails may
        // underflow to zero in the underlying CDF.
        let low = crate::dates::MIN as i128 * crate::dates::NANOS_PER_DAY;
        let high = (crate::dates::MAX as i128 + 1) * crate::dates::NANOS_PER_DAY;
        let outside = a.cdf(low.saturating_sub(origin).saturating_sub(1) as f64)
            + (1.0 - a.cdf(high.saturating_sub(origin).saturating_sub(1) as f64));
        if outside > 0.0 {
            return Err(analytic::unsupported(
                "datetime arithmetic outside years 1..9999 on part of its range",
            ));
        }
    }
    wrap(v, Kind::DateTime(origin))
}
pub(crate) fn negate(x: &Analytic, kind: Kind) -> OpResult<Value> {
    if matches!(kind, Kind::DateTime(_)) {
        return Err(bad());
    }
    wrap(
        analytic::negate(&Analytic {
            time: None,
            ..x.clone()
        })?,
        kind,
    )
}

/// Homogeneous temporal marginal with an exact shared epoch.
pub fn mixture(outcomes: &[(Value, f64)]) -> Option<(Kind, Mixture)> {
    let k = kind(&outcomes.first()?.0)?;
    let origin = match k {
        Kind::DateTime(t) => t,
        _ => 0,
    };
    let mut budget = Budget::unlimited();
    let parts = outcomes
        .iter()
        .map(|(v, p)| {
            if kind(v)?.name() != k.name() {
                return None;
            }
            let v = coordinate(v, origin, &mut budget).ok()?;
            let part = match v {
                Value::Analytic(a) => Part::Analytic((*a).clone()),
                v => Part::Point(v.as_f64()?),
            };
            Some((part, *p))
        })
        .collect::<Option<_>>()?;
    Some((k, Mixture { parts }))
}
pub fn quantile(outcomes: &[(Value, f64)], q: f64) -> Option<Value> {
    let (k, m) = mixture(outcomes)?;
    k.value(m.quantile(q)).ok()
}
pub fn median(outcomes: &[(Value, f64)]) -> Option<Value> {
    let (k, m) = mixture(outcomes)?;
    let (lo, hi) = m.median_bounds();
    k.value(crate::stats::midpoint(lo, hi)).ok()
}
pub fn moments(outcomes: &[(Value, f64)]) -> Option<crate::time_stats::Moments> {
    let (k, m) = mixture(outcomes)?;
    let squared = k == Kind::Duration(true);
    let mean = k.value(m.mean()).ok()?;
    let sd = m.sd();
    Some(crate::time_stats::Moments {
        mean,
        sd: duration::from_number(Value::Float(sd), squared).ok(),
        variance: if squared {
            None
        } else {
            duration::from_number(Value::Float(sd * sd), true).ok()
        },
    })
}

/// Typed queries on a `simulate` distribution of analytic temporal values.
pub(crate) fn query(b: probl_sema::Builtin, args: &[Value]) -> Option<OpResult<Value>> {
    use probl_sema::Builtin as B;
    let Value::Joint(d) = &args[0] else {
        return None;
    };
    let (kind, m) = mixture(&d.outcomes)?;
    Some((|| {
        let value = match b {
            B::Mean => kind.value(m.mean())?,
            B::Sd => duration::from_number(Value::Float(m.sd()), kind == Kind::Duration(true))?,
            B::Variance if kind != Kind::Duration(true) => duration::from_number(Value::Float(m.variance()), true)?,
            B::Quantile => kind.value(m.quantile(ops::to_prob(&args[1])?))?,
            B::Median | B::MedianLow | B::MedianHigh => {
                let (lo, hi) = m.median_bounds();
                kind.value(match b {
                    B::MedianLow => lo,
                    B::MedianHigh => hi,
                    _ => crate::stats::midpoint(lo, hi),
                })?
            }
            B::Cdf | B::Pmf => {
                if crate::analytic_time::kind(&args[1]).map(Kind::name) != Some(kind.name()) {
                    return Err(bad());
                }
                let origin = match kind {
                    Kind::DateTime(t) => t,
                    _ => 0,
                };
                let at = coordinate(&args[1], origin, &mut Budget::unlimited())?
                    .as_f64()
                    .ok_or_else(bad)?;
                let p = if b == B::Cdf {
                    m.cdf(at)
                } else {
                    match kind {
                        Kind::DateTime(_) => m.cdf(at) - m.cdf(at - 1.0),
                        _ => return Err(analytic::unsupported("pmf of analytic durations")),
                    }
                };
                ops::computed_prob(p, b.name())?
            }
            B::Pdf => {
                return Err(OpError::new(
                    "pdf needs a continuous numeric distribution; use cdf for temporal forecasts",
                ));
            }
            _ => {
                return Err(analytic::unsupported(&format!(
                    "`{}` on analytic time distributions",
                    b.name()
                )));
            }
        };
        Ok(value)
    })())
}
