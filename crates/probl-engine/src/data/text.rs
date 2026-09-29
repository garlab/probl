//! A single value from text: a CSV cell, a line, or a JSON string read as a
//! date, an enum, a percentage or a map key (docs/data-input.md, "Plain
//! values"). Text is kept as written; other values may have spaces around
//! them.

use super::{Problem, quoted};
use crate::ops;
use crate::value::Value;
use probl_sema::ir::{Program, TypeSpec};

pub(crate) fn plain(
    text: &str,
    ty: &TypeSpec,
    program: &Program,
    budget: &mut super::Budget,
) -> Result<Value, Problem> {
    if *ty == TypeSpec::Str {
        budget.max_string_bytes_seen = budget.max_string_bytes_seen.max(text.len());
        return Ok(Value::str(text));
    }
    let t = text.trim();
    if t.is_empty() {
        return Err(Problem::new("the value is missing")
            .help("the language has no missing values yet: only a `str` field can be empty"));
    }
    match ty {
        TypeSpec::Int => int(t, budget),
        TypeSpec::Float => float(t),
        TypeSpec::Prob => prob(t),
        TypeSpec::Bool => match t.to_ascii_lowercase().as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(Problem::new(format!("{} isn't a bool", quoted(t))).help("write `true` or `false`")),
        },
        TypeSpec::Date => match crate::dates::parse(t) {
            Some(d) => Ok(Value::Date(d)),
            None => Err(Problem::new(format!("{} isn't a date", quoted(t))).help("write dates like 2027-01-31")),
        },
        TypeSpec::Enum(e) => {
            let decl = &program.enums[*e as usize];
            match decl.variants.iter().position(|v| v == t) {
                Some(i) => Ok(ops::enum_value(*e, i as u32, t)),
                None => Err(Problem::new(format!("{} isn't a `{}`", quoted(t), decl.name))
                    .help(format!("it's one of {}", decl.variants.join(", ")))),
            }
        }
        other => Err(Problem::new(format!(
            "a `{}` can't be read from text",
            other.describe(program)
        ))),
    }
}

fn int(t: &str, budget: &mut super::Budget) -> Result<Value, Problem> {
    match t.parse::<probl_number::Integer>() {
        Ok(n) => {
            budget.integer(n.bits())?;
            Ok(Value::Int(n))
        }
        Err(probl_number::IntError::TooLarge) => Err(Problem::limit(format!(
            "integer size exceeds the limit of {} bits",
            budget.limits.max_integer_bits.min(probl_number::MAX_INTEGER_BITS)
        ))),
        Err(_) if t.parse::<f64>().is_ok_and(f64::is_finite) => {
            Err(Problem::new(format!("{} isn't a whole number", quoted(t))).help("declare the field as a `float`"))
        }
        Err(_) => Err(Problem::new(format!("{} isn't an int", quoted(t)))),
    }
}

fn float(t: &str) -> Result<Value, Problem> {
    match t.parse::<f64>() {
        Ok(x) if x.is_finite() => Ok(Value::Float(x)),
        Ok(_) if t.bytes().any(|b| b.is_ascii_digit()) => {
            Err(Problem::new(format!("{} is too large for a float", quoted(t))))
        }
        _ => Err(Problem::new(format!("{} isn't a number", quoted(t)))),
    }
}

fn prob(t: &str) -> Result<Value, Problem> {
    let number = |s: &str| s.trim().parse::<f64>().ok().filter(|x| x.is_finite());
    if let Some(p) = t.strip_suffix('%') {
        return match number(p) {
            Some(x) if (0.0..=100.0).contains(&x) => Ok(Value::Prob(x / 100.0)),
            Some(_) => Err(Problem::new(format!("{} isn't between 0% and 100%", quoted(t)))),
            None => Err(Problem::new(format!("{} isn't a percentage", quoted(t)))),
        };
    }
    match number(t) {
        Some(x) if (0.0..=1.0).contains(&x) => Ok(Value::Prob(x)),
        Some(x) if x > 1.0 && x <= 100.0 => {
            Err(Problem::new(format!("{} isn't between 0 and 1", quoted(t))).help(format!("did you mean `{t}%`?")))
        }
        Some(_) => Err(Problem::new(format!("{} isn't between 0 and 1", quoted(t)))),
        None => Err(Problem::new(format!("{} isn't a probability", quoted(t))).help("write it like 30% or 0.3")),
    }
}
