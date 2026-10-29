//! Runtime type descriptions, without drawing or lifting over distributions.
//! Collection types merge structurally; declarations are not widened this way.
use crate::dist::Budget;
use crate::error::{OpError, OpResult};
use crate::text;
use crate::value::{Hashed, Record, Value};
use probl_sema::ir::EnumType;
use std::sync::Arc;

const MAX_DEPTH: usize = 64;

enum Description {
    Unknown,
    Atom(String),
    Container(&'static str, Vec<Self>),
    Record(Vec<(String, Self)>),
    // Keep a named record opaque until an anonymous record needs its shape.
    NamedRecord(Vec<Arc<Hashed<Record>>>),
    Union(Vec<Self>),
}

enum Merge {
    Joined(Description),
    Separate(Description, Description),
}

pub fn of(v: &Value, enums: &[EnumType], budget: &mut Budget) -> OpResult<Value> {
    let mut names = Names { enums, budget };
    let ty = names.describe(v, 0)?;
    let mut name = String::new();
    names.render(&ty, &mut name)?;
    Ok(Value::str(&name))
}

struct Names<'a> {
    enums: &'a [EnumType],
    budget: &'a mut Budget,
}

impl Names<'_> {
    fn step(&mut self, depth: usize) -> OpResult<()> {
        self.budget.work(1)?;
        if depth > MAX_DEPTH {
            return Err(OpError::limit("typeof value nesting exceeds the limit of 64"));
        }
        Ok(())
    }

    fn common<'v>(
        &mut self,
        values: impl Iterator<Item = &'v Value>,
        depth: usize,
        outcomes: bool,
    ) -> OpResult<Description> {
        let mut found = Description::Unknown;
        for value in values {
            let ty = if outcomes && matches!(value, Value::Continuous(_)) {
                self.step(depth)?;
                Description::Atom("float".into())
            } else if outcomes && matches!(value, Value::MultivariateNormal(_)) {
                self.step(depth)?;
                Description::Container("list", vec![Description::Atom("float".into())])
            } else {
                self.describe(value, depth)?
            };
            found = self.join(found, ty, depth)?;
        }
        Ok(found)
    }

    fn record(&mut self, r: &Record, depth: usize) -> OpResult<Description> {
        let mut fields = Vec::new();
        for (name, value) in &r.fields {
            fields.push((name.to_string(), self.describe(value, depth + 1)?));
        }
        Ok(Description::Record(fields))
    }

    fn describe(&mut self, v: &Value, depth: usize) -> OpResult<Description> {
        self.step(depth)?;
        let atom = |s: &str| Description::Atom(s.into());
        let dist = |s: &str| Description::Container("dist", vec![atom(s)]);
        Ok(match v {
            Value::Dead => return Err(OpError::new("typeof needs an initialized value")),
            Value::Unit => atom("()"),
            Value::Bool(_) | Value::Event(_) => atom("bool"),
            Value::Int(_) => atom("int"),
            Value::Analytic(a) if a.int => atom("int"),
            Value::Float(_) | Value::Delayed(_) | Value::Analytic(_) => atom("float"),
            Value::Counts(c) => dist(if c.floats { "float" } else { "int" }),
            Value::Prob(_) => atom("prob"),
            Value::Complex(_) => atom("complex"),
            Value::Str(_) => atom("str"),
            Value::Date(_) => atom("date"),
            Value::Range(..) => atom("range"),
            Value::Closure(_) | Value::Builtin(_) => atom("fn"),
            Value::Continuous(_) => dist("float"),
            Value::MultivariateNormal(_) => {
                Description::Container("dist", vec![Description::Container("list", vec![atom("float")])])
            }
            Value::Enum(e) => atom(&self.enums[e.ty as usize].name),
            Value::Record(r) if r.ty.is_some() => Description::NamedRecord(vec![r.clone()]),
            Value::Record(r) => self.record(r, depth)?,
            Value::List(xs) => Description::Container("list", vec![self.common(xs.iter(), depth + 1, false)?]),
            Value::Bag(xs) => Description::Container("bag", vec![self.common(xs.keys(), depth + 1, false)?]),
            Value::Set(xs) => Description::Container("set", vec![self.common(xs.keys(), depth + 1, false)?]),
            Value::Map(xs) => Description::Container(
                "map",
                vec![
                    self.common(xs.keys(), depth + 1, false)?,
                    self.common(xs.values(), depth + 1, false)?,
                ],
            ),
            Value::Dist(d) | Value::Joint(d) => Description::Container(
                "dist",
                vec![self.common(d.outcomes.iter().map(|(v, _)| v), depth + 1, true)?],
            ),
        })
    }

    fn merge(&mut self, a: Description, b: Description, depth: usize) -> OpResult<Merge> {
        self.step(depth)?;
        use Description::*;
        Ok(Merge::Joined(match (a, b) {
            (Unknown, t) | (t, Unknown) => t,
            (Atom(a), Atom(b)) if a == b => Atom(a),
            (Container(a, xs), Container(b, ys)) if a == b => {
                let mut args = Vec::new();
                for (x, y) in xs.into_iter().zip(ys) {
                    args.push(self.join(x, y, depth + 1)?);
                }
                Container(a, args)
            }
            (NamedRecord(mut a), NamedRecord(b)) if a[0].ty == b[0].ty => {
                // The nominal name is enough until an anonymous record joins
                // this shape. Keep every instance for that later field merge.
                a.extend(b);
                NamedRecord(a)
            }
            (Record(fs), NamedRecord(rs)) | (NamedRecord(rs), Record(fs))
                if fs.len() == rs[0].fields.len()
                    && fs.iter().zip(&rs[0].fields).all(|((a, _), (b, _))| a == b.as_ref()) =>
            {
                let mut found = Record(fs);
                for r in rs {
                    let fields = self.record(&r, depth)?;
                    found = self.join(found, fields, depth)?;
                }
                found
            }
            (Record(xs), Record(ys)) if xs.len() == ys.len() && xs.iter().zip(&ys).all(|((a, _), (b, _))| a == b) => {
                let mut fields = Vec::new();
                for ((n, x), (_, y)) in xs.into_iter().zip(ys) {
                    fields.push((n, self.join(x, y, depth + 1)?));
                }
                Record(fields)
            }
            (a, b) => return Ok(Merge::Separate(a, b)),
        }))
    }

    fn join(&mut self, a: Description, b: Description, depth: usize) -> OpResult<Description> {
        let mut members = match a {
            Description::Union(ts) => ts,
            t => vec![t],
        };
        let additions = match b {
            Description::Union(ts) => ts,
            t => vec![t],
        };
        for mut t in additions {
            let mut i = 0;
            while i < members.len() {
                match self.merge(members.remove(i), t, depth)? {
                    Merge::Joined(merged) => {
                        t = merged;
                        i = 0; // An anonymous record can absorb several named types.
                    }
                    Merge::Separate(a, b) => {
                        members.insert(i, a);
                        t = b;
                        i += 1;
                    }
                }
            }
            members.push(t);
        }
        members.sort_by_cached_key(sort_key);
        Ok(if members.len() == 1 {
            members.pop().unwrap()
        } else {
            Description::Union(members)
        })
    }

    fn render(&mut self, ty: &Description, out: &mut String) -> OpResult<()> {
        self.budget.work(1)?;
        match ty {
            Description::Unknown => text::push(out, "unknown", self.budget)?,
            Description::Atom(s) => text::push(out, s, self.budget)?,
            Description::NamedRecord(rs) => text::push(out, rs[0].ty.as_deref().unwrap(), self.budget)?,
            Description::Container(name, args) => {
                text::push(out, name, self.budget)?;
                text::push(out, "[", self.budget)?;
                for (i, t) in args.iter().enumerate() {
                    if i != 0 {
                        text::push(out, ", ", self.budget)?;
                    }
                    self.render(t, out)?;
                }
                text::push(out, "]", self.budget)?;
            }
            Description::Record(fs) => {
                text::push(out, "{", self.budget)?;
                for (i, (n, t)) in fs.iter().enumerate() {
                    if i != 0 {
                        text::push(out, ", ", self.budget)?;
                    }
                    text::push(out, n, self.budget)?;
                    text::push(out, ": ", self.budget)?;
                    self.render(t, out)?;
                }
                text::push(out, "}", self.budget)?;
            }
            Description::Union(ts) => {
                for (i, t) in ts.iter().enumerate() {
                    if i != 0 {
                        text::push(out, " or ", self.budget)?;
                    }
                    self.render(t, out)?;
                }
            }
        }
        Ok(())
    }
}

fn sort_key(t: &Description) -> (usize, String) {
    let name = match t {
        Description::Unknown => "unknown".into(),
        Description::Atom(n) => n.clone(),
        Description::Container(n, _) => (*n).to_owned(),
        Description::NamedRecord(rs) => rs[0].ty.as_deref().unwrap().to_owned(),
        Description::Record(fs) => return (20, fs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(",")),
        _ => unreachable!("unions are flattened before sorting"),
    };
    let rank = [
        "unknown", "()", "bool", "int", "float", "prob", "complex", "str", "date", "range", "fn", "list", "set", "bag",
        "map", "dist",
    ]
    .iter()
    .position(|n| *n == name)
    .unwrap_or(21);
    (rank, name)
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
