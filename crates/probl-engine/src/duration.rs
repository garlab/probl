//! Typed elapsed time and dimensional arithmetic.
use crate::{
    dist::Budget,
    error::{Fault, OpError, OpResult},
    ops,
    value::Value,
};
use probl_number::Integer;
use probl_syntax::{
    ast::BinOp,
    duration::{Amount, Duration},
};

pub fn literal(d: &Duration, budget: &mut Budget) -> OpResult<Value> {
    if let Some(n) = d.amount.as_integer() {
        budget.integer_bits(n.bits())?;
    }
    Ok(Value::Duration(d.clone()))
}
pub fn number(d: &Duration) -> Value {
    d.amount
        .as_integer()
        .map_or_else(|| Value::Float(d.amount.as_float().unwrap()), |n| Value::Int(n.clone()))
}
pub fn from_number(v: Value, squared: bool) -> OpResult<Value> {
    let amount = match v {
        Value::Analytic(a) if a.time.is_none() => {
            return crate::analytic_time::wrap(Value::Analytic(a), crate::analytic_time::Kind::Duration(squared));
        }
        Value::Int(n) => Amount::integer(n),
        Value::Float(f) | Value::Prob(f) => {
            Amount::fractional(f).ok_or_else(|| OpError::fault(Fault::NumericOverflow, "duration must be finite"))?
        }
        _ => return Err(OpError::new("a duration needs a real number")),
    };
    Ok(Value::Duration(Duration { squared, amount }))
}
pub fn construct(n: &Value, scale: u64, budget: &mut Budget) -> OpResult<Value> {
    if !matches!(n, Value::Int(_) | Value::Float(_) | Value::Prob(_))
        && !matches!(n, Value::Analytic(a) if a.time.is_none())
    {
        return Err(OpError::new("a duration's count must be a real number"));
    }
    from_number(ops::binary(BinOp::Mul, n, &Value::Int(scale.into()), budget)?, false)
}

/// Some result when at least one operand is a duration, or matching dates or
/// datetimes are subtracted.
pub fn binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> Option<OpResult<Value>> {
    use BinOp::*;
    let elapsed = match (op, a, b) {
        (Sub, Value::Date(a), Value::Date(b)) => Some((i128::from(*a) - i128::from(*b)) * crate::dates::NANOS_PER_DAY),
        (Sub, Value::DateTime(a), Value::DateTime(b)) => Some(a.total_nanos() - b.total_nanos()),
        _ => None,
    };
    if let Some(nanos) = elapsed {
        let n = Integer::from(nanos);
        if let Err(error) = budget.integer_allocation(n.bits(), 1) {
            return Some(Err(error));
        }
        return Some(from_number(Value::Int(n), false));
    }
    if !matches!(a, Value::Duration(_)) && !matches!(b, Value::Duration(_)) {
        return None;
    }
    Some((|| {
        let value = match (a, b) {
            (Value::DateTime(t), Value::Duration(d)) if matches!(op, Add | Sub) && !d.squared => {
                let n = match d.amount.as_integer() {
                    Some(n) => {
                        budget.integer_work(n, &Integer::ONE, false)?;
                        if op == Sub { n.negated() } else { n.clone() }
                    }
                    None => {
                        let n = d.amount.as_float().unwrap();
                        Integer::from_f64(round_nanos(if op == Sub { -n } else { n }))
                            .ok_or_else(|| OpError::fault(Fault::NumericOverflow, "datetime delay out of range"))?
                    }
                };
                n.to_i128()
                    .and_then(|n| t.add_nanos(n))
                    .map(Value::DateTime)
                    .ok_or_else(|| OpError::fault(Fault::NumericOverflow, "datetime out of range (years 1..9999)"))
            }
            (Value::Date(day), Value::Duration(d)) if matches!(op, Add | Sub) && !d.squared => {
                let n = whole_nanos(d, budget)?;
                let (days, remainder) = n.div_mod(&Integer::from(crate::dates::NANOS_PER_DAY))?;
                if !remainder.is_zero() {
                    return Err(whole_days_error());
                }
                let days = if op == Sub { days.negated() } else { days };
                days.to_i64()
                    .and_then(|n| crate::dates::add_days(*day, n))
                    .map(Value::Date)
                    .ok_or_else(|| OpError::fault(Fault::NumericOverflow, "date out of range"))
            }
            (Value::Duration(_), Value::DateTime(_) | Value::Date(_)) if op == Add => binary(op, b, a, budget).unwrap(),
            (Value::Duration(a), Value::Duration(b)) => {
                let (x, y) = (number(a), number(b));
                match op {
                    Add | Sub | Mod if a.squared == b.squared => {
                        from_number(ops::binary(op, &x, &y, budget)?, a.squared)
                    }
                    Mul if !a.squared && !b.squared => from_number(ops::binary(op, &x, &y, budget)?, true),
                    Div | IntDiv if a.squared == b.squared => ops::binary(op, &x, &y, budget),
                    Div if a.squared && !b.squared => from_number(divide(&x, &y, budget)?, false),
                    _ => Err(OpError::new("incompatible time dimensions for this operation")),
                }
            }
            (Value::Duration(d), Value::Int(n)) if op == Pow => match n.to_i64() {
                Some(0) => Ok(Value::Int(1.into())),
                Some(1) => Ok(a.clone()),
                Some(2) if !d.squared => from_number(ops::binary(Mul, &number(d), &number(d), budget)?, true),
                _ => Err(OpError::new("only duration powers 0, 1 and 2 are supported")),
            },
            (Value::Duration(d), n @ (Value::Int(_) | Value::Float(_) | Value::Prob(_))) if matches!(op, Mul | Div) => {
                let value = if op == Div {
                    divide(&number(d), n, budget)?
                } else {
                    ops::binary(op, &number(d), n, budget)?
                };
                from_number(value, d.squared)
            }
            (n @ (Value::Int(_) | Value::Float(_) | Value::Prob(_)), Value::Duration(d)) if op == Mul => {
                from_number(ops::binary(op, n, &number(d), budget)?, d.squared)
            }
            _ => Err(OpError::new(
                "duration arithmetic requires compatible units; divide by a duration such as '1s' to obtain a number",
            )),
        }?;
        if let Value::Duration(d) = &value
            && let Some(n) = d.amount.as_integer()
        {
            budget.integer_allocation(n.bits(), 1)?;
        }
        Ok(value)
    })())
}

fn divide(a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    if let (Value::Int(a), Value::Int(b)) = (a, b)
        && !b.is_zero()
    {
        budget.integer_work(a, b, true)?;
        let (whole, remainder) = a.div_mod(b)?;
        if remainder.is_zero() {
            budget.integer_allocation(whole.bits(), 1)?;
            return Ok(Value::Int(whole));
        }
    }
    ops::binary(BinOp::Div, a, b, budget)
}
fn whole_nanos(d: &Duration, budget: &mut Budget) -> OpResult<Integer> {
    match d.amount.as_integer() {
        Some(n) => {
            budget.integer_work(n, &Integer::ONE, false)?;
            Ok(n.clone())
        }
        None => Err(whole_days_error()),
    }
}

/// Nearest nanosecond, ties toward the earlier time. Avoid subtracting 0.5
/// from an already integral large float, which can itself round incorrectly.
pub(crate) fn round_nanos(n: f64) -> f64 {
    let floor = n.floor();
    floor + if n - floor > 0.5 { 1.0 } else { 0.0 }
}

pub(crate) fn whole_days_error() -> OpError {
    OpError::new("adding a duration to a date requires a whole number of 24-hour days")
        .help("round explicitly, such as `start + days(round(x))`; use `datetime(start)` to retain fractional days")
}

pub(crate) fn day_count_hint(error: OpError) -> OpError {
    error.help("date subtraction returns a duration; divide by '1d' for a day count, or use `div '1d'` when an int is required")
}
