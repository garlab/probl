//! Moments with time dimensions. Datetime means round only once, after the
//! exact weighted sum, and population spread uses the unrounded mean.
use crate::{dates::DateTime, dist::Budget, duration, error::OpResult, value::Value};
use probl_number::{Integer, moments::IntegerMoments};

pub struct Moments {
    pub mean: Value,
    pub sd: Option<Value>,
    pub variance: Option<Value>,
}

pub fn kind(outcomes: &[(Value, f64)]) -> Option<&'static str> {
    let first = &outcomes.first()?.0;
    let name = match first {
        Value::DateTime(_) => "datetime",
        Value::Duration(d) => d.kind(),
        _ => return None,
    };
    outcomes
        .iter()
        .all(|(v, _)| match (first, v) {
            (Value::DateTime(_), Value::DateTime(_)) => true,
            (Value::Duration(a), Value::Duration(b)) => a.squared == b.squared,
            _ => false,
        })
        .then_some(name)
}

pub fn moments(outcomes: &[(Value, f64)], budget: Option<&mut Budget>) -> OpResult<Option<Moments>> {
    compute(outcomes.iter().map(|(v, w)| (v, *w)), outcomes.len(), budget)
}

/// Conservative summary work, including integer coordinates and binary64
/// weight alignment, for serializers that have their own work budget.
pub fn work_bound(outcomes: &[(Value, f64)]) -> u64 {
    let bits = outcomes
        .iter()
        .map(|(v, _)| match v {
            Value::Duration(d) => d.amount.as_integer().map_or(64, Integer::bits),
            _ => 128,
        })
        .max()
        .unwrap_or(0);
    let limbs = bits.div_ceil(64).max(1);
    let scratch = (2 * bits + 2200).div_ceil(64);
    (outcomes.len() as u64)
        .saturating_mul(128 + 4 * limbs * limbs)
        .saturating_add(16 + 8 * scratch * scratch)
}

pub fn list_moments(values: &[Value], budget: Option<&mut Budget>) -> OpResult<Option<Moments>> {
    compute(values.iter().map(|v| (v, 1.0)), values.len(), budget)
}

fn compute<'a>(
    outcomes: impl Iterator<Item = (&'a Value, f64)> + Clone,
    count: usize,
    mut budget: Option<&mut Budget>,
) -> OpResult<Option<Moments>> {
    let Some((first, _)) = outcomes.clone().next() else {
        return Ok(None);
    };
    let kind = match first {
        Value::DateTime(_) => "datetime",
        Value::Duration(d) => d.kind(),
        _ => return Ok(None),
    };
    if !outcomes.clone().all(|(v, _)| match (first, v) {
        (Value::DateTime(_), Value::DateTime(_)) => true,
        (Value::Duration(a), Value::Duration(b)) => a.squared == b.squared,
        _ => false,
    }) {
        return Ok(None);
    }
    if let Some(budget) = budget.as_deref_mut() {
        budget.work((count as u64).saturating_mul(64))?;
    }
    let coordinate = |v: &Value| match v {
        Value::DateTime(d) => Some(Integer::from(d.total_nanos())),
        Value::Duration(d) => d.amount.as_integer().cloned(),
        _ => None,
    };
    let original_squared = kind == "squared_duration";
    let result = if outcomes.clone().all(|(v, _)| coordinate(v).is_some()) {
        let mut m = IntegerMoments::default();
        for (v, w) in outcomes.clone() {
            let n = coordinate(v).unwrap();
            if let Some(budget) = budget.as_deref_mut() {
                let limbs = n.bits().div_ceil(64).max(1);
                budget.work(64 + 4 * limbs * limbs)?;
            }
            if !m.push_integer(&n, w) {
                return Ok(None);
            }
        }
        if let Some(budget) = budget {
            budget.work(m.finish_work())?;
        }
        let mean = if kind == "datetime" {
            m.rounded_mean()
                .and_then(DateTime::from_total_nanos)
                .map(Value::DateTime)
        } else {
            m.integral_mean()
                .map(|n| duration::from_number(Value::Int(n), original_squared))
                .or_else(|| {
                    m.fractional_mean()
                        .map(|n| duration::from_number(Value::Float(n), original_squared))
                })
                .and_then(Result::ok)
        };
        let Some(mean) = mean else {
            return Ok(None);
        };
        let Some((v, e)) = m.variance_parts() else {
            return Ok(None);
        };
        Moments {
            mean,
            sd: duration::from_number(Value::Float(libm::scalbn(libm::sqrt(v), e / 2)), original_squared).ok(),
            variance: (!original_squared)
                .then(|| duration::from_number(Value::Float(libm::scalbn(v, e)), true))
                .and_then(Result::ok),
        }
    } else {
        let numeric = || {
            outcomes.clone().map(|(v, w)| match v {
                Value::Duration(d) => (d.amount.as_float().unwrap_or(f64::NAN), w),
                _ => unreachable!(),
            })
        };
        let mean = crate::stats::weighted_mean(numeric());
        let sd = crate::stats::weighted_sd(numeric().map(|(v, w)| (v, 0.0, w)));
        let Ok(mean) = duration::from_number(Value::Float(mean), original_squared) else {
            return Ok(None);
        };
        Moments {
            mean,
            sd: duration::from_number(Value::Float(sd), original_squared).ok(),
            variance: (!original_squared)
                .then(|| duration::from_number(Value::Float(sd * sd), true))
                .and_then(Result::ok),
        }
    };
    Ok(Some(result))
}
