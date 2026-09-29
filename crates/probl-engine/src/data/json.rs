//! JSON documents, as any type data can have (RFC 8259). `serde_json`
//! parses, and the declared type, visited as it goes, says what each value
//! is: no tree is built besides the values kept. Integers never go through
//! a float, a key may appear only once in an object, and keys that read as
//! the same map key are an error.

use super::{Cx, PathPart, Problem, quoted, text};
use crate::ops;
use crate::value::Value;
use probl_sema::data::{field_key, is_key};
use probl_sema::ir::TypeSpec;
use rustc_hash::FxHashSet;
use serde::Deserialize;
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::error::Category;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub(crate) fn read(bytes: &[u8], ty: &TypeSpec, cx: &mut Cx) -> Result<Value, Problem> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let result = Seed { ty, cx: &mut *cx }
        .deserialize(&mut de)
        .and_then(|v| de.end().map(|()| v));
    result.map_err(|e| {
        let mut problem = cx.problem.take().unwrap_or_else(|| {
            let text = e.to_string();
            let message = text.split(" at line ").next().unwrap_or(&text).to_string();
            match e.classify() {
                Category::Syntax | Category::Eof => Problem::new(format!("this isn't valid JSON: {message}")),
                _ => Problem::new(message),
            }
        });
        let position = (e.line() > 0).then(|| format!("line {}, column {}", e.line(), e.column()));
        problem.at = match (position, problem.at.take()) {
            (Some(position), Some(path)) => Some(format!("{position}, {path}")),
            (position, path) => position.or(path),
        };
        problem
    })
}

impl Cx<'_> {
    /// Keep the problem, with where in the document it is, and give the
    /// parser an error to stop with (it adds the line and column).
    fn fail<E: de::Error>(&mut self, problem: Problem) -> E {
        if self.problem.is_none() {
            let problem = if self.path.is_empty() {
                problem
            } else {
                let mut path = String::new();
                for part in &self.path {
                    match part {
                        PathPart::Key(k) if path.is_empty() => path.push_str(k),
                        PathPart::Key(k) => {
                            path.push('.');
                            path.push_str(k);
                        }
                        PathPart::Index(i) => path.push_str(&format!("[{i}]")),
                    }
                }
                problem.at(format!("at {path}"))
            };
            self.problem = Some(problem);
        }
        E::custom("")
    }
}

/// A value of type `ty`, read by `serde_json`.
struct Seed<'t, 'c, 'a> {
    ty: &'t TypeSpec,
    cx: &'c mut Cx<'a>,
}

impl<'de> DeserializeSeed<'de> for Seed<'_, '_, '_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        let Seed { ty, cx } = self;
        if cx.depth >= cx.budget.limits.max_depth {
            let max = cx.budget.limits.max_depth;
            return Err(cx.fail(Problem::limit(format!("the data nests more than {max} levels deep"))));
        }
        if let Err(p) = cx.budget.value() {
            return Err(cx.fail(p));
        }
        // Read numeric tokens as raw JSON, never through f64 or serde's
        // synthetic arbitrary-precision map. A user object cannot impersonate
        // a number using a serde-private key.
        if matches!(ty, TypeSpec::Int | TypeSpec::Float | TypeSpec::Prob) {
            let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
            let t = raw.get();
            if t.starts_with('"') {
                let s: String = serde_json::from_str(t).map_err(de::Error::custom)?;
                return Visit { ty, cx }.visit_str(&s);
            }
            if t == "true" || t == "false" {
                return Visit { ty, cx }.visit_bool(t == "true");
            }
            if t.starts_with('{') {
                return Err(Visit { ty, cx }.mismatch("an object".into()));
            }
            if t.starts_with('[') {
                return Err(Visit { ty, cx }.mismatch("an array".into()));
            }
            if t == "null" {
                return Err(Visit { ty, cx }.mismatch("`null`".into()));
            }
            if *ty == TypeSpec::Int && t.contains(['.', 'e', 'E']) {
                return Err(cx.fail(Problem::new(format!("{} isn't an int", super::quoted(t)))));
            }
            return text::plain(t, ty, cx.program, &mut cx.budget).map_err(|p| cx.fail(p));
        }
        cx.depth += 1;
        let result = d.deserialize_any(Visit { ty, cx: &mut *cx });
        cx.depth -= 1;
        result
    }
}

struct Visit<'t, 'c, 'a> {
    ty: &'t TypeSpec,
    cx: &'c mut Cx<'a>,
}

impl Visit<'_, '_, '_> {
    fn expected(&self) -> String {
        format!("a `{}`", self.ty.describe(self.cx.program))
    }

    fn mismatch<E: de::Error>(self, found: String) -> E {
        let message = format!("expected {}, found {found}", self.expected());
        self.cx.fail(Problem::new(message))
    }

    fn int<E: de::Error>(self, n: i64) -> Result<Value, E> {
        match self.ty {
            TypeSpec::Int => Ok(Value::Int(n.into())),
            TypeSpec::Float => Ok(Value::Float(n as f64)),
            TypeSpec::Prob if n == 0 || n == 1 => Ok(Value::Prob(n as f64)),
            TypeSpec::Prob => Err(self.cx.fail(
                Problem::new(format!("`{n}` isn't between 0 and 1"))
                    .help(format!("for a percentage, write it as a string: \"{n}%\"")),
            )),
            _ => Err(self.mismatch(format!("the number `{n}`"))),
        }
    }
}

impl<'de> Visitor<'de> for Visit<'_, '_, '_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.expected())
    }

    fn visit_bool<E: de::Error>(self, b: bool) -> Result<Value, E> {
        match self.ty {
            TypeSpec::Bool => Ok(Value::Bool(b)),
            _ => Err(self.mismatch(format!("`{b}`"))),
        }
    }

    fn visit_i64<E: de::Error>(self, n: i64) -> Result<Value, E> {
        self.int(n)
    }

    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Value, E> {
        match i64::try_from(n) {
            Ok(n) => self.int(n),
            Err(_) => match self.ty {
                TypeSpec::Float => Ok(Value::Float(n as f64)),
                TypeSpec::Int => Err(self.cx.fail(
                    Problem::new(format!("`{n}` is too large for an int")).help("an int is between -2⁶³ and 2⁶³ - 1"),
                )),
                _ => Err(self.mismatch(format!("the number `{n}`"))),
            },
        }
    }

    fn visit_f64<E: de::Error>(self, x: f64) -> Result<Value, E> {
        match self.ty {
            TypeSpec::Float => Ok(Value::Float(x)),
            TypeSpec::Prob if (0.0..=1.0).contains(&x) => Ok(Value::Prob(x)),
            TypeSpec::Prob => Err(self.cx.fail(Problem::new(format!("`{x:?}` isn't between 0 and 1")))),
            TypeSpec::Int => Err(self.cx.fail(
                Problem::new(format!("`{x:?}` isn't an int"))
                    .help("ints are written without a decimal point or an exponent, like 12"),
            )),
            _ => Err(self.mismatch(format!("the number `{x:?}`"))),
        }
    }

    fn visit_str<E: de::Error>(self, s: &str) -> Result<Value, E> {
        match self.ty {
            TypeSpec::Str => {
                self.cx.budget.max_string_bytes_seen = self.cx.budget.max_string_bytes_seen.max(s.len());
                Ok(Value::str(s))
            }
            TypeSpec::Date | TypeSpec::Enum(_) | TypeSpec::Prob => {
                text::plain(s, self.ty, self.cx.program, &mut self.cx.budget).map_err(|p| self.cx.fail(p))
            }
            TypeSpec::Int | TypeSpec::Float => Err(self.cx.fail(
                Problem::new(format!("expected {}, found the string {}", self.expected(), quoted(s)))
                    .help("write numbers without quotes"),
            )),
            _ => Err(self.mismatch(format!("the string {}", quoted(s)))),
        }
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Err(self.cx.fail(
            Problem::new(format!("expected {}, found `null`", self.expected()))
                .help("the language has no missing values yet"),
        ))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let Visit { ty, cx } = self;
        match ty {
            TypeSpec::List(item) => {
                let mut items = Vec::new();
                loop {
                    cx.path.push(PathPart::Index(items.len()));
                    let next = seq.next_element_seed(Seed { ty: item, cx: &mut *cx })?;
                    let Some(v) = next else {
                        cx.path.pop();
                        break;
                    };
                    items.push(v);
                    if let Err(p) = cx.budget.collection(items.len()) {
                        return Err(cx.fail(p));
                    }
                    cx.path.pop();
                }
                Ok(Value::list(items))
            }
            TypeSpec::Bag(item) => {
                let mut counts: BTreeMap<Value, u64> = BTreeMap::new();
                let mut n = 0;
                loop {
                    cx.path.push(PathPart::Index(n));
                    let next = seq.next_element_seed(Seed { ty: item, cx: &mut *cx })?;
                    cx.path.pop();
                    let Some(v) = next else { break };
                    n += 1;
                    if let Err(p) = cx.budget.collection(n) {
                        return Err(cx.fail(p));
                    }
                    *counts.entry(v).or_insert(0) += 1;
                }
                Ok(Value::bag(counts))
            }
            _ => Err(Visit { ty, cx }.mismatch("an array".into())),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, access: A) -> Result<Value, A::Error> {
        let Visit { ty, cx } = self;
        match ty {
            TypeSpec::Record(r) => {
                let decl = &cx.program.records[*r as usize];
                let name = cx.name(&decl.name.clone());
                let fields: Vec<(String, TypeSpec)> =
                    decl.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect();
                record(cx, Some(name), &fields, access)
            }
            TypeSpec::AnonRecord(fields) => record(cx, None, fields, access),
            TypeSpec::Map(k, v) => map(cx, k, v, access),
            TypeSpec::Bag(item) if is_key(item) => counts(cx, item, access),
            TypeSpec::Bag(item) => {
                let item = item.describe(cx.program);
                Err(cx.fail(
                    Problem::new(format!("a bag of `{item}` is written as an array of its items"))
                        .help("an object of counts, like {\"ace\": 4}, only works for items that can be keys"),
                ))
            }
            _ => Err(Visit { ty, cx }.mismatch("an object".into())),
        }
    }
}

/// An object as a record: each field from the key that matches its name,
/// other keys ignored.
fn record<'de, A: MapAccess<'de>>(
    cx: &mut Cx,
    type_name: Option<Arc<str>>,
    fields: &[(String, TypeSpec)],
    mut access: A,
) -> Result<Value, A::Error> {
    let keys: Vec<String> = fields.iter().map(|(n, _)| field_key(n)).collect();
    let mut found: Vec<Option<(String, Value)>> = vec![None; fields.len()];
    let mut seen: Vec<String> = Vec::new();
    let mut seen_set: FxHashSet<String> = FxHashSet::default();
    while let Some(key) = access.next_key::<String>()? {
        if !seen_set.insert(key.clone()) {
            return Err(cx.fail(Problem::new(format!("the key {} appears twice", quoted(&key)))));
        }
        seen.push(key.clone());
        let Some(i) = keys.iter().position(|k| *k == field_key(&key)) else {
            access.next_value::<IgnoredAny>()?;
            continue;
        };
        if let Some((first, _)) = &found[i] {
            return Err(cx.fail(
                Problem::new(format!(
                    "the keys {} and {} both match the field `{}`",
                    quoted(first),
                    quoted(&key),
                    fields[i].0
                ))
                .help("names match ignoring case, spaces and punctuation"),
            ));
        }
        cx.path.push(PathPart::Key(key.clone()));
        let v = access.next_value_seed(Seed {
            ty: &fields[i].1,
            cx: &mut *cx,
        })?;
        cx.path.pop();
        found[i] = Some((key, v));
    }
    let mut values = Vec::with_capacity(fields.len());
    for (i, slot) in found.into_iter().enumerate() {
        match slot {
            Some((_, v)) => values.push((cx.name(&fields[i].0), v)),
            None => {
                let keys: Vec<String> = seen.iter().map(|k| quoted(k)).collect();
                let keys = if keys.is_empty() {
                    "none".to_string()
                } else {
                    keys.join(", ")
                };
                return Err(cx.fail(
                    Problem::new(format!("no key matches the field `{}`", fields[i].0))
                        .note(format!("the object's keys are {keys}"))
                        .help("names match ignoring case, spaces and punctuation"),
                ));
            }
        }
    }
    Ok(ops::make_record(type_name, values))
}

/// A key as a map key or bag item of type `ty`. Two different keys that
/// read as the same value are an error.
fn key<E: de::Error>(cx: &mut Cx, text: &str, ty: &TypeSpec, first: &mut BTreeMap<Value, String>) -> Result<Value, E> {
    cx.path.push(PathPart::Key(text.to_string()));
    let v = match super::text::plain(text, ty, cx.program, &mut cx.budget) {
        Ok(v) => v,
        Err(p) => return Err(cx.fail(p)),
    };
    if let Some(earlier) = first.get(&v) {
        let problem = Problem::new(format!(
            "the keys {} and {} are the same {}",
            quoted(earlier),
            quoted(text),
            ty.describe(cx.program)
        ));
        return Err(cx.fail(problem));
    }
    first.insert(v.clone(), text.to_string());
    Ok(v)
}

fn map<'de, A: MapAccess<'de>>(cx: &mut Cx, k: &TypeSpec, v: &TypeSpec, mut access: A) -> Result<Value, A::Error> {
    let mut entries = BTreeMap::new();
    let mut first = BTreeMap::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    while let Some(text) = access.next_key::<String>()? {
        if !seen.insert(text.clone()) {
            return Err(cx.fail(Problem::new(format!("the key {} appears twice", quoted(&text)))));
        }
        if let Err(p) = cx.budget.collection(entries.len() + 1) {
            return Err(cx.fail(p));
        }
        let kv = key(cx, &text, k, &mut first)?;
        let value = access.next_value_seed(Seed { ty: v, cx: &mut *cx })?;
        cx.path.pop();
        entries.insert(kv, value);
    }
    Ok(Value::map(entries))
}

/// A bag written as an object of counts: `{"ace": 4, "king": 4}`.
fn counts<'de, A: MapAccess<'de>>(cx: &mut Cx, item: &TypeSpec, mut access: A) -> Result<Value, A::Error> {
    let mut counts = BTreeMap::new();
    let mut first = BTreeMap::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();
    while let Some(text) = access.next_key::<String>()? {
        if !seen.insert(text.clone()) {
            return Err(cx.fail(Problem::new(format!("the key {} appears twice", quoted(&text)))));
        }
        if let Err(p) = cx.budget.collection(counts.len() + 1) {
            return Err(cx.fail(p));
        }
        let kv = key(cx, &text, item, &mut first)?;
        let n = access.next_value_seed(Seed {
            ty: &TypeSpec::Int,
            cx: &mut *cx,
        })?;
        let n = match n {
            Value::Int(n) if n >= 0 => n
                .to_u64()
                .ok_or_else(|| cx.fail(Problem::new("bag count exceeds the supported count range")))?,
            _ => return Err(cx.fail(Problem::new("a count can't be negative"))),
        };
        cx.path.pop();
        if n > 0 {
            counts.insert(kv, n);
        }
    }
    Ok(Value::bag(counts))
}
