//! The world-set interpreter.
//!
//! Every statement maps a set of worlds to a set of worlds. All the worlds in
//! a set are at the same point of the program, so control flow is shared and
//! only the data differs. Splitting happens in statements; expressions are
//! evaluated in one world at a time.

use crate::builtins;
use crate::dist::Dist;
use crate::error::{OpError, OpResult, Result, RuntimeError};
use crate::ops;
use crate::report::Sink;
use crate::value::{Closure, Value, fmt_prob};
use crate::world::{Flow, World, merge, merge_values, total_weight};
use probl_sema::builtins::Lifting;
use probl_sema::ir::*;
use probl_sema::{Builtin, Liveness};
use probl_syntax::Span;
use probl_syntax::ast::BinOp;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::sync::Arc;

/// How deep calls may nest.
const MAX_DEPTH: usize = 1_000;

#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Largest number of worlds any statement ran on.
    pub peak_worlds: usize,
    /// Statement executions, counting each world separately.
    pub world_steps: u64,
    pub calls: u64,
    pub memo_hits: u64,
}

/// The distribution of a function's return value.
#[derive(Debug)]
pub struct CallResult {
    pub outcomes: Vec<(Value, f64)>,
    pub unresolved: f64,
}

type CallKey = (FnId, Vec<Value>);

pub struct Engine<'p> {
    prog: &'p Program,
    live: &'p Liveness,
    epsilon: f64,
    max_iterations: u64,
    max_worlds: usize,
    merging: bool,
    memoizing: bool,
    memo: FxHashMap<CallKey, Arc<CallResult>>,
    active: FxHashSet<CallKey>,
    depth: usize,
    /// Probability left unaccounted for (loops cut at ε, truncated tails).
    pub unresolved: f64,
    /// Whether any `observe` ran.
    pub observed: bool,
    pub sinks: Vec<Sink>,
    dice: FxHashMap<(u32, u32), Value>,
    pools: FxHashMap<(u32, Value), Value>,
    record_names: Vec<Arc<str>>,
    enum_values: Vec<Vec<Value>>,
    print: &'p mut (dyn FnMut(&str) + Send),
    pub stats: Stats,
}

impl<'p> Engine<'p> {
    pub fn new(
        prog: &'p Program,
        live: &'p Liveness,
        epsilon: f64,
        merging: bool,
        memoizing: bool,
        print: &'p mut (dyn FnMut(&str) + Send),
    ) -> Engine<'p> {
        Engine {
            prog,
            live,
            epsilon,
            max_iterations: prog.settings.max_iterations,
            max_worlds: prog.settings.max_worlds,
            merging,
            memoizing,
            memo: FxHashMap::default(),
            active: FxHashSet::default(),
            depth: 0,
            unresolved: 0.0,
            observed: false,
            sinks: vec![Sink::default(); prog.reports.len()],
            dice: FxHashMap::default(),
            pools: FxHashMap::default(),
            record_names: prog.records.iter().map(|r| Arc::from(r.name.as_str())).collect(),
            enum_values: prog
                .enums
                .iter()
                .enumerate()
                .map(|(t, e)| {
                    e.variants
                        .iter()
                        .enumerate()
                        .map(|(v, name)| ops::enum_value(t as u32, v as u32, name))
                        .collect()
                })
                .collect(),
            print,
            stats: Stats::default(),
        }
    }

    /// Run the top level. Returns the total weight of the worlds that finish.
    pub fn run_main(&mut self) -> Result<f64> {
        let prog = self.prog;
        let main = prog.main();
        let world = World {
            slots: vec![Value::Dead; main.n_slots()],
            weight: 1.0,
        };
        let flow = self.exec_block(MAIN, &main.body, vec![world])?;
        Ok(total_weight(&flow.next))
    }

    fn merge(&self, worlds: Vec<World>, stmt: StmtId) -> Vec<World> {
        merge(worlds, &self.live.after[stmt as usize], self.merging)
    }

    // ── Statements ───────────────────────────────────────────────────────

    fn exec_block(&mut self, f: FnId, block: &'p Block, worlds: Vec<World>) -> Result<Flow> {
        let mut flow = Flow::next(worlds);
        for stmt in &block.stmts {
            if flow.next.is_empty() {
                break;
            }
            let input = std::mem::take(&mut flow.next);
            let out = self.exec_stmt(f, stmt, input)?;
            flow.next = out.next;
            flow.broke.extend(out.broke);
            flow.continued.extend(out.continued);
            flow.returned.extend(out.returned);
        }
        Ok(flow)
    }

    fn exec_stmt(&mut self, f: FnId, stmt: &'p Stmt, worlds: Vec<World>) -> Result<Flow> {
        let n = worlds.len();
        self.stats.world_steps += n as u64;
        self.stats.peak_worlds = self.stats.peak_worlds.max(n);
        if n > self.max_worlds {
            return Err(
                RuntimeError::new(stmt.span, format!("{n} worlds are too many to follow exactly")).with_help(
                    "simplify the model, raise the limit with `@max_worlds`, or wait for sample mode (v0.2)",
                ),
            );
        }
        let span = stmt.span;
        match &stmt.kind {
            StmtKind::Set { place, value } => {
                let mut worlds = worlds;
                for w in &mut worlds {
                    let v = self.eval(f, value, w)?;
                    self.assign(f, place, v, w, span)?;
                }
                Ok(Flow::next(worlds))
            }
            StmtKind::Draw { place, dist } => {
                let mut out = Vec::with_capacity(worlds.len());
                for w in worlds {
                    let d = self.eval(f, dist, &w)?;
                    self.split_by(f, place, d, w, span, &mut out)?;
                }
                Ok(Flow::next(self.merge(out, stmt.id)))
            }
            StmtKind::Take { place, bag } => {
                let mut out = Vec::new();
                for w in worlds {
                    let current = self.read_place(f, bag, &w, span)?;
                    let Value::Bag(cards) = &current else {
                        return Err(RuntimeError::new(
                            span,
                            format!("`take` needs a bag, found {}", ops::article(&current.kind())),
                        )
                        .with_help("make one with `bag([card: count, …])`"));
                    };
                    let total: u64 = cards.values().sum();
                    if total == 0 {
                        return Err(RuntimeError::new(span, "can't take a card from an empty bag"));
                    }
                    for (card, count) in cards.iter() {
                        let mut rest = (**cards).clone();
                        if *count > 1 {
                            *rest.get_mut(card).unwrap() -= 1;
                        } else {
                            rest.remove(card);
                        }
                        let mut nw = w.clone().scaled(*count as f64 / total as f64);
                        self.assign(f, bag, Value::Bag(Arc::new(rest)), &mut nw, span)?;
                        self.assign(f, place, card.clone(), &mut nw, span)?;
                        out.push(nw);
                    }
                }
                Ok(Flow::next(self.merge(out, stmt.id)))
            }
            StmtKind::Call { dest, callee, args } => {
                let mut out = Vec::with_capacity(worlds.len());
                for w in worlds {
                    let mut key = Vec::with_capacity(args.len() + 4);
                    for a in args {
                        key.push(self.eval(f, a, &w)?);
                    }
                    let func = match callee {
                        Callee::Fn { func, capture_args } => {
                            key.extend(capture_args.iter().map(|&s| w.slots[s as usize].clone()));
                            *func
                        }
                        Callee::Value(e) => match self.eval(f, e, &w)? {
                            Value::Closure(c) => {
                                self.check_arity(&c, args.len(), span)?;
                                key.extend(c.captured.iter().cloned());
                                c.func
                            }
                            other => {
                                return Err(RuntimeError::new(
                                    e.span,
                                    format!("can't call {}", ops::article(&other.kind())),
                                ));
                            }
                        },
                    };
                    let result = self.call(func, key, span)?;
                    self.unresolved += w.weight * result.unresolved;
                    let last = result.outcomes.len().saturating_sub(1);
                    let mut w = Some(w);
                    for (i, (v, p)) in result.outcomes.iter().enumerate() {
                        let base = if i == last {
                            w.take().unwrap()
                        } else {
                            w.clone().unwrap()
                        };
                        let mut nw = base.scaled(*p);
                        self.assign(f, dest, v.clone(), &mut nw, span)?;
                        out.push(nw);
                    }
                }
                Ok(Flow::next(self.merge(out, stmt.id)))
            }
            StmtKind::If { cond, then, otherwise } => {
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for w in worlds {
                    let p = self.eval_chance(f, cond, &w)?;
                    if p >= 1.0 {
                        yes.push(w);
                    } else if p <= 0.0 {
                        no.push(w);
                    } else {
                        no.push(w.clone().scaled(1.0 - p));
                        yes.push(w.scaled(p));
                    }
                }
                let mut flow = Flow::default();
                if !yes.is_empty() {
                    flow.join(self.exec_block(f, then, yes)?);
                }
                if !no.is_empty() {
                    flow.join(self.exec_block(f, otherwise, no)?);
                }
                flow.next = self.merge(flow.next, stmt.id);
                Ok(flow)
            }
            StmtKind::Chance {
                arms,
                otherwise,
                exhaustive,
            } => {
                let mut buckets: Vec<Vec<World>> = vec![Vec::new(); arms.len()];
                let mut rest = Vec::new();
                for w in worlds {
                    let mut chances = Vec::with_capacity(arms.len());
                    for (weight, _) in arms {
                        chances.push(self.eval_chance(f, weight, &w)?);
                    }
                    let sum: f64 = chances.iter().sum();
                    if sum > 1.0 + 1e-9 {
                        return Err(RuntimeError::new(
                            span,
                            format!(
                                "the chances in this `chance` add up to {}, more than 100%",
                                fmt_prob(sum)
                            ),
                        ));
                    }
                    let remainder = (1.0 - sum).max(0.0);
                    if remainder > 1e-9 && *exhaustive && otherwise.is_none() {
                        return Err(RuntimeError::new(
                            span,
                            format!("the chances add up to {} and there's no `else` arm", fmt_prob(sum)),
                        )
                        .with_help("a `chance` used as a value needs its chances to add up to 100%, or an `else`"));
                    }
                    for (bucket, p) in buckets.iter_mut().zip(&chances) {
                        if *p > 0.0 {
                            bucket.push(w.clone().scaled(*p));
                        }
                    }
                    if remainder > 1e-12 {
                        rest.push(w.scaled(remainder));
                    }
                }
                let mut flow = Flow::default();
                for ((_, body), bucket) in arms.iter().zip(buckets) {
                    if !bucket.is_empty() {
                        flow.join(self.exec_block(f, body, bucket)?);
                    }
                }
                if !rest.is_empty() {
                    match otherwise {
                        Some(body) => flow.join(self.exec_block(f, body, rest)?),
                        None if *exhaustive => {}
                        None => flow.next.extend(rest),
                    }
                }
                flow.next = self.merge(flow.next, stmt.id);
                Ok(flow)
            }
            StmtKind::Loop { body } => {
                let live = self.live;
                let mut inside = worlds;
                let mut out = Flow::default();
                let mut iterations: u64 = 0;
                while !inside.is_empty() {
                    let mass = total_weight(&inside);
                    if mass < self.epsilon {
                        self.unresolved += mass;
                        break;
                    }
                    iterations += 1;
                    if iterations > self.max_iterations {
                        return Err(RuntimeError::new(span, format!("this loop ran {} times without finishing", self.max_iterations))
                            .with_note(format!("worlds still inside the loop weigh {}", fmt_prob(mass)))
                            .with_help("check that every world can leave the loop; raise the limit with `@max_iterations` if it's intended"));
                    }
                    let flow = self.exec_block(f, body, inside)?;
                    out.next.extend(flow.broke);
                    out.returned.extend(flow.returned);
                    let mut again = flow.next;
                    again.extend(flow.continued);
                    inside = merge(again, &live.loop_head[stmt.id as usize], self.merging);
                }
                out.next = self.merge(out.next, stmt.id);
                Ok(out)
            }
            StmtKind::Break => Ok(Flow {
                broke: worlds,
                ..Flow::default()
            }),
            StmtKind::Continue => Ok(Flow {
                continued: worlds,
                ..Flow::default()
            }),
            StmtKind::Return(value) => {
                let mut flow = Flow::default();
                for w in worlds {
                    let v = self.eval(f, value, &w)?;
                    flow.returned.push((v, w.weight));
                }
                Ok(flow)
            }
            StmtKind::Observe { value, from } => {
                self.observed = true;
                let mut out = Vec::with_capacity(worlds.len());
                for mut w in worlds {
                    let factor = match from {
                        None => self.eval_chance(f, value, &w)?,
                        Some(d) => {
                            let v = self.eval(f, value, &w)?;
                            let dist = self.eval(f, d, &w)?;
                            likelihood(&dist, &v).map_err(|e| e.at(span))?
                        }
                    };
                    if factor > 0.0 {
                        w.weight *= factor;
                        out.push(w);
                    }
                }
                Ok(Flow::next(out))
            }
            StmtKind::Report { site, value, key } => {
                for w in &worlds {
                    let v = self.eval(f, value, w)?;
                    let k = match key {
                        Some(k) => {
                            let k_value = self.eval(f, k, w)?;
                            if k_value.is_dist() {
                                return Err(RuntimeError::new(
                                    k.span,
                                    "a `by` key must be a plain value, not a distribution",
                                )
                                .with_help("draw a value first with `~`"));
                            }
                            k_value
                        }
                        None => Value::Unit,
                    };
                    self.sinks[*site as usize].add(k, &v, w.weight);
                }
                Ok(Flow::next(worlds))
            }
            StmtKind::Fail { message } => Err(RuntimeError::new(span, message.clone())),
        }
    }

    /// Store each possible value of `d` into `place`, one world per outcome.
    fn split_by(&mut self, f: FnId, place: &Place, d: Value, w: World, span: Span, out: &mut Vec<World>) -> Result<()> {
        match d {
            Value::Dist(dist) => {
                self.unresolved += w.weight * dist.missing;
                let last = dist.outcomes.len().saturating_sub(1);
                let mut w = Some(w);
                for (i, (v, p)) in dist.outcomes.iter().enumerate() {
                    let base = if i == last {
                        w.take().unwrap()
                    } else {
                        w.clone().unwrap()
                    };
                    let mut nw = base.scaled(*p);
                    self.assign(f, place, v.clone(), &mut nw, span)?;
                    out.push(nw);
                }
            }
            Value::Prob(p) if p > 0.0 && p < 1.0 => {
                let mut no = w.clone().scaled(1.0 - p);
                self.assign(f, place, Value::Prob(0.0), &mut no, span)?;
                let mut yes = w.scaled(p);
                self.assign(f, place, Value::Prob(1.0), &mut yes, span)?;
                out.push(yes);
                out.push(no);
            }
            other => {
                let mut w = w;
                self.assign(f, place, other, &mut w, span)?;
                out.push(w);
            }
        }
        Ok(())
    }

    fn check_arity(&self, c: &Closure, given: usize, span: Span) -> Result<()> {
        let expected = self.prog.functions[c.func as usize].n_params as usize;
        if expected != given {
            return Err(RuntimeError::new(
                span,
                format!("this function takes {expected} argument(s), but {given} were given"),
            ));
        }
        Ok(())
    }

    // ── Calls ────────────────────────────────────────────────────────────

    /// Run a function as a sub-simulation and return the distribution of its
    /// result. Results are memoized: a function's behaviour depends only on
    /// its arguments and the outside values it reads, which are all in `key`.
    fn call(&mut self, func: FnId, key: Vec<Value>, span: Span) -> Result<Arc<CallResult>> {
        self.stats.calls += 1;
        let call_key = (func, key);
        if self.memoizing {
            if let Some(r) = self.memo.get(&call_key) {
                self.stats.memo_hits += 1;
                return Ok(r.clone());
            }
        }
        let prog = self.prog;
        let fun = &prog.functions[func as usize];
        if self.active.contains(&call_key) {
            let args: Vec<String> = call_key.1[..fun.n_params as usize]
                .iter()
                .map(|v| format!("{v:?}"))
                .collect();
            return Err(RuntimeError::new(
                span,
                format!(
                    "`{}({})` calls itself with the same arguments",
                    fun.name,
                    args.join(", ")
                ),
            )
            .with_note("exact mode can't solve recursion that loops back to the same call yet")
            .with_help("write it as a loop instead"));
        }
        if self.depth >= MAX_DEPTH {
            return Err(
                RuntimeError::new(span, format!("calls are nested more than {MAX_DEPTH} deep"))
                    .with_help("check for recursion that doesn't stop, or write it as a loop"),
            );
        }
        let mut slots = vec![Value::Dead; fun.n_slots()];
        let n = fun.n_params as usize;
        for (i, v) in call_key.1[..n].iter().enumerate() {
            slots[i] = v.clone();
        }
        for (cap, v) in fun.captures.iter().zip(&call_key.1[n..]) {
            slots[cap.slot as usize] = v.clone();
        }
        self.active.insert(call_key.clone());
        let saved = std::mem::replace(&mut self.unresolved, 0.0);
        self.depth += 1;
        let flow = self.exec_block(func, &fun.body, vec![World { slots, weight: 1.0 }]);
        self.depth -= 1;
        let unresolved = std::mem::replace(&mut self.unresolved, saved);
        self.active.remove(&call_key);
        let flow = flow.map_err(|e| match fun.kind {
            FnKind::Named => e.with_note(format!("in a call to `{}`", fun.name)),
            FnKind::Simulate => e.with_note("inside a `simulate` block"),
            FnKind::Lambda => e.with_note("inside a lambda"),
            FnKind::Main => e,
        })?;
        let result = Arc::new(CallResult {
            outcomes: merge_values(flow.returned),
            unresolved,
        });
        if self.memoizing {
            self.memo.insert(call_key, result.clone());
        }
        Ok(result)
    }

    /// Call a closure that must not split worlds (used by `map`, `filter`, …).
    fn call_pure(&mut self, closure: &Value, args: Vec<Value>, what: &str, span: Span) -> Result<Value> {
        let Value::Closure(c) = closure else {
            return Err(RuntimeError::new(
                span,
                format!(
                    "`{what}` needs a function, like `x -> x * 2`, found {}",
                    ops::article(&closure.kind())
                ),
            ));
        };
        self.check_arity(c, args.len(), span)?;
        let mut key = args;
        key.extend(c.captured.iter().cloned());
        let result = self.call(c.func, key, span)?;
        match result.outcomes.as_slice() {
            [(v, p)] if (p - 1.0).abs() < 1e-12 => Ok(v.clone()),
            _ => Err(RuntimeError::new(
                span,
                format!("the function given to `{what}` can't branch on chances or draw values"),
            )),
        }
    }

    // ── Places ───────────────────────────────────────────────────────────

    fn assign(&mut self, f: FnId, place: &Place, v: Value, w: &mut World, span: Span) -> Result<()> {
        if place.path.is_empty() {
            w.slots[place.slot as usize] = v;
            return Ok(());
        }
        let mut keys = Vec::with_capacity(place.path.len());
        for elem in &place.path {
            keys.push(match elem {
                PathElem::Field(name) => PathKey::Field(name.as_str()),
                PathElem::Index(e) => PathKey::Index(self.eval(f, e, w)?),
            });
        }
        let target = &mut w.slots[place.slot as usize];
        if matches!(target, Value::Dead) {
            return Err(self.no_value(f, place.slot, span));
        }
        update(target, &keys, v).map_err(|e| e.at(span))
    }

    fn read_place(&mut self, f: FnId, place: &Place, w: &World, span: Span) -> Result<Value> {
        let mut v = self.slot(f, place.slot, w, span)?;
        for elem in &place.path {
            v = match elem {
                PathElem::Field(name) => ops::field(&v, name),
                PathElem::Index(e) => {
                    let i = self.eval(f, e, w)?;
                    ops::index(&v, &i)
                }
            }
            .map_err(|e| e.at(span))?;
        }
        Ok(v)
    }

    fn slot(&self, f: FnId, s: SlotId, w: &World, span: Span) -> Result<Value> {
        match &w.slots[s as usize] {
            Value::Dead => Err(self.no_value(f, s, span)),
            v => Ok(v.clone()),
        }
    }

    fn no_value(&self, f: FnId, s: SlotId, span: Span) -> RuntimeError {
        let name = &self.prog.functions[f as usize].slots[s as usize].name;
        RuntimeError::new(span, format!("`{name}` has no value here")).with_help(
            "it's used before it's given a value; if a function reads it, call the function after the variable is set",
        )
    }

    // ── Expressions ──────────────────────────────────────────────────────

    fn eval_chance(&mut self, f: FnId, e: &Expr, w: &World) -> Result<f64> {
        let v = self.eval(f, e, w)?;
        ops::to_chance(&v).map_err(|err| err.at(e.span))
    }

    fn eval(&mut self, f: FnId, e: &Expr, w: &World) -> Result<Value> {
        let span = e.span;
        let at = |err: OpError| err.at(span);
        match &e.kind {
            ExprKind::Lit(l) => Ok(self.literal(l)),
            ExprKind::Slot(s) => self.slot(f, *s, w, span),
            ExprKind::Unary(op, x) => {
                let v = self.eval(f, x, w)?;
                ops::unary(*op, &v).map_err(at)
            }
            ExprKind::Binary(BinOp::And, a, b) => {
                let pa = self.eval_chance(f, a, w)?;
                if pa == 0.0 {
                    return Ok(Value::Prob(0.0));
                }
                let pb = self.eval_chance(f, b, w)?;
                Ok(Value::Prob(pa * pb))
            }
            ExprKind::Binary(BinOp::Or, a, b) => {
                let pa = self.eval_chance(f, a, w)?;
                if pa == 1.0 {
                    return Ok(Value::Prob(1.0));
                }
                let pb = self.eval_chance(f, b, w)?;
                Ok(Value::Prob(1.0 - (1.0 - pa) * (1.0 - pb)))
            }
            ExprKind::Binary(op, a, b) => {
                let va = self.eval(f, a, w)?;
                let vb = self.eval(f, b, w)?;
                ops::binary(*op, &va, &vb).map_err(at)
            }
            ExprKind::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval(f, item, w)?);
                }
                Ok(Value::list(values))
            }
            ExprKind::Map(entries) => {
                let mut map = BTreeMap::new();
                for (k, v) in entries {
                    let key = self.eval(f, k, w)?;
                    if key.is_dist() {
                        return Err(RuntimeError::new(k.span, "map keys can't be distributions"));
                    }
                    let value = self.eval(f, v, w)?;
                    map.insert(key, value);
                }
                Ok(Value::Map(Arc::new(map)))
            }
            ExprKind::Record { ty, fields } => {
                let mut values = Vec::with_capacity(fields.len());
                for (name, v) in fields {
                    values.push((Arc::from(name.as_str()), self.eval(f, v, w)?));
                }
                Ok(ops::make_record(
                    ty.map(|t| self.record_names[t as usize].clone()),
                    values,
                ))
            }
            ExprKind::Field(x, name) => {
                let v = self.eval(f, x, w)?;
                ops::field(&v, name).map_err(at)
            }
            ExprKind::Index(x, i) => {
                let v = self.eval(f, x, w)?;
                let i = self.eval(f, i, w)?;
                ops::index(&v, &i).map_err(at)
            }
            ExprKind::With(x, fields) => {
                let base = self.eval(f, x, w)?;
                let mut updates = Vec::with_capacity(fields.len());
                for (name, v) in fields {
                    updates.push((Arc::from(name.as_str()), self.eval(f, v, w)?));
                }
                ops::lift1(&base, |b| ops::with_fields(b, &updates)).map_err(at)
            }
            ExprKind::Builtin { func, args, .. } => self.builtin(f, *func, args, w, span),
            ExprKind::Closure { func, capture_args } => Ok(Value::Closure(Arc::new(Closure {
                func: *func,
                captured: capture_args.iter().map(|&s| w.slots[s as usize].clone()).collect(),
            }))),
            ExprKind::Simulate { func, capture_args } => {
                let key = capture_args.iter().map(|&s| w.slots[s as usize].clone()).collect();
                let result = self.call(*func, key, span)?;
                self.unresolved += w.weight * result.unresolved;
                let total: f64 = result.outcomes.iter().map(|(_, p)| p).sum();
                if total <= 0.0 {
                    return Err(RuntimeError::new(
                        span,
                        "every world in this `simulate` was ruled out by `observe`",
                    ));
                }
                let normalized = result.outcomes.iter().map(|(v, p)| (v.clone(), p / total)).collect();
                ops::combine(normalized, 0.0).map_err(at)
            }
            ExprKind::Interp(parts) => {
                let mut text = String::new();
                for part in parts {
                    match part {
                        InterpPart::Lit(s) => text.push_str(s),
                        InterpPart::Expr(e) => text.push_str(&self.eval(f, e, w)?.to_string()),
                    }
                }
                Ok(Value::str(&text))
            }
        }
    }

    fn literal(&mut self, l: &Lit) -> Value {
        match l {
            Lit::Unit => Value::Unit,
            Lit::Int(i) => Value::Int(*i),
            Lit::Float(x) => Value::Float(*x),
            Lit::Prob(p) => Value::Prob(*p),
            Lit::Str(s) => Value::str(s),
            Lit::Dice { count, sides } => self
                .dice
                .entry((*count, *sides))
                .or_insert_with(|| Dist::dice(*count, *sides).into_value())
                .clone(),
            Lit::Enum { ty, variant } => self.enum_values[*ty as usize][*variant as usize].clone(),
        }
    }

    fn builtin(&mut self, f: FnId, b: Builtin, args: &[Expr], w: &World, span: Span) -> Result<Value> {
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.eval(f, a, w)?);
        }
        let at = |err: OpError| err.at(span);
        match b {
            Builtin::Print => {
                let text: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                let text = text.join(" ");
                let line = if w.weight == 1.0 {
                    text
                } else {
                    format!("[{}] {text}", fmt_prob(w.weight))
                };
                (self.print)(&line);
                Ok(Value::Unit)
            }
            Builtin::Map | Builtin::Filter | Builtin::Reduce => self.higher_order(b, &values, span),
            Builtin::Count if values.len() == 2 => self.higher_order(b, &values, span),
            Builtin::Roll => self.roll(&values).map_err(at),
            Builtin::Take => Err(RuntimeError::new(span, "`take()` can only be used with `~`")),
            _ if b.lifting() == Lifting::Raw => builtins::call_raw(b, &values).map_err(at),
            _ => ops::lift_n(&values, &|a| builtins::call_plain(b, a)).map_err(at),
        }
    }

    fn roll(&mut self, values: &[Value]) -> OpResult<Value> {
        let count = match &values[0] {
            Value::Int(n) if (0..=1000).contains(n) => *n as u32,
            Value::Int(_) => return Err(OpError::new("roll needs between 0 and 1000 dice")),
            v if v.is_dist() => {
                return Err(OpError::new("the number of dice to roll must be a plain number")
                    .help("draw it first, like `let n ~ d4`"));
            }
            v => {
                return Err(OpError::new(format!(
                    "roll needs a number of dice, found {}",
                    ops::article(&v.kind())
                )));
            }
        };
        let die = match &values[1] {
            Value::Int(sides) if *sides >= 1 => Dist::dice(1, *sides as u32).into_value(),
            v @ Value::Dist(_) => v.clone(),
            v => {
                return Err(OpError::new(format!(
                    "roll needs a die, like `d6`, found {}",
                    ops::article(&v.kind())
                )));
            }
        };
        if let Some(pool) = self.pools.get(&(count, die.clone())) {
            return Ok(pool.clone());
        }
        let Value::Dist(d) = &die else { unreachable!() };
        let pool = Dist::pool(count, d).into_value();
        self.pools.insert((count, die), pool.clone());
        Ok(pool)
    }

    /// `map`, `filter`, `count(xs, test)` and `reduce`, which call a function.
    fn higher_order(&mut self, b: Builtin, values: &[Value], span: Span) -> Result<Value> {
        if let Value::Dist(d) = &values[0] {
            let mut results = Vec::with_capacity(d.outcomes.len());
            for (coll, p) in &d.outcomes {
                let mut args = values.to_vec();
                args[0] = coll.clone();
                results.push((self.higher_order(b, &args, span)?, *p));
            }
            return ops::combine(results, d.missing).map_err(|e| e.at(span));
        }
        let items: Vec<Value> = match &values[0] {
            Value::List(items) => (**items).clone(),
            Value::Range(lo, hi) => (*lo..=*hi).map(Value::Int).collect(),
            other => {
                return Err(RuntimeError::new(
                    span,
                    format!("`{}` needs a list, found {}", b.name(), ops::article(&other.kind())),
                ));
            }
        };
        match b {
            Builtin::Map => {
                let mut out = Vec::with_capacity(items.len());
                for x in items {
                    out.push(self.call_pure(&values[1], vec![x], "map", span)?);
                }
                Ok(Value::list(out))
            }
            Builtin::Filter | Builtin::Count => {
                let mut kept = Vec::new();
                for x in items {
                    let test = self.call_pure(&values[1], vec![x.clone()], b.name(), span)?;
                    match ops::to_chance(&test) {
                        Ok(1.0) => kept.push(x),
                        Ok(0.0) => {}
                        Ok(_) => {
                            return Err(RuntimeError::new(
                                span,
                                format!("the test given to `{}` must be certain for each item", b.name()),
                            )
                            .with_help("it returned a probability between 0% and 100%"));
                        }
                        Err(e) => return Err(e.at(span)),
                    }
                }
                Ok(if b == Builtin::Count {
                    Value::Int(kept.len() as i64)
                } else {
                    Value::list(kept)
                })
            }
            Builtin::Reduce => {
                let mut acc = values[1].clone();
                for x in items {
                    acc = self.call_pure(&values[2], vec![acc, x], "reduce", span)?;
                }
                Ok(acc)
            }
            _ => unreachable!(),
        }
    }
}

enum PathKey<'a> {
    Field(&'a str),
    Index(Value),
}

fn update(target: &mut Value, keys: &[PathKey], v: Value) -> OpResult<()> {
    let Some((first, rest)) = keys.split_first() else {
        *target = v;
        return Ok(());
    };
    match (first, target) {
        (PathKey::Field(name), Value::Record(r)) => {
            let r = Arc::make_mut(r);
            let slot = r
                .get_mut(name)
                .ok_or_else(|| OpError::new(format!("this record has no field `{name}`")))?;
            update(slot, rest, v)
        }
        (PathKey::Index(i), Value::List(items)) => {
            let items = Arc::make_mut(items);
            let k = ops::as_index(i, items.len())?;
            update(&mut items[k], rest, v)
        }
        (PathKey::Index(k), Value::Map(m)) => {
            let m = Arc::make_mut(m);
            if rest.is_empty() {
                m.insert(k.clone(), v);
                return Ok(());
            }
            let slot = m
                .get_mut(k)
                .ok_or_else(|| OpError::new(format!("the key {k:?} isn't in the map")))?;
            update(slot, rest, v)
        }
        (PathKey::Field(name), other) => Err(OpError::new(format!(
            "can't set the field `{name}` of {}",
            ops::article(&other.kind())
        ))),
        (PathKey::Index(_), other) => Err(OpError::new(format!(
            "can't set an element of {}",
            ops::article(&other.kind())
        ))),
    }
}

/// The probability of observing `v` from `d`.
fn likelihood(d: &Value, v: &Value) -> OpResult<f64> {
    match d {
        Value::Dist(dist) => Ok(dist
            .outcomes
            .iter()
            .filter(|(x, _)| ops::equals(x, v))
            .map(|(_, p)| p)
            .sum()),
        Value::Prob(p) => match v {
            Value::Prob(b) if *b == 1.0 => Ok(*p),
            Value::Prob(b) if *b == 0.0 => Ok(1.0 - p),
            other => Err(OpError::new(format!(
                "a probability can only produce `true` or `false`, not {other:?}"
            ))),
        },
        other => Ok(if ops::equals(other, v) { 1.0 } else { 0.0 }),
    }
}
