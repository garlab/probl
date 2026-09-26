//! Runtime values.
//!
//! Values have value semantics: collections are shared through `Arc` and
//! copied on write, so forking a world is cheap and worlds never affect each
//! other. Equality and hashing are *structural* (they decide which worlds
//! merge); the language's `==` lives in [`crate::ops`] and compares numbers
//! across types.

use crate::continuous::Family;
use crate::dist::Dist;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone)]
pub enum Value {
    /// A slot with no value: not assigned yet, or cleared because it's dead.
    Dead,
    Unit,
    /// A fact: true or false in a world.
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A probability: a number from 0 to 1. It's a parameter, not an event.
    Prob(f64),
    Str(Arc<str>),
    List(Arc<Vec<Value>>),
    Map(Arc<BTreeMap<Value, Value>>),
    /// A multiset: value → count.
    Bag(Arc<BTreeMap<Value, u64>>),
    /// Integers from `.0` to `.1`, both included.
    Range(i64, i64),
    Record(Arc<Record>),
    Enum(Arc<EnumValue>),
    Dist(Arc<Dist>),
    /// A continuous distribution (docs/semantics.md, section 13).
    Continuous(Arc<Family>),
    Closure(Arc<Closure>),
    /// Days since 1970-01-01.
    Date(i32),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Record {
    pub ty: Option<Arc<str>>,
    /// Sorted by name.
    pub fields: Vec<(Arc<str>, Value)>,
}

impl Record {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| &**n == name).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.fields.iter_mut().find(|(n, _)| &**n == name).map(|(_, v)| v)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnumValue {
    pub ty: u32,
    pub variant: u32,
    pub name: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Closure {
    pub func: u32,
    pub captured: Vec<Value>,
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Arc::new(items))
    }

    /// The kind of value, as named in error messages.
    pub fn kind(&self) -> String {
        match self {
            Value::Dead => "nothing".into(),
            Value::Unit => "()".into(),
            Value::Bool(_) => "bool".into(),
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Prob(_) => "prob".into(),
            Value::Str(_) => "str".into(),
            Value::List(_) => "list".into(),
            Value::Map(_) => "map".into(),
            Value::Bag(_) => "bag".into(),
            Value::Range(..) => "range".into(),
            Value::Record(r) => match &r.ty {
                Some(t) => t.to_string(),
                None => "record".into(),
            },
            Value::Enum(_) => "enum".into(),
            Value::Dist(d) if d.outcomes.iter().any(|(v, _)| matches!(v, Value::Continuous(_))) => {
                "mixture of distributions".into()
            }
            Value::Dist(d) => match d.outcomes.first() {
                Some((v, _)) => format!("distribution over {}", plural_kind(&v.kind())),
                None => "distribution".into(),
            },
            Value::Continuous(f) => format!("{} distribution", f.name()),
            Value::Closure(_) => "function".into(),
            Value::Date(_) => "date".into(),
        }
    }

    /// A distribution with outcomes that can be listed.
    pub fn is_dist(&self) -> bool {
        matches!(self, Value::Dist(_))
    }

    /// Any distribution, including continuous ones: not a settled value.
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Value::Dist(_) | Value::Continuous(_))
    }

    /// Numbers as f64: ints, floats and probabilities.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) | Value::Prob(f) => Some(*f),
            _ => None,
        }
    }

    /// Order of the value kinds, used to sort mixed values.
    fn rank(&self) -> u8 {
        match self {
            Value::Dead => 0,
            Value::Unit => 1,
            Value::Bool(_) => 2,
            Value::Int(_) | Value::Float(_) | Value::Prob(_) => 3,
            Value::Str(_) => 4,
            Value::Date(_) => 5,
            Value::Enum(_) => 6,
            Value::List(_) => 7,
            Value::Range(..) => 8,
            Value::Map(_) => 9,
            Value::Bag(_) => 10,
            Value::Record(_) => 11,
            Value::Dist(_) => 12,
            Value::Closure(_) => 13,
            Value::Continuous(_) => 14,
        }
    }

    fn number_rank(&self) -> u8 {
        match self {
            Value::Int(_) => 0,
            Value::Float(_) => 1,
            _ => 2,
        }
    }
}

fn plural_kind(kind: &str) -> String {
    match kind {
        "str" => "strings".into(),
        "prob" => "probabilities".into(),
        "list" => "lists".into(),
        k => format!("{k}s"),
    }
}

/// What identifies a continuous distribution: its family and parameters.
fn family_key(f: &Family) -> (&'static str, Vec<u64>) {
    (f.name(), f.params().into_iter().map(float_key).collect())
}

/// Float bits with 0.0 and -0.0 merged and a single NaN.
fn float_key(f: f64) -> u64 {
    if f == 0.0 {
        0
    } else if f.is_nan() {
        f64::NAN.to_bits()
    } else {
        f.to_bits()
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Dead, Value::Dead) | (Value::Unit, Value::Unit) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) | (Value::Prob(a), Value::Prob(b)) => float_key(*a) == float_key(*b),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::List(a), Value::List(b)) => Arc::ptr_eq(a, b) || a == b,
            (Value::Map(a), Value::Map(b)) => Arc::ptr_eq(a, b) || a == b,
            (Value::Bag(a), Value::Bag(b)) => Arc::ptr_eq(a, b) || a == b,
            (Value::Range(a, b), Value::Range(c, d)) => a == c && b == d,
            (Value::Record(a), Value::Record(b)) => Arc::ptr_eq(a, b) || a == b,
            (Value::Enum(a), Value::Enum(b)) => a.ty == b.ty && a.variant == b.variant,
            (Value::Dist(a), Value::Dist(b)) => Arc::ptr_eq(a, b) || a == b,
            (Value::Closure(a), Value::Closure(b)) => Arc::ptr_eq(a, b) || a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::Continuous(a), Value::Continuous(b)) => family_key(a) == family_key(b),
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Dead | Value::Unit => {}
            Value::Bool(b) => b.hash(state),
            Value::Int(i) => i.hash(state),
            Value::Float(f) | Value::Prob(f) => float_key(*f).hash(state),
            Value::Str(s) => s.hash(state),
            Value::List(items) => items.hash(state),
            Value::Map(m) => m.hash(state),
            Value::Bag(b) => b.hash(state),
            Value::Range(a, b) => (a, b).hash(state),
            Value::Record(r) => r.hash(state),
            Value::Enum(e) => (e.ty, e.variant).hash(state),
            Value::Dist(d) => d.hash(state),
            Value::Closure(c) => c.hash(state),
            Value::Date(d) => d.hash(state),
            Value::Continuous(f) => family_key(f).hash(state),
        }
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Value) -> Ordering {
        let (ra, rb) = (self.rank(), other.rank());
        if ra != rb {
            return ra.cmp(&rb);
        }
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (a, b) if ra == 3 => {
                // Numbers compare by value, then by kind so the order stays total.
                let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                x.total_cmp(&y)
                    .then_with(|| a.number_rank().cmp(&b.number_rank()))
                    .then_with(|| float_key(x).cmp(&float_key(y)))
            }
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Date(a), Value::Date(b)) => a.cmp(b),
            (Value::Enum(a), Value::Enum(b)) => (a.ty, a.variant).cmp(&(b.ty, b.variant)),
            (Value::List(a), Value::List(b)) => a.cmp(b),
            (Value::Range(a, b), Value::Range(c, d)) => (a, b).cmp(&(c, d)),
            (Value::Map(a), Value::Map(b)) => a.cmp(b),
            (Value::Bag(a), Value::Bag(b)) => a.cmp(b),
            (Value::Record(a), Value::Record(b)) => a.cmp(b),
            (Value::Dist(a), Value::Dist(b)) => a.cmp(b),
            (Value::Closure(a), Value::Closure(b)) => a.cmp(b),
            (Value::Continuous(a), Value::Continuous(b)) => family_key(a).cmp(&family_key(b)),
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Value) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ── Display ──────────────────────────────────────────────────────────────

/// A float the way Probl prints it: `3.0`, `0.25`, `1.5e-12`.
pub fn fmt_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let a = f.abs();
    if a != 0.0 && (a >= 1e15 || a < 1e-4) {
        return format!("{f:e}");
    }
    if f == f.trunc() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

/// A probability as a percentage with up to two decimals: `30%`, `16.67%`.
pub fn fmt_prob(p: f64) -> String {
    let pct = p * 100.0;
    if pct != 0.0 && pct.abs() < 0.01 {
        return format!("{pct:.1e}%");
    }
    let text = format!("{pct:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}%")
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_value(self, f, false)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_value(self, f, true)
    }
}

fn write_value(v: &Value, f: &mut fmt::Formatter<'_>, nested: bool) -> fmt::Result {
    match v {
        Value::Dead => write!(f, "<no value>"),
        Value::Unit => write!(f, "()"),
        Value::Int(i) => write!(f, "{i}"),
        Value::Float(x) => write!(f, "{}", fmt_float(*x)),
        Value::Bool(b) => write!(f, "{b}"),
        Value::Prob(p) => write!(f, "{}", fmt_prob(*p)),
        Value::Str(s) if nested => write!(f, "{s:?}"),
        Value::Str(s) => write!(f, "{s}"),
        Value::List(items) => {
            write!(f, "[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_value(item, f, true)?;
            }
            write!(f, "]")
        }
        Value::Map(m) => {
            if m.is_empty() {
                return write!(f, "[:]");
            }
            write!(f, "[")?;
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_value(k, f, true)?;
                write!(f, ": ")?;
                write_value(v, f, true)?;
            }
            write!(f, "]")
        }
        Value::Bag(b) => {
            write!(f, "bag([")?;
            for (i, (k, n)) in b.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_value(k, f, true)?;
                write!(f, ": {n}")?;
            }
            write!(f, "])")
        }
        Value::Range(a, b) => write!(f, "{a}..{b}"),
        Value::Record(r) => {
            if let Some(t) = &r.ty {
                write!(f, "{t} ")?;
            }
            write!(f, "{{ ")?;
            for (i, (name, v)) in r.fields.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{name}: ")?;
                write_value(v, f, true)?;
            }
            write!(f, " }}")
        }
        Value::Enum(e) => write!(f, "{}", e.name),
        Value::Dist(d) => {
            write!(f, "dist(")?;
            for (i, (v, p)) in d.outcomes.iter().enumerate() {
                if i == 8 {
                    write!(f, ", … {} more", d.outcomes.len() - 8)?;
                    break;
                }
                if i > 0 {
                    write!(f, ", ")?;
                }
                write_value(v, f, true)?;
                write!(f, ": {}", fmt_prob(*p))?;
            }
            write!(f, ")")
        }
        Value::Closure(_) => write!(f, "<function>"),
        Value::Date(d) => write!(f, "{}", crate::dates::format(*d)),
        Value::Continuous(family) => write!(f, "{family}"),
    }
}
