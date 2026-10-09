//! Runtime values.
//!
//! Values have value semantics: collections are shared through `Arc` and
//! copied on write, so forking a world is cheap and worlds never affect each
//! other. Equality and hashing are *structural* (they decide which worlds
//! merge); the language's `==` lives in [`crate::ops`] and compares numbers
//! across types.

use crate::complex::Complex;
use crate::continuous::Family;
use crate::dist::Dist;
use rustc_hash::FxHasher;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as Atomic};

#[derive(Clone)]
pub enum Value {
    /// A slot with no value: not assigned yet, or cleared because it's dead.
    Dead,
    Unit,
    /// A fact: true or false in a world.
    Bool(bool),
    Int(probl_number::Integer),
    Float(f64),
    Complex(Complex),
    /// A probability: a number from 0 to 1. It's a parameter, not an event.
    Prob(f64),
    Str(Arc<str>),
    List(Arc<Hashed<Vec<Value>>>),
    Map(Arc<Hashed<BTreeMap<Value, Value>>>),
    /// A multiset: value → count.
    Bag(Arc<Hashed<Multiset>>),
    /// Integers from `.0` to `.1`, both included.
    Range(probl_number::Integer, probl_number::Integer),
    Record(Arc<Hashed<Record>>),
    Enum(Arc<EnumValue>),
    Dist(Arc<Dist>),
    /// A continuous distribution (docs/semantics.md, section 13).
    Continuous(Arc<Family>),
    Closure(Arc<Closure>),
    Builtin(probl_sema::Builtin),
    /// Days since 1970-01-01.
    Date(i32),
    /// A variable's value that isn't drawn yet, internal to the engine
    /// (docs/semantics.md, section 14). The engine draws it before any
    /// value read; a direct `typeof` inspection only needs its outcome type.
    Delayed(Arc<Delayed>),
    /// A scalar outcome retained analytically during enumeration.
    Analytic(Arc<crate::analytic::Analytic>),
    /// A boolean predicate of that same outcome, retaining its identity.
    Event(Arc<crate::analytic::Event>),
    /// A count law too broad to list its outcomes, like `geometric(1e-12)`:
    /// drawn from directly when sampling, and asked by `pmf`, `cdf` and the
    /// other queries. Anything that needs its outcomes lists it, which
    /// fails on the outcome limit, as building it once did.
    Counts(Arc<crate::dist::Counts>),
}

/// The distribution of a variable whose draw is delayed, updated exactly by
/// the observations so far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delayed {
    pub family: Family,
    /// Which of the program's variables that may be delayed it is
    /// (`Conjugacy::variables`), for statistics.
    pub variable: u32,
}

/// A collection, and its hash once computed. Worlds are hashed whenever
/// they merge, and most of their collections haven't changed since the last
/// time: each is hashed once. Changing a collection (through `DerefMut`)
/// forgets its hash, and collections with different hashes are unequal
/// without comparing them.
pub struct Hashed<T> {
    /// 0 until computed.
    hash: AtomicU64,
    value: T,
}

impl<T> Hashed<T> {
    pub fn new(value: T) -> Hashed<T> {
        Hashed {
            hash: AtomicU64::new(0),
            value,
        }
    }

    fn known_hash(&self) -> u64 {
        self.hash.load(Atomic::Relaxed)
    }
}

impl<T: Hash> Hashed<T> {
    /// The collection's hash: computed the first time, then kept.
    pub fn hash_code(&self) -> u64 {
        let known = self.known_hash();
        if known != 0 {
            return known;
        }
        let mut hasher = FxHasher::default();
        self.value.hash(&mut hasher);
        let h = hasher.finish().max(1);
        self.hash.store(h, Atomic::Relaxed);
        h
    }
}

impl<T> Deref for Hashed<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Hashed<T> {
    fn deref_mut(&mut self) -> &mut T {
        *self.hash.get_mut() = 0;
        &mut self.value
    }
}

impl<T: Clone> Clone for Hashed<T> {
    fn clone(&self) -> Hashed<T> {
        Hashed {
            hash: AtomicU64::new(self.known_hash()),
            value: self.value.clone(),
        }
    }
}

impl<T: PartialEq> PartialEq for Hashed<T> {
    fn eq(&self, other: &Hashed<T>) -> bool {
        let (a, b) = (self.known_hash(), other.known_hash());
        (a == 0 || b == 0 || a == b) && self.value == other.value
    }
}

impl<T: Eq> Eq for Hashed<T> {}

impl<T: Hash> Hash for Hashed<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash_code());
    }
}

impl<T: Ord> PartialOrd for Hashed<T> {
    fn partial_cmp(&self, other: &Hashed<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Ord> Ord for Hashed<T> {
    fn cmp(&self, other: &Hashed<T>) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl<T: fmt::Debug> fmt::Debug for Hashed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// A bag's contents: each distinct value, sorted, and how many times it's
/// in the bag (never zero). A sorted vector rather than a map: bags are
/// small, copied whenever they change, and compared each time worlds merge,
/// which is fastest in contiguous memory.
#[derive(Clone, Debug, Default)]
pub struct Multiset {
    entries: Vec<(Value, u64)>,
    /// The sum of the entries' hashes, which is the bag's hash: taking a
    /// value out updates it without going through the other entries.
    sum: u64,
}

/// One entry's share of a bag's hash. Mixed thoroughly, so that bags with
/// counts moved from one value to another don't sum to the same hash.
fn entry_hash(v: &Value, n: u64) -> u64 {
    let mut hasher = FxHasher::default();
    v.hash(&mut hasher);
    n.hash(&mut hasher);
    crate::continuous::mix(hasher.finish())
}

impl Multiset {
    /// From counts, leaving out zeros.
    pub fn new(counts: BTreeMap<Value, u64>) -> Multiset {
        let entries: Vec<(Value, u64)> = counts.into_iter().filter(|(_, n)| *n > 0).collect();
        let sum = entries
            .iter()
            .fold(0u64, |sum, (v, n)| sum.wrapping_add(entry_hash(v, *n)));
        Multiset { entries, sum }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Value, &u64)> + Clone {
        self.entries.iter().map(|(v, n)| (v, n))
    }

    pub fn keys(&self) -> impl Iterator<Item = &Value> + Clone {
        self.entries.iter().map(|(v, _)| v)
    }

    pub fn values(&self) -> impl Iterator<Item = &u64> + Clone {
        self.entries.iter().map(|(_, n)| n)
    }

    /// How many times `v` is in the bag, if it is.
    pub fn get(&self, v: &Value) -> Option<&u64> {
        let i = self.entries.binary_search_by(|(k, _)| k.cmp(v)).ok()?;
        Some(&self.entries[i].1)
    }

    /// The bag without one of its `i`-th distinct value.
    pub fn without_nth(&self, i: usize) -> Multiset {
        let mut entries = self.entries.clone();
        let (v, n) = (&self.entries[i].0, self.entries[i].1);
        let mut sum = self.sum.wrapping_sub(entry_hash(v, n));
        if n > 1 {
            entries[i].1 = n - 1;
            sum = sum.wrapping_add(entry_hash(v, n - 1));
        } else {
            entries.remove(i);
        }
        Multiset { entries, sum }
    }

    /// The bag without one `v`, if it has one.
    pub fn without(&self, v: &Value) -> Option<Multiset> {
        let i = self.entries.binary_search_by(|(k, _)| k.cmp(v)).ok()?;
        Some(self.without_nth(i))
    }
}

impl PartialEq for Multiset {
    fn eq(&self, other: &Multiset) -> bool {
        self.sum == other.sum && self.entries == other.entries
    }
}

impl Eq for Multiset {}

impl Hash for Multiset {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.sum);
    }
}

impl PartialOrd for Multiset {
    fn partial_cmp(&self, other: &Multiset) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Multiset {
    fn cmp(&self, other: &Multiset) -> Ordering {
        self.entries.cmp(&other.entries)
    }
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
        Value::List(Arc::new(Hashed::new(items)))
    }

    pub fn map(entries: BTreeMap<Value, Value>) -> Value {
        Value::Map(Arc::new(Hashed::new(entries)))
    }

    pub fn bag(counts: BTreeMap<Value, u64>) -> Value {
        Value::multiset(Multiset::new(counts))
    }

    pub fn multiset(contents: Multiset) -> Value {
        Value::Bag(Arc::new(Hashed::new(contents)))
    }

    pub fn record(record: Record) -> Value {
        Value::Record(Arc::new(Hashed::new(record)))
    }

    /// A copy that shares no collection or string with the original. Values
    /// are shared through reference counts: threads that all read the same
    /// data would compete for those, so each sampling thread takes a copy.
    pub fn unshared(&self) -> Value {
        self.unshared_with(&mut rustc_hash::FxHashMap::default())
    }

    /// `names` keeps the copy's names (record types, fields, variants) shared
    /// within the copy.
    fn unshared_with(&self, names: &mut rustc_hash::FxHashMap<String, Arc<str>>) -> Value {
        fn name(names: &mut rustc_hash::FxHashMap<String, Arc<str>>, n: &str) -> Arc<str> {
            names.entry(n.to_string()).or_insert_with(|| Arc::from(n)).clone()
        }
        match self {
            Value::Str(s) => Value::str(s),
            Value::Enum(e) => Value::Enum(Arc::new(EnumValue {
                name: name(names, &e.name),
                ..(**e).clone()
            })),
            Value::Record(r) => {
                let ty = r.ty.as_deref().map(|t| name(names, t));
                let mut fields = Vec::with_capacity(r.fields.len());
                for (n, v) in &r.fields {
                    fields.push((name(names, n), v.unshared_with(names)));
                }
                Value::record(Record { ty, fields })
            }
            Value::List(items) => Value::list(items.iter().map(|x| x.unshared_with(names)).collect()),
            Value::Map(m) => Value::map(
                m.iter()
                    .map(|(k, v)| (k.unshared_with(names), v.unshared_with(names)))
                    .collect(),
            ),
            Value::Bag(b) => Value::multiset(Multiset {
                entries: b.entries.iter().map(|(v, n)| (v.unshared_with(names), *n)).collect(),
                sum: b.sum,
            }),
            other => other.clone(),
        }
    }

    /// The kind of value, as named in error messages.
    pub fn kind(&self) -> String {
        match self {
            Value::Dead => "nothing".into(),
            Value::Unit => "()".into(),
            Value::Bool(_) => "bool".into(),
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Complex(_) => "complex".into(),
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
            Value::Closure(_) | Value::Builtin(_) => "function".into(),
            Value::Date(_) => "date".into(),
            Value::Delayed(_) => "value not drawn yet".into(),
            Value::Analytic(a) if a.int => "int".into(),
            Value::Analytic(_) => "float".into(),
            Value::Event(_) => "bool".into(),
            Value::Counts(_) => "distribution over ints".into(),
        }
    }

    /// A distribution with outcomes that can be listed.
    pub fn is_dist(&self) -> bool {
        matches!(self, Value::Dist(_))
    }

    /// Any distribution, including continuous ones: not a settled value.
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Value::Dist(_) | Value::Continuous(_) | Value::Counts(_))
    }

    /// Numbers as f64: ints, floats and probabilities.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => i.to_f64(),
            Value::Float(f) | Value::Prob(f) => Some(*f),
            _ => None,
        }
    }

    /// Promote real numeric data to a complex scalar, without accepting facts
    /// or converting complex data back into real probabilities.
    pub fn as_complex(&self) -> Option<Complex> {
        match self {
            Value::Complex(z) => Some(*z),
            _ => self.as_f64().and_then(|x| Complex::new(x, 0.0).ok()),
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
            Value::Builtin(_) => 19,
            Value::Continuous(_) => 14,
            Value::Delayed(_) => 15,
            Value::Complex(_) => 16,
            Value::Analytic(_) => 17,
            Value::Event(_) => 18,
            Value::Counts(_) => 20,
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
pub(crate) fn family_key(f: &Family) -> (&'static str, Vec<u64>) {
    (f.name(), f.params().into_iter().map(float_key).collect())
}

/// Float bits with 0.0 and -0.0 merged and a single NaN.
pub(crate) fn float_key(f: f64) -> u64 {
    if f == 0.0 {
        0
    } else if f.is_nan() {
        f64::NAN.to_bits()
    } else {
        f.to_bits()
    }
}

impl PartialEq for Value {
    /// Worlds are compared slot by slot, and most slots hold small values:
    /// those are compared here, inlined, and the rest out of line.
    #[inline]
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Dead, Value::Dead) => true,
            _ => eq_other(self, other),
        }
    }
}

#[inline(never)]
fn eq_other(x: &Value, y: &Value) -> bool {
    match (x, y) {
        (Value::Unit, Value::Unit) => true,
        (Value::Float(a), Value::Float(b)) | (Value::Prob(a), Value::Prob(b)) => float_key(*a) == float_key(*b),
        (Value::Complex(a), Value::Complex(b)) => a == b,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::List(a), Value::List(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Map(a), Value::Map(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Bag(a), Value::Bag(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Range(a, b), Value::Range(c, d)) => a == c && b == d,
        (Value::Record(a), Value::Record(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Enum(a), Value::Enum(b)) => a.ty == b.ty && a.variant == b.variant,
        (Value::Dist(a), Value::Dist(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Closure(a), Value::Closure(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Builtin(a), Value::Builtin(b)) => a == b,
        (Value::Date(a), Value::Date(b)) => a == b,
        (Value::Continuous(a), Value::Continuous(b)) => family_key(a) == family_key(b),
        (Value::Analytic(a), Value::Analytic(b)) => a.key() == b.key(),
        (Value::Event(a), Value::Event(b)) => a.key() == b.key(),
        (Value::Counts(a), Value::Counts(b)) => a.key() == b.key(),
        (Value::Delayed(a), Value::Delayed(b)) => {
            a.variable == b.variable && family_key(&a.family) == family_key(&b.family)
        }
        _ => false,
    }
}

impl Eq for Value {}

impl Hash for Value {
    /// Like equality: small values inlined, the rest out of line.
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Dead | Value::Unit => {}
            Value::Bool(b) => b.hash(state),
            Value::Int(i) => i.hash(state),
            other => hash_other(other, state),
        }
    }
}

#[inline(never)]
fn hash_other<H: Hasher>(value: &Value, state: &mut H) {
    match value {
        Value::Dead | Value::Unit | Value::Bool(_) | Value::Int(_) => {}
        Value::Float(f) | Value::Prob(f) => float_key(*f).hash(state),
        Value::Complex(z) => (float_key(z.re()), float_key(z.im())).hash(state),
        Value::Str(s) => s.hash(state),
        Value::List(items) => items.hash(state),
        Value::Map(m) => m.hash(state),
        Value::Bag(b) => b.hash(state),
        Value::Range(a, b) => (a, b).hash(state),
        Value::Record(r) => r.hash(state),
        Value::Enum(e) => (e.ty, e.variant).hash(state),
        Value::Dist(d) => d.hash(state),
        Value::Closure(c) => c.hash(state),
        Value::Builtin(b) => b.hash(state),
        Value::Date(d) => d.hash(state),
        Value::Continuous(f) => family_key(f).hash(state),
        Value::Analytic(a) => a.key().hash(state),
        Value::Event(a) => a.key().hash(state),
        Value::Delayed(d) => (d.variable, family_key(&d.family)).hash(state),
        Value::Counts(c) => c.key().hash(state),
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
            (Value::Int(a), Value::Float(b) | Value::Prob(b)) => a
                .cmp_f64(*b)
                .unwrap_or(Ordering::Less)
                .then_with(|| self.number_rank().cmp(&other.number_rank())),
            (Value::Float(a) | Value::Prob(a), Value::Int(b)) => b
                .cmp_f64(*a)
                .unwrap_or(Ordering::Less)
                .reverse()
                .then_with(|| self.number_rank().cmp(&other.number_rank())),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (a, b) if ra == 3 => {
                // Numbers compare by value, then by kind so the order stays total.
                let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                f64::from_bits(float_key(x))
                    .total_cmp(&f64::from_bits(float_key(y)))
                    .then_with(|| a.number_rank().cmp(&b.number_rank()))
                    .then_with(|| float_key(x).cmp(&float_key(y)))
            }
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Date(a), Value::Date(b)) => a.cmp(b),
            // Storage order only; the language's comparison operators reject
            // complex values. Components are finite with canonical zeros.
            (Value::Complex(a), Value::Complex(b)) => a.re().total_cmp(&b.re()).then(a.im().total_cmp(&b.im())),
            (Value::Enum(a), Value::Enum(b)) => (a.ty, a.variant).cmp(&(b.ty, b.variant)),
            (Value::List(a), Value::List(b)) => a.cmp(b),
            (Value::Range(a, b), Value::Range(c, d)) => (a, b).cmp(&(c, d)),
            (Value::Map(a), Value::Map(b)) => a.cmp(b),
            (Value::Bag(a), Value::Bag(b)) => a.cmp(b),
            (Value::Record(a), Value::Record(b)) => a.cmp(b),
            (Value::Dist(a), Value::Dist(b)) => a.cmp(b),
            (Value::Closure(a), Value::Closure(b)) => a.cmp(b),
            (Value::Builtin(a), Value::Builtin(b)) => a.cmp(b),
            (Value::Continuous(a), Value::Continuous(b)) => family_key(a).cmp(&family_key(b)),
            (Value::Analytic(a), Value::Analytic(b)) => a.key().cmp(&b.key()),
            (Value::Event(a), Value::Event(b)) => a.key().cmp(&b.key()),
            (Value::Counts(a), Value::Counts(b)) => a.key().cmp(&b.key()),
            (Value::Delayed(a), Value::Delayed(b)) => {
                (a.variable, family_key(&a.family)).cmp(&(b.variable, family_key(&b.family)))
            }
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
        Value::Complex(z) => write!(f, "complex({}, {})", fmt_float(z.re()), fmt_float(z.im())),
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
        Value::Closure(_) | Value::Builtin(_) => write!(f, "<function>"),
        Value::Date(d) => write!(f, "{}", crate::dates::format(*d)),
        Value::Continuous(family) => write!(f, "{family}"),
        Value::Counts(c) => write!(f, "{c}"),
        Value::Analytic(a) => match a.affine() {
            Some(g) => write!(f, "<analytic float: {} * {} + {}>", g.scale, a.family, g.offset),
            None => write!(f, "<analytic float: piecewise in {}>", a.family),
        },
        Value::Event(_) => write!(f, "<analytic bool>"),
        Value::Delayed(d) => write!(f, "<not drawn yet: {}>", d.family),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bag(counts: &[(i64, u64)]) -> Multiset {
        Multiset::new(counts.iter().map(|&(v, n)| (Value::Int(v.into()), n)).collect())
    }

    #[test]
    fn a_bag_keeps_its_hash_up_to_date() {
        let full = bag(&[(1, 2), (2, 1), (3, 4)]);
        // Taking values out in any order gives the bag built directly.
        let taken = full
            .without(&Value::Int(3.into()))
            .unwrap()
            .without(&Value::Int(2.into()))
            .unwrap();
        let direct = bag(&[(1, 2), (3, 3)]);
        assert_eq!(taken, direct);
        assert_eq!(taken.sum, direct.sum);
        assert!(taken.without(&Value::Int(2.into())).is_none());
        // Moving a count from one value to another changes the hash.
        assert_ne!(bag(&[(1, 2), (2, 3)]).sum, bag(&[(1, 3), (2, 2)]).sum);
        assert_ne!(bag(&[(1, 1)]).sum, bag(&[(2, 1)]).sum);
    }

    #[test]
    fn an_unshared_copy_is_equal_and_separate() {
        let row = Value::record(Record {
            ty: Some(Arc::from("Day")),
            fields: vec![
                (Arc::from("n"), Value::Int(1.into())),
                (Arc::from("s"), Value::str("x")),
            ],
        });
        let original = Value::list(vec![row.clone(), row]);
        let copy = original.unshared();
        assert_eq!(copy, original);
        let (Value::List(a), Value::List(b)) = (&original, &copy) else {
            panic!()
        };
        assert!(!Arc::ptr_eq(a, b));
        let (Value::Record(x), Value::Record(y)) = (&a[0], &b[0]) else {
            panic!()
        };
        assert!(!Arc::ptr_eq(x, y));
        // Names are shared within the copy, not with the original.
        let Value::Record(y2) = &b[1] else { panic!() };
        assert!(Arc::ptr_eq(&y.fields[0].0, &y2.fields[0].0));
        assert!(!Arc::ptr_eq(&x.fields[0].0, &y.fields[0].0));
    }

    #[test]
    fn a_changed_collection_forgets_its_hash() {
        let mut list = Hashed::new(vec![Value::Int(1.into())]);
        let before = list.hash_code();
        list.push(Value::Int(2.into()));
        assert_ne!(list.hash_code(), before);
        assert_eq!(
            list.hash_code(),
            Hashed::new(vec![Value::Int(1.into()), Value::Int(2.into())]).hash_code()
        );
        assert_ne!(list, Hashed::new(vec![Value::Int(1.into())]));
    }
}
