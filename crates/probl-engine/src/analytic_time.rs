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
    Date(i32),
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Duration(false) => "duration",
            Self::Duration(true) => "squared_duration",
            Self::DateTime(_) => "datetime",
            Self::Date(_) => "date",
        }
    }
    pub fn value(self, n: f64) -> OpResult<Value> {
        match self {
            Self::Duration(squared) => duration::from_number(Value::Float(n), squared),
            Self::Date(origin) => probl_number::Integer::from_f64(duration::round_nanos(n))
                .and_then(|n| n.to_i64())
                .and_then(|n| crate::dates::add_days(origin, n))
                .map(Value::Date)
                .ok_or_else(|| OpError::fault(Fault::NumericOverflow, "date out of range (years 1..9999)")),
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
        Value::Date(d) => Some(Kind::Date(*d)),
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
                Some(Kind::Date(t)) => t as i128 - origin,
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
        Value::Date(t) => Ok(Value::Int((*t as i128 - origin).into())),
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
    if matches!(ka, Some(Date(_))) || matches!(kb, Some(Date(_))) {
        return date_binary(op, a, b, budget);
    }
    if compare(op) && ((ka == Some(Duration(false)) && kb.is_none()) || (kb == Some(Duration(false)) && ka.is_none())) {
        return Err(crate::duration::day_count_hint(OpError::new(format!(
            "can't compare {} with {}",
            ops::article(&a.kind()),
            ops::article(&b.kind())
        ))));
    }
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
            (Some(Duration(x)), Some(Duration(y))) if x == y && matches!(op, IntDiv | Mod) => {
                return Err(analytic::unsupported(&format!(
                    "`{}` on analytic durations",
                    op.symbol()
                )));
            }
            _ => return Err(bad()),
        };
    let (x, y) = (coordinate(a, origin, budget)?, coordinate(b, origin, budget)?);
    let v = ops::binary(op, &x, &y, budget).map_err(|e| {
        if e.kind == crate::error::ErrorKind::Unsupported && matches!(op, Add | Sub)
            && [a,b].iter().any(|v| matches!(v,Value::Analytic(x) if x.quantized())) {
            e.help("sum the numeric delays before applying the time unit and rounding, when one final rounding is intended; otherwise use `@mode sample(runs: 10_000)`")
        } else { e }
    })?;
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
        if !outside.is_finite() || outside > 0.0 {
            return Err(OpError::new(
                "datetime arithmetic has outcomes outside the supported calendar (years 1..9999)",
            )
            .help("use bounded delays or move the starting datetime away from the calendar boundary"));
        }
    }
    wrap(v, Kind::DateTime(origin))
}
pub(crate) fn negate(x: &Analytic, kind: Kind) -> OpResult<Value> {
    if matches!(kind, Kind::DateTime(_) | Kind::Date(_)) {
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
        Kind::Date(t) => t as i128,
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
    if matches!(k, Kind::Date(_)) {
        return None;
    }
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
            B::Sd | B::Variance if matches!(kind, Kind::Date(_)) => {
                return Err(OpError::new("dates do not support spread statistics"));
            }
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
                    Kind::Date(t) => t as i128,
                    _ => 0,
                };
                let at = coordinate(&args[1], origin, &mut Budget::unlimited())?
                    .as_f64()
                    .ok_or_else(bad)?;
                let p = if b == B::Cdf {
                    m.cdf(at)
                } else {
                    match kind {
                        Kind::DateTime(_) | Kind::Date(_) => m.cdf(at) - m.cdf(at - 1.0),
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

fn date_binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    use BinOp::*;
    use Kind::*;
    if op == Add && !matches!(kind(a), Some(Date(_))) {
        return date_binary(op, b, a, budget);
    }
    let Some(Date(origin)) = kind(a) else {
        return Err(bad());
    };
    let x = coordinate(a, origin as i128, budget)?;
    match kind(b) {
        Some(Date(_)) if compare(op) || op == Sub => {
            let y = coordinate(b, origin as i128, budget)?;
            let result = ops::binary(op, &x, &y, budget)?;
            if op == Sub {
                duration::construct(&result, crate::dates::NANOS_PER_DAY as u64, budget)
            } else {
                Ok(result)
            }
        }
        Some(Duration(false)) if matches!(op, Add | Sub) => {
            let n = coordinate(b, 0, budget)?;
            let days = ops::binary(Div, &n, &Value::Int(crate::dates::NANOS_PER_DAY.into()), budget)?;
            let integral = match &days {
                Value::Analytic(a) => a.pieces.iter().all(|(_, f)| f.integer_valued()),
                v => v.as_f64().is_some_and(|n| n.fract() == 0.0),
            };
            if !integral {
                return Err(crate::duration::whole_days_error());
            }
            let v = ops::binary(op, &x, &days, budget)?;
            date_from_days(v, origin)
        }
        None if matches!(op, Add | Sub) && (matches!(b, Value::Int(_)) || matches!(b,Value::Analytic(a) if a.int)) => {
            date_from_days(ops::binary(op, &x, b, budget)?, origin)
        }
        _ => Err(bad()),
    }
}
fn date_from_days(v: Value, origin: i32) -> OpResult<Value> {
    if let Value::Analytic(a) = &v {
        let outside = a.cdf((crate::dates::MIN as i64 - origin as i64 - 1) as f64)
            + (1.0 - a.cdf((crate::dates::MAX as i64 - origin as i64) as f64));
        if !outside.is_finite() || outside > 0.0 {
            return Err(
                OpError::new("date arithmetic has outcomes outside the supported calendar (years 1..9999)")
                    .help("use bounded delays or move the starting date away from the calendar boundary"),
            );
        }
    }
    wrap(v, Kind::Date(origin))
}

/// Calendar/clock projections on the same latent, preserving observations.
pub(crate) fn project(b: probl_sema::Builtin, a: &Analytic, budget: &mut Budget) -> OpResult<Value> {
    use probl_sema::Builtin as B;
    let n = Analytic {
        time: None,
        ..a.clone()
    };
    match (b, a.time) {
        (B::Date, Some(Kind::Date(_))) | (B::DateTime, Some(Kind::DateTime(_))) => {
            Ok(Value::Analytic(Arc::new(a.clone())))
        }
        (B::DateTime, Some(Kind::Date(origin))) => {
            let v = ops::binary(
                BinOp::Mul,
                &Value::Analytic(Arc::new(n)),
                &Value::Int(crate::dates::NANOS_PER_DAY.into()),
                budget,
            )?;
            wrap(v, Kind::DateTime(origin as i128 * crate::dates::NANOS_PER_DAY))
        }
        (B::Date, Some(Kind::DateTime(origin))) => {
            let day = origin.div_euclid(crate::dates::NANOS_PER_DAY);
            let clock = origin.rem_euclid(crate::dates::NANOS_PER_DAY);
            let v = analytic::time_floor(&n, clock as f64, crate::dates::NANOS_PER_DAY as f64, budget)?;
            wrap(v, Kind::Date(day as i32))
        }
        (B::Hour | B::Minute | B::Second | B::Nanosecond, Some(Kind::DateTime(origin))) => {
            let (unit, modulus) = match b {
                B::Hour => (3_600_000_000_000.0, 24.0),
                B::Minute => (60_000_000_000.0, 60.0),
                B::Second => (1_000_000_000.0, 60.0),
                _ => (1.0, 1_000_000_000.0),
            };
            let clock = origin.rem_euclid(crate::dates::NANOS_PER_DAY) as f64;
            match analytic::time_floor(&n, clock, unit, budget)? {
                Value::Analytic(x) => analytic::clock_field(&x, modulus, budget),
                v => Ok(Value::Int(
                    probl_number::Integer::from_f64(v.as_f64().unwrap().rem_euclid(modulus)).unwrap(),
                )),
            }
        }
        _ => Err(OpError::new(format!(
            "`{}` needs a compatible calendar value",
            b.name()
        ))),
    }
}

/// A continuous interval on the relative clock, rounded to nanoseconds when
/// drawn. The exact epoch is never folded into a binary64 timestamp.
pub(crate) fn distribution(b: probl_sema::Builtin, lo: DateTime, hi: DateTime, budget: &mut Budget) -> OpResult<Value> {
    use crate::{continuous::Family, dist::Dist};
    let width = hi.total_nanos() - lo.total_nanos();
    if width == 0 && b == probl_sema::Builtin::Uniform {
        return Ok(Dist::from_pairs(vec![(Value::DateTime(lo), 1.0)], 0.0).into_value());
    }
    if width <= 0 {
        return Err(OpError::fault(
            Fault::DomainError,
            "datetime interval needs its lower endpoint before its upper endpoint",
        ));
    }
    let family = if b == probl_sema::Builtin::Uniform {
        Family::uniform(0.0, width as f64)?
    } else {
        Family::normal_range(0.0, width as f64)?
    };
    let v = datetime(
        Value::Analytic(Arc::new(Analytic::new(0, family))),
        lo.total_nanos(),
        budget,
    )?;
    crate::joint::of(vec![(v, 1.0)], 0.0, budget)
}
