//! Operators, and applying operations to every outcome of a distribution.

use crate::dist::{Dist, MAX_OUTCOMES};
use crate::error::{OpError, OpResult};
use crate::value::{EnumValue, Record, Value, prob};
use probl_syntax::ast::{BinOp, UnOp};
use std::sync::Arc;

/// The outcomes of a value: those of a distribution, or the value itself.
pub fn outcomes(v: &Value) -> std::borrow::Cow<'_, [(Value, f64)]> {
    match v {
        Value::Dist(d) => std::borrow::Cow::Borrowed(&d.outcomes),
        other => std::borrow::Cow::Owned(vec![(other.clone(), 1.0)]),
    }
}

fn missing(v: &Value) -> f64 {
    match v {
        Value::Dist(d) => d.missing,
        _ => 0.0,
    }
}

/// Gather the results of applying an operation to each outcome.
///
/// If every result is a probability, the answer is the overall chance (so
/// `d6 > 4` is 33.33%). Results that are distributions are mixed in.
pub fn combine(results: Vec<(Value, f64)>, missing: f64) -> OpResult<Value> {
    if results.is_empty() {
        return Err(OpError::new("the result would be an empty distribution"));
    }
    if results.iter().all(|(v, _)| matches!(v, Value::Prob(_))) {
        let p: f64 = results
            .iter()
            .map(|(v, w)| match v {
                Value::Prob(p) => p * w,
                _ => unreachable!(),
            })
            .sum();
        return Ok(Value::Prob(p.clamp(0.0, 1.0)));
    }
    let mut flat = Vec::with_capacity(results.len());
    let mut missing = missing;
    for (v, w) in results {
        match v {
            Value::Dist(d) => {
                missing += w * d.missing;
                flat.extend(d.outcomes.iter().map(|(x, p)| (x.clone(), w * p)));
            }
            other => flat.push((other, w)),
        }
    }
    if flat.len() > MAX_OUTCOMES {
        return Err(too_many_outcomes(flat.len()));
    }
    Ok(Dist::from_pairs(flat, missing).into_value())
}

pub fn too_many_outcomes(n: usize) -> OpError {
    OpError::new(format!(
        "a distribution with {n} outcomes is too large to work with exactly"
    ))
    .help("simplify the model, or wait for sample mode (v0.2)")
}

/// Apply `f` to a value, or to every outcome of a distribution.
pub fn lift1(a: &Value, f: impl Fn(&Value) -> OpResult<Value>) -> OpResult<Value> {
    if !a.is_dist() {
        return f(a);
    }
    let mut results = Vec::new();
    for (v, w) in outcomes(a).iter() {
        results.push((f(v)?, *w));
    }
    combine(results, missing(a))
}

/// Apply `f` to two values; distributions are independent draws.
pub fn lift2(a: &Value, b: &Value, f: impl Fn(&Value, &Value) -> OpResult<Value>) -> OpResult<Value> {
    if !a.is_dist() && !b.is_dist() {
        return f(a, b);
    }
    let (oa, ob) = (outcomes(a), outcomes(b));
    if oa.len() * ob.len() > MAX_OUTCOMES {
        return Err(too_many_outcomes(oa.len() * ob.len()));
    }
    let mut results = Vec::with_capacity(oa.len() * ob.len());
    for (x, p) in oa.iter() {
        for (y, q) in ob.iter() {
            results.push((f(x, y)?, p * q));
        }
    }
    let m = 1.0 - (1.0 - missing(a)) * (1.0 - missing(b));
    combine(results, m)
}

/// Apply `f` to a list of values, lifting over any that are distributions.
pub fn lift_n(args: &[Value], f: &dyn Fn(&[Value]) -> OpResult<Value>) -> OpResult<Value> {
    if !args.iter().any(Value::is_dist) {
        return f(args);
    }
    let mut results = Vec::new();
    let mut size = 1usize;
    for a in args {
        size = size.saturating_mul(outcomes(a).len());
    }
    if size > MAX_OUTCOMES {
        return Err(too_many_outcomes(size));
    }
    let mut current = Vec::with_capacity(args.len());
    let mut kept = 1.0;
    for a in args {
        kept *= 1.0 - missing(a);
    }
    product(args, 0, &mut current, 1.0, f, &mut results)?;
    combine(results, 1.0 - kept)
}

fn product(
    args: &[Value],
    i: usize,
    current: &mut Vec<Value>,
    weight: f64,
    f: &dyn Fn(&[Value]) -> OpResult<Value>,
    results: &mut Vec<(Value, f64)>,
) -> OpResult<()> {
    if i == args.len() {
        results.push((f(current)?, weight));
        return Ok(());
    }
    for (v, w) in outcomes(&args[i]).iter() {
        current.push(v.clone());
        product(args, i + 1, current, weight * w, f, results)?;
        current.pop();
    }
    Ok(())
}

// ── Probabilities ────────────────────────────────────────────────────────

/// Interpret a value as a probability: a `prob`, a float between 0 and 1, or
/// a distribution over probabilities.
pub fn to_chance(v: &Value) -> OpResult<f64> {
    match v {
        Value::Prob(p) => Ok(*p),
        Value::Float(f) if (0.0..=1.0).contains(f) => Ok(*f),
        Value::Float(f) => Err(OpError::new(format!(
            "a probability must be between 0% and 100%, not {}",
            crate::value::fmt_float(*f)
        ))),
        Value::Int(i @ (0 | 1)) => Err(OpError::new(format!("expected a probability, found the int {i}")).help(
            if *i == 1 {
                "write `true` or `100%`"
            } else {
                "write `false` or `0%`"
            },
        )),
        Value::Dist(d) => d.chance().ok_or_else(|| {
            OpError::new(format!("expected a probability, found a {}", v.kind()))
                .help("compare it to get a probability, like `d6 > 4`, or draw a value with `~`")
        }),
        other => Err(OpError::new(format!(
            "expected a probability, found {}",
            article(&other.kind())
        ))),
    }
}

pub fn article(kind: &str) -> String {
    let vowel = kind.starts_with(['a', 'e', 'i', 'o', 'u']);
    format!("{} {kind}", if vowel { "an" } else { "a" })
}

// ── Operators ────────────────────────────────────────────────────────────

pub fn unary(op: UnOp, v: &Value) -> OpResult<Value> {
    match op {
        UnOp::Neg => lift1(v, |x| match x {
            Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(overflow),
            Value::Float(f) | Value::Prob(f) => Ok(Value::Float(-f)),
            other => Err(OpError::new(format!("can't negate {}", article(&other.kind())))),
        }),
        UnOp::Not => Ok(Value::Prob(1.0 - to_chance(v)?)),
    }
}

fn overflow() -> OpError {
    OpError::new("integer overflow").help("the result doesn't fit in 64 bits")
}

pub fn binary(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    match op {
        BinOp::And | BinOp::Or => {
            unreachable!("`and` and `or` are evaluated lazily by the interpreter")
        }
        BinOp::Range | BinOp::RangeExcl => range(op, a, b),
        BinOp::In | BinOp::NotIn => {
            let found = lift2(a, b, |x, coll| contains(coll, x).map(prob))?;
            if op == BinOp::In {
                Ok(found)
            } else {
                unary(UnOp::Not, &found)
            }
        }
        BinOp::To => unreachable!("`to` is lowered to a built-in"),
        _ => lift2(a, b, |x, y| binary_plain(op, x, y)),
    }
}

fn binary_plain(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    match op {
        BinOp::Eq => Ok(prob(equals(a, b))),
        BinOp::Ne => Ok(prob(!equals(a, b))),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare(a, b)?;
            Ok(prob(match op {
                BinOp::Lt => ord.is_lt(),
                BinOp::Le => ord.is_le(),
                BinOp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            }))
        }
        BinOp::Add => add(a, b),
        BinOp::Sub => sub(a, b),
        BinOp::Mul => arith(op, a, b),
        BinOp::Div | BinOp::IntDiv | BinOp::Mod | BinOp::Pow => arith(op, a, b),
        _ => unreachable!(),
    }
}

fn add(a: &Value, b: &Value) -> OpResult<Value> {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => Ok(Value::str(&format!("{x}{y}"))),
        (Value::List(x), Value::List(y)) => {
            let mut items = (**x).clone();
            items.extend(y.iter().cloned());
            Ok(Value::list(items))
        }
        (Value::Date(d), Value::Int(n)) | (Value::Int(n), Value::Date(d)) => date_plus(*d, *n),
        (Value::Str(_), _) | (_, Value::Str(_)) => {
            Err(
                OpError::new(format!("can't add {} and {}", article(&a.kind()), article(&b.kind())))
                    .help("to build text, use interpolation: \"total: {x}\""),
            )
        }
        _ => arith(BinOp::Add, a, b),
    }
}

fn sub(a: &Value, b: &Value) -> OpResult<Value> {
    match (a, b) {
        (Value::Date(x), Value::Date(y)) => Ok(Value::Int(*x as i64 - *y as i64)),
        (Value::Date(d), Value::Int(n)) => date_plus(*d, -n),
        _ => arith(BinOp::Sub, a, b),
    }
}

fn date_plus(d: i32, n: i64) -> OpResult<Value> {
    i32::try_from(d as i64 + n)
        .map(Value::Date)
        .map_err(|_| OpError::new("date out of range"))
}

fn arith(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    let bad = || {
        OpError::new(format!(
            "can't use `{}` with {} and {}",
            op.symbol(),
            article(&a.kind()),
            article(&b.kind())
        ))
    };
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        let (x, y) = (*x, *y);
        return match op {
            BinOp::Add => x.checked_add(y).map(Value::Int).ok_or_else(overflow),
            BinOp::Sub => x.checked_sub(y).map(Value::Int).ok_or_else(overflow),
            BinOp::Mul => x.checked_mul(y).map(Value::Int).ok_or_else(overflow),
            BinOp::Div => {
                if y == 0 {
                    Err(division_by_zero())
                } else {
                    Ok(Value::Float(x as f64 / y as f64))
                }
            }
            BinOp::IntDiv => {
                if y == 0 {
                    Err(division_by_zero())
                } else {
                    floor_div(x, y).map(Value::Int).ok_or_else(overflow)
                }
            }
            BinOp::Mod => {
                if y == 0 {
                    Err(division_by_zero())
                } else {
                    // The remainder takes the sign of the divisor: -1 mod 40 is 39.
                    Ok(Value::Int(
                        x.checked_rem(y)
                            .map_or(0, |r| if r != 0 && (r < 0) != (y < 0) { r + y } else { r }),
                    ))
                }
            }
            BinOp::Pow => {
                if y >= 0 {
                    u32::try_from(y)
                        .ok()
                        .and_then(|e| x.checked_pow(e))
                        .map(Value::Int)
                        .ok_or_else(overflow)
                } else {
                    Ok(Value::Float((x as f64).powi(y as i32)))
                }
            }
            _ => Err(bad()),
        };
    }
    let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) else {
        return Err(bad());
    };
    let v = match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => {
            if y == 0.0 {
                return Err(division_by_zero());
            }
            x / y
        }
        BinOp::IntDiv => {
            if y == 0.0 {
                return Err(division_by_zero());
            }
            return Ok(Value::Int((x / y).floor() as i64));
        }
        BinOp::Mod => {
            if y == 0.0 {
                return Err(division_by_zero());
            }
            x - (x / y).floor() * y
        }
        BinOp::Pow => x.powf(y),
        _ => return Err(bad()),
    };
    Ok(Value::Float(v))
}

/// Division rounding down (towards negative infinity).
fn floor_div(x: i64, y: i64) -> Option<i64> {
    let q = x.checked_div(y)?;
    Some(if x % y != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q })
}

fn division_by_zero() -> OpError {
    OpError::new("division by zero")
}

/// The language's `==`: numbers compare by value across int, float and prob.
pub fn equals(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Ordering for `<`, `>` and friends.
pub fn compare(a: &Value, b: &Value) -> OpResult<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        return x.partial_cmp(&y).ok_or_else(|| OpError::new("can't compare with NaN"));
    }
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
        (Value::Enum(x), Value::Enum(y)) if x.ty == y.ty => Ok(x.variant.cmp(&y.variant)),
        (Value::List(x), Value::List(y)) => {
            for (p, q) in x.iter().zip(y.iter()) {
                let c = compare(p, q)?;
                if c.is_ne() {
                    return Ok(c);
                }
            }
            Ok(x.len().cmp(&y.len()))
        }
        _ => Err(OpError::new(format!(
            "can't compare {} with {}",
            article(&a.kind()),
            article(&b.kind())
        ))),
    }
}

/// Whether `item` is in `coll` (for `in`).
pub fn contains(coll: &Value, item: &Value) -> OpResult<bool> {
    Ok(match coll {
        Value::List(items) => items.iter().any(|x| equals(x, item)),
        Value::Map(m) => m.contains_key(item) || m.keys().any(|k| equals(k, item)),
        Value::Bag(b) => b.iter().any(|(k, n)| *n > 0 && equals(k, item)),
        Value::Range(lo, hi) => match item.as_f64() {
            Some(x) => x.fract() == 0.0 && x >= *lo as f64 && x <= *hi as f64,
            None => false,
        },
        Value::Str(s) => match item {
            Value::Str(sub) => s.contains(&**sub),
            _ => {
                return Err(OpError::new(format!(
                    "can't look for {} in a string",
                    article(&item.kind())
                )));
            }
        },
        other => {
            return Err(OpError::new(format!("can't look inside {}", article(&other.kind()))));
        }
    })
}

fn range(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    match (a, b) {
        (Value::Int(lo), Value::Int(hi)) => {
            let hi = if op == BinOp::RangeExcl { hi - 1 } else { *hi };
            Ok(Value::Range(*lo, hi))
        }
        _ if a.is_dist() || b.is_dist() => Err(OpError::new("a range needs plain whole numbers, not distributions")
            .help("draw a value first, like `let n ~ d6`")),
        _ => Err(OpError::new(format!(
            "a range needs whole numbers, found {} and {}",
            article(&a.kind()),
            article(&b.kind())
        ))),
    }
}

// ── Collections ──────────────────────────────────────────────────────────

pub fn field(v: &Value, name: &str) -> OpResult<Value> {
    lift1(v, |x| match x {
        Value::Record(r) => r.get(name).cloned().ok_or_else(|| {
            let known: Vec<String> = r.fields.iter().map(|(n, _)| format!("`{n}`")).collect();
            OpError::new(format!("{} has no field `{name}`", article(&x.kind())))
                .help(format!("its fields are {}", known.join(", ")))
        }),
        other => Err(OpError::new(format!(
            "can't read the field `{name}` of {}",
            article(&other.kind())
        ))),
    })
}

pub fn index(coll: &Value, i: &Value) -> OpResult<Value> {
    lift2(coll, i, index_plain)
}

pub fn index_plain(coll: &Value, i: &Value) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let k = as_index(i, items.len())?;
            Ok(items[k].clone())
        }
        Value::Range(lo, hi) => {
            let len = (hi - lo + 1).max(0) as usize;
            let k = as_index(i, len)?;
            Ok(Value::Int(lo + k as i64))
        }
        Value::Str(s) => {
            let chars: Vec<char> = s.chars().collect();
            let k = as_index(i, chars.len())?;
            Ok(Value::str(&chars[k].to_string()))
        }
        Value::Map(m) => m
            .get(i)
            .or_else(|| m.iter().find(|(k, _)| equals(k, i)).map(|(_, v)| v))
            .cloned()
            .ok_or_else(|| {
                OpError::new(format!("the key {i:?} isn't in the map"))
                    .help("use `get(key, default)` for keys that may be missing")
            }),
        other => Err(OpError::new(format!("can't index {}", article(&other.kind())))),
    }
}

pub fn as_index(i: &Value, len: usize) -> OpResult<usize> {
    let k = match i {
        Value::Int(k) => *k,
        Value::Float(f) if f.fract() == 0.0 => *f as i64,
        other => {
            return Err(OpError::new(format!(
                "an index must be a whole number, not {}",
                article(&other.kind())
            )));
        }
    };
    if k < 0 || k as usize >= len {
        return Err(OpError::new(format!("index {k} is out of range for a length of {len}")).help("indices start at 0"));
    }
    Ok(k as usize)
}

pub fn make_record(ty: Option<Arc<str>>, mut fields: Vec<(Arc<str>, Value)>) -> Value {
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Record(Arc::new(Record { ty, fields }))
}

pub fn with_fields(base: &Value, updates: &[(Arc<str>, Value)]) -> OpResult<Value> {
    let Value::Record(r) = base else {
        return Err(OpError::new(format!(
            "`with` needs a record, found {}",
            article(&base.kind())
        )));
    };
    let mut r = (**r).clone();
    for (name, v) in updates {
        match r.get_mut(name) {
            Some(slot) => *slot = v.clone(),
            None => {
                return Err(OpError::new(format!("{} has no field `{name}`", article(&base.kind()))));
            }
        }
    }
    Ok(Value::Record(Arc::new(r)))
}

pub fn enum_value(ty: u32, variant: u32, name: &str) -> Value {
    Value::Enum(Arc::new(EnumValue {
        ty,
        variant,
        name: Arc::from(name),
    }))
}

/// Is `v` certainly false (for `and`) or certainly true (for `or`)?
pub fn is_certain(v: &Value, truth: bool) -> bool {
    matches!(to_chance(v), Ok(p) if p == if truth { 1.0 } else { 0.0 })
}
