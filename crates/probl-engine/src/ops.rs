//! Operators, and applying operations to every outcome of a distribution.

use crate::dist::{Budget, Dist};
use crate::error::{OpError, OpResult};
use crate::value::{EnumValue, Record, Value, fmt_prob};
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

/// Gather the results of applying an operation to each outcome of some
/// distributions into a distribution. Results that are themselves
/// distributions are mixed in. The result is always a distribution, even when
/// only one outcome is possible.
pub fn combine(results: Vec<(Value, f64)>, missing: f64, budget: &mut Budget) -> OpResult<Value> {
    if results.is_empty() {
        return Err(OpError::new("the result would be an empty distribution"));
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
    budget.outcomes(flat.len() as u128)?;
    budget.work(flat.len() as u64)?;
    Ok(Dist::from_pairs(flat, missing.min(1.0)).into_value())
}

/// Apply `f` to a value, or to every outcome of a distribution.
pub fn lift1(a: &Value, budget: &mut Budget, f: impl Fn(&Value) -> OpResult<Value>) -> OpResult<Value> {
    if !a.is_dist() {
        return f(a);
    }
    let mut results = Vec::new();
    for (v, w) in outcomes(a).iter() {
        results.push((f(v)?, *w));
    }
    combine(results, missing(a), budget)
}

/// Apply `f` to two values; distributions are independent draws.
pub fn lift2(
    a: &Value,
    b: &Value,
    budget: &mut Budget,
    f: impl Fn(&Value, &Value) -> OpResult<Value>,
) -> OpResult<Value> {
    if !a.is_dist() && !b.is_dist() {
        return f(a, b);
    }
    let (oa, ob) = (outcomes(a), outcomes(b));
    budget.outcomes(oa.len() as u128 * ob.len() as u128)?;
    budget.work((oa.len() * ob.len()) as u64)?;
    let mut results = Vec::with_capacity(oa.len() * ob.len());
    for (x, p) in oa.iter() {
        for (y, q) in ob.iter() {
            results.push((f(x, y)?, p * q));
        }
    }
    let m = 1.0 - (1.0 - missing(a)) * (1.0 - missing(b));
    combine(results, m, budget)
}

/// A function lifted by [`lift_n`]; it may need the budget itself (to build
/// distributions).
pub type NaryFn<'a> = &'a dyn Fn(&[Value], &mut Budget) -> OpResult<Value>;

/// Apply `f` to a list of values, lifting over any that are distributions.
pub fn lift_n(args: &[Value], budget: &mut Budget, f: NaryFn) -> OpResult<Value> {
    if !args.iter().any(Value::is_dist) {
        return f(args, budget);
    }
    let size = args
        .iter()
        .fold(1u128, |acc, a| acc.saturating_mul(outcomes(a).len() as u128));
    budget.outcomes(size)?;
    budget.work(size as u64)?;
    let kept: f64 = args.iter().map(|a| 1.0 - missing(a)).product();
    let mut results = Vec::new();
    let mut current = Vec::with_capacity(args.len());
    product(args, 0, &mut current, 1.0, f, budget, &mut results)?;
    combine(results, 1.0 - kept, budget)
}

fn product(
    args: &[Value],
    i: usize,
    current: &mut Vec<Value>,
    weight: f64,
    f: NaryFn,
    budget: &mut Budget,
    results: &mut Vec<(Value, f64)>,
) -> OpResult<()> {
    if i == args.len() {
        results.push((f(current, budget)?, weight));
        return Ok(());
    }
    for (v, w) in outcomes(&args[i]).iter() {
        current.push(v.clone());
        product(args, i + 1, current, weight * w, f, budget, results)?;
        current.pop();
    }
    Ok(())
}

// ── Probabilities, conditions and facts ──────────────────────────────────

pub fn article(kind: &str) -> String {
    let vowel = kind.starts_with(['a', 'e', 'i', 'o', 'u']);
    format!("{} {kind}", if vowel { "an" } else { "a" })
}

/// A value used as a probability (chance weights, `bernoulli`, `binomial`…):
/// a `prob`, or a float from 0 to 1.
pub fn to_prob(v: &Value) -> OpResult<f64> {
    match v {
        Value::Prob(p) => Ok(*p),
        Value::Float(f) if (0.0..=1.0).contains(f) => Ok(*f),
        Value::Float(f) => Err(OpError::new(format!(
            "a probability must be between 0% and 100%, not {}",
            crate::value::fmt_float(*f)
        ))),
        Value::Bool(_) => Err(OpError::new("expected a probability, found a fact (true or false)")
            .help("`P(fact)` gives the probability that a fact is true")),
        Value::Dist(_) => Err(OpError::new(format!("expected a probability, found a {}", v.kind()))
            .help("`P(…)` gives the probability that a distribution of facts is true")),
        other => Err(OpError::new(format!(
            "expected a probability, found {}",
            article(&other.kind())
        ))),
    }
}

/// How likely a condition is to hold (docs/semantics.md, section 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Condition {
    /// The probability that it's true.
    pub yes: f64,
    /// The probability that it's false.
    pub no: f64,
    /// The probability missing from a distribution of facts: it could be
    /// either. The three add up to one, up to rounding.
    pub missing: f64,
}

/// A condition of `if`, `while` or `observe`, or a weight of `chance`: a
/// fact, a probability, or a distribution of facts.
pub fn condition(v: &Value) -> OpResult<Condition> {
    let known = |yes: f64, no: f64| Condition { yes, no, missing: 0.0 };
    match v {
        Value::Bool(b) => Ok(if *b { known(1.0, 0.0) } else { known(0.0, 1.0) }),
        Value::Prob(_) | Value::Float(_) => {
            let p = to_prob(v)?;
            Ok(known(p, 1.0 - p))
        }
        Value::Dist(d) => match d.truth() {
            Some((yes, no)) => Ok(Condition {
                yes,
                no,
                missing: d.missing,
            }),
            None => Err(OpError::new(format!(
                "a condition needs a probability or a fact, found a {}",
                v.kind()
            ))
            .help("compare it to get a fact, like `d6 > 4`, or draw a value first with `~`")),
        },
        Value::Continuous(f) => Err(OpError::new(format!(
            "a condition needs a probability or a fact, found a {} distribution",
            f.name()
        ))
        .help("compare it with a number to get a fact, like `x > 5`")),
        Value::Int(i @ (0 | 1)) => Err(OpError::new(format!(
            "a condition needs a probability or a fact, found the int {i}"
        ))
        .help(if *i == 1 {
            "write `true` or `100%`"
        } else {
            "write `false` or `0%`"
        })),
        other => Err(OpError::new(format!(
            "a condition needs a probability or a fact, found {}",
            article(&other.kind())
        ))),
    }
}

/// A fact, or a distribution of facts.
pub enum Truth {
    Fact(bool),
    Uncertain(Arc<Dist>),
}

/// The operand of `and`, `or` or `not` (docs/semantics.md, section 2).
pub fn truth(v: &Value, op: &str) -> OpResult<Truth> {
    match v {
        Value::Bool(b) => Ok(Truth::Fact(*b)),
        Value::Dist(d) if d.truth().is_some() => Ok(Truth::Uncertain(d.clone())),
        Value::Prob(p) => Err(OpError::new(format!(
            "`{op}` needs facts (true or false), but {} is a probability",
            fmt_prob(*p)
        ))
        .help("a probability isn't an event: draw one with `let e ~ bernoulli(p)`, then combine the facts")),
        other => Err(OpError::new(format!(
            "`{op}` needs facts (true or false), found {}",
            article(&other.kind())
        ))),
    }
}

pub fn not(v: &Value, budget: &mut Budget) -> OpResult<Value> {
    match truth(v, "not")? {
        Truth::Fact(b) => Ok(Value::Bool(!b)),
        Truth::Uncertain(d) => lift1(&Value::Dist(d), budget, |x| match x {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            _ => unreachable!("checked by `truth`"),
        }),
    }
}

/// `and` or `or` of two operands, at most one of them uncertain.
pub fn logic(and: bool, a: Truth, b: Truth, budget: &mut Budget) -> OpResult<Value> {
    let op = |x: bool, y: bool| if and { x && y } else { x || y };
    match (a, b) {
        (Truth::Fact(x), Truth::Fact(y)) => Ok(Value::Bool(op(x, y))),
        (Truth::Fact(x), Truth::Uncertain(d)) | (Truth::Uncertain(d), Truth::Fact(x)) => {
            lift1(&Value::Dist(d), budget, |v| match v {
                Value::Bool(y) => Ok(Value::Bool(op(x, *y))),
                _ => unreachable!("checked by `truth`"),
            })
        }
        (Truth::Uncertain(_), Truth::Uncertain(_)) => {
            let word = if and { "and" } else { "or" };
            Err(OpError::new(format!(
                "`{word}` can't combine two uncertain facts: they might be the same event"
            ))
            .help(
                "give each event an identity by drawing it first, like `let a ~ d6 > 4`, then combine the drawn facts",
            ))
        }
    }
}

// ── Operators ────────────────────────────────────────────────────────────

pub fn unary(op: UnOp, v: &Value, budget: &mut Budget) -> OpResult<Value> {
    match op {
        UnOp::Neg => lift1(v, budget, |x| match x {
            Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(overflow),
            Value::Float(f) | Value::Prob(f) => Ok(Value::Float(-f)),
            other => Err(OpError::new(format!("can't negate {}", article(&other.kind())))),
        }),
        UnOp::Not => not(v, budget),
    }
}

fn overflow() -> OpError {
    OpError::new("integer overflow").help("the result doesn't fit in 64 bits")
}

pub fn binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match op {
        BinOp::And | BinOp::Or => unreachable!("`and` and `or` are evaluated lazily by the interpreter"),
        BinOp::To => unreachable!("`to` is lowered to a built-in"),
        BinOp::Range | BinOp::RangeExcl => range(op, a, b),
        BinOp::In => lift2(a, b, budget, |x, coll| contains(coll, x).map(Value::Bool)),
        BinOp::NotIn => lift2(a, b, budget, |x, coll| contains(coll, x).map(|c| Value::Bool(!c))),
        _ => lift2(a, b, budget, |x, y| binary_plain(op, x, y)),
    }
}

fn binary_plain(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    if let Some(v) = continuous_binary(op, a, b)? {
        return Ok(v);
    }
    match op {
        BinOp::Eq => Ok(Value::Bool(equals(a, b))),
        BinOp::Ne => Ok(Value::Bool(!equals(a, b))),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare(a, b)?;
            Ok(Value::Bool(match op {
                BinOp::Lt => ord.is_lt(),
                BinOp::Le => ord.is_le(),
                BinOp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            }))
        }
        BinOp::Add => add(a, b),
        BinOp::Sub => sub(a, b),
        _ => arith(op, a, b),
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
        (Value::Date(d), Value::Int(n)) => match n.checked_neg() {
            Some(m) => date_plus(*d, m),
            None => Err(OpError::new("date out of range")),
        },
        _ => arith(BinOp::Sub, a, b),
    }
}

fn date_plus(d: i32, n: i64) -> OpResult<Value> {
    (d as i64)
        .checked_add(n)
        .and_then(|x| i32::try_from(x).ok())
        .map(Value::Date)
        .ok_or_else(|| OpError::new("date out of range"))
}

/// Numbers for arithmetic: ints, floats and probabilities (not facts).
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(_) => None,
        _ => v.as_f64(),
    }
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
                    Ok(Value::Float((x as f64).powf(y as f64)))
                }
            }
            _ => Err(bad()),
        };
    }
    let (Some(x), Some(y)) = (number(a), number(b)) else {
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
            let q = (x / y).floor();
            if !q.is_finite() || q.abs() > 9.2e18 {
                return Err(overflow());
            }
            return Ok(Value::Int(q as i64));
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
    if !v.is_finite() {
        return Err(OpError::new(format!(
            "`{}` gave a result that isn't a finite number",
            op.symbol()
        )));
    }
    Ok(Value::Float(v))
}

/// A continuous distribution compared with a number: a distribution of
/// facts, from its CDF. Nothing else is defined on one yet
/// (docs/semantics.md, section 13).
fn continuous_binary(op: BinOp, a: &Value, b: &Value) -> OpResult<Option<Value>> {
    let (family, other, flipped) = match (a, b) {
        (Value::Continuous(f), other) => (f, other, false),
        (other, Value::Continuous(f)) => (f, other, true),
        _ => return Ok(None),
    };
    let needs_value = || {
        OpError::new(format!(
            "`{}` needs a value, not a {} distribution",
            op.symbol(),
            family.name()
        ))
        .help("draw a value first, like `let x ~ normal(0, 1)`; comparing a distribution with a number works too")
    };
    let x = match number(other) {
        Some(x) if !x.is_nan() => x,
        _ => return Err(needs_value()),
    };
    // For a continuous X, P(X < x) = P(X ≤ x) = F(x).
    let below = family.cdf(x);
    let yes = match (op, flipped) {
        (BinOp::Lt | BinOp::Le, false) | (BinOp::Gt | BinOp::Ge, true) => below,
        (BinOp::Gt | BinOp::Ge, false) | (BinOp::Lt | BinOp::Le, true) => 1.0 - below,
        (BinOp::Eq, _) => 0.0,
        (BinOp::Ne, _) => 1.0,
        _ => return Err(needs_value()),
    };
    Ok(Some(Dist::bernoulli(yes).into_value()))
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
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Ordering for `<`, `>` and friends.
pub fn compare(a: &Value, b: &Value) -> OpResult<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (number(a), number(b)) {
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
        Value::Range(lo, hi) => match number(item) {
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
        other => return Err(OpError::new(format!("can't look inside {}", article(&other.kind())))),
    })
}

fn range(op: BinOp, a: &Value, b: &Value) -> OpResult<Value> {
    match (a, b) {
        (Value::Int(lo), Value::Int(hi)) => {
            let hi = if op == BinOp::RangeExcl {
                hi.checked_sub(1).ok_or_else(|| OpError::new("range out of bounds"))?
            } else {
                *hi
            };
            Ok(Value::Range(*lo, hi))
        }
        _ if a.is_uncertain() || b.is_uncertain() => {
            Err(OpError::new("a range needs plain whole numbers, not distributions")
                .help("draw a value first, like `let n ~ d6`"))
        }
        _ => Err(OpError::new(format!(
            "a range needs whole numbers, found {} and {}",
            article(&a.kind()),
            article(&b.kind())
        ))),
    }
}

/// The number of integers in `lo..=hi`, without overflowing.
pub fn range_len(lo: i64, hi: i64) -> u128 {
    if hi < lo {
        0
    } else {
        (hi as i128 - lo as i128 + 1) as u128
    }
}

// ── Collections ──────────────────────────────────────────────────────────

pub fn field(v: &Value, name: &str, budget: &mut Budget) -> OpResult<Value> {
    lift1(v, budget, |x| match x {
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

pub fn index(coll: &Value, i: &Value, budget: &mut Budget) -> OpResult<Value> {
    lift2(coll, i, budget, index_plain)
}

pub fn index_plain(coll: &Value, i: &Value) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let k = as_index(i, items.len() as u128)?;
            Ok(items[k as usize].clone())
        }
        Value::Range(lo, hi) => {
            let k = as_index(i, range_len(*lo, *hi))?;
            Ok(Value::Int((*lo as i128 + k as i128) as i64))
        }
        Value::Str(s) => {
            let chars: Vec<char> = s.chars().collect();
            let k = as_index(i, chars.len() as u128)?;
            Ok(Value::str(&chars[k as usize].to_string()))
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

pub fn as_index(i: &Value, len: u128) -> OpResult<u128> {
    let k = match i {
        Value::Int(k) => *k,
        Value::Float(f) if f.fract() == 0.0 && f.abs() < 9.2e18 => *f as i64,
        other => {
            return Err(OpError::new(format!(
                "an index must be a whole number, not {}",
                article(&other.kind())
            )));
        }
    };
    if k < 0 || k as u128 >= len {
        return Err(OpError::new(format!("index {k} is out of range for a length of {len}")).help("indices start at 0"));
    }
    Ok(k as u128)
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

/// Is `v` certainly `truth`? (For `and`/`or` whose right side has statements.)
pub fn is_certain(v: &Value, truth: bool) -> bool {
    matches!(v, Value::Bool(b) if *b == truth)
}
