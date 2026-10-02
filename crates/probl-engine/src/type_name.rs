//! Runtime type descriptions, without drawing or lifting over distributions.
//! These describe observed values, not inferred static types or annotations.

use crate::dist::Budget;
use crate::error::{OpError, OpResult};
use crate::text;
use crate::value::Value;
use probl_sema::ir::EnumType;

const MAX_DEPTH: usize = 64;

pub fn of(v: &Value, enums: &[EnumType], budget: &mut Budget) -> OpResult<Value> {
    let name = Names { enums, budget }.describe(v, 0)?;
    Ok(Value::str(&name))
}

struct Names<'a> {
    enums: &'a [EnumType],
    budget: &'a mut Budget,
}

impl Names<'_> {
    fn name(&mut self, name: &str) -> OpResult<String> {
        let mut out = String::new();
        text::push(&mut out, name, self.budget)?;
        Ok(out)
    }

    fn parameterized(&mut self, name: &str, args: &[String]) -> OpResult<String> {
        let mut out = self.name(name)?;
        text::push(&mut out, "[", self.budget)?;
        for (i, arg) in args.iter().enumerate() {
            if i != 0 {
                text::push(&mut out, ", ", self.budget)?;
            }
            text::push(&mut out, arg, self.budget)?;
        }
        text::push(&mut out, "]", self.budget)?;
        Ok(out)
    }

    /// No numeric promotion: heterogeneous runtime types give `any`.
    /// Empty collections have no observed element type, hence `unknown`.
    fn common<'v>(
        &mut self,
        values: impl Iterator<Item = &'v Value>,
        depth: usize,
        outcomes: bool,
    ) -> OpResult<String> {
        let mut found: Option<String> = None;
        for value in values {
            // A continuous component of a finite mixture yields a float,
            // rather than another distribution, when the mixture is drawn.
            let name = if outcomes && matches!(value, Value::Continuous(_)) {
                self.budget.work(1)?;
                self.name("float")?
            } else {
                self.describe(value, depth)?
            };
            match &found {
                Some(first) if *first != name => return self.name("any"),
                Some(_) => {}
                None => found = Some(name),
            }
        }
        found.map(Ok).unwrap_or_else(|| self.name("unknown"))
    }

    fn describe(&mut self, v: &Value, depth: usize) -> OpResult<String> {
        self.budget.work(1)?;
        if depth > MAX_DEPTH {
            return Err(OpError::limit("typeof value nesting exceeds the limit of 64"));
        }
        let simple = match v {
            Value::Dead => return Err(OpError::new("typeof needs an initialized value")),
            Value::Unit => "()",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) | Value::Delayed(_) => "float",
            Value::Prob(_) => "prob",
            Value::Complex(_) => "complex",
            Value::Str(_) => "str",
            Value::Date(_) => "date",
            Value::Range(..) => "range",
            Value::Closure(_) => "fn",
            Value::Continuous(_) => "dist[float]",
            Value::Enum(e) => &self.enums[e.ty as usize].name,
            Value::Record(r) if r.ty.is_some() => r.ty.as_deref().unwrap(),
            Value::List(items) => {
                let item = self.common(items.iter(), depth + 1, false)?;
                return self.parameterized("list", &[item]);
            }
            Value::Bag(items) => {
                let item = self.common(items.keys(), depth + 1, false)?;
                return self.parameterized("bag", &[item]);
            }
            Value::Map(items) => {
                let key = self.common(items.keys(), depth + 1, false)?;
                let value = self.common(items.values(), depth + 1, false)?;
                return self.parameterized("map", &[key, value]);
            }
            Value::Dist(d) => {
                let item = self.common(d.outcomes.iter().map(|(v, _)| v), depth + 1, true)?;
                return self.parameterized("dist", &[item]);
            }
            Value::Record(r) => {
                let mut out = self.name("{")?;
                for (i, (name, value)) in r.fields.iter().enumerate() {
                    if i != 0 {
                        text::push(&mut out, ", ", self.budget)?;
                    }
                    text::push(&mut out, name, self.budget)?;
                    text::push(&mut out, ": ", self.budget)?;
                    let ty = self.describe(value, depth + 1)?;
                    text::push(&mut out, &ty, self.budget)?;
                }
                text::push(&mut out, "}", self.budget)?;
                return Ok(out);
            }
        };
        self.name(simple)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_respects_work_and_string_limits() {
        let mut budget = Budget::unlimited();
        budget.work_left = 10;
        let list = Value::list(vec![Value::Bool(true); 100]);
        assert!(of(&list, &[], &mut budget).is_err());

        let mut budget = Budget::unlimited();
        budget.max_string_bytes = 5;
        assert!(of(&Value::list(vec![Value::Bool(true)]), &[], &mut budget).is_err());
    }
}
