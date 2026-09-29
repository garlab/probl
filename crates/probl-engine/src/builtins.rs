//! Built-in functions that don't need the interpreter.

use crate::continuous::{Family, Mixture, Part};
use crate::dates;
use crate::dist::{Budget, Counts, Dist};
use crate::error::{OpError, OpResult};
use crate::ops::{self, article, as_index, equals, range_len, to_prob};
use crate::value::{Value, fmt_float};
use probl_sema::Builtin;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Call a built-in on plain (non-distribution) arguments.
pub fn call_plain(b: Builtin, args: &[Value], budget: &mut Budget) -> OpResult<Value> {
    use Builtin as B;
    let a = |i: usize| &args[i];
    match b {
        B::Min | B::Max => min_max(args, b == B::Max, budget),
        B::Abs => num1(a(0), "abs", |x| x.abs(), |i| i.checked_abs()),
        B::Floor => to_int(a(0), f64::floor),
        B::Ceil => to_int(a(0), f64::ceil),
        B::Round => round(a(0), args.get(1)),
        B::Sqrt => float1(a(0), "sqrt", |x| (x >= 0.0).then(|| x.sqrt())),
        B::Exp => float1(a(0), "exp", |x| Some(libm::exp(x)).filter(|y| y.is_finite())),
        B::Ln => float1(a(0), "ln", |x| (x > 0.0).then(|| libm::log(x))),
        B::Log10 => float1(a(0), "log10", |x| (x > 0.0).then(|| libm::log10(x))),
        B::Log2 => float1(a(0), "log2", |x| (x > 0.0).then(|| libm::log2(x))),
        B::Log1p => float1(a(0), "log1p", |x| (x > -1.0).then(|| libm::log1p(x))),
        B::Expm1 => float1(a(0), "expm1", |x| Some(libm::expm1(x)).filter(|y| y.is_finite())),
        B::Sin => float1(a(0), "sin", |x| Some(libm::sin(x))),
        B::Cos => float1(a(0), "cos", |x| Some(libm::cos(x))),
        B::Tan => float1(a(0), "tan", |x| Some(libm::tan(x))),
        B::Asin => float1(a(0), "asin", |x| (-1.0..=1.0).contains(&x).then(|| libm::asin(x))),
        B::Acos => float1(a(0), "acos", |x| (-1.0..=1.0).contains(&x).then(|| libm::acos(x))),
        B::Atan => float1(a(0), "atan", |x| Some(libm::atan(x))),
        B::Atan2 => float2(a(0), a(1), "atan2", libm::atan2),
        B::Hypot => float2(a(0), a(1), "hypot", libm::hypot),
        B::Sinh => float1(a(0), "sinh", |x| Some(libm::sinh(x)).filter(|y| y.is_finite())),
        B::Cosh => float1(a(0), "cosh", |x| Some(libm::cosh(x)).filter(|y| y.is_finite())),
        B::Tanh => float1(a(0), "tanh", |x| Some(libm::tanh(x))),
        B::Asinh => float1(a(0), "asinh", |x| Some(libm::asinh(x))),
        B::Acosh => float1(a(0), "acosh", |x| (x >= 1.0).then(|| libm::acosh(x))),
        B::Atanh => float1(a(0), "atanh", |x| (x.abs() < 1.0).then(|| libm::atanh(x))),
        B::Choose => choose(
            nonnegative_int(a(0), "choose")?,
            nonnegative_int(a(1), "choose")?,
            budget,
        ),
        B::Factorial => factorial(nonnegative_int(a(0), "factorial")?, budget),
        B::Gcd | B::Lcm => {
            let (x, y) = (
                integer(a(0), b.name())?.unsigned_abs(),
                integer(a(1), b.name())?.unsigned_abs(),
            );
            let result = if b == B::Gcd {
                u128::from(gcd(x, y))
            } else if x == 0 || y == 0 {
                0
            } else {
                u128::from(x / gcd(x, y)) * u128::from(y)
            };
            checked_int(result, b.name()).map(Value::Int)
        }
        B::EulerPhi => euler_phi(nonnegative_int(a(0), "euler_phi")?, budget),
        B::LnGamma => float1(a(0), "ln_gamma", |x| (x > 0.0).then(|| libm::lgamma(x))),
        B::Erf => float1(a(0), "erf", |x| Some(libm::erf(x))),
        B::Clamp => {
            let (lo, hi) = (a(1), a(2));
            if ops::compare(lo, hi)?.is_gt() {
                return Err(OpError::new("clamp's lower bound is above its upper bound"));
            }
            if ops::compare(a(0), lo)?.is_lt() {
                Ok(lo.clone())
            } else if ops::compare(a(0), hi)?.is_gt() {
                Ok(hi.clone())
            } else {
                Ok(a(0).clone())
            }
        }
        B::Str => Ok(Value::str(&a(0).to_string())),
        B::Upper => text(a(0), "upper").map(|s| Value::str(&s.to_uppercase())),
        B::Lower => text(a(0), "lower").map(|s| Value::str(&s.to_lowercase())),
        B::Split => {
            let (s, sep) = (text(a(0), "split")?, text(a(1), "split")?);
            let parts = if sep.is_empty() {
                s.chars().map(|c| Value::str(&c.to_string())).collect()
            } else {
                s.split(&*sep).map(Value::str).collect()
            };
            Ok(Value::list(parts))
        }
        B::Join => {
            let items = list(a(0), "join")?;
            let sep = text(a(1), "join")?;
            let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
            Ok(Value::str(&parts.join(&sep)))
        }
        B::Len => len(a(0)),
        B::Sum => sum(a(0), budget),
        B::Count if args.len() == 1 => len(a(0)),
        B::Sort | B::SortDesc => {
            let mut items = items(a(0), b.name(), budget)?;
            sort(&mut items)?;
            if b == B::SortDesc {
                items.reverse();
            }
            Ok(Value::list(items))
        }
        B::Reverse => match a(0) {
            Value::Str(s) => Ok(Value::str(&s.chars().rev().collect::<String>())),
            v => {
                let mut items = items(v, "reverse", budget)?;
                items.reverse();
                Ok(Value::list(items))
            }
        },
        B::Keys => match a(0) {
            Value::Map(m) => Ok(Value::list(m.keys().cloned().collect())),
            Value::Bag(m) => Ok(Value::list(m.keys().cloned().collect())),
            v => Err(expected("a map", v, "keys")),
        },
        B::Values => match a(0) {
            Value::Map(m) => Ok(Value::list(m.values().cloned().collect())),
            v => Err(expected("a map", v, "values")),
        },
        B::Get => get(a(0), a(1), args.get(2)),
        B::Contains => ops::contains(a(0), a(1)).map(Value::Bool),
        B::Highest | B::Lowest => extremes(a(0), args.get(1), b == B::Highest, budget),
        B::Enumerate => {
            let items = items(a(0), "enumerate", budget)?;
            Ok(Value::list(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, x)| Value::list(vec![Value::Int(i as i64), x]))
                    .collect(),
            ))
        }
        B::Zip => {
            let (xs, ys) = (items(a(0), "zip", budget)?, items(a(1), "zip", budget)?);
            Ok(Value::list(
                xs.into_iter().zip(ys).map(|(x, y)| Value::list(vec![x, y])).collect(),
            ))
        }
        B::Push => {
            let items = list(a(0), "push")?;
            budget.collection(items.len() as u128 + 1)?;
            let mut items = items.to_vec();
            items.push(a(1).clone());
            Ok(Value::list(items))
        }
        B::Insert => insert(a(0), a(1), a(2), budget),
        B::Remove => remove(a(0), a(1)),
        B::Pop => Err(
            OpError::new("`pop` changes a list, so call it as a method on a variable")
                .help("write `let top = xs.pop()`"),
        ),
        B::Last => {
            let items = list(a(0), "pop")?;
            items
                .last()
                .cloned()
                .ok_or_else(|| OpError::new("can't pop from an empty list"))
        }
        B::DropLast => {
            let items = list(a(0), "pop")?;
            let n = items.len().saturating_sub(1);
            Ok(Value::list(items[..n].to_vec()))
        }
        B::Bernoulli => Ok(Dist::bernoulli(to_prob(a(0))?).into_value()),
        B::OneOf => one_of(a(0), budget),
        B::Binomial | B::Poisson | B::Geometric => {
            let counts = counts(b, args)?.expect("called with plain arguments");
            Ok(counts.list(budget)?.into_value())
        }
        B::Bag => bag(a(0)),
        B::Normal => continuous(Family::normal(number(a(0), "normal")?, number(a(1), "normal")?)),
        B::Lognormal => continuous(Family::lognormal(
            number(a(0), "lognormal")?,
            number(a(1), "lognormal")?,
        )),
        B::Uniform => continuous(Family::uniform(number(a(0), "uniform")?, number(a(1), "uniform")?)),
        B::Beta => continuous(Family::beta(number(a(0), "beta")?, number(a(1), "beta")?)),
        B::Gamma => continuous(Family::gamma(number(a(0), "gamma")?, number(a(1), "gamma")?)),
        B::Exponential => continuous(Family::exponential(number(a(0), "exponential")?)),
        B::Triangular => continuous(Family::triangular(
            number(a(0), "triangular")?,
            number(a(1), "triangular")?,
            number(a(2), "triangular")?,
        )),
        B::Pert => continuous(Family::pert(
            number(a(0), "pert")?,
            number(a(1), "pert")?,
            number(a(2), "pert")?,
        )),
        B::NormalRange => continuous(Family::normal_range(
            number(a(0), "normal_range")?,
            number(a(1), "normal_range")?,
        )),
        B::To => continuous(Family::estimate(number(a(0), "to")?, number(a(1), "to")?)),
        B::Mixture | B::Truncate | B::Bins | B::Today => {
            Err(OpError::unsupported(format!("`{}` isn't implemented yet", b.name())))
        }
        B::Odds => {
            let p = to_prob(a(0))?;
            if p >= 1.0 {
                return Err(OpError::new("the odds of a certain event are infinite"));
            }
            Ok(Value::Float(p / (1.0 - p)))
        }
        B::Logit => {
            let p = to_prob(a(0))?;
            if p <= 0.0 || p >= 1.0 {
                return Err(OpError::new("logit needs a probability strictly between 0% and 100%"));
            }
            Ok(Value::Float(libm::log(p / (1.0 - p))))
        }
        B::InvLogit => Ok(Value::Prob(1.0 / (1.0 + libm::exp(-number(a(0), "inv_logit")?)))),
        B::Date => {
            let s = text(a(0), "date")?;
            dates::parse(&s)
                .map(Value::Date)
                .ok_or_else(|| OpError::new(format!("`{s}` isn't a valid date")).help("write dates as \"YYYY-MM-DD\""))
        }
        B::Days => to_int(a(0), f64::round),
        B::Weeks => {
            let n = number(a(0), "weeks")?;
            to_int(&Value::Float(n * 7.0), f64::round)
        }
        B::AddWorkdays => match a(0) {
            Value::Date(d) => {
                let n = whole(a(1), "add_workdays")?;
                budget.work(n.unsigned_abs())?;
                Ok(Value::Date(dates::add_workdays(*d, n)))
            }
            v => Err(expected("a date", v, "add_workdays")),
        },
        B::Weekday => match a(0) {
            Value::Date(d) => Ok(Value::str(dates::WEEKDAYS[dates::weekday(*d) as usize])),
            v => Err(expected("a date", v, "weekday")),
        },
        B::IsFalse => Ok(Value::Bool(ops::is_certain(a(0), false))),
        B::IsTrue => Ok(Value::Bool(ops::is_certain(a(0), true))),
        B::IsListOfLen => Ok(Value::Bool(
            matches!((a(0), a(1)), (Value::List(items), Value::Int(n)) if items.len() as i64 == *n),
        )),
        B::Count | B::Map | B::Filter | B::Reduce | B::Print | B::Roll | B::Take => {
            unreachable!("`{}` is handled by the interpreter", b.name())
        }
        B::P
        | B::Mean
        | B::Sd
        | B::Variance
        | B::Median
        | B::Quantile
        | B::Support
        | B::Cdf
        | B::Pmf
        | B::IterItems
        | B::RepeatCount
        | B::Pdf
        | B::Settled => unreachable!("`{}` takes distributions as they are", b.name()),
    }
}

/// Built-ins that receive distributions whole.
pub fn call_raw(b: Builtin, args: &[Value], budget: &mut Budget) -> OpResult<Value> {
    use Builtin as B;
    let v = &args[0];
    if matches!(
        b,
        B::Mean | B::Variance | B::Sd | B::Median | B::Quantile | B::Cdf | B::Pdf | B::Pmf | B::Support
    ) && continuous_parts(v)
    {
        return continuous_query(b, args);
    }
    match b {
        B::P => probability_of(v),
        B::Pdf => Err(OpError::new("pdf needs a continuous distribution")
            .help("for a distribution whose outcomes can be listed, use `pmf`")),
        B::Mean => numeric_dist(v, "mean").map(|d| Value::Float(d.mean().unwrap())),
        B::Variance => numeric_dist(v, "variance").map(|d| Value::Float(d.variance().unwrap())),
        B::Sd => numeric_dist(v, "sd").map(|d| Value::Float(d.variance().unwrap().sqrt())),
        B::Median => Ok(as_dist(v).quantile(0.5).unwrap()),
        B::Quantile => {
            let q = to_prob(&args[1])?;
            Ok(as_dist(v).quantile(q).unwrap())
        }
        B::Support => {
            let d = as_dist(v);
            budget.collection(d.outcomes.len() as u128)?;
            Ok(Value::list(d.outcomes.iter().map(|(x, _)| x.clone()).collect()))
        }
        B::Cdf => {
            let d = as_dist(v);
            let mut p = 0.0;
            for (x, w) in &d.outcomes {
                if ops::compare(x, &args[1])?.is_le() {
                    p += w;
                }
            }
            Ok(Value::Prob(p))
        }
        B::Pmf => {
            let d = as_dist(v);
            let p: f64 = d
                .outcomes
                .iter()
                .filter(|(x, _)| equals(x, &args[1]))
                .map(|(_, w)| w)
                .sum();
            Ok(Value::Prob(p))
        }
        B::IterItems => iter_items(v, budget),
        B::RepeatCount => match v {
            Value::Int(n) if *n >= 0 => Ok(Value::Int(*n)),
            Value::Int(_) => Err(OpError::new("`repeat` needs a count of 0 or more")),
            Value::Float(f) if f.fract() == 0.0 && *f >= 0.0 && *f < 9.2e18 => Ok(Value::Int(*f as i64)),
            v if v.is_uncertain() => Err(OpError::new(format!("`repeat` needs a number, not a {}", v.kind()))
                .help("draw a value first, like `let n ~ d6`, then `repeat n { … }`")),
            other => Err(OpError::new(format!(
                "`repeat` needs a whole number, found {}",
                article(&other.kind())
            ))),
        },
        B::Settled => match v {
            v if v.is_uncertain() => Err(
                OpError::new(format!("`match` needs a settled value, not a {}", v.kind()))
                    .help("draw a value first, like `let x ~ d6`, and match on `x`"),
            ),
            other => Ok(other.clone()),
        },
        // Internal helpers that take their arguments as they are.
        _ => call_plain(b, args, budget),
    }
}

/// `P(x)`: the probability of a fact, of a distribution of facts, or a
/// probability itself.
fn probability_of(v: &Value) -> OpResult<Value> {
    match v {
        Value::Bool(b) => Ok(Value::Prob(if *b { 1.0 } else { 0.0 })),
        Value::Prob(_) | Value::Float(_) => to_prob(v).map(Value::Prob),
        Value::Dist(d) => match d.truth() {
            Some((yes, _)) => Ok(Value::Prob(yes)),
            None => Err(OpError::new(format!("P needs a condition, found a {}", v.kind()))
                .help("compare it to get a condition, like `P(d6 > 4)`")),
        },
        Value::Continuous(_) => Err(OpError::new(format!("P needs a condition, found a {}", v.kind()))
            .help("compare it with a number, like `P(x > 5)`")),
        other => Err(
            OpError::new(format!("P needs a condition, found {}", article(&other.kind())))
                .help("for example `P(d6 > 4)`"),
        ),
    }
}

/// The parameters of `binomial`, `poisson` or `geometric`, checked; `None`
/// for another built-in, or when an argument is a distribution.
pub fn counts(b: Builtin, args: &[Value]) -> OpResult<Option<Counts>> {
    use Builtin as B;
    if args.iter().any(Value::is_uncertain) {
        return Ok(None);
    }
    Ok(Some(match b {
        B::Binomial => {
            let n = whole(&args[0], "binomial's number of trials")?;
            if n < 0 {
                return Err(OpError::new("binomial needs a number of trials of 0 or more"));
            }
            Counts::Binomial {
                n: n as u64,
                p: to_prob(&args[1])?,
            }
        }
        B::Poisson => {
            let rate = number(&args[0], "poisson")?;
            if rate < 0.0 || !rate.is_finite() {
                return Err(OpError::new("poisson needs a rate of 0 or more"));
            }
            if rate > 1e15 {
                return Err(OpError::new("poisson's rate is too large to count exactly")
                    .help("above 10¹⁵, use a normal distribution with the same mean and variance"));
            }
            Counts::Poisson { rate }
        }
        B::Geometric => {
            let p = to_prob(&args[0])?;
            if p <= 0.0 {
                return Err(OpError::new("geometric needs a chance of success above 0%"));
            }
            Counts::Geometric { p }
        }
        _ => return Ok(None),
    }))
}

fn continuous(family: OpResult<Family>) -> OpResult<Value> {
    family.map(|f| Value::Continuous(Arc::new(f)))
}

/// Whether a value is, or mixes in, a continuous distribution.
fn continuous_parts(v: &Value) -> bool {
    match v {
        Value::Continuous(_) => true,
        Value::Dist(d) => d.outcomes.iter().any(|(x, _)| matches!(x, Value::Continuous(_))),
        _ => false,
    }
}

/// Questions about a continuous distribution, or a mixture that includes
/// one, answered from the formulas (docs/semantics.md, section 13).
fn continuous_query(b: Builtin, args: &[Value]) -> OpResult<Value> {
    use Builtin as B;
    let parts = match &args[0] {
        Value::Continuous(f) => vec![(Part::Continuous(**f), 1.0)],
        Value::Dist(d) => d
            .outcomes
            .iter()
            .map(|(x, p)| {
                let part = match x {
                    Value::Continuous(f) => Part::Continuous(**f),
                    other => Part::Point(number(other, b.name())?),
                };
                Ok((part, *p))
            })
            .collect::<OpResult<Vec<_>>>()?,
        _ => unreachable!("checked by `continuous_parts`"),
    };
    let m = Mixture { parts };
    match b {
        B::Mean => Ok(Value::Float(m.mean())),
        B::Variance => Ok(Value::Float(m.variance())),
        B::Sd => Ok(Value::Float(m.variance().sqrt())),
        B::Median => Ok(Value::Float(m.quantile(0.5))),
        B::Quantile => Ok(Value::Float(m.quantile(to_prob(&args[1])?))),
        B::Cdf => Ok(Value::Prob(m.cdf(number(&args[1], "cdf")?))),
        B::Pmf => {
            let x = number(&args[1], "pmf")?;
            let total: f64 = m.parts.iter().map(|(_, p)| p).sum();
            let at: f64 = m
                .parts
                .iter()
                .filter(|(part, _)| matches!(part, Part::Point(v) if *v == x))
                .map(|(_, p)| p)
                .sum();
            Ok(Value::Prob(at / total))
        }
        B::Pdf => {
            let x = number(&args[1], "pdf")?;
            let mut density = 0.0;
            for (part, p) in &m.parts {
                match part {
                    Part::Continuous(f) => density += p * f.pdf(x),
                    Part::Point(_) => {
                        return Err(OpError::new(
                            "pdf needs a continuous distribution, without single values mixed in",
                        ));
                    }
                }
            }
            Ok(Value::Float(density))
        }
        _ => Err(OpError::new(format!(
            "`{}` needs a distribution whose outcomes can be listed, not a continuous one",
            b.name()
        ))),
    }
}

fn as_dist(v: &Value) -> Dist {
    match v {
        Value::Dist(d) => (**d).clone(),
        other => Dist::point(other.clone()),
    }
}

fn numeric_dist(v: &Value, what: &str) -> OpResult<Dist> {
    let d = as_dist(v);
    if d.mean().is_none() {
        return Err(OpError::new(format!(
            "{what} needs numbers, found {}",
            article(&v.kind())
        )));
    }
    Ok(d)
}

fn iter_items(v: &Value, budget: &mut Budget) -> OpResult<Value> {
    match v {
        Value::List(_) | Value::Range(..) => Ok(v.clone()),
        Value::Map(m) => Ok(Value::list(
            m.iter().map(|(k, x)| Value::list(vec![k.clone(), x.clone()])).collect(),
        )),
        Value::Bag(b) => {
            let n: u128 = b.values().map(|n| *n as u128).sum();
            budget.collection(n)?;
            Ok(Value::list(
                b.iter()
                    .flat_map(|(k, n)| std::iter::repeat_n(k.clone(), *n as usize))
                    .collect(),
            ))
        }
        Value::Str(s) => Ok(Value::list(s.chars().map(|c| Value::str(&c.to_string())).collect())),
        v if v.is_uncertain() => Err(OpError::new(format!("can't loop over a {}", v.kind()))
            .help("draw a value first with `~`, or loop over `support(…)`")),
        other => Err(OpError::new(format!("can't loop over {}", article(&other.kind())))),
    }
}

fn expected(what: &str, v: &Value, func: &str) -> OpError {
    OpError::new(format!("`{func}` needs {what}, found {}", article(&v.kind())))
}

fn number(v: &Value, func: &str) -> OpResult<f64> {
    match v {
        Value::Bool(_) => Err(expected("a number", v, func)),
        Value::Continuous(_) => {
            Err(expected("a number", v, func).help("draw a value first, like `let x ~ normal(0, 1)`"))
        }
        _ => v.as_f64().ok_or_else(|| expected("a number", v, func)),
    }
}

fn whole(v: &Value, what: &str) -> OpResult<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Float(f) if f.fract() == 0.0 && f.abs() < 9.2e18 => Ok(*f as i64),
        other => Err(OpError::new(format!(
            "{what} must be a whole number, not {}",
            article(&other.kind())
        ))),
    }
}

/// Integer mathematics must not round its inputs through floating point.
fn integer(v: &Value, func: &str) -> OpResult<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        other => Err(expected("an int", other, func)),
    }
}

fn nonnegative_int(v: &Value, func: &str) -> OpResult<u64> {
    u64::try_from(integer(v, func)?).map_err(|_| OpError::new(format!("`{func}` needs nonnegative integers")))
}

fn int_overflow(func: &str) -> OpError {
    OpError::new(format!(
        "integer overflow in `{func}`: the result is too large for an int"
    ))
}

fn checked_int(n: u128, func: &str) -> OpResult<i64> {
    i64::try_from(n).map_err(|_| int_overflow(func))
}

fn choose(n: u64, k: u64, budget: &mut Budget) -> OpResult<Value> {
    if k > n {
        return Ok(Value::Int(0));
    }
    let k = k.min(n - k);
    let mut result = 1i64;
    for i in 1..=k {
        budget.work(1)?;
        // The division is exact at every step. A 128-bit intermediate
        // holds the product of two nonnegative i64s even near the limit.
        result = checked_int(result as u128 * u128::from(n - k + i) / u128::from(i), "choose")?;
    }
    Ok(Value::Int(result))
}

fn factorial(n: u64, budget: &mut Budget) -> OpResult<Value> {
    let mut result = 1i64;
    for i in 2..=n {
        budget.work(1)?;
        // Overflow terminates even a call with an enormous n after at most 20 multiplications.
        result = result.checked_mul(i as i64).ok_or_else(|| int_overflow("factorial"))?;
    }
    Ok(Value::Int(result))
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn euler_phi(mut n: u64, budget: &mut Budget) -> OpResult<Value> {
    if n == 0 {
        return Err(OpError::new("`euler_phi` needs a positive integer"));
    }
    let mut result = n;
    let mut divisor = 2;
    while divisor <= n / divisor {
        // Factoring large integers can be expensive; every attempted divisor
        // and repeated factor is charged to the host's work budget.
        budget.work(1)?;
        if n % divisor == 0 {
            result -= result / divisor;
            while n % divisor == 0 {
                budget.work(1)?;
                n /= divisor;
            }
        }
        divisor = if divisor == 2 { 3 } else { divisor + 2 };
    }
    if n > 1 {
        result -= result / n;
    }
    Ok(Value::Int(result as i64))
}

fn text(v: &Value, func: &str) -> OpResult<Arc<str>> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(expected("a string", other, func)),
    }
}

fn list<'a>(v: &'a Value, func: &str) -> OpResult<&'a [Value]> {
    match v {
        Value::List(items) => Ok(items),
        other => Err(expected("a list", other, func)),
    }
}

/// The elements of a list-like value, checking that a range isn't too long
/// to spell out.
pub fn items(v: &Value, func: &str, budget: &mut Budget) -> OpResult<Vec<Value>> {
    match v {
        Value::List(items) => Ok(items.to_vec()),
        Value::Range(lo, hi) => {
            budget.collection(range_len(*lo, *hi))?;
            Ok((*lo..=*hi).map(Value::Int).collect())
        }
        other => Err(expected("a list", other, func)),
    }
}

fn sort(items: &mut [Value]) -> OpResult<()> {
    let mut err = None;
    items.sort_by(|x, y| {
        ops::compare(x, y).unwrap_or_else(|e| {
            err.get_or_insert(e);
            std::cmp::Ordering::Equal
        })
    });
    err.map_or(Ok(()), Err)
}

fn num1(v: &Value, func: &str, f: fn(f64) -> f64, i: fn(i64) -> Option<i64>) -> OpResult<Value> {
    match v {
        Value::Int(x) => i(*x).map(Value::Int).ok_or_else(|| OpError::new("integer overflow")),
        Value::Float(x) => Ok(Value::Float(f(*x))),
        Value::Prob(x) => Ok(Value::Prob(f(*x))),
        other => Err(expected("a number", other, func)),
    }
}

fn float1(v: &Value, func: &str, f: impl Fn(f64) -> Option<f64>) -> OpResult<Value> {
    let x = number(v, func)?;
    f(x).filter(|y| x.is_finite() && y.is_finite())
        .map(Value::Float)
        .ok_or_else(|| OpError::new(format!("`{func}` isn't defined for {}", fmt_float(x))))
}

fn float2(a: &Value, b: &Value, func: &str, f: fn(f64, f64) -> f64) -> OpResult<Value> {
    let (a, b) = (number(a, func)?, number(b, func)?);
    if !a.is_finite() || !b.is_finite() {
        return Err(OpError::new(format!("`{func}` needs finite numbers")));
    }
    // Value equality merges signed zeros. In particular, atan2 must not
    // distinguish worlds (or memoized arguments) that the engine considers equal.
    let a = if a == 0.0 { 0.0 } else { a };
    let b = if b == 0.0 { 0.0 } else { b };
    let value = f(a, b);
    if !value.is_finite() {
        return Err(OpError::new(format!(
            "`{func}` gave a result that isn't a finite number"
        )));
    }
    Ok(Value::Float(value))
}

fn to_int(v: &Value, f: fn(f64) -> f64) -> OpResult<Value> {
    match v {
        Value::Int(i) => Ok(Value::Int(*i)),
        other => {
            let x = number(other, "rounding")?;
            let r = f(x);
            if !r.is_finite() || r.abs() > 9.2e18 {
                return Err(OpError::new(format!("{} is too large to be an int", fmt_float(x))));
            }
            Ok(Value::Int(r as i64))
        }
    }
}

fn round(v: &Value, digits: Option<&Value>) -> OpResult<Value> {
    let Some(digits) = digits else {
        return to_int(v, f64::round);
    };
    let digits = integer(digits, "round's digits")?;
    if let Value::Int(n) = v {
        if digits >= 0 {
            return Ok(v.clone());
        }
        // Even i64::MIN is less than half of 10^20 in magnitude. Bound the
        // exponent before negating it, since digits itself can be i64::MIN.
        if digits <= -20 {
            return Ok(Value::Int(0));
        }
        let scale = 10i128.pow((-digits) as u32);
        let magnitude = i128::from(*n).abs();
        let rounded = (magnitude + scale / 2) / scale * scale * i128::from(n.signum());
        return i64::try_from(rounded)
            .map(Value::Int)
            .map_err(|_| int_overflow("round"));
    }
    float1(v, "round", |x| {
        // Beyond these bounds a decimal place cannot change a finite f64,
        // or every finite f64 rounds to zero. No unbounded powers or loops.
        if digits > 323 || x == 0.0 {
            return Some(x);
        }
        if digits < -308 {
            return Some(0.0);
        }
        let rounded = if digits >= 0 {
            // Splitting the scale supports subnormals (up to 323 places)
            // without forming an infinite power of ten.
            let high = libm::pow(10.0, digits.min(308) as f64);
            let low = libm::pow(10.0, (digits - 308).max(0) as f64);
            let scaled = (x * high) * low;
            // At this precision the decimal adjustment is smaller than half
            // an f64 step; scaling back could only introduce a new error.
            if scaled.abs() >= 1e16 || x.fract() == 0.0 {
                return Some(x);
            }
            (scaled.round() / low) / high
        } else {
            let scale = libm::pow(10.0, -digits as f64);
            let scaled = x / scale;
            if scaled.abs() >= 1e16 {
                return Some(x);
            }
            scaled.round() * scale
        };
        Some(rounded)
    })
}

fn min_max(args: &[Value], want_max: bool, budget: &mut Budget) -> OpResult<Value> {
    let name = if want_max { "max" } else { "min" };
    let values: Vec<Value> = if let [single] = args {
        match single {
            Value::Range(lo, hi) => {
                if hi < lo {
                    return Err(OpError::new(format!("`{name}` of an empty range")));
                }
                return Ok(Value::Int(if want_max { *hi } else { *lo }));
            }
            Value::List(_) => items(single, name, budget)?,
            other => vec![other.clone()],
        }
    } else {
        args.to_vec()
    };
    let mut best: Option<Value> = None;
    for v in values {
        best = Some(match best {
            None => v,
            Some(b) => {
                let ord = ops::compare(&v, &b)?;
                if (want_max && ord.is_gt()) || (!want_max && ord.is_lt()) {
                    v
                } else {
                    b
                }
            }
        });
    }
    best.ok_or_else(|| OpError::new(format!("`{name}` of an empty list")))
}

fn len(v: &Value) -> OpResult<Value> {
    let n: u128 = match v {
        Value::List(items) => items.len() as u128,
        Value::Str(s) => s.chars().count() as u128,
        Value::Map(m) => m.len() as u128,
        Value::Bag(b) => b.values().map(|n| *n as u128).sum(),
        Value::Range(lo, hi) => range_len(*lo, *hi),
        other => return Err(expected("a collection", other, "len")),
    };
    i64::try_from(n)
        .map(Value::Int)
        .map_err(|_| OpError::new("the length is too large to be an int"))
}

fn sum(v: &Value, budget: &mut Budget) -> OpResult<Value> {
    let items = items(v, "sum", budget)?;
    let mut acc = Value::Int(0);
    for x in &items {
        acc = ops::binary(probl_syntax::ast::BinOp::Add, &acc, x, budget)?;
    }
    Ok(acc)
}

fn get(coll: &Value, key: &Value, default: Option<&Value>) -> OpResult<Value> {
    let found = match coll {
        Value::Map(m) => m
            .get(key)
            .or_else(|| m.iter().find(|(k, _)| equals(k, key)).map(|(_, v)| v))
            .cloned(),
        Value::List(items) => match key {
            Value::Int(i) if *i >= 0 && (*i as usize) < items.len() => Some(items[*i as usize].clone()),
            _ => None,
        },
        Value::Bag(b) => Some(Value::Int(b.get(key).copied().unwrap_or(0).min(i64::MAX as u64) as i64)),
        other => return Err(expected("a map or a list", other, "get")),
    };
    match (found, default) {
        (Some(v), _) => Ok(v),
        (None, Some(d)) => Ok(d.clone()),
        (None, None) => {
            Err(OpError::new(format!("the key {key:?} isn't there")).help("give a default: `get(key, default)`"))
        }
    }
}

fn extremes(v: &Value, n: Option<&Value>, highest: bool, budget: &mut Budget) -> OpResult<Value> {
    let mut items = items(v, if highest { "highest" } else { "lowest" }, budget)?;
    sort(&mut items)?;
    if highest {
        items.reverse();
    }
    match n {
        None => items.into_iter().next().ok_or_else(|| OpError::new("empty list")),
        Some(n) => {
            let n = whole(n, "the count")?.max(0) as usize;
            items.truncate(n);
            Ok(Value::list(items))
        }
    }
}

fn insert(coll: &Value, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            budget.collection(items.len() as u128 + 1)?;
            let mut items = items.to_vec();
            let i = as_index(a, items.len() as u128 + 1)? as usize;
            items.insert(i, b.clone());
            Ok(Value::list(items))
        }
        Value::Map(m) => {
            budget.collection(m.len() as u128 + 1)?;
            let mut m = BTreeMap::clone(m);
            m.insert(a.clone(), b.clone());
            Ok(Value::map(m))
        }
        other => Err(expected("a list or a map", other, "insert")),
    }
}

fn remove(coll: &Value, key: &Value) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let mut items = items.to_vec();
            let i = as_index(key, items.len() as u128)? as usize;
            items.remove(i);
            Ok(Value::list(items))
        }
        Value::Map(m) => {
            let mut m = BTreeMap::clone(m);
            m.remove(key);
            Ok(Value::map(m))
        }
        Value::Bag(b) => match b.without(key) {
            Some(rest) => Ok(Value::multiset(rest)),
            None => Err(OpError::new(format!("{key:?} isn't in the bag"))),
        },
        other => Err(expected("a list, map or bag", other, "remove")),
    }
}

/// A choice among options. Options that are distributions are mixed in, so
/// that a value drawn from the result is settled (docs/semantics.md, section 2).
fn one_of(v: &Value, budget: &mut Budget) -> OpResult<Value> {
    match v {
        Value::List(items) if !items.is_empty() => {
            budget.outcomes(items.len() as u128)?;
            let p = 1.0 / items.len() as f64;
            ops::combine(items.iter().map(|x| (x.clone(), p)).collect(), 0.0, budget)
        }
        Value::Range(lo, hi) if hi >= lo => {
            budget.outcomes(range_len(*lo, *hi))?;
            Ok(Dist::uniform((*lo..=*hi).map(Value::Int).collect()).into_value())
        }
        Value::Map(m) if !m.is_empty() => {
            let all_probs = m.values().all(|w| matches!(w, Value::Prob(_)));
            let mut pairs = Vec::new();
            for (k, w) in m.iter() {
                let w = match w {
                    Value::Bool(_) => None,
                    _ => w.as_f64().filter(|w| *w >= 0.0 && w.is_finite()),
                }
                .ok_or_else(|| {
                    OpError::new(format!(
                        "one_of needs weights that are numbers of 0 or more, found {w:?}"
                    ))
                })?;
                pairs.push((k.clone(), w));
            }
            let total: f64 = pairs.iter().map(|(_, w)| w).sum();
            if total <= 0.0 {
                return Err(OpError::new("one_of needs at least one positive weight"));
            }
            if all_probs && (total - 1.0).abs() > 1e-9 {
                return Err(OpError::new(format!(
                    "the chances add up to {}, not 100%",
                    crate::value::fmt_prob(total)
                ))
                .help("use plain numbers for relative weights, like [\"a\": 3, \"b\": 1]"));
            }
            ops::combine(pairs.into_iter().map(|(k, w)| (k, w / total)).collect(), 0.0, budget)
        }
        Value::Bag(b) => {
            let total: u128 = b.values().map(|n| *n as u128).sum();
            if total == 0 {
                return Err(OpError::new("the bag is empty"));
            }
            let pairs = b.iter().map(|(k, n)| (k.clone(), *n as f64 / total as f64)).collect();
            ops::combine(pairs, 0.0, budget)
        }
        Value::List(_) | Value::Range(..) | Value::Map(_) => Err(OpError::new("one_of needs at least one option")),
        other => Err(expected("a list, range, map or bag", other, "one_of")),
    }
}

fn bag(v: &Value) -> OpResult<Value> {
    let mut counts = BTreeMap::new();
    match v {
        Value::Map(m) => {
            for (k, n) in m.iter() {
                let n = match n {
                    Value::Int(n) if *n >= 0 => *n as u64,
                    other => {
                        return Err(OpError::new(format!(
                            "bag counts must be whole numbers of 0 or more, found {other:?}"
                        )));
                    }
                };
                if n > 0 {
                    counts.insert(k.clone(), n);
                }
            }
        }
        Value::List(items) => {
            for item in items.iter() {
                *counts.entry(item.clone()).or_insert(0) += 1;
            }
        }
        other => return Err(expected("a map of counts or a list", other, "bag")),
    }
    Ok(Value::bag(counts))
}
