//! Runtime values.
//!
//! Values have value semantics: collections are shared through `Arc` and
//! copied on write, so forking a world is cheap and worlds never affect each
//! other. Equality is *structural* and typed: it decides which worlds merge,
//! so `1` and `1.0` stay apart. The language's `==` compares numbers by
//! value at any depth ([`same`]), and map keys and bag items follow it: the
//! order puts values it finds the same next to each other, and the hash
//! agrees with it.

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
    Counts(Arc<crate::dist::Law>),
    /// A distribution whose outcomes own continuous draws, from `simulate`
    /// (docs/semantics.md, section 8): drawing from it gives each outcome's
    /// draws fresh identities (see `crate::joint`).
    Joint(Arc<crate::dist::Dist>),
    /// Sampling-only joint vector distribution with a shared covariance factor.
    MultivariateNormal(Arc<crate::multivariate::Normal>),
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

    pub(crate) fn known_hash(&self) -> u64 {
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
    pub(crate) fn capacity(&self) -> usize {
        self.entries.capacity()
    }
    /// From counts, leaving out zeros. No two items may be the same, as
    /// `1` and `1.0` are: [`add_count`] keeps them one.
    pub fn new(counts: BTreeMap<Value, u64>) -> Multiset {
        Multiset::from_sorted(counts.into_iter().filter(|(_, n)| *n > 0).collect())
    }

    /// From counts sorted by item, none of them zero.
    fn from_sorted(entries: Vec<(Value, u64)>) -> Multiset {
        debug_assert!(entries.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 > 0));
        debug_assert!(
            entries.windows(2).all(|w| !same(&w[0].0, &w[1].0)),
            "a bag with two items that are the same"
        );
        let sum = entries
            .iter()
            .fold(0u64, |sum, (v, n)| sum.wrapping_add(entry_hash(v, *n)));
        Multiset { entries, sum }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Value, &u64)> + Clone {
        self.entries.iter().map(|(v, n)| (v, n))
    }

    pub fn distinct_len(&self) -> usize {
        self.entries.len()
    }

    pub fn keys(&self) -> impl Iterator<Item = &Value> + Clone {
        self.entries.iter().map(|(v, _)| v)
    }

    pub fn values(&self) -> impl Iterator<Item = &u64> + Clone {
        self.entries.iter().map(|(_, n)| n)
    }

    /// Where the item that's the same as `v` is, if the bag has one: `1` and
    /// `1.0` are one item. Items the same as `v` sort next to it.
    fn position(&self, v: &Value) -> Option<usize> {
        let i = match self.entries.binary_search_by(|(k, _)| k.cmp(v)) {
            Ok(i) => return Some(i),
            Err(i) => i,
        };
        let is = |j: usize| self.entries.get(j).is_some_and(|(k, _)| same(k, v));
        if is(i) {
            return Some(i);
        }
        if !kinds_matter(v) || ints_only(v) {
            return None;
        }
        i.checked_sub(1).filter(|&j| is(j))
    }

    /// How many times `v` is in the bag, if it is.
    pub fn get(&self, v: &Value) -> Option<&u64> {
        Some(&self.entries[self.position(v)?].1)
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
        Some(self.without_nth(self.position(v)?))
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

#[derive(Clone, Debug)]
pub struct Closure {
    pub func: u32,
    pub captured: Vec<Value>,
    /// A named function, rather than a lambda. It reads its globals when
    /// it's called, so the values it holds don't make it a different
    /// function: `f == fee` after `let f = fee`, whatever has changed since.
    pub named: bool,
}

impl Closure {
    fn identity(&self) -> (u32, &[Value]) {
        (self.func, if self.named { &[] } else { &self.captured })
    }
}

impl PartialEq for Closure {
    fn eq(&self, other: &Closure) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Closure {}

impl Hash for Closure {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state)
    }
}

impl PartialOrd for Closure {
    fn partial_cmp(&self, other: &Closure) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Closure {
    fn cmp(&self, other: &Closure) -> Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Arc::from(s))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Arc::new(Hashed::new(items)))
    }

    /// No two keys may be the same, as `1` and `1.0` are: [`insert_key`]
    /// keeps them one.
    pub fn map(entries: BTreeMap<Value, Value>) -> Value {
        debug_assert!(
            entries.keys().zip(entries.keys().skip(1)).all(|(a, b)| !same(a, b)),
            "a map with two keys that are the same"
        );
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
            Value::Dist(d)
                if d.outcomes
                    .iter()
                    .any(|(v, _)| matches!(v, Value::Continuous(_) | Value::MultivariateNormal(_))) =>
            {
                "mixture of distributions".into()
            }
            Value::Dist(d) => match d.outcomes.first() {
                Some((v, _)) => format!("distribution over {}", plural_kind(&v.kind())),
                None => "distribution".into(),
            },
            Value::Continuous(f) => format!("{} distribution", f.name()),
            Value::MultivariateNormal(_) => "multivariate_normal distribution".into(),
            Value::Closure(_) | Value::Builtin(_) => "function".into(),
            Value::Date(_) => "date".into(),
            Value::Delayed(_) => "value not drawn yet".into(),
            Value::Analytic(a) if a.int => "int".into(),
            Value::Analytic(_) => "float".into(),
            Value::Event(_) => "bool".into(),
            Value::Counts(c) if c.floats => "distribution over floats".into(),
            Value::Counts(_) => "distribution over ints".into(),
            Value::Joint(d) => match d.outcomes.first() {
                Some((v, _)) => format!("distribution over {}", plural_kind(&v.kind())),
                None => "distribution".into(),
            },
        }
    }

    /// A distribution with outcomes that can be listed.
    pub fn is_dist(&self) -> bool {
        matches!(self, Value::Dist(_))
    }

    /// Any distribution, including continuous ones: not a settled value.
    pub fn is_uncertain(&self) -> bool {
        matches!(
            self,
            Value::Dist(_) | Value::Continuous(_) | Value::Counts(_) | Value::Joint(_) | Value::MultivariateNormal(_)
        )
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
            Value::Int(_) | Value::Float(_) | Value::Prob(_) | Value::Complex(_) => 3,
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
            Value::Analytic(_) => 17,
            Value::Event(_) => 18,
            Value::Counts(_) => 20,
            Value::Joint(_) => 21,
            Value::MultivariateNormal(_) => 22,
        }
    }

    /// Order of the number kinds, among numbers that are equal: an int, a
    /// probability, a float, then a complex number. `support` keeps the
    /// first of them.
    fn number_rank(&self) -> u8 {
        match self {
            Value::Int(_) => 0,
            Value::Prob(_) => 1,
            Value::Float(_) => 2,
            _ => 3,
        }
    }
}

/// Whether a value's kinds can tell it apart from one the language sees as
/// the same: a number, or something that can hold one.
pub(crate) fn kinds_matter(v: &Value) -> bool {
    matches!(
        v,
        Value::Int(_)
            | Value::Float(_)
            | Value::Prob(_)
            | Value::Complex(_)
            | Value::List(_)
            | Value::Map(_)
            | Value::Bag(_)
            | Value::Record(_)
    )
}

/// Whether the language sees two values as the same, as `==` does: numbers
/// by value, at any depth, whatever their kinds. `[1] == [1.0]`, and `1`
/// and `1.0` are one map key.
pub fn same(a: &Value, b: &Value) -> bool {
    a.order(b).0.is_eq()
}

/// Whether every number in `v` is an int. Ints come first among numbers
/// that are equal, so `v` then sorts first among the values the language
/// sees as the same as it: only the next key can be one.
fn ints_only(v: &Value) -> bool {
    match v {
        Value::Float(_) | Value::Prob(_) | Value::Complex(_) => false,
        Value::List(xs) => xs.iter().all(ints_only),
        Value::Map(m) => m.iter().all(|(k, v)| ints_only(k) && ints_only(v)),
        Value::Bag(b) => b.keys().all(ints_only),
        Value::Record(r) => r.fields.iter().all(|(_, v)| ints_only(v)),
        _ => true,
    }
}

/// The key in `m` that's the same as `k`, if there is one. Keys the same
/// as `k` sort next to it, and a map has at most one.
pub fn key_in<'a, V>(m: &'a BTreeMap<Value, V>, k: &Value) -> Option<&'a Value> {
    if let Some((key, _)) = m.range(k..).next().filter(|(key, _)| same(key, k)) {
        return Some(key);
    }
    if !kinds_matter(k) || ints_only(k) {
        return None;
    }
    m.range(..k).next_back().map(|(key, _)| key).filter(|key| same(key, k))
}

/// The value at the key that's the same as `k`.
pub fn get_key<'a, V>(m: &'a BTreeMap<Value, V>, k: &Value) -> Option<&'a V> {
    m.get(key_in(m, k)?)
}

pub fn get_key_mut<'a, V>(m: &'a mut BTreeMap<Value, V>, k: &Value) -> Option<&'a mut V> {
    let key = key_in(m, k)?.clone();
    m.get_mut(&key)
}

/// Sets the value at `k`. A key that's the same as `k` already there stays,
/// as it does when a literal repeats a key: `[1: "a", 1.0: "b"]` is
/// `[1: "b"]`.
pub fn insert_key<V>(m: &mut BTreeMap<Value, V>, k: Value, v: V) -> Option<V> {
    match get_key_mut(m, &k) {
        Some(slot) => Some(std::mem::replace(slot, v)),
        None => m.insert(k, v),
    }
}

pub fn remove_key<V>(m: &mut BTreeMap<Value, V>, k: &Value) -> Option<V> {
    let key = key_in(m, k)?.clone();
    m.remove(&key)
}

/// A bag of `items`, in order: an item the same as one before it, like
/// `1.0` after `1`, counts as that one. The items are sorted, which puts
/// those the language sees as the same together, and then counted.
pub fn count_items<'a>(items: impl Iterator<Item = &'a Value>) -> Multiset {
    let mut sorted: Vec<(&Value, usize)> = items.enumerate().map(|(i, v)| (v, i)).collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut entries: Vec<(Value, u64)> = Vec::new();
    // The item that came first among those the same as each other, where
    // it came, and how many there are.
    let mut run: Option<(&Value, usize, u64)> = None;
    for (v, i) in sorted {
        match &mut run {
            Some((first, at, total)) if same(first, v) => {
                if i < *at {
                    (*first, *at) = (v, i);
                }
                *total += 1;
            }
            _ => {
                if let Some((first, _, total)) = run.replace((v, i, 1)) {
                    entries.push((first.clone(), total));
                }
            }
        }
    }
    if let Some((first, _, total)) = run {
        entries.push((first.clone(), total));
    }
    Multiset::from_sorted(entries)
}

/// Adds `n` of `item` to a bag's counts, as the item already there if one
/// is the same: `bag([1, 1.0])` holds two of `1`.
pub fn add_count(counts: &mut BTreeMap<Value, u64>, item: Value, n: u64) -> Option<()> {
    match get_key_mut(counts, &item) {
        Some(count) => *count = count.checked_add(n)?,
        None => {
            counts.insert(item, n);
        }
    }
    Some(())
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
        (Value::MultivariateNormal(a), Value::MultivariateNormal(b)) => {
            Arc::ptr_eq(a, b) || a.identity().eq(b.identity())
        }
        (Value::Analytic(a), Value::Analytic(b)) => a.key() == b.key(),
        (Value::Event(a), Value::Event(b)) => a.key() == b.key(),
        (Value::Counts(a), Value::Counts(b)) => a.key() == b.key(),
        (Value::Joint(a), Value::Joint(b)) => Arc::ptr_eq(a, b) || a == b,
        (Value::Delayed(a), Value::Delayed(b)) => {
            a.variable == b.variable && family_key(&a.family) == family_key(&b.family)
        }
        _ => false,
    }
}

impl Eq for Value {}

/// [`same`], counting a unit of work for each pair of elements of
/// collections it compares, and taking `Value`'s `==` shortcuts: hashes
/// agree with both.
pub fn eq_counted(a: &Value, b: &Value, budget: &mut crate::dist::Budget) -> crate::error::OpResult<bool> {
    // Different hashes, both known, decide it, as `Hashed`'s `==` does.
    fn differ<T>(x: &Hashed<T>, y: &Hashed<T>) -> bool {
        let (p, q) = (x.known_hash(), y.known_hash());
        p != 0 && q != 0 && p != q
    }
    fn same(p: &Value, q: &Value, budget: &mut crate::dist::Budget) -> crate::error::OpResult<bool> {
        match p {
            Value::List(_) | Value::Map(_) | Value::Bag(_) | Value::Record(_) => eq_counted(p, q, budget),
            _ => Ok(crate::value::same(p, q)),
        }
    }
    // Elements are counted as they're compared, then charged together: a
    // charge for each costs as much as comparing two ints.
    let mut compared = 0;
    let mut equal = true;
    match (a, b) {
        (Value::List(x), Value::List(y)) => {
            if Arc::ptr_eq(x, y) {
                return Ok(true);
            }
            if x.len() != y.len() || differ(x, y) {
                return Ok(false);
            }
            for (p, q) in x.iter().zip(y.iter()) {
                compared += 1;
                if !same(p, q, budget)? {
                    equal = false;
                    break;
                }
            }
        }
        (Value::Map(x), Value::Map(y)) => {
            if Arc::ptr_eq(x, y) {
                return Ok(true);
            }
            if x.len() != y.len() || differ(x, y) {
                return Ok(false);
            }
            for ((k, v), (l, u)) in x.iter().zip(y.iter()) {
                compared += 1;
                if !same(k, l, budget)? || !same(v, u, budget)? {
                    equal = false;
                    break;
                }
            }
        }
        (Value::Bag(x), Value::Bag(y)) => {
            if Arc::ptr_eq(x, y) {
                return Ok(true);
            }
            if x.sum != y.sum || x.entries.len() != y.entries.len() || differ(x, y) {
                return Ok(false);
            }
            for ((v, n), (u, m)) in x.entries.iter().zip(y.entries.iter()) {
                compared += 1;
                if n != m || !same(v, u, budget)? {
                    equal = false;
                    break;
                }
            }
        }
        (Value::Record(x), Value::Record(y)) => {
            if Arc::ptr_eq(x, y) {
                return Ok(true);
            }
            if x.ty != y.ty || x.fields.len() != y.fields.len() || differ(x, y) {
                return Ok(false);
            }
            for ((n, v), (m, u)) in x.fields.iter().zip(y.fields.iter()) {
                compared += 1;
                if n != m || !same(v, u, budget)? {
                    equal = false;
                    break;
                }
            }
        }
        _ => return Ok(crate::value::same(a, b)),
    }
    budget.work(compared)?;
    Ok(equal)
}

/// What a number's hash starts with, whatever its kind.
const NUMBER: u8 = u8::MAX;

impl Hash for Value {
    /// Like equality: small values inlined, the rest out of line. Numbers
    /// the language sees as the same, like `1`, `1.0` and `complex(1, 0)`,
    /// hash alike, and so do values that hold them: different hashes then
    /// tell `==` that two collections differ, as they tell `Value`'s.
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Int(i) => {
                NUMBER.hash(state);
                i.hash(state);
            }
            Value::Dead | Value::Unit => std::mem::discriminant(self).hash(state),
            Value::Bool(b) => {
                std::mem::discriminant(self).hash(state);
                b.hash(state);
            }
            other => hash_other(other, state),
        }
    }
}

/// A number other than an int: as an int if it's a whole real number.
fn hash_number<H: Hasher>(re: f64, im: f64, state: &mut H) {
    NUMBER.hash(state);
    if float_key(im) != 0 {
        return (float_key(re), float_key(im)).hash(state);
    }
    if re.is_finite() && re.fract() == 0.0 {
        if let Some(n) = probl_number::Integer::from_f64(re) {
            return n.hash(state);
        }
    }
    float_key(re).hash(state)
}

#[inline(never)]
fn hash_other<H: Hasher>(value: &Value, state: &mut H) {
    match value {
        Value::Dead | Value::Unit | Value::Bool(_) | Value::Int(_) => {}
        Value::Float(f) | Value::Prob(f) => return hash_number(*f, 0.0, state),
        Value::Complex(z) => return hash_number(z.re(), z.im(), state),
        _ => {}
    }
    std::mem::discriminant(value).hash(state);
    match value {
        Value::Dead | Value::Unit | Value::Bool(_) | Value::Int(_) => {}
        Value::Float(_) | Value::Prob(_) | Value::Complex(_) => {}
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
        Value::MultivariateNormal(n) => n.identity().for_each(|x| x.hash(state)),
        Value::Analytic(a) => a.key().hash(state),
        Value::Event(a) => a.key().hash(state),
        Value::Delayed(d) => (d.variable, family_key(&d.family)).hash(state),
        Value::Counts(c) => c.key().hash(state),
        Value::Joint(d) => d.hash(state),
    }
}

impl Ord for Value {
    #[inline]
    fn cmp(&self, other: &Value) -> Ordering {
        // Scalar report keys/populations dominate comparisons during sampling.
        // Keep their order checks small enough to inline into tree lookups.
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) | (Value::Prob(a), Value::Prob(b)) => {
                f64::from_bits(float_key(*a)).total_cmp(&f64::from_bits(float_key(*b)))
            }
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Date(a), Value::Date(b)) => a.cmp(b),
            _ => self.cmp_other(other),
        }
    }
}

impl Value {
    #[inline(never)]
    fn cmp_other(&self, other: &Value) -> Ordering {
        let (by_value, by_kind) = self.order(other);
        by_value.then(by_kind)
    }

    /// How two values compare: first as the language sees them, with
    /// numbers by value at any depth, whatever their kinds; then, between
    /// values it sees as the same, like `[1]` and `[1.0]`, by those kinds.
    /// So values the language sees as the same sort next to each other,
    /// which map keys and bag items rely on, and the order still tells
    /// apart every two values that `==` on `Value` does.
    pub(crate) fn order(&self, other: &Value) -> (Ordering, Ordering) {
        // Elements of collections are mostly scalars of one kind: compare
        // those here, as `cmp` does, before working out kinds.
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => return (a.cmp(b), Ordering::Equal),
            (Value::Float(a), Value::Float(b)) | (Value::Prob(a), Value::Prob(b)) => {
                let o = f64::from_bits(float_key(*a)).total_cmp(&f64::from_bits(float_key(*b)));
                return (o, Ordering::Equal);
            }
            (Value::Str(a), Value::Str(b)) => return (a.cmp(b), Ordering::Equal),
            (Value::Bool(a), Value::Bool(b)) => return (a.cmp(b), Ordering::Equal),
            (Value::Date(a), Value::Date(b)) => return (a.cmp(b), Ordering::Equal),
            _ => {}
        }
        let (ra, rb) = (self.rank(), other.rank());
        if ra != rb {
            return (ra.cmp(&rb), Ordering::Equal);
        }
        let by_value = |o: Ordering| (o, Ordering::Equal);
        match (self, other) {
            (a, b) if ra == 3 => (number_order(a, b), a.number_rank().cmp(&b.number_rank())),
            (Value::List(a), Value::List(b)) => {
                if Arc::ptr_eq(a, b) {
                    return (Ordering::Equal, Ordering::Equal);
                }
                in_order(a.iter().zip(b.iter()).map(|(p, q)| p.order(q)), a.len().cmp(&b.len()))
            }
            (Value::Map(a), Value::Map(b)) => {
                if Arc::ptr_eq(a, b) {
                    return (Ordering::Equal, Ordering::Equal);
                }
                in_order(
                    a.iter()
                        .zip(b.iter())
                        .map(|((k, v), (l, u))| then(k.order(l), || v.order(u))),
                    a.len().cmp(&b.len()),
                )
            }
            (Value::Bag(a), Value::Bag(b)) => {
                if Arc::ptr_eq(a, b) {
                    return (Ordering::Equal, Ordering::Equal);
                }
                in_order(
                    a.entries
                        .iter()
                        .zip(b.entries.iter())
                        .map(|((v, n), (u, m))| then(v.order(u), || by_value(n.cmp(m)))),
                    a.entries.len().cmp(&b.entries.len()),
                )
            }
            (Value::Record(a), Value::Record(b)) => {
                if Arc::ptr_eq(a, b) {
                    return (Ordering::Equal, Ordering::Equal);
                }
                then(by_value(a.ty.cmp(&b.ty)), || {
                    in_order(
                        a.fields
                            .iter()
                            .zip(b.fields.iter())
                            .map(|((n, v), (m, u))| then(by_value(n.cmp(m)), || v.order(u))),
                        a.fields.len().cmp(&b.fields.len()),
                    )
                })
            }
            (Value::Bool(a), Value::Bool(b)) => by_value(a.cmp(b)),
            (Value::Str(a), Value::Str(b)) => by_value(a.cmp(b)),
            (Value::Date(a), Value::Date(b)) => by_value(a.cmp(b)),
            (Value::Enum(a), Value::Enum(b)) => by_value((a.ty, a.variant).cmp(&(b.ty, b.variant))),
            (Value::Range(a, b), Value::Range(c, d)) => by_value((a, b).cmp(&(c, d))),
            (Value::Dist(a), Value::Dist(b)) => by_value(a.cmp(b)),
            (Value::Closure(a), Value::Closure(b)) => by_value(a.cmp(b)),
            (Value::Builtin(a), Value::Builtin(b)) => by_value(a.cmp(b)),
            (Value::Continuous(a), Value::Continuous(b)) => by_value(family_key(a).cmp(&family_key(b))),
            (Value::MultivariateNormal(a), Value::MultivariateNormal(b)) => by_value(a.identity().cmp(b.identity())),
            (Value::Analytic(a), Value::Analytic(b)) => by_value(a.key().cmp(&b.key())),
            (Value::Event(a), Value::Event(b)) => by_value(a.key().cmp(&b.key())),
            (Value::Counts(a), Value::Counts(b)) => by_value(a.key().cmp(&b.key())),
            (Value::Joint(a), Value::Joint(b)) => by_value(a.cmp(b)),
            (Value::Delayed(a), Value::Delayed(b)) => {
                by_value((a.variable, family_key(&a.family)).cmp(&(b.variable, family_key(&b.family))))
            }
            _ => (Ordering::Equal, Ordering::Equal),
        }
    }
}

/// Two numbers by value, whatever their kinds: their real parts, then their
/// imaginary parts, which are 0 for an int, a float or a probability. The
/// language's comparison operators reject complex values; this is storage
/// order. Components are finite with canonical zeros.
fn number_order(a: &Value, b: &Value) -> Ordering {
    fn parts(v: &Value) -> (f64, f64) {
        match v {
            Value::Float(x) | Value::Prob(x) => (f64::from_bits(float_key(*x)), 0.0),
            Value::Complex(z) => (z.re(), z.im()),
            _ => (0.0, 0.0),
        }
    }
    let re = match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Int(x), _) => x.cmp_f64(parts(b).0).unwrap_or(Ordering::Less),
        (_, Value::Int(y)) => y.cmp_f64(parts(a).0).unwrap_or(Ordering::Less).reverse(),
        _ => parts(a).0.total_cmp(&parts(b).0),
    };
    re.then_with(|| parts(a).1.total_cmp(&parts(b).1))
}

/// `a`, or `b` if `a` sees two values as the same.
fn then(a: (Ordering, Ordering), b: impl FnOnce() -> (Ordering, Ordering)) -> (Ordering, Ordering) {
    if a.0.is_ne() {
        return a;
    }
    let b = b();
    (b.0, if b.0.is_eq() { a.1.then(b.1) } else { Ordering::Equal })
}

/// Sequences in lexicographic order, then by length: the first part that
/// differs in value decides, and between sequences the language sees as the
/// same, the first that differs in kind.
fn in_order(parts: impl Iterator<Item = (Ordering, Ordering)>, lengths: Ordering) -> (Ordering, Ordering) {
    let mut by_kind = Ordering::Equal;
    for (v, k) in parts {
        if v.is_ne() {
            return (v, Ordering::Equal);
        }
        by_kind = by_kind.then(k);
    }
    if lengths.is_ne() {
        return (lengths, Ordering::Equal);
    }
    (Ordering::Equal, by_kind)
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
        Value::Dist(d) | Value::Joint(d) => {
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
        Value::MultivariateNormal(normal) => {
            fn row(values: &[f64], f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "[")?;
                for (i, x) in values.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", fmt_float(*x))?;
                }
                write!(f, "]")
            }
            write!(f, "multivariate_normal(")?;
            row(normal.means(), f)?;
            write!(f, ", [")?;
            for (i, values) in normal.factor().covariance().chunks(normal.means().len()).enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                row(values, f)?;
            }
            write!(f, "])")
        }
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

    #[test]
    fn scalar_storage_order_preserves_mixed_types_and_float_keys() {
        let mut values = vec![
            Value::Unit,
            Value::Bool(false),
            Value::Bool(true),
            Value::Int((-1).into()),
            Value::Int(0.into()),
            Value::Int(u64::MAX.into()),
            Value::Date(-1),
            Value::Date(0),
            Value::str("0"),
            Value::list(vec![]),
        ];
        for x in [
            f64::NEG_INFINITY,
            -1.0,
            -0.0,
            0.0,
            f64::from_bits(1),
            1.0,
            f64::INFINITY,
            f64::NAN,
            f64::from_bits(0xfff8000000000001),
        ] {
            values.push(Value::Float(x));
            values.push(Value::Prob(x));
        }
        for a in &values {
            for b in &values {
                assert_eq!(a.cmp(b), a.cmp_other(b));
                assert_eq!(a.cmp(b).is_eq(), a == b);
            }
        }
    }

    fn hash_of(v: &Value) -> u64 {
        let mut hasher = FxHasher::default();
        v.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn values_the_language_sees_as_the_same_sort_together_and_hash_alike() {
        let int = |n: i64| Value::Int(n.into());
        let complex = |re: f64, im: f64| Value::Complex(Complex::new(re, im).unwrap());
        let list = |xs: Vec<Value>| Value::list(xs);
        let record = |x: Value| {
            Value::record(Record {
                ty: None,
                fields: vec![(Arc::from("x"), x)],
            })
        };
        let map = |k: Value, v: Value| Value::map([(k, v)].into_iter().collect());
        let big = probl_number::Integer::from_f64(2f64.powi(60)).unwrap();
        let values = vec![
            int(1),
            Value::Float(1.0),
            Value::Prob(1.0),
            complex(1.0, 0.0),
            complex(1.0, 1.0),
            int(2),
            Value::Float(2.0),
            Value::Float(-0.0),
            int(0),
            Value::Float(0.5),
            Value::Prob(0.5),
            Value::Int(big.clone()),
            Value::Int(big.add(&probl_number::Integer::ONE).unwrap()),
            Value::Float(2f64.powi(60)),
            Value::str("1"),
            list(vec![int(1)]),
            list(vec![Value::Float(1.0)]),
            list(vec![int(1), int(5)]),
            list(vec![Value::Float(1.0), int(2)]),
            list(vec![Value::Float(1.0), Value::Float(5.0)]),
            list(vec![int(1), int(2), int(3)]),
            list(vec![]),
            record(int(1)),
            record(Value::Float(1.0)),
            map(int(1), Value::str("a")),
            map(Value::Float(1.0), Value::str("a")),
            map(int(1), Value::str("b")),
            Value::bag([(int(1), 2)].into_iter().collect()),
            Value::bag([(Value::Float(1.0), 2)].into_iter().collect()),
        ];
        for a in &values {
            for b in &values {
                // The order tells apart what `Value`'s `==` does, and is antisymmetric.
                assert_eq!(a.cmp(b).is_eq(), a == b, "{a:?} {b:?}");
                assert_eq!(a.cmp(b), b.cmp(a).reverse(), "{a:?} {b:?}");
                assert_eq!(same(a, b), a.order(b).0.is_eq(), "{a:?} {b:?}");
                if same(a, b) {
                    assert_eq!(hash_of(a), hash_of(b), "{a:?} {b:?}");
                }
            }
        }
        assert!(same(&values[0], &values[3]) && !same(&values[0], &values[4]));
        assert!(same(&values[15], &values[16]) && same(&values[17], &values[19]));
        assert!(!same(&values[11], &values[12]) && same(&values[11], &values[13]));
        // Sorted, the values the language sees as the same are next to each other.
        let mut sorted = values.clone();
        sorted.sort();
        for (i, a) in sorted.iter().enumerate() {
            let group: Vec<usize> = (0..sorted.len()).filter(|&j| same(a, &sorted[j])).collect();
            assert_eq!(group, (group[0]..=group[group.len() - 1]).collect::<Vec<_>>(), "{a:?}");
            assert!(group.contains(&i));
        }
        // Lookups find a key or an item by any value that's the same.
        let keys: BTreeMap<Value, Value> = [
            (list(vec![int(1), int(5)]), Value::str("a")),
            (list(vec![Value::Float(1.0), int(2)]), Value::str("b")),
            (int(7), Value::str("c")),
        ]
        .into_iter()
        .collect();
        let found = |k: Value| get_key(&keys, &k).cloned();
        assert_eq!(
            found(list(vec![Value::Float(1.0), Value::Float(5.0)])),
            Some(Value::str("a"))
        );
        assert_eq!(found(list(vec![int(1), Value::Prob(1.0)])), None);
        assert_eq!(found(list(vec![int(1), int(2)])), Some(Value::str("b")));
        assert_eq!(found(complex(7.0, 0.0)), Some(Value::str("c")));
        let mut m = keys.clone();
        insert_key(&mut m, Value::Float(7.0), Value::str("d"));
        assert_eq!(m.len(), 3);
        assert_eq!(m.get(&int(7)), Some(&Value::str("d")));
        assert_eq!(remove_key(&mut m, &Value::Prob(7.0)), Some(Value::str("d")));
        let mut counts = BTreeMap::new();
        for item in [Value::Float(1.0), int(1), Value::Prob(1.0), int(2)] {
            add_count(&mut counts, item, 1).unwrap();
        }
        let b = Multiset::new(counts);
        assert_eq!(b.distinct_len(), 2);
        assert_eq!(b.get(&complex(1.0, 0.0)), Some(&3));
        assert!(b.keys().next().is_some_and(|k| matches!(k, Value::Float(_))));
        assert_eq!(b.without(&Value::Float(2.0)).map(|b| b.distinct_len()), Some(1));
    }

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
