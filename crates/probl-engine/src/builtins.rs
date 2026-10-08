//! Built-in functions that don't need the interpreter.

use crate::complex::Complex;
use crate::continuous::{Family, Mixture, Part};
use crate::dates;
use crate::dist::{Budget, Counts, Dist};
use crate::error::{Fault, OpError, OpResult};
use crate::ops::{self, article, as_index, integer, range_count, range_len, to_prob};
use crate::value::{Value, fmt_float};
use probl_number::Integer;
use probl_sema::Builtin;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Call a built-in on plain (non-distribution) arguments.
pub fn call_plain(b: Builtin, args: &[Value], budget: &mut Budget) -> OpResult<Value> {
    for arg in args {
        if let Value::Str(s) = arg {
            budget.string_work(s)?;
        }
        if let Value::Int(n) = arg {
            if matches!(b, Builtin::BitLength | Builtin::ILog2) {
                // The stored length and highest word suffice, even for a bigint.
                budget.integer_bits(n.bits())?;
                budget.work(1)?;
            } else {
                budget.integer_work(n, if b == Builtin::Str { n } else { &Integer::ONE }, b == Builtin::Str)?;
            }
        }
    }
    let value = call_plain_inner(b, args, budget)?;
    if let Value::Int(n) = &value {
        budget.integer_allocation(n.bits(), 1)?;
    }
    Ok(value)
}

fn call_plain_inner(b: Builtin, args: &[Value], budget: &mut Budget) -> OpResult<Value> {
    use Builtin as B;
    let a = |i: usize| &args[i];
    match b {
        B::Min | B::Max => min_max(args, b == B::Max, budget),
        B::Abs => match a(0) {
            Value::Complex(z) => finite_float(z.abs(), "abs"),
            Value::Int(n) => Ok(Value::Int(n.abs())),
            Value::Prob(p) => Ok(Value::Prob(p.abs())),
            v => float1(v, "abs", |x| Some(x.abs())),
        },
        B::Floor => to_int(a(0), f64::floor),
        B::Ceil => to_int(a(0), f64::ceil),
        B::Trunc => to_int(a(0), libm::trunc),
        B::Round => round(a(0), args.get(1), budget),
        B::Sqrt => elementary1(a(0), "sqrt", Complex::sqrt, |x| (x >= 0.0).then(|| x.sqrt())),
        B::Cbrt => elementary1(a(0), "cbrt", Complex::cbrt, |x| Some(libm::cbrt(x))),
        B::Exp => elementary1(a(0), "exp", Complex::exp, |x| {
            Some(crate::math::exp(x)).filter(|y| y.is_finite())
        }),
        B::Exp2 => elementary1(a(0), "exp2", Complex::exp2, |x| Some(libm::exp2(x))),
        B::Ln => elementary1(a(0), "ln", Complex::ln, |x| (x > 0.0).then(|| libm::log(x))),
        B::Log10 => elementary1(a(0), "log10", Complex::log10, |x| (x > 0.0).then(|| libm::log10(x))),
        B::Log2 => elementary1(a(0), "log2", Complex::log2, |x| (x > 0.0).then(|| libm::log2(x))),
        B::Log1p => elementary1(a(0), "log1p", Complex::log1p, |x| (x > -1.0).then(|| libm::log1p(x))),
        B::Expm1 => elementary1(a(0), "expm1", Complex::expm1, |x| {
            Some(libm::expm1(x)).filter(|y| y.is_finite())
        }),
        B::Sin => elementary1(a(0), "sin", Complex::sin, |x| Some(libm::sin(x))),
        B::Cos => elementary1(a(0), "cos", Complex::cos, |x| Some(libm::cos(x))),
        B::Tan => elementary1(a(0), "tan", Complex::tan, |x| Some(libm::tan(x))),
        B::Asin => elementary1(a(0), "asin", Complex::asin, |x| {
            (-1.0..=1.0).contains(&x).then(|| libm::asin(x))
        }),
        B::Acos => elementary1(a(0), "acos", Complex::acos, |x| {
            (-1.0..=1.0).contains(&x).then(|| libm::acos(x))
        }),
        B::Atan => elementary1(a(0), "atan", Complex::atan, |x| Some(libm::atan(x))),
        B::Atan2 => float2(a(0), a(1), "atan2", libm::atan2),
        B::Hypot => float2(a(0), a(1), "hypot", libm::hypot),
        B::Sinh => elementary1(a(0), "sinh", Complex::sinh, |x| {
            Some(libm::sinh(x)).filter(|y| y.is_finite())
        }),
        B::Cosh => elementary1(a(0), "cosh", Complex::cosh, |x| {
            Some(libm::cosh(x)).filter(|y| y.is_finite())
        }),
        B::Tanh => elementary1(a(0), "tanh", Complex::tanh, |x| Some(libm::tanh(x))),
        B::Asinh => elementary1(a(0), "asinh", Complex::asinh, |x| Some(libm::asinh(x))),
        B::Acosh => elementary1(a(0), "acosh", Complex::acosh, |x| (x >= 1.0).then(|| libm::acosh(x))),
        B::Atanh => elementary1(a(0), "atanh", Complex::atanh, |x| {
            (x.abs() < 1.0).then(|| libm::atanh(x))
        }),
        B::BitLength => Ok(Value::Int(integer(a(0), "bit_length", budget)?.bits().into())),
        B::BitAnd => Ok(Value::Int(
            integer(a(0), b.name(), budget)?.bit_and(integer(a(1), b.name(), budget)?.as_ref())?,
        )),
        B::BitOr => Ok(Value::Int(
            integer(a(0), b.name(), budget)?.bit_or(integer(a(1), b.name(), budget)?.as_ref())?,
        )),
        B::BitXor => Ok(Value::Int(
            integer(a(0), b.name(), budget)?.bit_xor(integer(a(1), b.name(), budget)?.as_ref())?,
        )),
        B::BitNot => Ok(Value::Int(integer(a(0), b.name(), budget)?.bit_not()?)),
        B::BitCount => Ok(Value::Int(integer(a(0), b.name(), budget)?.bit_count().into())),
        B::ILog2 => {
            let n = integer(a(0), "ilog2", budget)?;
            if n.is_zero() || n.is_negative() {
                return Err(OpError::fault(Fault::Domain, "`ilog2` needs a positive integer"));
            }
            Ok(Value::Int((n.bits() - 1).into()))
        }
        B::Choose => choose(
            nonnegative_int(a(0), "choose", budget)?.as_ref(),
            nonnegative_int(a(1), "choose", budget)?.as_ref(),
            budget,
        ),
        B::Factorial => factorial(nonnegative_int(a(0), "factorial", budget)?.as_ref(), budget),
        B::Gcd | B::Lcm => {
            let (x, y) = (
                integer(a(0), b.name(), budget)?.abs(),
                integer(a(1), b.name(), budget)?.abs(),
            );
            let d = gcd(x.clone(), y.clone(), budget)?;
            let result = if b == B::Gcd {
                d
            } else if x.is_zero() || y.is_zero() {
                Integer::ZERO
            } else {
                let q = x.div_mod(&d)?.0;
                budget.integer_work(&q, &y, true)?;
                q.mul(&y)?
            };
            Ok(Value::Int(result))
        }
        B::EulerPhi => euler_phi(nonnegative_int(a(0), "euler_phi", budget)?.as_ref(), budget),
        B::LnGamma => float1(a(0), "ln_gamma", |x| (x > 0.0).then(|| libm::lgamma(x))),
        B::Erf => float1(a(0), "erf", |x| Some(libm::erf(x))),
        B::Erfc => float1(a(0), "erfc", |x| Some(libm::erfc(x))),
        B::Complex => {
            let z = if args.len() == 1 {
                complex_number(a(0), "complex")?
            } else {
                Complex::new(number(a(0), "complex")?, number(a(1), "complex")?)?
            };
            Ok(Value::Complex(z))
        }
        B::Real => Ok(Value::Float(complex_number(a(0), "real")?.re())),
        B::Imag => Ok(Value::Float(complex_number(a(0), "imag")?.im())),
        B::Conj => Ok(Value::Complex(complex_number(a(0), "conj")?.conjugate())),
        B::Abs2 => finite_float(complex_number(a(0), "abs2")?.abs2(), "abs2"),
        B::Arg => Ok(Value::Float(complex_number(a(0), "arg")?.arg())),
        B::Cis => {
            let theta = number(a(0), "cis")?;
            Complex::new(libm::cos(theta), libm::sin(theta)).map(Value::Complex)
        }
        B::Clamp => {
            let (lo, hi) = (a(1), a(2));
            if ops::compare(lo, hi)?.is_gt() {
                return Err(OpError::fault(
                    Fault::Domain,
                    "clamp's lower bound is above its upper bound",
                ));
            }
            if ops::compare(a(0), lo)?.is_lt() {
                Ok(lo.clone())
            } else if ops::compare(a(0), hi)?.is_gt() {
                Ok(hi.clone())
            } else {
                Ok(a(0).clone())
            }
        }
        B::Str => crate::text::formatted(a(0), budget),
        B::Upper | B::Lower => crate::text::case(&text(a(0), b.name())?, b == B::Upper, budget),
        B::Trim | B::TrimStart | B::TrimEnd => {
            let s = text(a(0), b.name())?;
            let set = args.get(1).map(|v| text(v, b.name())).transpose()?;
            let set = set
                .as_ref()
                .map(|s| {
                    let mut set = rustc_hash::FxHashSet::default();
                    for c in s.chars() {
                        if !set.contains(&c) {
                            budget.collection(set.len() as u128 + 1)?;
                            set.insert(c);
                        }
                    }
                    Ok::<_, OpError>(set)
                })
                .transpose()?;
            let matches = |c: char| set.as_ref().map_or_else(|| c.is_whitespace(), |set| set.contains(&c));
            let trimmed = match b {
                B::TrimStart => s.trim_start_matches(matches),
                B::TrimEnd => s.trim_end_matches(matches),
                _ => s.trim_matches(matches),
            };
            crate::text::value(trimmed, budget)
        }
        B::StartsWith | B::EndsWith => {
            let (s, part) = (text(a(0), b.name())?, text(a(1), b.name())?);
            Ok(Value::Bool(if b == B::StartsWith {
                s.starts_with(&*part)
            } else {
                s.ends_with(&*part)
            }))
        }
        B::Chars => crate::text::chars(&text(a(0), "chars")?, budget).map(Value::list),
        B::Split => {
            let (s, sep) = (text(a(0), "split")?, text(a(1), "split")?);
            if sep.is_empty() {
                return crate::text::chars(&s, budget).map(Value::list);
            }
            let mut parts = Vec::new();
            for part in s.split(&*sep) {
                budget.collection(parts.len() as u128 + 1)?;
                parts.push(crate::text::value(part, budget)?);
            }
            Ok(Value::list(parts))
        }
        B::Join => {
            let items = list(a(0), "join")?;
            let sep = text(a(1), "join")?;
            budget.collection(items.len() as u128)?;
            budget.work(items.len() as u64)?;
            let mut out = String::new();
            for (i, item) in items.iter().enumerate() {
                if i != 0 {
                    crate::text::push(&mut out, &sep, budget)?;
                }
                crate::text::push_value(&mut out, item, budget)?;
            }
            Ok(Value::str(&out))
        }
        B::Len => len(a(0)),
        B::Slice => slice(a(0), a(1), args.get(2), budget),
        B::Sum => sum(a(0), budget),
        B::Count if args.len() == 1 => len(a(0)),
        B::Sort | B::SortDesc => {
            assert_eq!(args.len(), 1, "comparator sorting is handled by the interpreter");
            let mut items = items(a(0), b.name(), budget)?;
            sort(&mut items, b == B::SortDesc, budget)?;
            Ok(Value::list(items))
        }
        B::Reverse => match a(0) {
            Value::Str(s) => {
                budget.string_allocation(s.len())?;
                Ok(Value::str(&s.chars().rev().collect::<String>()))
            }
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
        B::Get => get(a(0), a(1), args.get(2), budget),
        B::Contains => ops::contains(a(0), a(1), budget).map(Value::Bool),
        B::Highest | B::Lowest => extremes(a(0), a(1), b == B::Highest, budget),
        B::Enumerate => {
            let items = items(a(0), "enumerate", budget)?;
            Ok(Value::list(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, x)| Value::list(vec![Value::Int((i as i64).into()), x]))
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
        B::Remove => remove(a(0), a(1), budget),
        B::Pop => Err(
            OpError::new("`pop` changes a list, so call it as a method on a variable")
                .help("write `let top = xs.pop()`"),
        ),
        B::Last => {
            let items = list(a(0), "pop")?;
            items
                .last()
                .cloned()
                .ok_or_else(|| OpError::fault(Fault::Empty, "can't pop from an empty list"))
        }
        B::DropLast => {
            let items = list(a(0), "pop")?;
            let n = items.len().saturating_sub(1);
            Ok(Value::list(items[..n].to_vec()))
        }
        B::Prob => ops::make_prob(a(0)),
        B::BooleanLaw => ops::boolean_law(a(0)),
        B::Bernoulli | B::ScoreLaw => Ok(Dist::bernoulli(to_prob(a(0))?).into_value()),
        B::OneOf => one_of(a(0), budget),
        B::Binomial | B::Poisson | B::Geometric => {
            let counts = counts(b, args, budget)?.expect("called with plain arguments");
            Ok(counts.list(budget)?.into_value())
        }
        B::Bag => bag(a(0), budget),
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
        B::Mixture | B::Truncate | B::Bins => {
            Err(OpError::unsupported(format!("`{}` isn't implemented yet", b.name())))
        }
        B::Odds => {
            let p = to_prob(a(0))?;
            if p >= 1.0 {
                return Err(OpError::fault(
                    Fault::Domain,
                    "the odds of a certain event are infinite",
                ));
            }
            Ok(Value::Float(p / (1.0 - p)))
        }
        B::Logit => {
            let p = to_prob(a(0))?;
            if p <= 0.0 || p >= 1.0 {
                return Err(OpError::fault(
                    Fault::Domain,
                    "logit needs a probability strictly between 0% and 100%",
                ));
            }
            Ok(Value::Float(libm::log(p / (1.0 - p))))
        }
        B::InvLogit => ops::computed_prob(1.0 / (1.0 + crate::math::exp(-number(a(0), "inv_logit")?)), "inv_logit"),
        B::Date => {
            let date = match args.len() {
                1 => dates::parse(&text(a(0), "date")?),
                3 => dates::from_parts(
                    date_count(a(0), "date", budget)?,
                    date_count(a(1), "date", budget)?,
                    date_count(a(2), "date", budget)?,
                ),
                _ => {
                    return Err(OpError::new(
                        "`date` takes one ISO string or three integers (year, month, day)",
                    ));
                }
            };
            date.map(Value::Date).ok_or_else(|| {
                OpError::fault(
                    Fault::Conversion,
                    "invalid date; use YYYY-MM-DD within 0001-01-01..9999-12-31",
                )
            })
        }
        B::Days => to_int(a(0), f64::round),
        B::Weeks if matches!(a(0), Value::Int(_)) => {
            ops::binary(probl_syntax::ast::BinOp::Mul, a(0), &Value::Int(7.into()), budget)
        }
        B::Weeks => {
            let n = number(a(0), "weeks")?;
            to_int(&Value::Float(n * 7.0), f64::round)
        }
        B::AddWorkdays => {
            let d = date_value(a(0), b.name())?;
            let n = date_count(a(1), b.name(), budget)?;
            let holidays = holiday_calendar(args.get(2), b.name(), budget)?;
            date_result(dates::add_workdays_with_holidays(d, n, &holidays))
        }
        B::IsWorkday => {
            let d = date_value(a(0), b.name())?;
            let holidays = holiday_calendar(args.get(1), b.name(), budget)?;
            Ok(Value::Bool(
                dates::weekday(d) < 5 && holidays.binary_search(&d).is_err(),
            ))
        }
        B::AddMonths | B::AddYears => {
            let d = date_value(a(0), b.name())?;
            let n = date_count(a(1), b.name(), budget)?;
            date_result(if b == B::AddMonths {
                dates::add_months(d, n)
            } else {
                dates::add_years(d, n)
            })
        }
        B::StartOfMonth | B::EndOfMonth => {
            let d = date_value(a(0), b.name())?;
            date_result(if b == B::StartOfMonth {
                dates::start_of_month(d)
            } else {
                dates::end_of_month(d)
            })
        }
        B::Year | B::Month | B::Day => {
            let (y, m, d) = dates::to_civil(date_value(a(0), b.name())? as i64);
            Ok(Value::Int(match b {
                B::Year => y.into(),
                B::Month => m.into(),
                _ => d.into(),
            }))
        }
        B::Weekday => crate::text::value(
            dates::WEEKDAYS[dates::weekday(date_value(a(0), b.name())?) as usize],
            budget,
        ),
        B::IsFalse => Ok(Value::Bool(ops::is_certain(a(0), false))),
        B::IsTrue => Ok(Value::Bool(ops::is_certain(a(0), true))),
        B::IsListOfLen => Ok(Value::Bool(
            matches!((a(0), a(1)), (Value::List(items), Value::Int(n)) if *n == items.len() as i64),
        )),
        B::Minimum | B::Maximum => unreachable!("population extrema receive distributions whole"),
        B::Typeof | B::RunDate | B::Count | B::Map | B::Filter | B::Reduce | B::Print | B::Roll | B::Take => {
            unreachable!("`{}` is handled by the interpreter", b.name())
        }
        B::P
        | B::Mean
        | B::Sd
        | B::Variance
        | B::Median
        | B::MedianLow
        | B::MedianHigh
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

/// Statistical queries take an explicit population, never an implicit point
/// distribution. Also called before the interpreter's analytic-value guard so
/// a scalar has the same type error in enumeration and sampling.
pub fn check_query_input(b: Builtin, args: &[Value]) -> OpResult<()> {
    use Builtin as B;
    let expected = match b {
        B::Minimum | B::Maximum => "a distribution or nonempty list, range or string",
        B::P => "a boolean distribution (dist[bool])",
        B::Pdf => "a continuous distribution",
        B::Mean
        | B::Variance
        | B::Sd
        | B::Median
        | B::MedianLow
        | B::MedianHigh
        | B::Quantile
        | B::Cdf
        | B::Pmf
        | B::Support => "a distribution, nonempty list or nonempty range",
        _ => return Ok(()),
    };
    let v = &args[0];
    let valid = match b {
        B::Minimum | B::Maximum => matches!(
            v,
            Value::Dist(_) | Value::Continuous(_) | Value::List(_) | Value::Range(..) | Value::Str(_)
        ),
        B::P => matches!(v, Value::Dist(_)),
        B::Pdf => matches!(v, Value::Dist(_) | Value::Continuous(_)),
        _ => matches!(
            v,
            Value::Dist(_) | Value::Continuous(_) | Value::List(_) | Value::Range(..)
        ),
    };
    if !valid {
        let help = if b == B::P {
            "use `report event` to measure a fact across worlds, or `prob(event)` to convert a bool to 0 or 1"
        } else {
            "use `report x` to summarize values across worlds, or put the model inside `simulate { ... }` to obtain a distribution"
        };
        return Err(OpError::new(format!(
            "`{}` expects {expected}, found {}",
            b.name(),
            article(&v.kind())
        ))
        .help(help));
    }
    if !matches!(b, B::Minimum | B::Maximum) && matches!(v, Value::List(xs) if xs.is_empty()) {
        return Err(OpError::fault(
            Fault::Empty,
            format!("`{}` needs a nonempty list", b.name()),
        ));
    }
    Ok(())
}

/// Built-ins that receive distributions whole.
pub fn call_raw(b: Builtin, args: &[Value], budget: &mut Budget) -> OpResult<Value> {
    use Builtin as B;
    check_query_input(b, args)?;
    let v = &args[0];
    if let Value::Range(lo, hi) = v {
        if matches!(
            b,
            B::Mean
                | B::Variance
                | B::Sd
                | B::Median
                | B::MedianLow
                | B::MedianHigh
                | B::Quantile
                | B::Cdf
                | B::Pmf
                | B::Support
        ) {
            return range_query(b, lo, hi, args.get(1), budget);
        }
    }
    if matches!(b, B::P | B::Cdf | B::Pmf | B::Pdf) {
        if let Value::Dist(d) = v {
            if d.missing > 0.0 {
                return Err(OpError::new(format!("`{}` cannot return an exact scalar probability while the distribution has unresolved mass", b.name()))
                    .help("report a distribution comparison to retain probability bounds; scalar queries require a fully resolved distribution"));
            }
        }
    }
    if matches!(
        b,
        B::Mean
            | B::Variance
            | B::Sd
            | B::Median
            | B::MedianLow
            | B::MedianHigh
            | B::Quantile
            | B::Cdf
            | B::Pdf
            | B::Pmf
            | B::Support
    ) && continuous_parts(v)
    {
        if matches!(b, B::Median | B::MedianLow | B::MedianHigh) {
            let n = match v {
                Value::Dist(d) => d.outcomes.len(),
                _ => 1,
            };
            // Median bounds sort component supports before any CDF inversion.
            budget.work((n as u64).saturating_mul(n.max(1).ilog2() as u64 + 1))?;
        }
        return continuous_query(b, args);
    }
    match b {
        B::Minimum | B::Maximum => population_extreme(v, b == B::Maximum, None, budget),
        B::P => probability_of(v),
        B::Pdf => Err(OpError::new("pdf needs a continuous distribution")
            .help("for a distribution whose outcomes can be listed, use `pmf`")),
        B::Mean => {
            let d = stat_dist(v, b.name(), budget)?;
            if d.outcomes.iter().any(|(x, _)| matches!(x, Value::Date(_))) {
                return date_mean(v, &d);
            }
            if d.outcomes.iter().any(|(x, _)| matches!(x, Value::Complex(_))) {
                budget.work(d.outcomes.len() as u64)?;
                let mut sum = Complex::new(0.0, 0.0)?;
                let total = d.total();
                for (x, p) in &d.outcomes {
                    let x = complex_number(x, "mean")?;
                    sum = sum.plus(x.times(Complex::new(p / total, 0.0)?)?)?;
                }
                Ok(Value::Complex(sum))
            } else {
                check_numeric(&d, "mean")?;
                finite_float(d.mean().unwrap(), "mean")
            }
        }
        B::Variance => {
            numeric_dist(v, "variance", budget).and_then(|d| finite_float(d.variance().unwrap(), "variance"))
        }
        B::Sd => numeric_dist(v, "sd", budget).and_then(|d| finite_float(d.sd().unwrap(), "sd")),
        B::Median | B::MedianLow | B::MedianHigh => median(v, b, budget),
        B::Quantile => {
            let q = to_prob(&args[1])?;
            quantile(v, q, "quantile", budget)
        }
        B::Support => {
            let d = stat_dist(v, b.name(), budget)?;
            budget.collection(d.outcomes.len() as u128)?;
            Ok(Value::list(d.outcomes.iter().map(|(x, _)| x.clone()).collect()))
        }
        B::Cdf => {
            let d = stat_dist(v, b.name(), budget)?;
            let mut p = crate::stats::Sum::default();
            for (x, w) in &d.outcomes {
                if statistical_compare(x, &args[1])?.is_le() {
                    p.add(*w);
                }
            }
            ops::computed_prob(p.value() / d.total(), "cdf")
        }
        B::Pmf => {
            let d = stat_dist(v, b.name(), budget)?;
            let p = crate::stats::sum(d.outcomes.iter().filter(|(x, _)| *x == args[1]).map(|(_, w)| *w));
            ops::computed_prob(p / d.total(), "pmf")
        }
        B::IterItems => iter_items(v, budget),
        B::RepeatCount => {
            if v.is_uncertain() {
                return Err(OpError::new(format!("`repeat` needs a number, not a {}", v.kind()))
                    .help("draw a value first, like `let n ~ d6`, then `repeat n { … }`"));
            }
            let n = integer(v, "repeat count", budget)?;
            if n.is_negative() {
                return Err(OpError::fault(Fault::Domain, "`repeat` needs a count of 0 or more"));
            }
            Ok(Value::Int(n.into_owned()))
        }
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

/// `P(d)`: query an explicitly constructed boolean distribution.
fn probability_of(v: &Value) -> OpResult<Value> {
    match v {
        Value::Dist(d) if d.truth().is_some() => ops::computed_prob(d.truth().unwrap().0 / d.total(), "P"),
        _ => Err(OpError::new(format!(
            "P needs a boolean distribution (dist[bool]), found {}",
            article(&v.kind())
        ))
        .help("compare a distribution, like `P(d6 > 4)`; use `report event` for a fact across worlds")),
    }
}

/// The parameters of `binomial`, `poisson` or `geometric`, checked; `None`
/// for another built-in, or when an argument is a distribution.
pub fn counts(b: Builtin, args: &[Value], budget: &mut Budget) -> OpResult<Option<Counts>> {
    use Builtin as B;
    if args.iter().any(Value::is_uncertain) {
        return Ok(None);
    }
    Ok(Some(match b {
        B::Binomial => {
            let n = whole(&args[0], "binomial's number of trials", budget)?;
            if n < 0 {
                return Err(OpError::fault(
                    Fault::Domain,
                    "binomial needs a number of trials of 0 or more",
                ));
            }
            Counts::Binomial {
                n: n as u64,
                p: to_prob(&args[1])?,
            }
        }
        B::Poisson => {
            let rate = number(&args[0], "poisson")?;
            if rate < 0.0 || !rate.is_finite() {
                return Err(OpError::fault(Fault::Domain, "poisson needs a rate of 0 or more"));
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
                return Err(OpError::fault(
                    Fault::Domain,
                    "geometric needs a chance of success above 0%",
                ));
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
    // PMF is a typed-outcome query even when continuous components are mixed
    // in. Converting atoms to f64 here would collapse distinct outcomes.
    if b == B::Pmf {
        return match &args[0] {
            Value::Dist(d) => {
                let at = crate::stats::sum(
                    d.outcomes
                        .iter()
                        .filter(|(x, _)| !matches!(x, Value::Continuous(_)) && *x == args[1])
                        .map(|(_, p)| *p),
                );
                ops::computed_prob(at / d.total(), "pmf")
            }
            Value::Continuous(_) => ops::computed_prob(0.0, "pmf"),
            _ => unreachable!("checked continuous input"),
        };
    }
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
        B::Mean => finite_float(m.mean(), "mean"),
        B::Variance => finite_float(m.variance(), "variance"),
        B::Sd => finite_float(m.sd(), "sd"),
        B::Median | B::MedianLow | B::MedianHigh => {
            let (lo, hi) = m.median_bounds();
            finite_float(
                match b {
                    B::MedianLow => lo,
                    B::MedianHigh => hi,
                    _ => crate::stats::midpoint(lo, hi),
                },
                b.name(),
            )
        }
        B::Quantile => finite_float(m.quantile(to_prob(&args[1])?), "quantile"),
        B::Cdf => ops::computed_prob(m.cdf(number(&args[1], "cdf")?), "cdf"),
        B::Pdf => {
            let x = number(&args[1], "pdf")?;
            let mut density = 0.0;
            for (part, p) in &m.parts {
                match part {
                    Part::Continuous(f) => density += p * f.pdf(x),
                    Part::Analytic(a) => density += p * a.pdf(x),
                    Part::Point(_) => {
                        return Err(OpError::new(
                            "pdf needs a continuous distribution, without single values mixed in",
                        ));
                    }
                }
            }
            finite_float(density, "pdf")
        }
        _ => Err(OpError::new(format!(
            "`{}` needs a distribution whose outcomes can be listed, not a continuous one",
            b.name()
        ))),
    }
}

/// Unit-step integer ranges are uniform populations; scalar queries do not
/// allocate their support. Rank arithmetic remains exact for arbitrary integers.
fn range_query(b: Builtin, lo: &Integer, hi: &Integer, arg: Option<&Value>, budget: &mut Budget) -> OpResult<Value> {
    use Builtin as B;
    if hi < lo {
        return Err(OpError::fault(
            Fault::Empty,
            format!("`{}` needs a nonempty range", b.name()),
        ));
    }
    budget.integer_work(lo, hi, false)?;
    let n = range_len(lo, hi)?;
    budget.integer_allocation(n.bits(), 1)?;
    let two = Integer::from(2);
    match b {
        B::Support => Ok(Value::list(range_items(lo, hi, budget)?)),
        B::Mean => lo
            .add(hi)?
            .ratio(&two)
            .map(Value::Float)
            .ok_or_else(|| OpError::fault(Fault::Overflow, "`mean` gave a result that isn't a finite number")),
        B::Median | B::MedianLow | B::MedianHigh => {
            budget.integer_work(&n, &two, true)?;
            let (half, odd) = n.div_mod(&two)?;
            let high = Value::Int(lo.add(&half)?);
            let low = if odd.is_zero() {
                Value::Int(lo.add(&half)?.sub(&Integer::ONE)?)
            } else {
                high.clone()
            };
            match b {
                B::MedianLow => Ok(low),
                B::MedianHigh => Ok(high),
                _ => midpoint(&low, &high, budget),
            }
        }
        B::Sd | B::Variance => {
            let quarter = n.ratio(&Integer::from(4)).unwrap_or(f64::INFINITY);
            let inv = Integer::ONE.ratio(&n).unwrap_or(0.0);
            let sd = quarter * (4.0 / libm::sqrt(12.0)) * libm::sqrt((1.0 - inv) * (1.0 + inv));
            finite_float(if b == B::Sd { sd } else { sd * sd }, b.name())
        }
        B::Quantile => {
            let q = to_prob(arg.expect("quantile argument"))?;
            if q == 0.0 {
                return Ok(Value::Int(lo.clone()));
            }
            budget.work(n.bits().div_ceil(64).saturating_mul(17))?;
            let rank = if let Some(count) = n.to_u64().filter(|n| *n <= 1 << 53) {
                // Match the rounded cumulative weights of a finite uniform
                // population, including decimal boundaries such as 10% of 10.
                let mut weight = 1.0 / count as f64;
                let total = weight * count as f64;
                if total != 1.0 {
                    weight *= 1.0 / total;
                }
                let target = q * (weight * count as f64);
                let (mut left, mut right) = (1, count);
                while left < right {
                    budget.work(1)?;
                    let mid = left + (right - left) / 2;
                    if mid as f64 * weight >= target {
                        right = mid;
                    } else {
                        left = mid + 1;
                    }
                }
                Integer::from(left)
            } else {
                // Above exact f64 integer precision, keep the rank arithmetic
                // exact instead of rounding the population size to a float.
                n.probability_rank(q)?
            };
            Ok(Value::Int(lo.add(&rank.sub(&Integer::ONE)?)?))
        }
        B::Pmf => {
            let present = matches!(arg, Some(Value::Int(k)) if lo <= k && k <= hi);
            ops::computed_prob(
                if present {
                    Integer::ONE.ratio(&n).unwrap_or(0.0)
                } else {
                    0.0
                },
                "pmf",
            )
        }
        B::Cdf => {
            let x = arg.expect("cdf argument");
            if ops::compare(x, &Value::Int(lo.clone()))?.is_lt() {
                return ops::computed_prob(0.0, "cdf");
            }
            if ops::compare(x, &Value::Int(hi.clone()))?.is_ge() {
                return ops::computed_prob(1.0, "cdf");
            }
            let k = match x {
                Value::Int(k) => k.clone(),
                _ => Integer::from_f64(number(x, "cdf")?.floor()).expect("finite numeric bound"),
            };
            let count = k.sub(lo)?.add(&Integer::ONE)?;
            budget.integer_work(&count, &n, true)?;
            ops::computed_prob(count.ratio(&n).unwrap_or(0.0), "cdf")
        }
        _ => unreachable!("checked range query"),
    }
}

/// A list is an empirical distribution: each element has equal weight,
/// including repetitions. Its elements remain values, never implicit draws.
fn stat_dist<'a>(v: &'a Value, what: &str, budget: &mut Budget) -> OpResult<Cow<'a, Dist>> {
    match v {
        Value::Dist(d) => {
            budget.work(d.outcomes.len() as u64)?;
            Ok(Cow::Borrowed(d))
        }
        Value::List(xs) if !xs.is_empty() => {
            budget.collection(xs.len() as u128)?;
            let comparisons = (xs.len().ilog2() + 1) as u64;
            budget.work((xs.len() as u64).saturating_mul(comparisons))?;
            for x in xs.iter() {
                match x {
                    Value::Int(n) => {
                        budget.integer_work(n, &Integer::ONE, false)?;
                    }
                    Value::Str(s) => {
                        budget.string_work(s)?;
                    }
                    x if x.is_uncertain() => {
                        return Err(OpError::new(format!(
                            "`{what}` needs list elements that are values, found {}",
                            article(&x.kind())
                        ))
                        .help("draw the elements first, or explicitly build a mixture with `one_of`"));
                    }
                    _ => {}
                }
            }
            Ok(Cow::Owned(Dist::uniform(xs.to_vec())))
        }
        _ => Err(OpError::new(format!(
            "`{what}` needs a distribution or a nonempty list"
        ))),
    }
}

fn numeric_dist<'a>(v: &'a Value, what: &str, budget: &mut Budget) -> OpResult<Cow<'a, Dist>> {
    let d = stat_dist(v, what, budget)?;
    check_numeric(&d, what)?;
    Ok(d)
}

fn check_numeric(d: &Dist, what: &str) -> OpResult<()> {
    for (x, _) in &d.outcomes {
        if !matches!(x, Value::Int(_) | Value::Float(_) | Value::Prob(_)) {
            return Err(OpError::new(format!(
                "`{what}` needs real numeric elements, found {}",
                article(&x.kind())
            )));
        }
        if x.as_f64().is_none_or(|x| !x.is_finite()) {
            return Err(OpError::new(format!(
                "`{what}` needs numbers representable as finite floats"
            )));
        }
    }
    if d.outcomes.is_empty() {
        return Err(OpError::new(format!("`{what}` needs at least one resolved outcome")));
    }
    Ok(())
}

fn statistical_compare(a: &Value, b: &Value) -> OpResult<std::cmp::Ordering> {
    match (a, b) {
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        _ => ops::compare(a, b),
    }
}

/// Public ordering is separate from the typed total order used for storage.
/// Never put this reordered vector back into a Dist or a world/cache key.
fn ordered_outcomes(d: &Dist, budget: &mut Budget) -> OpResult<Vec<(Value, f64)>> {
    budget.collection(d.outcomes.len() as u128)?;
    for (x, _) in &d.outcomes {
        budget.work(1)?;
        statistical_compare(x, x)?;
    }
    crate::ordering::reserve_sort(d.outcomes.len(), budget)?;
    let mut outcomes = d.outcomes.clone();
    crate::ordering::try_sort_by(&mut outcomes, |(a, _), (b, _)| {
        budget.work(1)?;
        statistical_compare(a, b)
    })?;
    Ok(outcomes)
}

fn quantile(v: &Value, q: f64, what: &str, budget: &mut Budget) -> OpResult<Value> {
    let d = stat_dist(v, what, budget)?;
    let outcomes = ordered_outcomes(&d, budget)?;
    crate::stats::quantile(&outcomes, q)
        .cloned()
        .ok_or_else(|| OpError::new(format!("`{what}` needs at least one resolved outcome")))
}

fn median(v: &Value, b: Builtin, budget: &mut Budget) -> OpResult<Value> {
    let d = stat_dist(v, b.name(), budget)?;
    for (x, _) in &d.outcomes {
        budget.work(1)?;
        // Validate even singletons. Being sortable internally does not give a
        // record, complex number or recipe a mathematical ordering.
        statistical_compare(x, x)?;
        if b == Builtin::Median && !matches!(x, Value::Int(_) | Value::Float(_) | Value::Prob(_) | Value::Date(_)) {
            return Err(OpError::new(format!(
                "`median` needs real numeric or date elements, found {}",
                article(&x.kind())
            ))
            .help("use `median_low` or `median_high` for ordered values such as strings"));
        }
    }
    let outcomes = ordered_outcomes(&d, budget)?;
    let (lo, hi) = crate::stats::median_bounds(&outcomes)
        .ok_or_else(|| OpError::new(format!("`{}` needs at least one resolved outcome", b.name())))?;
    match b {
        Builtin::MedianLow => Ok(lo.clone()),
        Builtin::MedianHigh => Ok(hi.clone()),
        _ if lo == hi => Ok(lo.clone()),
        _ => midpoint(lo, hi, budget),
    }
}

fn date_mean(v: &Value, d: &Dist) -> OpResult<Value> {
    // Lists permit exact integer arithmetic, so rounding cannot depend on
    // repeated values, list order, or a date's distance from the epoch.
    if let Value::List(xs) = v {
        let total = xs
            .iter()
            .try_fold(0i128, |sum, x| Ok::<_, OpError>(sum + date_value(x, "mean")? as i128))?;
        let n = xs.len() as i128;
        let day = total.div_euclid(n) + i128::from(2 * total.rem_euclid(n) > n);
        return Ok(Value::Date(day as i32));
    }
    let origin = date_value(&d.outcomes[0].0, "mean")?;
    let mut offsets = crate::stats::Sum::default();
    let mut weights = crate::stats::Sum::default();
    for (x, w) in &d.outcomes {
        offsets.add((date_value(x, "mean")? - origin) as f64 * w);
        weights.add(*w);
    }
    let days = offsets.value() / weights.value();
    let floor = days.floor();
    let tie_error = 4.0 * f64::EPSILON * days.abs().max(1.0);
    let rounded = floor + f64::from(days - floor > 0.5 + tie_error);
    date_result(
        i32::try_from(origin as i64 + rounded as i64)
            .ok()
            .filter(|d| dates::valid(*d)),
    )
}

/// Preserve exact integral midpoints, including bigints beyond float range.
pub(crate) fn midpoint(a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    if let (Value::Date(a), Value::Date(b)) = (a, b) {
        return Ok(Value::Date(((*a as i64 + *b as i64).div_euclid(2)) as i32));
    }
    if let (Value::Int(a), Value::Int(b)) = (a, b) {
        budget.integer_work(a, b, false)?;
        let sum = a.add(b)?;
        budget.integer_allocation(sum.bits(), 1)?;
        let two = Integer::from(2);
        budget.integer_work(&sum, &two, true)?;
        let (whole, remainder) = sum.div_mod(&two)?;
        if remainder.is_zero() {
            return Ok(Value::Int(whole));
        }
        return sum.ratio(&two).map(Value::Float).ok_or_else(|| {
            OpError::fault(
                Fault::Overflow,
                "median's fractional midpoint is too large for a finite float",
            )
        });
    }
    let (a, b) = (number(a, "median")?, number(b, "median")?);
    // Same-sign subtraction and opposite-sign addition avoid overflow; this
    // also preserves equal subnormal values instead of halving both to zero.
    finite_float(crate::stats::midpoint(a, b), "median")
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
        Value::Str(s) => crate::text::chars(s, budget).map(Value::list),
        v if v.is_uncertain() => Err(OpError::new(format!("can't loop over a {}", v.kind()))
            .help("draw a value first with `~`, or loop over `support(…)`")),
        other => Err(OpError::new(format!("can't loop over {}", article(&other.kind())))),
    }
}

fn date_count(v: &Value, func: &str, budget: &mut Budget) -> OpResult<i64> {
    integer(v, func, budget)?
        .to_i64()
        .ok_or_else(|| OpError::fault(Fault::Overflow, "date out of range"))
}

fn date_value(v: &Value, func: &str) -> OpResult<i32> {
    match v {
        Value::Date(d) if dates::valid(*d) => Ok(*d),
        Value::Date(_) => Err(OpError::fault(Fault::Overflow, "date out of range")),
        _ => Err(expected("a date", v, func)),
    }
}

fn date_result(date: Option<i32>) -> OpResult<Value> {
    date.map(Value::Date)
        .ok_or_else(|| OpError::fault(Fault::Overflow, "date out of range (0001-01-01..9999-12-31)"))
}

fn holiday_calendar(v: Option<&Value>, func: &str, budget: &mut Budget) -> OpResult<Vec<i32>> {
    let Some(v) = v else {
        return Ok(Vec::new());
    };
    let values = list(v, func)?;
    budget.collection(values.len() as u128)?;
    let log = (values.len() as u64).checked_ilog2().unwrap_or(0) as u64 + 1;
    budget.work((values.len() as u64).saturating_mul(log))?;
    let mut holidays = Vec::new();
    for value in values.iter() {
        let d = date_value(value, func)?;
        if dates::weekday(d) < 5 {
            holidays.push(d);
        }
    }
    holidays.sort_unstable();
    holidays.dedup();
    Ok(holidays)
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
        Value::Int(n) => n.to_f64().ok_or_else(|| {
            OpError::fault(
                Fault::Overflow,
                format!("`{func}` needs an integer that fits in a finite float"),
            )
        }),
        _ => v.as_f64().ok_or_else(|| expected("a number", v, func)),
    }
}

fn complex_number(v: &Value, func: &str) -> OpResult<Complex> {
    v.as_complex()
        .ok_or_else(|| expected("a finite real or complex number", v, func))
}

fn finite_float(x: f64, func: &str) -> OpResult<Value> {
    if x.is_finite() {
        Ok(Value::Float(x))
    } else {
        Err(OpError::new(format!(
            "`{func}` gave a result that isn't a finite number"
        )))
    }
}

fn whole(v: &Value, what: &str, budget: &mut Budget) -> OpResult<i64> {
    integer(v, what, budget)?
        .to_i64()
        .ok_or_else(|| OpError::new(format!("{what} is outside the supported count range")))
}

fn nonnegative_int<'a>(v: &'a Value, func: &str, budget: &mut Budget) -> OpResult<Cow<'a, Integer>> {
    let n = integer(v, func, budget)?;
    if n.is_negative() {
        Err(OpError::fault(
            Fault::Domain,
            format!("`{func}` needs nonnegative integers"),
        ))
    } else {
        Ok(n)
    }
}

fn choose(n: &Integer, k: &Integer, budget: &mut Budget) -> OpResult<Value> {
    if k > n {
        return Ok(Value::Int(Integer::ZERO));
    }
    let k = k.min(&n.sub(k)?).clone();
    let steps = k
        .to_u64()
        .ok_or_else(|| OpError::limit("choose needs too many iterations"))?;
    budget.work(steps)?;
    let offset = n.sub(&k)?;
    let mut result = Integer::ONE;
    for i in 1..=steps {
        let numerator = offset.add(&i.into())?;
        let divisor = Integer::from(i);
        // Cancel first: an intermediate product must not exceed the integer
        // ceiling when the final binomial coefficient fits.
        let d = gcd(numerator.clone(), divisor.clone(), budget)?;
        let numerator = numerator.div_mod(&d)?.0;
        let divisor = divisor.div_mod(&d)?.0;
        budget.integer_work(&result, &divisor, true)?;
        result = result.div_mod(&divisor)?.0;
        budget.integer_work(&result, &numerator, true)?;
        result = result.mul(&numerator)?;
        budget.integer_bits(result.bits())?;
    }
    Ok(Value::Int(result))
}

fn factorial(n: &Integer, budget: &mut Budget) -> OpResult<Value> {
    let n = n
        .to_u64()
        .ok_or_else(|| OpError::limit("factorial exceeds the integer size limit"))?;
    // n! contains at least n/2 factors of n/2 or more.
    let half = n / 2;
    let lower_bits = half.saturating_mul(63u64.saturating_sub(u64::from(half.leading_zeros())));
    budget.integer_bits(lower_bits)?;
    budget.work(n)?;
    let mut result = Integer::ONE;
    for i in 2..=n {
        let factor = Integer::from(i);
        budget.integer_work(&result, &factor, true)?;
        result = result.mul(&factor)?;
        budget.integer_bits(result.bits())?;
    }
    Ok(Value::Int(result))
}

fn gcd(mut a: Integer, mut b: Integer, budget: &mut Budget) -> OpResult<Integer> {
    while !b.is_zero() {
        budget.integer_work(&a, &b, true)?;
        let r = a.div_mod(&b)?.1;
        a = b;
        b = r;
    }
    Ok(a)
}

fn euler_phi(n: &Integer, budget: &mut Budget) -> OpResult<Value> {
    if n.is_zero() {
        return Err(OpError::fault(Fault::Domain, "`euler_phi` needs a positive integer"));
    }
    let mut n = n.clone();
    let mut result = n.clone();
    let mut divisor = Integer::from(2);
    loop {
        budget.integer_work(&n, &divisor, true)?;
        let (q, rem) = n.div_mod(&divisor)?;
        if divisor > q {
            break;
        }
        if rem.is_zero() {
            budget.integer_work(&result, &divisor, true)?;
            result = result.sub(&result.div_mod(&divisor)?.0)?;
            n = q;
            loop {
                budget.integer_work(&n, &divisor, true)?;
                let (q, rem) = n.div_mod(&divisor)?;
                if !rem.is_zero() {
                    break;
                }
                n = q;
            }
        }
        divisor = divisor.add(&if divisor == 2 { Integer::ONE } else { 2.into() })?;
    }
    if n > 1 {
        budget.integer_work(&result, &n, true)?;
        result = result.sub(&result.div_mod(&n)?.0)?;
    }
    Ok(Value::Int(result))
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

fn range_items(lo: &Integer, hi: &Integer, budget: &mut Budget) -> OpResult<Vec<Value>> {
    let n = range_count(lo, hi)?;
    budget.collection(n)?;
    let n = usize::try_from(n).map_err(|_| OpError::limit("range has too many elements"))?;
    budget.integer_allocation(lo.bits().max(hi.bits()), n as u64)?;
    budget.work((n as u64).saturating_mul(lo.bits().max(hi.bits()).div_ceil(64).max(1)))?;
    (0..n)
        .map(|i| lo.add(&Integer::from(i)).map(Value::Int).map_err(OpError::from))
        .collect()
}

/// The elements of a list-like value, checking that a range isn't too long
/// to spell out.
pub fn items(v: &Value, func: &str, budget: &mut Budget) -> OpResult<Vec<Value>> {
    match v {
        Value::List(items) => {
            budget.collection(items.len() as u128)?;
            budget.work(items.len() as u64)?;
            Ok(items.to_vec())
        }
        Value::Range(lo, hi) => range_items(lo, hi, budget),
        Value::Str(s) => crate::text::chars(s, budget),
        other => Err(expected("a list, range or string", other, func)),
    }
}

fn sort(items: &mut [Value], descending: bool, budget: &mut Budget) -> OpResult<()> {
    // Even a singleton must have a language ordering; internal storage order
    // doesn't make complex values, records or distributions sortable.
    for x in items.iter() {
        budget.work(1)?;
        ops::compare(x, x)?;
    }
    crate::ordering::reserve_sort(items.len(), budget)?;
    crate::ordering::try_sort_by(items, |x, y| {
        budget.work(1)?;
        ops::compare(x, y).map(|c| if descending { c.reverse() } else { c })
    })
}

/// Real inputs keep their real domains; an explicit complex input requests the
/// principal complex extension. Distribution lifting happens in the interpreter.
fn elementary1(
    v: &Value,
    func: &str,
    complex: fn(Complex) -> OpResult<Complex>,
    real: impl Fn(f64) -> Option<f64>,
) -> OpResult<Value> {
    match v {
        Value::Complex(z) => complex(*z).map(Value::Complex).map_err(|_| {
            OpError::fault(
                Fault::Domain,
                format!("`{func}` isn't defined for {v} or its result isn't finite"),
            )
        }),
        _ => float1(v, func, real),
    }
}

fn float1(v: &Value, func: &str, f: impl Fn(f64) -> Option<f64>) -> OpResult<Value> {
    let x = number(v, func)?;
    f(x).filter(|y| x.is_finite() && y.is_finite())
        .map(Value::Float)
        .ok_or_else(|| OpError::fault(Fault::Domain, format!("`{func}` isn't defined for {}", fmt_float(x))))
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
        Value::Int(i) => Ok(Value::Int(i.clone())),
        other => {
            let x = number(other, "rounding")?;
            let r = f(x);
            Integer::from_f64(r).map(Value::Int).ok_or_else(|| {
                OpError::fault(
                    Fault::Overflow,
                    format!("{} cannot be rounded to a finite int", fmt_float(x)),
                )
            })
        }
    }
}

fn round(v: &Value, digits: Option<&Value>, budget: &mut Budget) -> OpResult<Value> {
    let Some(digits) = digits else {
        return to_int(v, f64::round);
    };
    let digits = integer(digits, "round's digits", budget)?;
    if let Value::Int(n) = v {
        if *digits >= 0 {
            return Ok(v.clone());
        }
        let places = digits.abs();
        if places > Integer::from(probl_number::MAX_INTEGER_DIGITS) {
            return Ok(Value::Int(Integer::ZERO));
        }
        budget.integer_work(n, n, true)?;
        return Ok(Value::Int(n.round_decimal(places.to_u64().unwrap() as u32)?));
    }
    // Once outside this interval the exact magnitude of digits is irrelevant.
    let digits = if *digits > 323 {
        324
    } else if *digits < -308 {
        -309
    } else {
        digits.to_i64().unwrap()
    };
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
    let mut best: Option<Value> = None;
    for v in args {
        // Validate candidate outcomes, then lift each comparison. Reducing incrementally avoids a Cartesian product of
        // every candidate and preserves missing mass without drawing.
        let v = ops::lift1(v, budget, |v, budget| {
            budget.work(1)?;
            ops::compare(v, v)?;
            Ok(v.clone())
        })?;
        best = Some(match best {
            None => v,
            Some(b) => ops::lift2(&b, &v, budget, |b, v, _| {
                let ord = ops::compare(v, b)?;
                if (want_max && ord.is_gt()) || (!want_max && ord.is_lt()) {
                    Ok(v.clone())
                } else {
                    Ok(b.clone())
                }
            })?,
        });
    }
    best.ok_or_else(|| OpError::fault(Fault::Empty, format!("`{name}` of an empty list")))
}

fn slice(v: &Value, start: &Value, end: Option<&Value>, budget: &mut Budget) -> OpResult<Value> {
    let start = integer(start, "slice", budget)?;
    let end = end.map(|v| integer(v, "slice", budget)).transpose()?;
    let length: Integer = match v {
        Value::List(xs) => xs.len().into(),
        Value::Str(s) => s.chars().count().into(),
        Value::Range(lo, hi) => {
            budget.integer_work(lo, hi, false)?;
            range_len(lo, hi)?
        }
        other => return Err(expected("a list, range or string", other, "slice")),
    };
    let end = end.as_deref().unwrap_or(&length);
    let start = start.as_ref();
    if start.is_negative() || start > end || end > &length {
        return Err(OpError::fault(
            Fault::Index,
            "`slice` needs 0 <= start <= end <= length",
        ));
    }
    Ok(match v {
        Value::Range(lo, _) => {
            // Empty slices have a canonical empty range. Subtract one from the
            // offset before adding, so a slice ending at the largest int works.
            let (first, last) = if start == end {
                (Integer::ZERO, (-1).into())
            } else {
                (lo.add(start)?, lo.add(&end.sub(&Integer::ONE)?)?)
            };
            budget.integer_allocation(first.bits(), 1)?;
            budget.integer_allocation(last.bits(), 1)?;
            Value::Range(first, last)
        }
        Value::List(xs) => {
            let (start, end) = (start.to_u64().unwrap() as usize, end.to_u64().unwrap() as usize);
            budget.collection((end - start) as u128)?;
            budget.work((end - start) as u64)?;
            Value::list(xs[start..end].to_vec())
        }
        Value::Str(s) => {
            let (start, end) = (start.to_u64().unwrap() as usize, end.to_u64().unwrap() as usize);
            let boundary = |n| s.char_indices().nth(n).map_or(s.len(), |(i, _)| i);
            crate::text::value(&s[boundary(start)..boundary(end)], budget)?
        }
        _ => unreachable!(),
    })
}

fn len(v: &Value) -> OpResult<Value> {
    let n: u128 = match v {
        Value::List(items) => items.len() as u128,
        Value::Str(s) => s.chars().count() as u128,
        Value::Map(m) => m.len() as u128,
        Value::Bag(b) => b.values().map(|n| *n as u128).sum(),
        Value::Range(lo, hi) => return Ok(Value::Int(range_len(lo, hi)?)),
        other => return Err(expected("a collection", other, "len")),
    };
    Ok(Value::Int(n.into()))
}

fn sum(v: &Value, budget: &mut Budget) -> OpResult<Value> {
    let items = items(v, "sum", budget)?;
    let mut acc = Value::Int(0.into());
    for x in &items {
        acc = ops::binary(probl_syntax::ast::BinOp::Add, &acc, x, budget)?;
    }
    Ok(acc)
}

fn get(coll: &Value, key: &Value, default: Option<&Value>, budget: &mut Budget) -> OpResult<Value> {
    let found = match coll {
        Value::Map(m) => m.get(key).cloned(),
        Value::List(items) => integer(key, "index", budget)?
            .to_u64()
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| items.get(n))
            .cloned(),
        Value::Str(s) => {
            let index = integer(key, "index", budget)?
                .to_u64()
                .and_then(|n| usize::try_from(n).ok());
            budget.string_work(s)?;
            index
                .and_then(|n| s.chars().nth(n))
                .map(|c| crate::text::value(c.encode_utf8(&mut [0; 4]), budget))
                .transpose()?
        }
        Value::Range(lo, hi) => {
            let index = integer(key, "index", budget)?;
            budget.integer_work(lo, hi, false)?;
            let len = range_len(lo, hi)?;
            if index.is_negative() || *index >= len {
                None
            } else {
                Some(Value::Int(lo.add(&index)?))
            }
        }
        Value::Bag(b) => Some(Value::Int(b.get(key).copied().unwrap_or(0).into())),
        other => return Err(expected("a map, bag or sequence", other, "get")),
    };
    match (found, default) {
        (Some(v), _) => Ok(v),
        (None, Some(d)) => Ok(d.clone()),
        (None, None) => Err(OpError::fault(Fault::Index, format!("the key {key:?} isn't there"))
            .help("give a default: `get(key, default)`")),
    }
}

/// Select an existing element; recipes inside collections are never lifted.
pub fn population_extreme(v: &Value, want_max: bool, default: Option<&Value>, budget: &mut Budget) -> OpResult<Value> {
    let name = if want_max { "maximum" } else { "minimum" };
    match v {
        Value::Range(lo, hi) => {
            budget.integer_work(lo, hi, false)?;
            if hi < lo {
                return empty_extreme(name, default);
            }
            Ok(Value::Int(if want_max { hi.clone() } else { lo.clone() }))
        }
        Value::Continuous(f) => {
            let (lo, hi) = f.support();
            finite_bound(if want_max { hi } else { lo }, name)
        }
        Value::Dist(d) => {
            if d.missing > 0.0 {
                return Err(OpError::new(format!(
                    "`{name}` cannot determine a support bound while the distribution has unresolved mass"
                )));
            }
            let mut best = None;
            for (x, p) in &d.outcomes {
                if *p <= 0.0 {
                    continue;
                }
                let x = match x {
                    Value::Continuous(_) | Value::Dist(_) => population_extreme(x, want_max, None, budget)?,
                    x => x.clone(),
                };
                select_extreme(&mut best, x, want_max, budget)?;
            }
            best.ok_or_else(|| OpError::new(format!("`{name}` needs a nonempty distribution")))
        }
        Value::List(_) | Value::Str(_) => {
            let mut best = None;
            for x in items(v, name, budget)? {
                select_extreme(&mut best, x, want_max, budget)?;
            }
            best.map_or_else(|| empty_extreme(name, default), Ok)
        }
        other => Err(expected(
            "a distribution or nonempty list, range or string",
            other,
            name,
        )),
    }
}

pub fn empty_extreme(name: &str, default: Option<&Value>) -> OpResult<Value> {
    default.cloned().ok_or_else(|| {
        OpError::fault(
            Fault::Empty,
            format!("`{name}` needs a nonempty collection or a default"),
        )
        .help(format!("use `{name}(xs, default: value)` to handle empty collections"))
    })
}

fn finite_bound(x: f64, name: &str) -> OpResult<Value> {
    if !x.is_finite() {
        return Err(OpError::new(format!("`{name}` has no finite support bound")));
    }
    Ok(Value::Float(x))
}

fn select_extreme(best: &mut Option<Value>, x: Value, want_max: bool, budget: &mut Budget) -> OpResult<()> {
    budget.work(1)?;
    ops::compare(&x, &x)?;
    let replace = match best {
        None => true,
        Some(b) => {
            let ord = ops::compare(&x, b)?;
            if want_max { ord.is_gt() } else { ord.is_lt() }
        }
    };
    if replace {
        *best = Some(x);
    }
    Ok(())
}

pub fn extreme_count(n: &Value, budget: &mut Budget) -> OpResult<usize> {
    let n = integer(n, "the count", budget)?;
    if n.is_negative() {
        return Err(OpError::fault(Fault::Domain, "the count must be nonnegative"));
    }
    Ok(n.to_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(usize::MAX))
}

fn extremes(v: &Value, n: &Value, highest: bool, budget: &mut Budget) -> OpResult<Value> {
    let n = extreme_count(n, budget)?;
    let mut items = items(v, if highest { "highest" } else { "lowest" }, budget)?;
    sort(&mut items, highest, budget)?;
    items.truncate(n);
    Ok(Value::list(items))
}

fn insert(coll: &Value, a: &Value, b: &Value, budget: &mut Budget) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            budget.collection(items.len() as u128 + 1)?;
            let mut items = items.to_vec();
            let i = as_index(a, items.len() as u128 + 1, budget)? as usize;
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

fn remove(coll: &Value, key: &Value, budget: &mut Budget) -> OpResult<Value> {
    match coll {
        Value::List(items) => {
            let mut items = items.to_vec();
            let i = as_index(key, items.len() as u128, budget)? as usize;
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
            None => Err(OpError::fault(Fault::Index, format!("{key:?} isn't in the bag"))),
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
            budget.outcomes(range_count(lo, hi)?)?;
            Ok(Dist::uniform(range_items(lo, hi, budget)?).into_value())
        }
        Value::Map(m) if !m.is_empty() => {
            let all_probs = m.values().all(|w| matches!(w, Value::Prob(_)));
            if !all_probs && m.values().any(|w| matches!(w, Value::Prob(_))) {
                return Err(
                    OpError::new("one_of can't mix probabilities and relative numeric weights")
                        .help("use prob(...) for every absolute probability, or numbers for every relative weight"),
                );
            }
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
            let largest = pairs.iter().map(|(_, w)| *w).fold(0.0, f64::max);
            if largest == 0.0 {
                return Err(OpError::new("one_of needs at least one positive weight"));
            }
            // Absolute probabilities are checked before normalization. Relative
            // weights may have an unrepresentable sum even though each is valid.
            let total = crate::stats::sum(pairs.iter().map(|(_, w)| *w));
            if all_probs && (total - 1.0).abs() > 1e-9 {
                return Err(OpError::new(format!(
                    "the chances add up to {}, not 100%",
                    crate::value::fmt_prob(total)
                ))
                .help("use plain numbers for relative weights, like [\"a\": 3, \"b\": 1]"));
            }
            for (_, w) in &mut pairs {
                *w /= largest;
            }
            let total = crate::stats::sum(pairs.iter().map(|(_, w)| *w));
            ops::combine(pairs.into_iter().map(|(k, w)| (k, w / total)).collect(), 0.0, budget)
        }
        Value::Bag(b) => {
            let total: u128 = b.values().map(|n| *n as u128).sum();
            if total == 0 {
                return Err(OpError::fault(Fault::Empty, "the bag is empty"));
            }
            let pairs = b.iter().map(|(k, n)| (k.clone(), *n as f64 / total as f64)).collect();
            ops::combine(pairs, 0.0, budget)
        }
        Value::List(_) | Value::Range(..) | Value::Map(_) => {
            Err(OpError::fault(Fault::Empty, "one_of needs at least one option"))
        }
        other => Err(expected("a list, range, map or bag", other, "one_of")),
    }
}

fn bag(v: &Value, budget: &mut Budget) -> OpResult<Value> {
    let mut counts = BTreeMap::new();
    match v {
        Value::Map(m) => {
            for (k, n) in m.iter() {
                let n = integer(n, "bag count", budget)?;
                if n.is_negative() {
                    return Err(OpError::new("bag counts must be whole numbers of 0 or more"));
                }
                let n = n
                    .to_u64()
                    .ok_or_else(|| OpError::new("bag count exceeds the supported count range"))?;
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
