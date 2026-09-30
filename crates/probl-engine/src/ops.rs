//! Operators, and applying operations to every outcome of a distribution.

use crate::dist::{Budget, Dist};
use crate::error::{OpError, OpResult};
use crate::value::{EnumValue, Record, Value, fmt_prob};
use probl_number::Integer;
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
pub fn lift1(a: &Value, budget: &mut Budget, f: impl Fn(&Value, &mut Budget) -> OpResult<Value>) -> OpResult<Value> {
    if !a.is_dist() {
        return f(a, budget);
    }
    let mut results = Vec::new();
    for (v, w) in outcomes(a).iter() {
        results.push((f(v, budget)?, *w));
    }
    combine(results, missing(a), budget)
}

/// Apply `f` to two values; distributions are independent draws.
pub fn lift2(
    a: &Value,
    b: &Value,
    budget: &mut Budget,
    f: impl Fn(&Value, &Value, &mut Budget) -> OpResult<Value>,
) -> OpResult<Value> {
    if !a.is_dist() && !b.is_dist() {
        return f(a, b, budget);
    }
    let (oa, ob) = (outcomes(a), outcomes(b));
    budget.outcomes(oa.len() as u128 * ob.len() as u128)?;
    budget.work((oa.len() * ob.len()) as u64)?;
    let mut results = Vec::with_capacity(oa.len() * ob.len());
    for (x, p) in oa.iter() {
        for (y, q) in ob.iter() {
            results.push((f(x, y, budget)?, p * q));
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
        Value::Int(i) if *i == 0 || *i == 1 => Err(OpError::new(format!(
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
        Truth::Uncertain(d) => lift1(&Value::Dist(d), budget, |x, _| match x {
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
            lift1(&Value::Dist(d), budget, |v, _| match v {
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
        UnOp::Neg => lift1(v, budget, |x, budget| match x {
            Value::Int(i) => {
                budget.integer_work(i, &Integer::ZERO, false)?;
                {
                    let n = i.negated();
                    budget.integer_allocation(n.bits(), 1)?;
                    Ok(Value::Int(n))
                }
            }
            Value::Float(f) | Value::Prob(f) => Ok(Value::Float(-f)),
            Value::Complex(z) => Ok(Value::Complex(z.negated())),
            other => Err(OpError::new(format!("can't negate {}", article(&other.kind())))),
        }),
        UnOp::Not => not(v, budget),
    }
}

pub fn binary(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match op {
        BinOp::And | BinOp::Or => unreachable!("`and` and `or` are evaluated lazily by the interpreter"),
        BinOp::To => unreachable!("`to` is lowered to a built-in"),
        BinOp::Range | BinOp::RangeExcl => range(op, a, b, budget),
        BinOp::In => lift2(a, b, budget, |x, coll, budget| {
            contains(coll, x, budget).map(Value::Bool)
        }),
        BinOp::NotIn => lift2(a, b, budget, |x, coll, budget| {
            contains(coll, x, budget).map(|c| Value::Bool(!c))
        }),
        _ => lift2(a, b, budget, |x, y, budget| binary_plain(op, x, y, budget)),
    }
}

fn binary_plain(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    if let Some(v) = continuous_binary(op, a, b)? {
        return Ok(v);
    }
    if matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
    ) {
        for v in [a, b] {
            if let Value::Int(n) = v {
                budget.integer_work(n, &Integer::ONE, false)?;
            }
            if let Value::Str(s) = v {
                budget.string_work(s)?;
            }
        }
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
        BinOp::Add => add(a, b, budget),
        BinOp::Sub => sub(a, b, budget),
        _ => arith(op, a, b, budget),
    }
}

fn add(a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => {
            let size = x
                .len()
                .checked_add(y.len())
                .ok_or_else(|| OpError::limit("string size overflow"))?;
            budget.string_size(size)?;
            let mut out = String::new();
            crate::text::push(&mut out, x, budget)?;
            crate::text::push(&mut out, y, budget)?;
            Ok(Value::str(&out))
        }
        (Value::List(x), Value::List(y)) => {
            budget.collection(x.len() as u128 + y.len() as u128)?;
            budget.work(x.len() as u64 + y.len() as u64)?;
            let mut items = x.to_vec();
            items.extend(y.iter().cloned());
            Ok(Value::list(items))
        }
        (Value::Date(d), Value::Int(n)) | (Value::Int(n), Value::Date(d)) => {
            date_plus(*d, n.to_i64().ok_or_else(|| OpError::new("date out of range"))?)
        }
        (Value::Str(_), _) | (_, Value::Str(_)) => {
            Err(
                OpError::new(format!("can't add {} and {}", article(&a.kind()), article(&b.kind())))
                    .help("to build text, use interpolation: \"total: {x}\""),
            )
        }
        _ => arith(BinOp::Add, a, b, budget),
    }
}

fn sub(a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match (a, b) {
        (Value::Date(x), Value::Date(y)) => Ok(Value::Int((*x as i64 - *y as i64).into())),
        (Value::Date(d), Value::Int(n)) => match n.negated().to_i64() {
            Some(m) => date_plus(*d, m),
            None => Err(OpError::new("date out of range")),
        },
        _ => arith(BinOp::Sub, a, b, budget),
    }
}

fn date_plus(d: i32, n: i64) -> OpResult<Value> {
    crate::dates::add_days(d, n)
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

fn arith(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    let bad = || {
        OpError::new(format!(
            "can't use `{}` with {} and {}",
            op.symbol(),
            article(&a.kind()),
            article(&b.kind())
        ))
    };
    if let (BinOp::Pow, Value::Complex(z), Value::Int(n)) = (op, a, b) {
        budget.integer_work(n, &Integer::ONE, false)?;
        budget.work(n.bits())?;
        return Ok(Value::Complex(z.pow_integer(n)?));
    }
    if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
        let (Some(x), Some(y)) = (a.as_complex(), b.as_complex()) else {
            return Err(bad());
        };
        let z = match op {
            BinOp::Add => x.plus(y),
            BinOp::Sub => x.minus(y),
            BinOp::Mul => x.times(y),
            BinOp::Div => x.divided_by(y),
            BinOp::Pow => match b {
                Value::Int(n) => {
                    budget.integer_work(n, &Integer::ONE, false)?;
                    budget.work(n.bits())?;
                    x.pow_integer(n)
                }
                _ => Err(OpError::new("a complex power needs an int exponent")),
            },
            _ => Err(bad()),
        }?;
        return Ok(Value::Complex(z));
    }
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        budget.integer_work(x, y, matches!(op, BinOp::Mul | BinOp::Div | BinOp::IntDiv | BinOp::Mod))?;
        let n = match op {
            BinOp::Add => x.add(y)?,
            BinOp::Sub => x.sub(y)?,
            BinOp::Mul => {
                if !x.is_zero() && !y.is_zero() {
                    budget.integer_bits((x.bits() + y.bits()).saturating_sub(1))?;
                }
                x.mul(y)?
            }
            BinOp::Div => {
                if y.is_zero() {
                    return Err(division_by_zero());
                }
                return x
                    .ratio(y)
                    .map(Value::Float)
                    .ok_or_else(|| OpError::new("division result is too large for a finite float"));
            }
            BinOp::IntDiv => x.div_mod(y)?.0,
            BinOp::Mod => x.div_mod(y)?.1,
            BinOp::Pow => return int_power(x, y, budget),
            _ => return Err(bad()),
        };
        budget.integer_allocation(n.bits(), 1)?;
        return Ok(Value::Int(n));
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
            let n = Integer::from_f64(q).ok_or_else(|| OpError::new("integer division result is not finite"))?;
            budget.integer_allocation(n.bits(), 1)?;
            return Ok(Value::Int(n));
        }
        BinOp::Mod => {
            if y == 0.0 {
                return Err(division_by_zero());
            }
            x - (x / y).floor() * y
        }
        BinOp::Pow => libm::pow(x, y),
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

fn division_by_zero() -> OpError {
    OpError::new("division by zero")
}

fn int_power(base: &Integer, exponent: &Integer, budget: &mut Budget) -> OpResult<Value> {
    if exponent.is_zero() {
        return Ok(Value::Int(Integer::ONE));
    }
    if base.is_zero() {
        return if exponent.is_negative() {
            Err(division_by_zero())
        } else {
            Ok(Value::Int(Integer::ZERO))
        };
    }
    if exponent.is_negative() {
        let reciprocal = Integer::ONE.ratio(&base.abs()).unwrap_or(0.0);
        let power = exponent.abs().to_f64().unwrap_or(f64::INFINITY);
        let result = libm::pow(reciprocal, power);
        return Ok(Value::Float(if base.is_negative() && exponent.is_odd() {
            -result
        } else {
            result
        }));
    }
    if *base == 1 || *base == -1 {
        return Ok(Value::Int(if *base == -1 && exponent.is_odd() {
            (-1).into()
        } else {
            Integer::ONE
        }));
    }
    let mut n = exponent
        .to_u64()
        .ok_or_else(|| OpError::limit("integer power exceeds the integer size limit"))?;
    budget.integer_bits(base.bits().saturating_sub(1).saturating_mul(n).saturating_add(1))?;
    let mut result = Integer::ONE;
    let mut b = base.clone();
    while n != 0 {
        if n & 1 != 0 {
            budget.integer_work(&result, &b, true)?;
            result = result.mul(&b)?;
            budget.integer_bits(result.bits())?;
        }
        n >>= 1;
        if n != 0 {
            budget.integer_work(&b, &b, true)?;
            b = b.mul(&b)?;
            budget.integer_bits(b.bits())?;
        }
    }
    budget.integer_allocation(result.bits(), 1)?;
    Ok(Value::Int(result))
}

fn numeric_compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Int(a), Value::Float(b) | Value::Prob(b)) => a.cmp_f64(*b),
        (Value::Float(a) | Value::Prob(a), Value::Int(b)) => b.cmp_f64(*a).map(std::cmp::Ordering::reverse),
        (Value::Float(a) | Value::Prob(a), Value::Float(b) | Value::Prob(b)) => a.partial_cmp(b),
        _ => None,
    }
}

/// The language's `==`: numbers compare by value across int, float and prob.
pub fn equals(a: &Value, b: &Value) -> bool {
    if let (Value::Complex(x), Value::Complex(y)) = (a, b) {
        return x == y;
    }
    if let (Value::Complex(z), real) | (real, Value::Complex(z)) = (a, b) {
        if z.im() != 0.0 {
            return false;
        }
        return match real {
            // Compare integers without rounding them through f64.
            Value::Int(n) => {
                let x = z.re();
                n.cmp_f64(x).is_some_and(|c| c.is_eq())
            }
            Value::Float(x) | Value::Prob(x) => z.re() == *x,
            _ => false,
        };
    }
    if let Some(c) = numeric_compare(a, b) {
        return c.is_eq();
    }
    if matches!(a, Value::Float(x) | Value::Prob(x) if x.is_nan())
        || matches!(b, Value::Float(x) | Value::Prob(x) if x.is_nan())
    {
        return false;
    }
    a == b
}

/// Ordering for `<`, `>` and friends.
pub fn compare(a: &Value, b: &Value) -> OpResult<std::cmp::Ordering> {
    if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
        return Err(OpError::new("complex values have no ordering").help("compare `abs(z)`, `real(z)` or `imag(z)`"));
    }
    if let Some(c) = numeric_compare(a, b) {
        return Ok(c);
    }
    if matches!(a, Value::Float(x) | Value::Prob(x) if x.is_nan())
        || matches!(b, Value::Float(x) | Value::Prob(x) if x.is_nan())
    {
        return Err(OpError::new("can't order NaN"));
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
pub fn contains(coll: &Value, item: &Value, budget: &mut Budget) -> OpResult<bool> {
    if let Value::Str(s) = coll {
        budget.string_work(s)?;
        if let Value::Str(s) = item {
            budget.string_work(s)?;
        }
    }
    Ok(match coll {
        Value::List(items) => items.iter().any(|x| equals(x, item)),
        Value::Map(m) => m.contains_key(item) || m.keys().any(|k| equals(k, item)),
        Value::Bag(b) => b.iter().any(|(k, n)| *n > 0 && equals(k, item)),
        Value::Range(lo, hi) => match item {
            Value::Int(n) => n >= lo && n <= hi,
            Value::Float(x) | Value::Prob(x) if x.is_finite() && x.fract() == 0.0 => {
                lo.cmp_f64(*x).is_some_and(|c| !c.is_gt()) && hi.cmp_f64(*x).is_some_and(|c| !c.is_lt())
            }
            _ => false,
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

fn range(op: BinOp, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match (a, b) {
        (Value::Int(lo), Value::Int(hi)) => {
            let hi = if op == BinOp::RangeExcl {
                hi.sub(&Integer::ONE)?
            } else {
                hi.clone()
            };
            budget.integer_work(lo, &hi, false)?;
            if op == BinOp::RangeExcl {
                budget.integer_allocation(hi.bits(), 1)?;
            }
            Ok(Value::Range(lo.clone(), hi))
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
pub fn range_len(lo: &Integer, hi: &Integer) -> OpResult<Integer> {
    if hi < lo {
        Ok(Integer::ZERO)
    } else {
        Ok(hi.sub(lo)?.add(&Integer::ONE)?)
    }
}

/// Bounded materialization count, never truncating a large length.
pub fn range_count(lo: &Integer, hi: &Integer) -> OpResult<u128> {
    range_len(lo, hi)?
        .to_u128()
        .ok_or_else(|| OpError::limit("range has too many elements to materialize"))
}

// ── Collections ──────────────────────────────────────────────────────────

pub fn field(v: &Value, name: &str, budget: &mut Budget) -> OpResult<Value> {
    lift1(v, budget, |x, _| match x {
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
    lift2(coll, i, budget, |c, i, budget| {
        let v = index_plain(c, i, budget)?;
        if let (Value::Range(lo, hi), Value::Int(n)) = (c, &v) {
            budget.integer_work(lo, hi, false)?;
            budget.integer_allocation(n.bits(), 1)?;
        }
        Ok(v)
    })
}

pub fn index_plain(coll: &Value, i: &Value, budget: &mut Budget) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let k = as_index(i, items.len() as u128)?;
            Ok(items[k as usize].clone())
        }
        Value::Range(lo, hi) => {
            let k = match i {
                Value::Int(n) if !n.is_negative() => n.clone(),
                Value::Float(f) if *f >= 0.0 && f.fract() == 0.0 => {
                    Integer::from_f64(*f).ok_or_else(|| OpError::new("index out of range"))?
                }
                _ => return Err(OpError::new("an index must be a nonnegative integer")),
            };
            let n = lo.add(&k)?;
            if &n > hi {
                return Err(OpError::new("index out of range"));
            }
            Ok(Value::Int(n))
        }
        Value::Str(s) => {
            budget.string_work(s)?;
            let k = as_index(i, s.chars().count() as u128)?;
            let c = s.chars().nth(k as usize).expect("checked scalar index");
            crate::text::value(c.encode_utf8(&mut [0; 4]), budget)
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
        Value::Int(k) => k.to_u128(),
        Value::Float(f) if f.is_finite() && f.fract() == 0.0 => Integer::from_f64(*f).and_then(|n| n.to_u128()),
        other => {
            return Err(OpError::new(format!(
                "an index must be a whole number, not {}",
                article(&other.kind())
            )));
        }
    };
    k.filter(|k| *k < len).ok_or_else(|| {
        OpError::new(format!("index {i} is out of range for a length of {len}")).help("indices start at 0")
    })
}

pub fn make_record(ty: Option<Arc<str>>, mut fields: Vec<(Arc<str>, Value)>) -> Value {
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    Value::record(Record { ty, fields })
}

pub fn with_fields(base: &Value, updates: &[(Arc<str>, Value)]) -> OpResult<Value> {
    let Value::Record(r) = base else {
        return Err(OpError::new(format!(
            "`with` needs a record, found {}",
            article(&base.kind())
        )));
    };
    let mut r = Record::clone(r);
    for (name, v) in updates {
        match r.get_mut(name) {
            Some(slot) => *slot = v.clone(),
            None => {
                return Err(OpError::new(format!("{} has no field `{name}`", article(&base.kind()))));
            }
        }
    }
    Ok(Value::record(r))
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
