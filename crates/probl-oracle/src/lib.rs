//! An independent reference interpreter for part of Probl, used to check the
//! engine against docs/semantics.md (audit finding I4).
//!
//! It is deliberately naive, so that it can be checked by reading it:
//!
//! - it walks the syntax tree, and shares nothing with the engine but the
//!   parser;
//! - every world is kept on its own: no merging, no liveness analysis, and no
//!   memoization;
//! - operands are evaluated left to right, one world at a time;
//! - weights are exact fractions.
//!
//! It covers the discrete, bounded part of the language: ints, facts,
//! probabilities, dice, `bernoulli`, `one_of`, lists, functions, `simulate`,
//! `observe`, `report`, and loops that end. The number of worlds grows with
//! every split, so it gives up ([`Stop::TooBig`]) beyond its limits. Its
//! reference integer arithmetic is limited to i64; the engine's arbitrary-
//! precision cases have separate arithmetic and language regression tests.

pub mod generate;

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use probl_syntax::ast::{
    self, AssignOp, BinOp, BindOp, Block, ChanceArm, Expr, ExprKind, Item, MatchArm, Pattern, PatternKind, Stmt,
    StmtKind, StrSegment, UnOp,
};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

/// An exact probability or weight.
pub type Q = BigRational;

fn int(n: i64) -> Q {
    Q::from_integer(BigInt::from(n))
}

// ── Values ───────────────────────────────────────────────────────────────

/// Equality is structural: an int and a float are different values, as they
/// are in the engine's reports. The language's `==` is [`equals`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(Q),
    Prob(Q),
    Str(Rc<str>),
    List(Rc<Vec<Value>>),
    /// Integers from `.0` to `.1`, both included.
    Range(i64, i64),
    /// Distinct outcomes in order, with positive probabilities adding up to 1.
    Dist(Rc<Vec<(Value, Q)>>),
}

/// A distribution from weighted outcomes. Outcomes that are distributions
/// themselves are mixed in.
fn dist(pairs: Vec<(Value, Q)>) -> R<Value> {
    let mut outcomes: BTreeMap<Value, Q> = BTreeMap::new();
    for (v, p) in pairs {
        match v {
            Value::Dist(d) => {
                for (x, r) in d.iter() {
                    *outcomes.entry(x.clone()).or_insert_with(Q::zero) += &p * r;
                }
            }
            other => *outcomes.entry(other).or_insert_with(Q::zero) += p,
        }
    }
    outcomes.retain(|_, p| !p.is_zero());
    if outcomes.is_empty() {
        return err("the result would be an empty distribution");
    }
    Ok(Value::Dist(Rc::new(outcomes.into_iter().collect())))
}

fn uniform(items: Vec<Value>) -> R<Value> {
    let p = Q::new(BigInt::one(), BigInt::from(items.len()));
    dist(items.into_iter().map(|v| (v, p.clone())).collect())
}

fn dice(count: u32, sides: u32) -> R<Value> {
    if count == 0 || sides == 0 {
        return unsupported("dice with no sides or no dice");
    }
    if count as u64 * sides as u64 > 1000 {
        return too_big("dice with too many outcomes");
    }
    let mut sums: BTreeMap<i64, Q> = BTreeMap::from([(0, Q::one())]);
    let face = Q::new(BigInt::one(), BigInt::from(sides));
    for _ in 0..count {
        let mut next: BTreeMap<i64, Q> = BTreeMap::new();
        for (s, p) in &sums {
            for f in 1..=sides as i64 {
                *next.entry(s + f).or_insert_with(Q::zero) += p * &face;
            }
        }
        sums = next;
    }
    dist(sums.into_iter().map(|(s, p)| (Value::Int(s), p)).collect())
}

/// A key for comparing values with the engine's: the kind and the value,
/// with numbers to ten significant digits.
pub fn canon(v: &Value) -> String {
    match v {
        Value::Unit => "()".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(x) => format!("float {}", digits(x)),
        Value::Prob(x) => format!("prob {}", digits(x)),
        Value::Str(s) => format!("{:?}", &**s),
        Value::List(items) => format!("[{}]", items.iter().map(canon).collect::<Vec<_>>().join(", ")),
        Value::Range(lo, hi) => format!("{lo}..{hi}"),
        Value::Dist(d) => {
            let mut parts: Vec<String> = d.iter().map(|(x, p)| format!("{}: {}", canon(x), digits(p))).collect();
            parts.sort();
            format!("dist({})", parts.join(", "))
        }
    }
}

fn digits(x: &Q) -> String {
    format!("{:.9e}", x.to_f64().unwrap_or(f64::NAN))
}

// ── Results ──────────────────────────────────────────────────────────────

/// Why a run didn't produce a measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The program is wrong; the engine must reject it too.
    Error(String),
    /// The program uses something this interpreter doesn't implement.
    Unsupported(String),
    /// The program needs more than the limits allow.
    TooBig(String),
}

type R<T> = Result<T, Stop>;

fn err<T>(message: impl Into<String>) -> R<T> {
    Err(Stop::Error(message.into()))
}

fn unsupported<T>(what: impl Into<String>) -> R<T> {
    Err(Stop::Unsupported(what.into()))
}

fn too_big<T>(what: impl Into<String>) -> R<T> {
    Err(Stop::TooBig(what.into()))
}

/// What one `report` saw for one key: the weight that reached it, and the
/// weight of each value (the outcomes of a reported distribution count with
/// their probabilities).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub total: Q,
    pub values: BTreeMap<String, Q>,
}

/// The unnormalized result of a program.
#[derive(Clone, Debug)]
pub struct Measure {
    /// Whether any `observe` ran outside of `simulate`.
    pub observed: bool,
    /// The total weight of the worlds that finished: the evidence.
    pub total: Q,
    /// For each report, by the byte range of its statement: the groups by
    /// key (`()` without `by`), keyed with [`canon`].
    pub reports: BTreeMap<(u32, u32), BTreeMap<String, Group>>,
}

#[derive(Clone, Debug)]
pub struct Limits {
    /// Evaluation steps, summed over all worlds.
    pub max_steps: u64,
    /// Worlds alive at once in a loop or at the top level.
    pub max_worlds: usize,
    /// Iterations of one `while` or `loop`.
    pub max_iterations: usize,
    /// Nested calls.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_steps: 2_000_000,
            max_worlds: 50_000,
            max_iterations: 1_000,
            max_depth: 40,
        }
    }
}

/// Parse and run a program.
pub fn run_source(src: &str, limits: &Limits) -> Result<Measure, Stop> {
    let (program, diags) = probl_syntax::parse_program(src);
    if diags.iter().any(|d| d.severity == probl_syntax::Severity::Error) {
        return unsupported("a program that doesn't parse");
    }
    run(&program, src, limits)
}

pub fn run(program: &ast::Program, src: &str, limits: &Limits) -> Result<Measure, Stop> {
    for pragma in &program.pragmas {
        if pragma.name.name != "mode" {
            return unsupported(format!("`@{}`", pragma.name.name));
        }
    }
    let mut it = Interp {
        src,
        fns: HashMap::new(),
        limits: limits.clone(),
        steps: 0,
        depth: 0,
        in_simulate: 0,
        observed: false,
        reports: BTreeMap::new(),
    };
    for item in &program.items {
        match item {
            Item::Fn(decl) => {
                if decl.ret.is_some() || decl.params.iter().any(|p| p.ty.is_some()) {
                    return unsupported("type annotations");
                }
                it.fns.insert(decl.name.name.as_str(), decl);
            }
            Item::Stmt(_) => {}
            _ => return unsupported("type, enum and import declarations"),
        }
    }

    let mut worlds = vec![World {
        weight: Q::one(),
        scopes: vec![Vec::new()],
        globals: None,
    }];
    for item in &program.items {
        let Item::Stmt(stmt) = item else { continue };
        let mut next = Vec::new();
        for w in worlds {
            for (w, r) in it.stmt(stmt, w, false)? {
                match r {
                    Ok(_) => next.push(w),
                    Err(_) => return unsupported("`break`, `continue` or `return` at the top level"),
                }
            }
        }
        it.check_worlds(next.len())?;
        worlds = next;
    }
    let total = worlds.iter().fold(Q::zero(), |acc, w| acc + &w.weight);
    if it.observed && total.is_zero() {
        return err("the evidence is impossible");
    }
    Ok(Measure {
        observed: it.observed,
        total,
        reports: it.reports,
    })
}

// ── Worlds ───────────────────────────────────────────────────────────────

type Scope = Vec<(String, Value)>;

#[derive(Clone, Debug)]
struct World {
    weight: Q,
    /// The current function's variables, innermost scope last.
    scopes: Vec<Scope>,
    /// In a function: the top-level variables, copied when it was called.
    globals: Option<Rc<Scope>>,
}

impl World {
    fn get(&self, name: &str) -> Option<&Value> {
        fn find<'s>(scope: &'s Scope, name: &str) -> Option<&'s Value> {
            scope.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
        }
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| find(scope, name))
            .or_else(|| find(self.globals.as_deref()?, name))
    }

    fn set(&mut self, name: &str, value: Value) -> R<()> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some((_, v)) = scope.iter_mut().rev().find(|(n, _)| n == name) {
                *v = value;
                return Ok(());
            }
        }
        unsupported(format!("assigning `{name}`, which isn't a variable of this function"))
    }

    fn declare(&mut self, name: &str, value: Value) {
        self.scopes.last_mut().expect("a scope").push((name.to_string(), value));
    }

    fn scaled(&self, p: &Q) -> World {
        World {
            weight: &self.weight * p,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug)]
enum Flow {
    Break,
    Continue,
    Return(Value),
}

/// The worlds an expression or statement ends in, each with its value or
/// the way it left (`break`, `continue`, `return`).
type Outs = Vec<(World, Result<Value, Flow>)>;
type ListOuts = Vec<(World, Result<Vec<Value>, Flow>)>;

// ── The interpreter ──────────────────────────────────────────────────────

struct Interp<'a> {
    src: &'a str,
    fns: HashMap<&'a str, &'a ast::FnDecl>,
    limits: Limits,
    steps: u64,
    depth: usize,
    in_simulate: usize,
    observed: bool,
    reports: BTreeMap<(u32, u32), BTreeMap<String, Group>>,
}

impl<'a> Interp<'a> {
    fn tick(&mut self) -> R<()> {
        self.steps += 1;
        if self.steps > self.limits.max_steps {
            return too_big("too many steps");
        }
        Ok(())
    }

    fn check_worlds(&self, n: usize) -> R<()> {
        if n > self.limits.max_worlds {
            return too_big("too many worlds");
        }
        Ok(())
    }

    /// Continue each world that has a value; pass the others on.
    fn each(&mut self, outs: Outs, mut f: impl FnMut(&mut Self, World, Value) -> R<Outs>) -> R<Outs> {
        let mut result = Vec::with_capacity(outs.len());
        for (w, r) in outs {
            match r {
                Ok(v) => result.extend(f(self, w, v)?),
                Err(flow) => result.push((w, Err(flow))),
            }
        }
        Ok(result)
    }

    fn each_list(&mut self, outs: ListOuts, mut f: impl FnMut(&mut Self, World, Vec<Value>) -> R<Outs>) -> R<Outs> {
        let mut result = Vec::with_capacity(outs.len());
        for (w, r) in outs {
            match r {
                Ok(vs) => result.extend(f(self, w, vs)?),
                Err(flow) => result.push((w, Err(flow))),
            }
        }
        Ok(result)
    }

    /// Evaluate expressions left to right, each in the worlds the previous
    /// one left (docs/semantics.md, section 4).
    fn list(&mut self, exprs: &[&'a Expr], w: World) -> R<ListOuts> {
        let mut acc: ListOuts = vec![(w, Ok(Vec::new()))];
        for e in exprs {
            let mut next = Vec::new();
            for (w, vals) in acc {
                match vals {
                    Err(flow) => next.push((w, Err(flow))),
                    Ok(vals) => {
                        for (w, r) in self.expr(e, w, true)? {
                            next.push((
                                w,
                                r.map(|v| {
                                    let mut vs = vals.clone();
                                    vs.push(v);
                                    vs
                                }),
                            ));
                        }
                    }
                }
            }
            acc = next;
        }
        Ok(acc)
    }

    fn map_list(&mut self, exprs: &[&'a Expr], w: World, f: impl Fn(Vec<Value>) -> R<Value>) -> R<Outs> {
        let outs = self.list(exprs, w)?;
        self.each_list(outs, |_, w, vs| Ok(vec![(w, Ok(f(vs)?))]))
    }

    // ── Statements ───────────────────────────────────────────────────────

    /// Run a statement. With `want`, its value is used (it's the last
    /// statement of a block whose value is used).
    fn stmt(&mut self, s: &'a Stmt, w: World, want: bool) -> R<Outs> {
        self.tick()?;
        let done = |w: World| -> Outs { vec![(w, Ok(Value::Unit))] };
        match &s.kind {
            StmtKind::Expr(e) => self.expr(e, w, want),
            StmtKind::Let {
                pattern, ty, op, value, ..
            } => {
                if ty.is_some() {
                    return unsupported("type annotations");
                }
                let name = match &pattern.kind {
                    PatternKind::Name(n) => Some(n.as_str()),
                    PatternKind::Wildcard => None,
                    _ => return unsupported("patterns in `let`"),
                };
                let draw = *op == BindOp::Draw;
                let outs = self.expr(value, w, true)?;
                self.each(outs, |_, w, v| {
                    let mut out = Vec::new();
                    for (mut w, x) in bind(w, v, draw)? {
                        if let Some(n) = name {
                            w.declare(n, x);
                        }
                        out.extend(done(w));
                    }
                    Ok(out)
                })
            }
            StmtKind::Assign { target, op, value } => match op {
                AssignOp::Set | AssignOp::Draw => {
                    let draw = *op == AssignOp::Draw;
                    let outs = self.expr(value, w, true)?;
                    self.each(outs, |me, w, v| {
                        let mut out = Vec::new();
                        for (w, place) in me.place(target, w)? {
                            let place = match place {
                                Ok(p) => p,
                                Err(flow) => {
                                    out.push((w, Err(flow)));
                                    continue;
                                }
                            };
                            for (mut w, x) in bind(w, v.clone(), draw)? {
                                write(&mut w, &place, x)?;
                                out.extend(done(w));
                            }
                        }
                        Ok(out)
                    })
                }
                AssignOp::Add | AssignOp::Sub | AssignOp::Mul | AssignOp::Div => {
                    let bin = match op {
                        AssignOp::Add => BinOp::Add,
                        AssignOp::Sub => BinOp::Sub,
                        AssignOp::Mul => BinOp::Mul,
                        _ => BinOp::Div,
                    };
                    let mut out = Vec::new();
                    for (w, place) in self.place(target, w)? {
                        let place = match place {
                            Ok(p) => p,
                            Err(flow) => {
                                out.push((w, Err(flow)));
                                continue;
                            }
                        };
                        // The current value is read before the right side runs.
                        let old = read(&w, &place)?;
                        let outs = self.expr(value, w, true)?;
                        out.extend(self.each(outs, |_, mut w, v| {
                            write(&mut w, &place, binary(bin, &old, &v)?)?;
                            Ok(done(w))
                        })?);
                    }
                    Ok(out)
                }
            },
            StmtKind::For { pattern, iter, body } => {
                let var = match &pattern.kind {
                    PatternKind::Name(n) => Some(n.as_str()),
                    PatternKind::Wildcard => None,
                    _ => return unsupported("patterns in `for`"),
                };
                let outs = self.expr(iter, w, true)?;
                self.each(outs, |me, w, coll| {
                    let items: Vec<Value> = match coll {
                        Value::Range(lo, hi) => {
                            if hi >= lo && (hi as i128 - lo as i128) > 10_000 {
                                return too_big("a long range");
                            }
                            (lo..=hi).map(Value::Int).collect()
                        }
                        Value::List(xs) => xs.to_vec(),
                        Value::Dist(_) => return err("`for` needs a settled collection, not a distribution"),
                        _ => return err("`for` needs a list or a range"),
                    };
                    let rounds = items.into_iter().map(|item| (var, Some(item)));
                    me.repeat(body, w, rounds)
                })
            }
            StmtKind::Repeat { count, body } => {
                let outs = self.expr(count, w, true)?;
                self.each(outs, |me, w, n| {
                    let n = match n {
                        Value::Int(n) if n >= 0 => n,
                        Value::Float(x) if x.is_integer() && !x.is_negative() => {
                            x.to_integer().to_i64().unwrap_or(i64::MAX)
                        }
                        _ => return err("`repeat` needs a whole number of 0 or more"),
                    };
                    if n > 10_000 {
                        return too_big("a long `repeat`");
                    }
                    me.repeat(body, w, (0..n).map(|_| (None, None)))
                })
            }
            StmtKind::While { cond, body } => self.while_loop(Some(cond), body, w),
            StmtKind::Loop { body } => self.while_loop(None, body, w),
            StmtKind::Break => Ok(vec![(w, Err(Flow::Break))]),
            StmtKind::Continue => Ok(vec![(w, Err(Flow::Continue))]),
            StmtKind::Return(None) => Ok(vec![(w, Err(Flow::Return(Value::Unit)))]),
            StmtKind::Return(Some(e)) => {
                let outs = self.expr(e, w, true)?;
                self.each(outs, |_, w, v| Ok(vec![(w, Err(Flow::Return(v)))]))
            }
            StmtKind::Observe { value, from } => {
                let mut exprs = vec![value];
                exprs.extend(from.as_ref());
                let outs = self.list(&exprs, w)?;
                self.each_list(outs, |me, w, vs| {
                    let factor = match vs.as_slice() {
                        [c] => condition(c)?.0,
                        [v, d] => likelihood(d, v)?,
                        _ => unreachable!(),
                    };
                    if me.in_simulate == 0 {
                        me.observed = true;
                    }
                    if factor.is_zero() {
                        return Ok(Vec::new());
                    }
                    Ok(done(w.scaled(&factor)))
                })
            }
            StmtKind::Report { value, by, .. } => {
                let mut exprs = vec![value];
                exprs.extend(by.as_ref());
                let outs = self.list(&exprs, w)?;
                let site = (s.span.lo, s.span.hi);
                self.each_list(outs, |me, w, vs| {
                    let key = vs.get(1).cloned().unwrap_or(Value::Unit);
                    me.record(site, &key, &vs[0], &w.weight);
                    Ok(done(w))
                })
            }
        }
    }

    fn record(&mut self, site: (u32, u32), key: &Value, value: &Value, weight: &Q) {
        let group = self
            .reports
            .entry(site)
            .or_default()
            .entry(canon(key))
            .or_insert_with(|| Group {
                total: Q::zero(),
                values: BTreeMap::new(),
            });
        group.total += weight;
        let mut add = |v: &Value, w: Q| *group.values.entry(canon(v)).or_insert_with(Q::zero) += w;
        match value {
            Value::Dist(d) => {
                for (x, p) in d.iter() {
                    add(x, weight * p);
                }
            }
            other => add(other, weight.clone()),
        }
    }

    /// Run `body` once per round, binding the round's item to the variable.
    fn repeat(
        &mut self,
        body: &'a Block,
        w: World,
        rounds: impl Iterator<Item = (Option<&'a str>, Option<Value>)>,
    ) -> R<Outs> {
        let depth = w.scopes.len();
        let mut inside = vec![w];
        let mut out = Vec::new();
        for (var, item) in rounds {
            if inside.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for mut w in inside {
                w.scopes.push(Vec::new());
                if let (Some(var), Some(item)) = (var, &item) {
                    w.declare(var, item.clone());
                }
                for (mut w, r) in self.block(body, w, false)? {
                    w.scopes.truncate(depth);
                    match r {
                        Ok(_) | Err(Flow::Continue) => next.push(w),
                        Err(Flow::Break) => out.push((w, Ok(Value::Unit))),
                        Err(ret) => out.push((w, Err(ret))),
                    }
                }
            }
            self.check_worlds(next.len())?;
            inside = next;
        }
        out.extend(inside.into_iter().map(|w| (w, Ok(Value::Unit))));
        Ok(out)
    }

    fn while_loop(&mut self, cond: Option<&'a Expr>, body: &'a Block, w: World) -> R<Outs> {
        let depth = w.scopes.len();
        let mut inside = vec![w];
        let mut out = Vec::new();
        let mut iterations = 0;
        while !inside.is_empty() {
            iterations += 1;
            if iterations > self.limits.max_iterations {
                return too_big("a long loop");
            }
            let mut go = Vec::new();
            for w in inside {
                let Some(cond) = cond else {
                    go.push(w);
                    continue;
                };
                for (w, c) in self.expr(cond, w, true)? {
                    let c = match c {
                        Ok(c) => c,
                        Err(flow) => {
                            out.push((w, Err(flow)));
                            continue;
                        }
                    };
                    let (yes, no) = condition(&c)?;
                    if !no.is_zero() {
                        out.push((w.scaled(&no), Ok(Value::Unit)));
                    }
                    if !yes.is_zero() {
                        go.push(w.scaled(&yes));
                    }
                }
            }
            let mut next = Vec::new();
            for w in go {
                for (mut w, r) in self.block(body, w, false)? {
                    w.scopes.truncate(depth);
                    match r {
                        Ok(_) | Err(Flow::Continue) => next.push(w),
                        Err(Flow::Break) => out.push((w, Ok(Value::Unit))),
                        Err(ret) => out.push((w, Err(ret))),
                    }
                }
            }
            self.check_worlds(next.len())?;
            inside = next;
        }
        Ok(out)
    }

    /// Run a block in a scope of its own; its value is that of its last
    /// statement, if that's an expression.
    fn block(&mut self, b: &'a Block, w: World, want: bool) -> R<Outs> {
        let depth = w.scopes.len();
        let mut w = w;
        w.scopes.push(Vec::new());
        let mut acc: Outs = vec![(w, Ok(Value::Unit))];
        for (i, s) in b.stmts.iter().enumerate() {
            let last = i + 1 == b.stmts.len();
            let mut next = Vec::new();
            for (w, r) in acc {
                match r {
                    Ok(_) => next.extend(self.stmt(s, w, want && last)?),
                    Err(flow) => next.push((w, Err(flow))),
                }
            }
            acc = next;
        }
        Ok(leave(acc, depth))
    }

    /// An arm of `chance` or `match`: a statement in a scope of its own.
    fn arm(&mut self, body: &'a Stmt, mut w: World, want: bool) -> R<Outs> {
        let depth = w.scopes.len();
        w.scopes.push(Vec::new());
        let outs = self.stmt(body, w, want)?;
        Ok(leave(outs, depth))
    }

    // ── Places ───────────────────────────────────────────────────────────

    fn place(&mut self, target: &'a Expr, w: World) -> R<Vec<(World, Result<Place, Flow>)>> {
        match &target.kind {
            ExprKind::Name(n) => Ok(vec![(
                w,
                Ok(Place {
                    name: n.clone(),
                    path: Vec::new(),
                }),
            )]),
            ExprKind::Index { expr, index } => {
                let mut out = Vec::new();
                for (w, base) in self.place(expr, w)? {
                    let base = match base {
                        Ok(b) => b,
                        Err(flow) => {
                            out.push((w, Err(flow)));
                            continue;
                        }
                    };
                    for (w, i) in self.expr(index, w, true)? {
                        out.push((
                            w,
                            i.map(|i| {
                                let mut p = base.clone();
                                p.path.push(i);
                                p
                            }),
                        ));
                    }
                }
                Ok(out)
            }
            _ => unsupported("assigning to a field"),
        }
    }

    // ── Expressions ──────────────────────────────────────────────────────

    /// Evaluate an expression. With `want`, its value is used; otherwise
    /// it's an expression statement.
    fn expr(&mut self, e: &'a Expr, w: World, want: bool) -> R<Outs> {
        self.tick()?;
        let one = |w: World, v: Value| -> R<Outs> { Ok(vec![(w, Ok(v))]) };
        match &e.kind {
            ExprKind::Int(i) => one(
                w,
                Value::Int(
                    i.to_i64()
                        .ok_or_else(|| Stop::TooBig("oracle integer out of range".into()))?,
                ),
            ),
            ExprKind::Float(f) => one(w, Value::Float(self.number(e, *f))),
            ExprKind::Percent(f) => {
                let x = self.number(e, *f * 100.0) / int(100);
                let v = if !x.is_negative() && x <= Q::one() {
                    Value::Prob(x)
                } else {
                    Value::Float(x)
                };
                one(w, v)
            }
            ExprKind::Bool(b) => one(w, Value::Bool(*b)),
            ExprKind::Dice { count, sides } => one(w, dice(*count, *sides)?),
            ExprKind::Str(segments) => match segments.as_slice() {
                [] => one(w, Value::Str(Rc::from(""))),
                [StrSegment::Lit(text)] => one(w, Value::Str(Rc::from(text.as_str()))),
                _ => unsupported("string interpolation"),
            },
            ExprKind::Name(n) => match w.get(n) {
                Some(v) => {
                    let v = v.clone();
                    one(w, v)
                }
                None => unsupported(format!("the name `{n}`")),
            },
            ExprKind::List(items) => {
                let items: Vec<&Expr> = items.iter().collect();
                self.map_list(&items, w, |vs| Ok(Value::List(Rc::new(vs))))
            }
            ExprKind::Unary { op, expr } => {
                let op = *op;
                self.map_list(&[expr], w, move |vs| unary(op, &vs[0]))
            }
            ExprKind::Binary {
                op: op @ (BinOp::And | BinOp::Or),
                lhs,
                rhs,
            } => self.logic(*op == BinOp::And, lhs, rhs, w),
            ExprKind::Binary { op, lhs, rhs } => {
                let op = *op;
                self.map_list(&[lhs, rhs], w, move |vs| binary(op, &vs[0], &vs[1]))
            }
            ExprKind::Call { callee, args } => self.call(callee, args, w),
            ExprKind::Index { expr, index } => {
                self.map_list(&[expr, index], w, |vs| lift2(&vs[0], &vs[1], index_plain))
            }
            ExprKind::If { cond, then, otherwise } => self.if_expr(cond, then, otherwise.as_deref(), w, want),
            ExprKind::Chance { arms } => self.chance(arms, w, want),
            ExprKind::Match { scrutinee, arms } => self.match_expr(scrutinee, arms, w, want),
            ExprKind::Simulate(block) => self.simulate(block, w),
            ExprKind::Block(block) => self.block(block, w, want),
            ExprKind::Map(_)
            | ExprKind::Record { .. }
            | ExprKind::Method { .. }
            | ExprKind::Field { .. }
            | ExprKind::With { .. }
            | ExprKind::Lambda { .. } => unsupported("maps, records, methods and lambdas"),
        }
    }

    /// The exact value of a number literal, from its source text.
    fn number(&self, e: &Expr, fallback: f64) -> Q {
        let text: String = self.src[e.span.range()]
            .chars()
            .filter(|c| *c != '_' && *c != '%')
            .collect();
        parse_decimal(&text)
            .or_else(|| Q::from_float(fallback))
            .unwrap_or_else(Q::zero)
    }

    /// `and` and `or`: the right side runs only when the left side isn't a
    /// fact that decides the answer (docs/semantics.md, sections 2 and 4).
    fn logic(&mut self, and: bool, lhs: &'a Expr, rhs: &'a Expr, w: World) -> R<Outs> {
        let word = if and { "and" } else { "or" };
        let outs = self.expr(lhs, w, true)?;
        self.each(outs, |me, w, a| {
            let left = truth(&a, word)?;
            if let Truth::Fact(x) = left {
                if x != and {
                    return Ok(vec![(w, Ok(Value::Bool(x)))]);
                }
            }
            let outs = me.expr(rhs, w, true)?;
            me.each(outs, |_, w, b| {
                let op = |x: bool, y: bool| if and { x && y } else { x || y };
                let v = match (&left, truth(&b, word)?) {
                    (Truth::Fact(x), Truth::Fact(y)) => Value::Bool(op(*x, y)),
                    (Truth::Fact(x), Truth::Uncertain(d)) => map_bools(&d, |y| op(*x, y))?,
                    (Truth::Uncertain(d), Truth::Fact(y)) => map_bools(d, |x| op(x, y))?,
                    (Truth::Uncertain(_), Truth::Uncertain(_)) => {
                        return err(format!("`{word}` can't combine two uncertain facts"));
                    }
                };
                Ok(vec![(w, Ok(v))])
            })
        })
    }

    fn if_expr(
        &mut self,
        cond: &'a Expr,
        then: &'a Block,
        otherwise: Option<&'a Expr>,
        w: World,
        want: bool,
    ) -> R<Outs> {
        if want && otherwise.is_none() {
            return unsupported("an `if` without `else` used as a value");
        }
        let outs = self.expr(cond, w, true)?;
        self.each(outs, |me, w, c| {
            let (yes, no) = condition(&c)?;
            let mut out = Vec::new();
            if !yes.is_zero() {
                out.extend(me.block(then, w.scaled(&yes), want)?);
            }
            if !no.is_zero() {
                let w = w.scaled(&no);
                match otherwise {
                    Some(e) => out.extend(me.expr(e, w, want)?),
                    None => out.push((w, Ok(Value::Unit))),
                }
            }
            Ok(out)
        })
    }

    /// `chance`: every weight is evaluated first, then the world splits
    /// (docs/semantics.md, section 3).
    fn chance(&mut self, arms: &'a [ChanceArm], w: World, want: bool) -> R<Outs> {
        let weights: Vec<&Expr> = arms.iter().filter_map(|a| a.weight.as_ref()).collect();
        let has_else = arms.iter().any(|a| a.weight.is_none());
        let outs = self.list(&weights, w)?;
        self.each_list(outs, |me, w, vs| {
            let ps = vs.iter().map(|v| condition(v).map(|c| c.0)).collect::<R<Vec<Q>>>()?;
            let sum = ps.iter().fold(Q::zero(), |acc, p| acc + p);
            if sum > Q::one() {
                return err("the chances add up to more than 100%");
            }
            let rest = Q::one() - sum;
            let mut ps = ps.into_iter();
            let mut out = Vec::new();
            for arm in arms {
                let p = match arm.weight {
                    Some(_) => ps.next().unwrap(),
                    None => rest.clone(),
                };
                if !p.is_zero() {
                    out.extend(me.arm(&arm.body, w.scaled(&p), want)?);
                }
            }
            if !has_else && !rest.is_zero() {
                if want {
                    return err("the chances add up to less than 100%, and there's no `else`");
                }
                out.push((w.scaled(&rest), Ok(Value::Unit)));
            }
            Ok(out)
        })
    }

    fn match_expr(&mut self, scrutinee: &'a Expr, arms: &'a [MatchArm], w: World, want: bool) -> R<Outs> {
        let outs = self.expr(scrutinee, w, true)?;
        self.each(outs, |me, w, subject| {
            if matches!(subject, Value::Dist(_)) {
                return err("`match` needs a settled value, not a distribution");
            }
            me.match_arms(arms, &subject, w, want)
        })
    }

    /// Try the arms in order. A guard is a condition: where it doesn't hold,
    /// the next arms are tried.
    fn match_arms(&mut self, arms: &'a [MatchArm], subject: &Value, w: World, want: bool) -> R<Outs> {
        let Some((arm, rest)) = arms.split_first() else {
            return err("no arm of the `match` matches");
        };
        let mut binds = Vec::new();
        if !self.pattern(&arm.pattern, subject, &mut binds)? {
            return self.match_arms(rest, subject, w, want);
        }
        let depth = w.scopes.len();
        let mut w = w;
        w.scopes.push(binds);
        let Some(guard) = &arm.guard else {
            let outs = self.arm(&arm.body, w, want)?;
            return Ok(leave(outs, depth));
        };
        let mut out = Vec::new();
        for (w, g) in self.expr(guard, w, true)? {
            let g = match g {
                Ok(g) => g,
                Err(flow) => {
                    out.push((w, Err(flow)));
                    continue;
                }
            };
            let (yes, no) = condition(&g)?;
            if !yes.is_zero() {
                let outs = self.arm(&arm.body, w.scaled(&yes), want)?;
                out.extend(leave(outs, depth));
            }
            if !no.is_zero() {
                let mut w = w.scaled(&no);
                w.scopes.truncate(depth);
                out.extend(self.match_arms(rest, subject, w, want)?);
            }
        }
        Ok(out)
    }

    fn pattern(&self, p: &Pattern, v: &Value, binds: &mut Scope) -> R<bool> {
        match &p.kind {
            PatternKind::Wildcard => Ok(true),
            PatternKind::Name(n) => {
                binds.push((n.clone(), v.clone()));
                Ok(true)
            }
            PatternKind::Literal(e) => Ok(equals(v, &self.literal(e)?)),
            PatternKind::Or(alts) => {
                for alt in alts {
                    if self.pattern(alt, v, &mut Vec::new())? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            PatternKind::List(_) => unsupported("list patterns"),
        }
    }

    fn literal(&self, e: &Expr) -> R<Value> {
        match &e.kind {
            ExprKind::Int(i) => Ok(Value::Int(
                i.to_i64()
                    .ok_or_else(|| Stop::TooBig("oracle integer out of range".into()))?,
            )),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Str(segments) => match segments.as_slice() {
                [StrSegment::Lit(text)] => Ok(Value::Str(Rc::from(text.as_str()))),
                _ => unsupported("this pattern"),
            },
            ExprKind::Unary {
                op: UnOp::Neg,
                expr: inner,
            } => unary(UnOp::Neg, &self.literal(inner)?),
            _ => unsupported("this pattern"),
        }
    }

    /// `simulate { B }`: a separate model, starting from a copy of the
    /// current world with weight 1; its normalized result (section 8).
    fn simulate(&mut self, block: &'a Block, w: World) -> R<Outs> {
        let start = World {
            weight: Q::one(),
            ..w.clone()
        };
        self.in_simulate += 1;
        let outs = self.block(block, start, true);
        self.in_simulate -= 1;
        let mut pairs = Vec::new();
        let mut total = Q::zero();
        for (sw, r) in outs? {
            match r {
                Ok(v) => {
                    total += &sw.weight;
                    pairs.push((v, sw.weight));
                }
                Err(_) => return unsupported("leaving a `simulate` block with `break`, `continue` or `return`"),
            }
        }
        if total.is_zero() {
            return err("every world of the `simulate` block was ruled out");
        }
        let d = dist(pairs.into_iter().map(|(v, p)| (v, p / &total)).collect())?;
        Ok(vec![(w, Ok(d))])
    }

    fn call(&mut self, callee: &'a Expr, args: &'a [ast::Arg], w: World) -> R<Outs> {
        let ExprKind::Name(name) = &callee.kind else {
            return unsupported("calling a value");
        };
        if args.iter().any(|a| a.name.is_some()) {
            return unsupported("named arguments");
        }
        if w.get(name).is_some() {
            return unsupported("calling a variable");
        }
        let exprs: Vec<&Expr> = args.iter().map(|a| &a.value).collect();
        let outs = self.list(&exprs, w)?;
        match self.fns.get(name.as_str()).copied() {
            Some(decl) => self.each_list(outs, |me, w, vs| me.call_fn(decl, vs, w)),
            None => self.each_list(outs, |_, w, vs| Ok(vec![(w, Ok(builtin(name, &vs)?))])),
        }
    }

    /// A call: the body runs in the caller's world, with the parameters and
    /// a copy of the top-level variables; its splits and observations stay
    /// in the weights (section 6).
    fn call_fn(&mut self, decl: &'a ast::FnDecl, args: Vec<Value>, w: World) -> R<Outs> {
        if args.len() != decl.params.len() {
            return unsupported("a call with the wrong number of arguments");
        }
        if self.depth >= self.limits.max_depth {
            return too_big("deep recursion");
        }
        let globals = match &w.globals {
            Some(g) => g.clone(),
            None => Rc::new(w.scopes[0].clone()),
        };
        let params: Scope = decl.params.iter().map(|p| p.name.name.clone()).zip(args).collect();
        let start = World {
            weight: w.weight.clone(),
            scopes: vec![params],
            globals: Some(globals),
        };
        self.depth += 1;
        let outs = self.block(&decl.body, start, true);
        self.depth -= 1;
        let mut out = Vec::new();
        for (fw, r) in outs? {
            let v = match r {
                Ok(v) | Err(Flow::Return(v)) => v,
                Err(_) => return unsupported("`break` or `continue` outside of a loop"),
            };
            out.push((
                World {
                    weight: fw.weight,
                    ..w.clone()
                },
                Ok(v),
            ));
        }
        Ok(out)
    }
}

/// Pop the scopes of worlds that finished normally. (Worlds leaving with
/// `break`, `continue` or `return` are trimmed by whoever catches them.)
fn leave(outs: Outs, depth: usize) -> Outs {
    outs.into_iter()
        .map(|(mut w, r)| {
            if r.is_ok() {
                w.scopes.truncate(depth);
            }
            (w, r)
        })
        .collect()
}

/// The worlds and values that `=` (or `~`, with `draw`) produces.
fn bind(w: World, v: Value, draw: bool) -> R<Vec<(World, Value)>> {
    if !draw {
        return Ok(vec![(w, v)]);
    }
    match v {
        Value::Dist(d) => Ok(d.iter().map(|(x, p)| (w.scaled(p), x.clone())).collect()),
        Value::Prob(_) => err("can't draw from a probability; use `bernoulli`"),
        other => Ok(vec![(w, other)]),
    }
}

#[derive(Clone, Debug)]
struct Place {
    name: String,
    path: Vec<Value>,
}

fn read(w: &World, place: &Place) -> R<Value> {
    let mut v = match w.get(&place.name) {
        Some(v) => v.clone(),
        None => return unsupported(format!("the name `{}`", place.name)),
    };
    for i in &place.path {
        v = index_plain(&v, i)?;
    }
    Ok(v)
}

fn write(w: &mut World, place: &Place, x: Value) -> R<()> {
    let old = match w.get(&place.name) {
        Some(v) => v.clone(),
        None => return unsupported(format!("the name `{}`", place.name)),
    };
    let new = replace(&old, &place.path, x)?;
    w.set(&place.name, new)
}

fn replace(v: &Value, path: &[Value], x: Value) -> R<Value> {
    let Some((i, rest)) = path.split_first() else {
        return Ok(x);
    };
    match v {
        Value::List(items) => {
            let k = as_index(i, items.len())?;
            let mut items = (**items).clone();
            items[k] = replace(&items[k], rest, x)?;
            Ok(Value::List(Rc::new(items)))
        }
        _ => unsupported("assigning into something other than a list"),
    }
}

// ── Operations ───────────────────────────────────────────────────────────

fn lift1(a: &Value, f: impl Fn(&Value) -> R<Value>) -> R<Value> {
    match a {
        Value::Dist(d) => {
            let pairs = d.iter().map(|(x, p)| Ok((f(x)?, p.clone()))).collect::<R<Vec<_>>>()?;
            dist(pairs)
        }
        other => f(other),
    }
}

/// Apply `f` to two values; distributions are independent draws.
fn lift2(a: &Value, b: &Value, f: impl Fn(&Value, &Value) -> R<Value>) -> R<Value> {
    if !matches!(a, Value::Dist(_)) && !matches!(b, Value::Dist(_)) {
        return f(a, b);
    }
    let mut pairs = Vec::new();
    for (x, p) in outcomes(a) {
        for (y, q) in outcomes(b) {
            pairs.push((f(&x, &y)?, &p * &q));
        }
    }
    dist(pairs)
}

fn lift_n(args: &[Value], f: impl Fn(&[Value]) -> R<Value>) -> R<Value> {
    if !args.iter().any(|a| matches!(a, Value::Dist(_))) {
        return f(args);
    }
    let mut combos: Vec<(Vec<Value>, Q)> = vec![(Vec::new(), Q::one())];
    for a in args {
        let mut next = Vec::new();
        for (vs, p) in &combos {
            for (x, q) in outcomes(a) {
                let mut vs = vs.clone();
                vs.push(x);
                next.push((vs, p * &q));
            }
        }
        combos = next;
    }
    let pairs = combos
        .into_iter()
        .map(|(vs, p)| Ok((f(&vs)?, p)))
        .collect::<R<Vec<_>>>()?;
    dist(pairs)
}

fn outcomes(v: &Value) -> Vec<(Value, Q)> {
    match v {
        Value::Dist(d) => d.to_vec(),
        other => vec![(other.clone(), Q::one())],
    }
}

enum Truth {
    Fact(bool),
    Uncertain(Rc<Vec<(Value, Q)>>),
}

/// An operand of `and`, `or` or `not`: a fact, or a distribution of facts.
fn truth(v: &Value, op: &str) -> R<Truth> {
    match v {
        Value::Bool(b) => Ok(Truth::Fact(*b)),
        Value::Dist(d) if d.iter().all(|(x, _)| matches!(x, Value::Bool(_))) => Ok(Truth::Uncertain(d.clone())),
        _ => err(format!("`{op}` needs facts")),
    }
}

fn map_bools(d: &Rc<Vec<(Value, Q)>>, f: impl Fn(bool) -> bool) -> R<Value> {
    lift1(&Value::Dist(d.clone()), |x| match x {
        Value::Bool(b) => Ok(Value::Bool(f(*b))),
        _ => unreachable!("checked by `truth`"),
    })
}

/// The probabilities that a condition holds and that it doesn't (section 3).
fn condition(v: &Value) -> R<(Q, Q)> {
    match v {
        Value::Bool(true) => Ok((Q::one(), Q::zero())),
        Value::Bool(false) => Ok((Q::zero(), Q::one())),
        Value::Prob(p) => Ok((p.clone(), Q::one() - p)),
        Value::Float(p) if !p.is_negative() && *p <= Q::one() => Ok((p.clone(), Q::one() - p)),
        Value::Dist(d) if d.iter().all(|(x, _)| matches!(x, Value::Bool(_))) => {
            let yes = d
                .iter()
                .filter(|(x, _)| *x == Value::Bool(true))
                .fold(Q::zero(), |acc, (_, p)| acc + p);
            let no = Q::one() - &yes;
            Ok((yes, no))
        }
        _ => err("a condition needs a probability or a fact"),
    }
}

fn to_prob(v: &Value) -> R<Q> {
    match v {
        Value::Prob(p) => Ok(p.clone()),
        Value::Float(p) if !p.is_negative() && *p <= Q::one() => Ok(p.clone()),
        _ => err("expected a probability"),
    }
}

/// P(D = v), for `observe v from D`.
fn likelihood(d: &Value, v: &Value) -> R<Q> {
    match d {
        Value::Dist(outcomes) => Ok(outcomes
            .iter()
            .filter(|(x, _)| equals(x, v))
            .fold(Q::zero(), |acc, (_, p)| acc + p)),
        Value::Prob(_) => err("`observe … from` needs a distribution, not a probability"),
        other => Ok(if equals(other, v) { Q::one() } else { Q::zero() }),
    }
}

fn number(v: &Value) -> Option<Q> {
    match v {
        Value::Int(i) => Some(int(*i)),
        Value::Float(x) | Value::Prob(x) => Some(x.clone()),
        _ => None,
    }
}

/// The language's `==`: numbers compare by value, whatever their kind.
fn equals(a: &Value, b: &Value) -> bool {
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn compare(a: &Value, b: &Value) -> R<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (number(a), number(b)) {
        return Ok(x.cmp(&y));
    }
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
        (Value::List(x), Value::List(y)) => {
            for (p, q) in x.iter().zip(y.iter()) {
                let c = compare(p, q)?;
                if c.is_ne() {
                    return Ok(c);
                }
            }
            Ok(x.len().cmp(&y.len()))
        }
        _ => err("can't compare these values"),
    }
}

fn contains(coll: &Value, item: &Value) -> R<bool> {
    match coll {
        Value::List(items) => Ok(items.iter().any(|x| equals(x, item))),
        Value::Range(lo, hi) => Ok(match number(item) {
            Some(x) => x.is_integer() && x >= int(*lo) && x <= int(*hi),
            None => false,
        }),
        _ => unsupported("`in` on something other than a list or a range"),
    }
}

fn unary(op: UnOp, v: &Value) -> R<Value> {
    match op {
        UnOp::Typeof => Ok(Value::Str(runtime_type(v, 0)?.into())),
        UnOp::Neg => lift1(v, |x| match x {
            Value::Int(i) => i.checked_neg().map(Value::Int).ok_or_else(overflow),
            Value::Float(f) | Value::Prob(f) => Ok(Value::Float(-f)),
            _ => err("can't negate this"),
        }),
        UnOp::Not => match truth(v, "not")? {
            Truth::Fact(b) => Ok(Value::Bool(!b)),
            Truth::Uncertain(d) => map_bools(&d, |b| !b),
        },
    }
}

fn runtime_type(v: &Value, depth: usize) -> R<String> {
    if depth > 64 {
        return too_big("typeof nesting");
    }
    fn common<'a>(values: impl Iterator<Item = &'a Value>, depth: usize) -> R<String> {
        let mut name = None;
        for value in values {
            let ty = runtime_type(value, depth)?;
            if name.as_ref().is_some_and(|n| n != &ty) {
                return Ok("any".into());
            }
            name = Some(ty);
        }
        Ok(name.unwrap_or_else(|| "unknown".into()))
    }
    Ok(match v {
        Value::Unit => "()".into(),
        Value::Bool(_) => "bool".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Prob(_) => "prob".into(),
        Value::Str(_) => "str".into(),
        Value::Range(..) => "range".into(),
        Value::List(xs) => format!("list[{}]", common(xs.iter(), depth + 1)?),
        Value::Dist(d) => format!("dist[{}]", common(d.iter().map(|(v, _)| v), depth + 1)?),
    })
}

fn overflow() -> Stop {
    // This deliberately small reference interpreter has a narrower resource
    // bound than the production arbitrary-precision integer implementation.
    Stop::TooBig("oracle integer out of range".into())
}

fn binary(op: BinOp, a: &Value, b: &Value) -> R<Value> {
    match op {
        BinOp::Range | BinOp::RangeExcl => match (a, b) {
            (Value::Int(lo), Value::Int(hi)) => {
                let hi = if op == BinOp::RangeExcl {
                    hi.checked_sub(1).ok_or_else(overflow)?
                } else {
                    *hi
                };
                Ok(Value::Range(*lo, hi))
            }
            _ => err("a range needs plain whole numbers"),
        },
        BinOp::In => lift2(a, b, |x, coll| contains(coll, x).map(Value::Bool)),
        BinOp::NotIn => lift2(a, b, |x, coll| contains(coll, x).map(|c| Value::Bool(!c))),
        BinOp::And | BinOp::Or => unreachable!("evaluated lazily"),
        BinOp::To => unsupported("`to`"),
        _ => lift2(a, b, |x, y| binary_plain(op, x, y)),
    }
}

fn binary_plain(op: BinOp, a: &Value, b: &Value) -> R<Value> {
    use std::cmp::Ordering;
    match op {
        BinOp::Eq => Ok(Value::Bool(equals(a, b))),
        BinOp::Ne => Ok(Value::Bool(!equals(a, b))),
        BinOp::Lt => Ok(Value::Bool(compare(a, b)? == Ordering::Less)),
        BinOp::Le => Ok(Value::Bool(compare(a, b)? != Ordering::Greater)),
        BinOp::Gt => Ok(Value::Bool(compare(a, b)? == Ordering::Greater)),
        BinOp::Ge => Ok(Value::Bool(compare(a, b)? != Ordering::Less)),
        _ => arith(op, a, b),
    }
}

fn arith(op: BinOp, a: &Value, b: &Value) -> R<Value> {
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        let (x, y) = (*x, *y);
        return match op {
            BinOp::Add => x.checked_add(y).map(Value::Int).ok_or_else(overflow),
            BinOp::Sub => x.checked_sub(y).map(Value::Int).ok_or_else(overflow),
            BinOp::Mul => x.checked_mul(y).map(Value::Int).ok_or_else(overflow),
            BinOp::Div if y == 0 => err("division by zero"),
            BinOp::Div => Ok(Value::Float(Q::new(BigInt::from(x), BigInt::from(y)))),
            BinOp::IntDiv | BinOp::Mod if y == 0 => err("division by zero"),
            // Rounding down; the remainder takes the sign of the divisor.
            BinOp::IntDiv => floor_div(x, y).map(Value::Int).ok_or_else(overflow),
            BinOp::Mod => {
                let q = floor_div(x, y).ok_or_else(overflow)?;
                Ok(Value::Int((x as i128 - q as i128 * y as i128) as i64))
            }
            BinOp::Pow if y >= 0 => u32::try_from(y)
                .ok()
                .and_then(|e| x.checked_pow(e))
                .map(Value::Int)
                .ok_or_else(overflow),
            _ => unsupported("this operation on ints"),
        };
    }
    let (Some(x), Some(y)) = (number(a), number(b)) else {
        return err(format!("can't use `{}` with these values", op.symbol()));
    };
    match op {
        BinOp::Add => Ok(Value::Float(x + y)),
        BinOp::Sub => Ok(Value::Float(x - y)),
        BinOp::Mul => Ok(Value::Float(x * y)),
        BinOp::Div if y.is_zero() => err("division by zero"),
        BinOp::Div => Ok(Value::Float(x / y)),
        _ => unsupported("this operation on floats"),
    }
}

fn floor_div(x: i64, y: i64) -> Option<i64> {
    let q = x.checked_div(y)?;
    Some(if x % y != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q })
}

fn as_index(i: &Value, len: usize) -> R<usize> {
    match i {
        Value::Int(k) if *k >= 0 && (*k as u128) < len as u128 => Ok(*k as usize),
        Value::Int(_) => err("index out of range"),
        _ => err("an index must be a whole number"),
    }
}

fn index_plain(coll: &Value, i: &Value) -> R<Value> {
    match coll {
        Value::List(items) => Ok(items[as_index(i, items.len())?].clone()),
        Value::Range(lo, hi) => {
            let len = if hi < lo {
                0
            } else {
                (*hi as i128 - *lo as i128 + 1) as usize
            };
            Ok(Value::Int(lo + as_index(i, len)? as i64))
        }
        _ => unsupported("indexing something other than a list or a range"),
    }
}

fn builtin(name: &str, args: &[Value]) -> R<Value> {
    match (name, args) {
        ("bernoulli", [p]) => lift1(p, |x| {
            let p = to_prob(x)?;
            dist(vec![(Value::Bool(true), p.clone()), (Value::Bool(false), Q::one() - p)])
        }),
        ("one_of", [xs]) => lift1(xs, |x| match x {
            Value::List(items) if !items.is_empty() => uniform(items.to_vec()),
            Value::Range(lo, hi) if hi >= lo && hi - lo < 10_000 => uniform((*lo..=*hi).map(Value::Int).collect()),
            _ => err("one_of needs a list with at least one element"),
        }),
        ("P", [x]) => match x {
            Value::Bool(b) => Ok(Value::Prob(if *b { Q::one() } else { Q::zero() })),
            Value::Prob(_) | Value::Float(_) => to_prob(x).map(Value::Prob),
            Value::Dist(_) => condition(x).map(|(yes, _)| Value::Prob(yes)),
            _ => err("P needs a condition"),
        },
        ("min" | "max", args) if args.len() >= 2 => lift_n(args, |vs| {
            let mut ints = Vec::new();
            for v in vs {
                match v {
                    Value::Int(i) => ints.push(*i),
                    _ => return unsupported("min and max of something other than ints"),
                }
            }
            let pick = if name == "min" {
                ints.iter().min()
            } else {
                ints.iter().max()
            };
            Ok(Value::Int(*pick.unwrap()))
        }),
        ("abs", [x]) => lift1(x, |v| match v {
            Value::Int(i) => i.checked_abs().map(Value::Int).ok_or_else(overflow),
            _ => unsupported("abs of something other than an int"),
        }),
        ("len", [x]) => lift1(x, |v| match v {
            Value::List(items) => Ok(Value::Int(items.len() as i64)),
            _ => unsupported("len of something other than a list"),
        }),
        ("print", _) => Ok(Value::Unit),
        _ => unsupported(format!("the built-in `{name}` with {} argument(s)", args.len())),
    }
}

/// The exact value of a decimal literal like `12.5` or `1e-3`.
pub fn parse_decimal(s: &str) -> Option<Q> {
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].parse::<i32>().ok()?),
        None => (s, 0),
    };
    let (whole, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{frac}");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = digits.parse::<BigInt>().ok()?;
    let scale = exp.checked_sub(i32::try_from(frac.len()).ok()?)?;
    if scale.unsigned_abs() > 400 {
        return None;
    }
    let ten = BigInt::from(10);
    Some(if scale >= 0 {
        Q::from_integer(n * ten.pow(scale as u32))
    } else {
        Q::new(n, ten.pow(scale.unsigned_abs()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(src: &str) -> Measure {
        run_source(src, &Limits::default()).unwrap_or_else(|e| panic!("{e:?}\n{src}"))
    }

    /// The probability of each value of the only report.
    fn only(src: &str) -> BTreeMap<String, Q> {
        let m = measure(src);
        assert_eq!(m.reports.len(), 1, "{src}");
        let groups = m.reports.values().next().unwrap();
        let group = &groups["()"];
        group
            .values
            .iter()
            .map(|(k, w)| (k.clone(), w / &group.total))
            .collect()
    }

    fn frac(n: i64, d: i64) -> Q {
        Q::new(BigInt::from(n), BigInt::from(d))
    }

    #[test]
    fn decimals_are_exact() {
        assert_eq!(parse_decimal("30"), Some(int(30)));
        assert_eq!(parse_decimal("12.5"), Some(frac(25, 2)));
        assert_eq!(parse_decimal("1e-3"), Some(frac(1, 1000)));
        assert_eq!(parse_decimal("0.1"), Some(frac(1, 10)));
        assert_eq!(parse_decimal("x"), None);
    }

    #[test]
    fn events_and_recipes() {
        let p = only("let rain ~ bernoulli(30%)\nreport rain and rain");
        assert_eq!(p["true"], frac(3, 10));
        let p = only("let die = d6\nreport die + die == 2");
        assert_eq!(p["true"], frac(1, 36));
        let p = only("let r ~ d6\nreport r + r == 2");
        assert_eq!(p["true"], frac(1, 6));
    }

    #[test]
    fn the_audit_posterior() {
        let p = only(
            "var win = false\nif 0.5% { repeat 1 { win = true } }\nobserve if win { true } else { 0.00001 }\nreport win",
        );
        assert_eq!(
            p["true"],
            frac(5, 1000) / (frac(5, 1000) + frac(995, 1000) * frac(1, 100_000))
        );
    }

    #[test]
    fn shared_rates_are_kept() {
        let p = only(
            "let p ~ simulate { if 50% { 10% } else { 90% } }\nlet a ~ bernoulli(p)\nlet b ~ bernoulli(p)\nreport a and b",
        );
        assert_eq!(p["true"], frac(41, 100));
    }

    #[test]
    fn operands_run_left_to_right() {
        let p = only("var x = 1\nreport x + { x = 2; 0 }");
        assert_eq!(p["1"], Q::one());
        let p = only("var x = 1\nreport [x, { x = 3; x }]");
        assert_eq!(p["[1, 3]"], Q::one());
        let p = only("var x = 1\nx += { x = 10; 1 }\nreport x");
        assert_eq!(p["2"], Q::one());
    }

    #[test]
    fn evidence_and_simulate() {
        let m = measure("let s ~ bernoulli(1%)\nobserve if s { 95% } else { 8% }\nreport s");
        assert!(m.observed);
        assert_eq!(m.total, frac(1, 100) * frac(95, 100) + frac(99, 100) * frac(8, 100));
        let m = measure("let d = simulate { let s ~ d6\nobserve s > 4\ns }\nreport d");
        assert!(!m.observed);
        assert_eq!(m.total, Q::one());
        assert_eq!(
            run_source("let s ~ d6\nobserve s > 6\nreport s", &Limits::default()).unwrap_err(),
            Stop::Error("the evidence is impossible".into())
        );
    }

    #[test]
    fn loops_breaks_and_calls() {
        let p = only("var n = 0\nwhile n < 3 { n += 1\nif 50% { break } }\nreport n");
        assert_eq!(p["1"], frac(1, 2));
        assert_eq!(p["3"], frac(1, 4));
        let p = only("fn f(a) { if a > 2 { return a }\n0 }\nlet x ~ d4\nreport f(x)");
        assert_eq!(p["0"], frac(1, 2));
        let p = only("let g ~ d2\nfn f() { g * 10 }\nreport f()");
        assert_eq!(p["20"], frac(1, 2));
    }

    #[test]
    fn match_guards_are_conditions() {
        let p = only("let x ~ d2\nreport match x { 1 if 50% => \"a\", 1 => \"b\", _ => \"c\" }");
        assert_eq!(p["\"a\""], frac(1, 4));
        assert_eq!(p["\"b\""], frac(1, 4));
        assert_eq!(p["\"c\""], frac(1, 2));
    }
}
