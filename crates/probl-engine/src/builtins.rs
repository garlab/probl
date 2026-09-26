//! Built-in functions that don't need the interpreter.

use crate::dates;
use crate::dist::Dist;
use crate::error::{OpError, OpResult};
use crate::ops::{self, article, as_index, equals, to_chance};
use crate::value::{Value, fmt_float, prob};
use probl_sema::Builtin;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Call a built-in on plain (non-distribution) arguments.
pub fn call_plain(b: Builtin, args: &[Value]) -> OpResult<Value> {
    use Builtin as B;
    let a = |i: usize| &args[i];
    match b {
        B::Min | B::Max => min_max(args, b == B::Max),
        B::Abs => num1(a(0), "abs", |x| x.abs(), |i| i.checked_abs()),
        B::Floor => to_int(a(0), f64::floor),
        B::Ceil => to_int(a(0), f64::ceil),
        B::Round => to_int(a(0), f64::round),
        B::Sqrt => float1(a(0), "sqrt", |x| (x >= 0.0).then(|| x.sqrt())),
        B::Exp => float1(a(0), "exp", |x| Some(x.exp())),
        B::Ln => float1(a(0), "ln", |x| (x > 0.0).then(|| x.ln())),
        B::Log10 => float1(a(0), "log10", |x| (x > 0.0).then(|| x.log10())),
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
        B::Len => len(a(0)).map(|n| Value::Int(n as i64)),
        B::Sum => sum(a(0)),
        B::Count if args.len() == 1 => len(a(0)).map(|n| Value::Int(n as i64)),
        B::Sort | B::SortDesc => {
            let mut items = items(a(0), b.name())?;
            let mut err = None;
            items.sort_by(|x, y| {
                ops::compare(x, y).unwrap_or_else(|e| {
                    err.get_or_insert(e);
                    std::cmp::Ordering::Equal
                })
            });
            if let Some(e) = err {
                return Err(e);
            }
            if b == B::SortDesc {
                items.reverse();
            }
            Ok(Value::list(items))
        }
        B::Reverse => match a(0) {
            Value::Str(s) => Ok(Value::str(&s.chars().rev().collect::<String>())),
            v => {
                let mut items = items(v, "reverse")?;
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
        B::Contains => ops::contains(a(0), a(1)).map(prob),
        B::Highest | B::Lowest => extremes(a(0), args.get(1), b == B::Highest),
        B::Enumerate => {
            let items = items(a(0), "enumerate")?;
            Ok(Value::list(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, x)| Value::list(vec![Value::Int(i as i64), x]))
                    .collect(),
            ))
        }
        B::Zip => {
            let (xs, ys) = (items(a(0), "zip")?, items(a(1), "zip")?);
            Ok(Value::list(
                xs.into_iter().zip(ys).map(|(x, y)| Value::list(vec![x, y])).collect(),
            ))
        }
        B::Push => {
            let mut items = list(a(0), "push")?.to_vec();
            items.push(a(1).clone());
            Ok(Value::list(items))
        }
        B::Insert => insert(a(0), a(1), a(2)),
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
        B::OneOf => one_of(a(0)),
        B::Binomial => {
            let n = whole(a(0), "binomial's number of trials")?;
            if n < 0 {
                return Err(OpError::new("binomial needs a number of trials of 0 or more"));
            }
            Ok(Dist::binomial(n as u64, to_chance(a(1))?).into_value())
        }
        B::Poisson => {
            let rate = number(a(0), "poisson")?;
            if rate < 0.0 || !rate.is_finite() {
                return Err(OpError::new("poisson needs a rate of 0 or more"));
            }
            Ok(Dist::poisson(rate).into_value())
        }
        B::Geometric => {
            let p = to_chance(a(0))?;
            if p <= 0.0 {
                return Err(OpError::new("geometric needs a chance of success above 0%"));
            }
            Ok(Dist::geometric(p).into_value())
        }
        B::Bag => bag(a(0)),
        B::Normal
        | B::Lognormal
        | B::Uniform
        | B::Beta
        | B::Gamma
        | B::Exponential
        | B::Triangular
        | B::Pert
        | B::To => {
            let name = if b == B::To {
                "`a to b` estimates".to_string()
            } else {
                format!("`{}`", b.name())
            };
            Err(
                OpError::unsupported(format!("{name} (continuous distributions) aren't implemented yet"))
                    .help("they arrive with sample mode in v0.2"),
            )
        }
        B::Mixture | B::Truncate | B::Bins | B::Pdf | B::Today => {
            Err(OpError::unsupported(format!("`{}` isn't implemented yet", b.name())))
        }
        B::Odds => {
            let p = to_chance(a(0))?;
            if p >= 1.0 {
                return Err(OpError::new("the odds of a certain event are infinite"));
            }
            Ok(Value::Float(p / (1.0 - p)))
        }
        B::Logit => {
            let p = to_chance(a(0))?;
            if p <= 0.0 || p >= 1.0 {
                return Err(OpError::new("logit needs a probability strictly between 0% and 100%"));
            }
            Ok(Value::Float((p / (1.0 - p)).ln()))
        }
        B::InvLogit => Ok(Value::Prob(1.0 / (1.0 + (-number(a(0), "inv_logit")?).exp()))),
        B::Date => {
            let s = text(a(0), "date")?;
            dates::parse(&s)
                .map(Value::Date)
                .ok_or_else(|| OpError::new(format!("`{s}` isn't a valid date")).help("write dates as \"YYYY-MM-DD\""))
        }
        B::Days => to_int(a(0), f64::round),
        B::Weeks => {
            let n = number(a(0), "weeks")?;
            Ok(Value::Int((n * 7.0).round() as i64))
        }
        B::AddWorkdays => match a(0) {
            Value::Date(d) => Ok(Value::Date(dates::add_workdays(*d, whole(a(1), "add_workdays")?))),
            v => Err(expected("a date", v, "add_workdays")),
        },
        B::Weekday => match a(0) {
            Value::Date(d) => Ok(Value::str(dates::WEEKDAYS[dates::weekday(*d) as usize])),
            v => Err(expected("a date", v, "weekday")),
        },
        B::IsFalse => Ok(prob(ops::is_certain(a(0), false))),
        B::IsTrue => Ok(prob(ops::is_certain(a(0), true))),
        B::IsListOfLen => Ok(prob(
            matches!((a(0), a(1)), (Value::List(items), Value::Int(n)) if items.len() as i64 == *n),
        )),
        B::Count | B::Map | B::Filter | B::Reduce | B::Print | B::Roll | B::Take | B::IterItems | B::RepeatCount => {
            unreachable!("`{}` is handled by the interpreter", b.name())
        }
        B::P | B::Mean | B::Sd | B::Variance | B::Median | B::Quantile | B::Support | B::Cdf | B::Pmf => {
            unreachable!("`{}` takes distributions as they are", b.name())
        }
    }
}

/// Built-ins that receive distributions whole.
pub fn call_raw(b: Builtin, args: &[Value]) -> OpResult<Value> {
    use Builtin as B;
    let v = &args[0];
    match b {
        B::P => Ok(Value::Prob(to_chance(v).map_err(|_| {
            OpError::new(format!("P needs a condition, found {}", article(&v.kind()))).help("for example `P(d6 > 4)`")
        })?)),
        B::Mean => numeric_dist(v, "mean").map(|d| Value::Float(d.mean().unwrap())),
        B::Variance => numeric_dist(v, "variance").map(|d| Value::Float(d.variance().unwrap())),
        B::Sd => numeric_dist(v, "sd").map(|d| Value::Float(d.variance().unwrap().sqrt())),
        B::Median => Ok(as_dist(v).quantile(0.5).unwrap()),
        B::Quantile => {
            let q = to_chance(&args[1])?;
            Ok(as_dist(v).quantile(q).unwrap())
        }
        B::Support => Ok(Value::list(
            as_dist(v).outcomes.iter().map(|(x, _)| x.clone()).collect(),
        )),
        B::Cdf => {
            let d = as_dist(v);
            let total = d.total();
            let mut p = 0.0;
            for (x, w) in &d.outcomes {
                if ops::compare(x, &args[1])?.is_le() {
                    p += w;
                }
            }
            Ok(Value::Prob(p / total))
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
        B::IterItems => iter_items(v),
        B::RepeatCount => match v {
            Value::Int(n) if *n >= 0 => Ok(Value::Int(*n)),
            Value::Int(_) => Err(OpError::new("`repeat` needs a count of 0 or more")),
            Value::Float(f) if f.fract() == 0.0 && *f >= 0.0 => Ok(Value::Int(*f as i64)),
            Value::Dist(_) => Err(OpError::new(format!("`repeat` needs a number, not a {}", v.kind()))
                .help("draw a value first, like `let n ~ d6`, then `repeat n { … }`")),
            other => Err(OpError::new(format!(
                "`repeat` needs a whole number, found {}",
                article(&other.kind())
            ))),
        },
        // Internal helpers that take their argument as it is.
        _ => call_plain(b, args),
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

fn iter_items(v: &Value) -> OpResult<Value> {
    match v {
        Value::List(_) | Value::Range(..) => Ok(v.clone()),
        Value::Map(m) => Ok(Value::list(
            m.iter().map(|(k, x)| Value::list(vec![k.clone(), x.clone()])).collect(),
        )),
        Value::Bag(b) => Ok(Value::list(
            b.iter()
                .flat_map(|(k, n)| std::iter::repeat_n(k.clone(), *n as usize))
                .collect(),
        )),
        Value::Str(s) => Ok(Value::list(s.chars().map(|c| Value::str(&c.to_string())).collect())),
        Value::Dist(_) => Err(OpError::new(format!("can't loop over a {}", v.kind()))
            .help("draw a value first with `~`, or loop over `support(…)`")),
        other => Err(OpError::new(format!("can't loop over {}", article(&other.kind())))),
    }
}

fn expected(what: &str, v: &Value, func: &str) -> OpError {
    OpError::new(format!("`{func}` needs {what}, found {}", article(&v.kind())))
}

fn number(v: &Value, func: &str) -> OpResult<f64> {
    v.as_f64().ok_or_else(|| expected("a number", v, func))
}

fn whole(v: &Value, what: &str) -> OpResult<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Float(f) if f.fract() == 0.0 => Ok(*f as i64),
        other => Err(OpError::new(format!(
            "{what} must be a whole number, not {}",
            article(&other.kind())
        ))),
    }
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

/// The elements of a list-like value.
fn items(v: &Value, func: &str) -> OpResult<Vec<Value>> {
    match v {
        Value::List(items) => Ok((**items).clone()),
        Value::Range(lo, hi) => Ok((*lo..=*hi).map(Value::Int).collect()),
        other => Err(expected("a list", other, func)),
    }
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
    f(x).map(Value::Float)
        .ok_or_else(|| OpError::new(format!("`{func}` isn't defined for {}", fmt_float(x))))
}

fn to_int(v: &Value, f: fn(f64) -> f64) -> OpResult<Value> {
    match v {
        Value::Int(i) => Ok(Value::Int(*i)),
        other => {
            let x = other
                .as_f64()
                .ok_or_else(|| OpError::new(format!("expected a number, found {}", article(&other.kind()))))?;
            let r = f(x);
            if !r.is_finite() || r.abs() > 9.2e18 {
                return Err(OpError::new(format!("{} is too large to be an int", fmt_float(x))));
            }
            Ok(Value::Int(r as i64))
        }
    }
}

fn min_max(args: &[Value], want_max: bool) -> OpResult<Value> {
    let values: Vec<Value> = if let [single] = args {
        match single {
            Value::List(_) | Value::Range(..) => items(single, if want_max { "max" } else { "min" })?,
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
    best.ok_or_else(|| OpError::new(format!("`{}` of an empty list", if want_max { "max" } else { "min" })))
}

fn len(v: &Value) -> OpResult<usize> {
    Ok(match v {
        Value::List(items) => items.len(),
        Value::Str(s) => s.chars().count(),
        Value::Map(m) => m.len(),
        Value::Bag(b) => b.values().sum::<u64>() as usize,
        Value::Range(lo, hi) => (hi - lo + 1).max(0) as usize,
        other => return Err(expected("a collection", other, "len")),
    })
}

fn sum(v: &Value) -> OpResult<Value> {
    let items = items(v, "sum")?;
    let mut acc = Value::Int(0);
    for x in &items {
        acc = ops::binary(probl_syntax::ast::BinOp::Add, &acc, x)?;
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
        Value::Bag(b) => Some(Value::Int(b.get(key).copied().unwrap_or(0) as i64)),
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

fn extremes(v: &Value, n: Option<&Value>, highest: bool) -> OpResult<Value> {
    let mut items = items(v, if highest { "highest" } else { "lowest" })?;
    let mut err = None;
    items.sort_by(|x, y| {
        ops::compare(x, y).unwrap_or_else(|e| {
            err.get_or_insert(e);
            std::cmp::Ordering::Equal
        })
    });
    if let Some(e) = err {
        return Err(e);
    }
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

fn insert(coll: &Value, a: &Value, b: &Value) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let mut items = (**items).clone();
            let i = as_index(a, items.len() + 1)?;
            items.insert(i, b.clone());
            Ok(Value::list(items))
        }
        Value::Map(m) => {
            let mut m = (**m).clone();
            m.insert(a.clone(), b.clone());
            Ok(Value::Map(Arc::new(m)))
        }
        other => Err(expected("a list or a map", other, "insert")),
    }
}

fn remove(coll: &Value, key: &Value) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let mut items = (**items).clone();
            let i = as_index(key, items.len())?;
            items.remove(i);
            Ok(Value::list(items))
        }
        Value::Map(m) => {
            let mut m = (**m).clone();
            m.remove(key);
            Ok(Value::Map(Arc::new(m)))
        }
        Value::Bag(b) => {
            let mut b = (**b).clone();
            match b.get_mut(key) {
                Some(n) if *n > 1 => *n -= 1,
                Some(_) => {
                    b.remove(key);
                }
                None => return Err(OpError::new(format!("{key:?} isn't in the bag"))),
            }
            Ok(Value::Bag(Arc::new(b)))
        }
        other => Err(expected("a list, map or bag", other, "remove")),
    }
}

fn one_of(v: &Value) -> OpResult<Value> {
    match v {
        Value::List(items) if !items.is_empty() => Ok(Dist::uniform((**items).clone()).into_value()),
        Value::Range(lo, hi) if hi >= lo => Ok(Dist::uniform((*lo..=*hi).map(Value::Int).collect()).into_value()),
        Value::Map(m) if !m.is_empty() => {
            let all_probs = m.values().all(|w| matches!(w, Value::Prob(_)));
            let mut pairs = Vec::new();
            for (k, w) in m.iter() {
                let w = w.as_f64().filter(|w| *w >= 0.0).ok_or_else(|| {
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
            Ok(Dist::from_pairs(pairs.into_iter().map(|(k, w)| (k, w / total)).collect(), 0.0).into_value())
        }
        Value::Bag(b) => {
            let total: u64 = b.values().sum();
            if total == 0 {
                return Err(OpError::new("the bag is empty"));
            }
            Ok(Dist::from_pairs(
                b.iter().map(|(k, n)| (k.clone(), *n as f64 / total as f64)).collect(),
                0.0,
            )
            .into_value())
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
    Ok(Value::Bag(Arc::new(counts)))
}
