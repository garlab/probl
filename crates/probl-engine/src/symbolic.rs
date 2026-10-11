//! Experimental: answering finite Boolean programs with decision diagrams
//! instead of worlds (docs/design/symbolic-inference.md, stage 1).
//!
//! Each draw is a variable, each Boolean a diagram over the variables, and
//! the evidence one diagram. A report is then weighted model counts: of the
//! worlds that reach it with its value true, and with it false. Enumeration
//! would keep a world for each assignment of the draws that are still live,
//! which can be all of them; a diagram shares what the assignments have in
//! common. The results are the same, up to rounding.
//!
//! It runs programs whose top level is made of draws from Boolean laws
//! (like `bernoulli(1%)`, or a law that depends on Booleans, like
//! `if sick { bernoulli(95%) } else { bernoulli(8%) }`), Boolean
//! expressions, assignments, `if`, observations, `score` and reports
//! without `by`. Values other than Booleans, like probabilities and laws,
//! are constants, or one constant in each of a few sets of worlds. Anything
//! else, and any operation that fails, is unsupported: the engine
//! enumerates such a program instead, or, with `Symbolic::Only`, says what
//! isn't supported.

mod bdd;

use crate::dist::{Budget, Dist};
use crate::error::{OpError, OpResult, RuntimeError};
use crate::ops;
use crate::report::Sink;
use crate::value::Value;
use crate::weight::Weight;
use bdd::{Bdd, Counts, Manager, Stop};
use probl_sema::Builtin;
use probl_sema::ir::{Block, Expr, ExprKind, Lit, Program, SlotId, StmtKind, TypeSpec};
use probl_syntax::Span;
use probl_syntax::ast::{BinOp, UnOp};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether a program is answered with decision diagrams (docs/design/
/// symbolic-inference.md). Experimental, and off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Symbolic {
    /// Enumerate worlds.
    #[default]
    Off,
    /// Use decision diagrams for the programs they support, and enumerate
    /// the others, and those whose diagrams outgrow `Limits::max_nodes`.
    Prefer,
    /// Use decision diagrams, and fail on a program they don't support.
    Only,
}

/// The size of a run's decision diagrams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diagrams {
    /// One for each trial that wasn't certain.
    pub variables: u64,
    pub nodes: u64,
}

/// What the diagrams found: what enumeration would have.
pub(crate) struct Answer {
    pub sinks: Vec<Sink>,
    /// The weight of the worlds that finish: the evidence.
    pub finished: Weight,
    /// Whether an `observe` was reached.
    pub observed: bool,
    /// The last `observe` that ruled out a world.
    pub last_ruling_out: Option<Span>,
    pub diagrams: Diagrams,
}

/// Why diagrams didn't answer.
pub(crate) enum Fallback {
    /// The program uses something they don't support.
    Unsupported(RuntimeError),
    /// They would have more nodes than allowed.
    TooLarge(RuntimeError),
    /// The run stops: it was cancelled.
    Stopped(RuntimeError),
}

fn unsupported(what: impl std::fmt::Display, span: Span) -> Fallback {
    Fallback::Unsupported(
        OpError::unsupported(format!("{what} isn't supported by the symbolic backend"))
            .help("it's experimental: it supports draws from Boolean laws like `bernoulli(30%)`, booleans, `if`, `observe`, `score` and reports without `by`, at the top level")
            .at(span),
    )
}

/// Answer `program` with decision diagrams of at most `max_nodes` nodes.
pub(crate) fn run(
    program: &Program,
    budget: Budget,
    max_nodes: usize,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Answer, Fallback> {
    let main = program.main();
    check_block(&main.body)?;
    let mut run = Run {
        dd: Manager::new(max_nodes, cancel.clone()),
        weights: Vec::new(),
        slots: vec![Sym::Unset; main.n_slots()],
        log: Vec::new(),
        evidence: Bdd::TRUE,
        observed: false,
        last_ruling_out: None,
        sinks: vec![Sink::default(); program.reports.len()],
        counts: Counts::default(),
        budget,
        cancel,
        max_nodes,
    };
    run.block(&main.body, Bdd::TRUE)?;
    let (finished, _) = run.dd.count(run.evidence, &run.weights, &mut run.counts);
    Ok(Answer {
        sinks: run.sinks,
        finished,
        observed: run.observed,
        last_ruling_out: run.last_ruling_out,
        diagrams: Diagrams {
            variables: run.dd.vars() as u64,
            nodes: run.dd.nodes() as u64,
        },
    })
}

// ── What's supported ─────────────────────────────────────────────────────

/// Whether a block uses only what diagrams support, checked before running
/// it. Running it can still find values they don't, like a probability out
/// of range.
fn check_block(block: &Block) -> Result<(), Fallback> {
    for stmt in &block.stmts {
        let span = stmt.span;
        match &stmt.kind {
            StmtKind::Set { place, value } | StmtKind::Draw { place, dist: value } => {
                if !place.path.is_empty() {
                    return Err(unsupported("assigning part of a variable", span));
                }
                check_expr(value)?;
            }
            StmtKind::If { cond, then, otherwise } => {
                check_expr(cond)?;
                check_block(then)?;
                check_block(otherwise)?;
            }
            StmtKind::Observe { value, from } => {
                check_expr(value)?;
                from.iter().try_for_each(check_expr)?;
            }
            StmtKind::Report { value, key: None, .. } => check_expr(value)?,
            StmtKind::Report { key: Some(k), .. } => return Err(unsupported("a report with `by`", k.span)),
            StmtKind::Check { .. } => {}
            StmtKind::Take { .. } => return Err(unsupported("taking from a bag", span)),
            StmtKind::Call { .. } => return Err(unsupported("calling a function", span)),
            StmtKind::Chance { .. } => return Err(unsupported("`chance`", span)),
            StmtKind::Loop { .. } => return Err(unsupported("a loop", span)),
            StmtKind::Break | StmtKind::Continue | StmtKind::Return(_) => {
                return Err(unsupported("leaving a block early", span));
            }
            StmtKind::Fail { .. } => return Err(unsupported("an error", span)),
            StmtKind::Try { .. } => return Err(unsupported("`try`", span)),
        }
    }
    Ok(())
}

/// The built-ins diagrams support: Boolean laws, and the tests the
/// lowering of `and` and `or` makes.
fn supported(b: Builtin) -> bool {
    matches!(
        b,
        Builtin::Bernoulli | Builtin::ScoreLaw | Builtin::BooleanLaw | Builtin::IsTrue | Builtin::IsFalse
    )
}

fn check_expr(e: &Expr) -> Result<(), Fallback> {
    match &e.kind {
        ExprKind::Lit(Lit::Dice { .. }) => Err(unsupported("dice", e.span)),
        ExprKind::Lit(Lit::Enum { .. }) => Err(unsupported("an enum", e.span)),
        ExprKind::Lit(_) | ExprKind::Slot(_) => Ok(()),
        ExprKind::Unary(UnOp::Typeof, _) => Err(unsupported("`typeof`", e.span)),
        ExprKind::Unary(_, x) => check_expr(x),
        ExprKind::Binary(_, a, b) => {
            check_expr(a)?;
            check_expr(b)
        }
        ExprKind::Builtin { func, args, named } if supported(*func) && named.is_empty() => {
            args.iter().try_for_each(check_expr)
        }
        ExprKind::Builtin { func, .. } => Err(unsupported(format_args!("`{}`", func.name()), e.span)),
        ExprKind::List(_) | ExprKind::Map(_) | ExprKind::Record { .. } | ExprKind::With(..) => {
            Err(unsupported("a collection", e.span))
        }
        ExprKind::Field(..) | ExprKind::Index(..) => Err(unsupported("a field or an index", e.span)),
        ExprKind::Closure { .. } => Err(unsupported("a function value", e.span)),
        ExprKind::Simulate { .. } => Err(unsupported("`simulate`", e.span)),
        ExprKind::Interp(_) => Err(unsupported("string interpolation", e.span)),
        ExprKind::Input(_) => Err(unsupported("data", e.span)),
    }
}

// ── Running ──────────────────────────────────────────────────────────────

/// A variable's value in every world on the current path.
#[derive(Clone, Debug, PartialEq)]
enum Sym {
    /// Not assigned on this path.
    Unset,
    Bool(Bdd),
    /// A value other than a Boolean in each set of worlds: a probability, a
    /// law… The sets don't overlap, and together they're every world where
    /// the variable is assigned. A constant is one value in every world.
    Values(Vec<(Bdd, Value)>),
    /// A Boolean on some paths, and another value on others.
    Mixed,
}

impl Sym {
    fn fact(b: bool) -> Sym {
        Sym::Bool(if b { Bdd::TRUE } else { Bdd::FALSE })
    }

    fn constant(v: Value) -> Sym {
        Sym::Values(vec![(Bdd::TRUE, v)])
    }
}

struct Run {
    dd: Manager,
    /// Each variable's weight when false and when true.
    weights: Vec<(Weight, Weight)>,
    slots: Vec<Sym>,
    /// What each assignment replaced, oldest first, to undo a branch.
    log: Vec<(SlotId, Sym)>,
    evidence: Bdd,
    observed: bool,
    last_ruling_out: Option<Span>,
    sinks: Vec<Sink>,
    counts: Counts,
    budget: Budget,
    cancel: Option<Arc<AtomicBool>>,
    max_nodes: usize,
}

impl Run {
    /// Run a block in the worlds where `guard` holds.
    fn block(&mut self, block: &Block, guard: Bdd) -> Result<(), Fallback> {
        for stmt in &block.stmts {
            let span = stmt.span;
            // Like enumeration, run nothing where no world is left: the
            // values of the variables don't matter there.
            if self.dd.and(guard, self.evidence) == Bdd::FALSE {
                return Ok(());
            }
            match &stmt.kind {
                StmtKind::Set { place, value } => {
                    let v = self.eval(value)?;
                    self.assign(place.slot, v);
                }
                StmtKind::Draw { place, dist } => {
                    let d = self.eval(dist)?;
                    let v = self.draw(d, dist.span)?;
                    self.assign(place.slot, v);
                }
                StmtKind::If { cond, then, otherwise } => {
                    let c = self.eval(cond)?;
                    let c = self.condition(c, cond.span)?;
                    self.branch(c, then, otherwise, guard)?;
                }
                StmtKind::Observe { value, from } => {
                    let v = self.eval(value)?;
                    let Sym::Bool(b) = v else {
                        return Err(unsupported("observing something other than a boolean", value.span));
                    };
                    // The worlds that keep their weight, or part of it, and
                    // those that are ruled out.
                    let (kept, ruled_out) = match from {
                        None => (b, b.not()),
                        Some(d) => {
                            let d = self.eval(d)?;
                            self.likelihood(b, d, span)?
                        }
                    };
                    self.observe(guard, kept, ruled_out, span);
                }
                StmtKind::Report { site, value, .. } => {
                    let v = self.eval(value)?;
                    self.report(*site as usize, v, guard, value.span)?;
                }
                StmtKind::Check { slot, ty } => {
                    let fits = |v: &Value| {
                        matches!(
                            (v, ty),
                            (Value::Unit, TypeSpec::Unit)
                                | (Value::Int(_), TypeSpec::Int)
                                | (Value::Float(_), TypeSpec::Float)
                                | (Value::Prob(_), TypeSpec::Prob)
                                | (Value::Str(_), TypeSpec::Str)
                        )
                    };
                    let ok = match (&self.slots[*slot as usize], ty) {
                        (Sym::Bool(_), TypeSpec::Bool) => true,
                        (Sym::Values(vs), _) => vs.iter().all(|(_, v)| fits(v)),
                        _ => false,
                    };
                    if !ok {
                        return Err(unsupported("this type check", span));
                    }
                }
                _ => unreachable!("checked before running"),
            }
            self.check(span)?;
        }
        Ok(())
    }

    /// Stop if the diagrams outgrew their budget, or the run was cancelled.
    fn check(&self, span: Span) -> Result<(), Fallback> {
        let cancelled = || self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed));
        match self.dd.stopped() {
            Some(Stop::TooLarge) => Err(Fallback::TooLarge(RuntimeError::limit(
                span,
                format!("the decision diagrams would need more than {} nodes", self.max_nodes),
            ))),
            Some(Stop::Cancelled) => Err(Fallback::Stopped(RuntimeError::limit(span, "the run was cancelled"))),
            None if cancelled() => Err(Fallback::Stopped(RuntimeError::limit(span, "the run was cancelled"))),
            None => Ok(()),
        }
    }

    fn assign(&mut self, slot: SlotId, v: Sym) {
        let old = std::mem::replace(&mut self.slots[slot as usize], v);
        self.log.push((slot, old));
    }

    /// `if c`: each branch in its worlds, and then each variable either
    /// branch assigned is one or the other, as `c` says.
    fn branch(&mut self, c: Bdd, then: &Block, otherwise: &Block, guard: Bdd) -> Result<(), Fallback> {
        let yes = self.dd.and(guard, c);
        let no = self.dd.and(guard, c.not());
        // Enumeration runs a branch only if a world gets there.
        let reaches = |run: &mut Run, path: Bdd| run.dd.and(path, run.evidence) != Bdd::FALSE;
        let (go_then, go_else) = (reaches(self, yes), reaches(self, no));
        if !go_then || !go_else {
            // The variables of the other branch's worlds don't matter: it
            // has none.
            if go_then {
                self.block(then, yes)?;
            } else if go_else {
                self.block(otherwise, no)?;
            }
            return Ok(());
        }
        let mark = self.log.len();
        self.block(then, yes)?;
        // What the first branch left, and the values before it, back.
        let mut after_then: FxHashMap<SlotId, Sym> = FxHashMap::default();
        for (slot, _) in &self.log[mark..] {
            after_then
                .entry(*slot)
                .or_insert_with(|| self.slots[*slot as usize].clone());
        }
        for (slot, old) in self.log.drain(mark..).rev() {
            self.slots[slot as usize] = old;
        }
        self.block(otherwise, no)?;
        let mut touched: Vec<SlotId> = self.log[mark..].iter().map(|(s, _)| *s).collect();
        touched.extend(after_then.keys());
        touched.sort_unstable();
        touched.dedup();
        for slot in touched {
            // Before the branches: what the second one replaced first, or
            // what's there if it didn't assign it.
            let before = self.log[mark..]
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, old)| old.clone());
            let then_value = after_then
                .remove(&slot)
                .or_else(|| before.clone())
                .unwrap_or_else(|| self.slots[slot as usize].clone());
            let else_value = self.slots[slot as usize].clone();
            let merged = self.merge(c, then_value, else_value);
            if before.is_none() {
                self.log.push((slot, self.slots[slot as usize].clone()));
            }
            self.slots[slot as usize] = merged;
        }
        Ok(())
    }

    /// `c ? a : b`.
    fn merge(&mut self, c: Bdd, a: Sym, b: Sym) -> Sym {
        match (a, b) {
            (a, b) if a == b => a,
            (Sym::Unset, x) | (x, Sym::Unset) => x,
            (Sym::Bool(a), Sym::Bool(b)) => Sym::Bool(self.dd.ite(c, a, b)),
            (Sym::Values(a), Sym::Values(b)) => {
                let mut parts = Vec::with_capacity(a.len() + b.len());
                for (g, v) in a {
                    parts.push((self.dd.and(c, g), v));
                }
                for (g, v) in b {
                    parts.push((self.dd.and(c.not(), g), v));
                }
                Sym::Values(self.gather(parts))
            }
            _ => Sym::Mixed,
        }
    }

    /// Values by set of worlds, each value once, without empty sets.
    fn gather(&mut self, parts: Vec<(Bdd, Value)>) -> Vec<(Bdd, Value)> {
        let mut out: Vec<(Bdd, Value)> = Vec::with_capacity(parts.len());
        for (g, v) in parts {
            if g == Bdd::FALSE {
                continue;
            }
            match out.iter_mut().find(|(_, x)| *x == v) {
                Some((h, _)) => *h = self.dd.or(*h, g),
                None => out.push((g, v)),
            }
        }
        out
    }

    /// The values of `f` applied to each value, in the same worlds.
    fn map(
        &mut self,
        vs: Vec<(Bdd, Value)>,
        span: Span,
        f: impl Fn(&Value, &mut Budget) -> OpResult<Value>,
    ) -> Result<Sym, Fallback> {
        let mut parts = Vec::with_capacity(vs.len());
        for (g, v) in vs {
            match f(&v, &mut self.budget) {
                Ok(x) => parts.push((g, x)),
                Err(_) => return Err(unsupported("an operation that fails", span)),
            }
        }
        self.values(parts, span)
    }

    /// Values as a variable holds them: Booleans as the worlds where
    /// they're true.
    fn values(&mut self, parts: Vec<(Bdd, Value)>, span: Span) -> Result<Sym, Fallback> {
        let booleans = parts.iter().filter(|(_, v)| matches!(v, Value::Bool(_))).count();
        if booleans == 0 {
            if parts.iter().any(|(_, v)| !plain(v)) {
                return Err(unsupported("this value", span));
            }
            return Ok(Sym::Values(self.gather(parts)));
        }
        if booleans < parts.len() {
            return Err(unsupported(
                "a value that's a boolean in some worlds and not in others",
                span,
            ));
        }
        let mut b = Bdd::FALSE;
        for (g, v) in parts {
            if v == Value::Bool(true) {
                b = self.dd.or(b, g);
            }
        }
        Ok(Sym::Bool(b))
    }

    /// A variable for a fresh trial with these weights, or a constant if
    /// it's certain.
    fn trial(&mut self, no: f64, yes: f64) -> Bdd {
        if yes <= 0.0 {
            return Bdd::FALSE;
        }
        if no <= 0.0 {
            return Bdd::TRUE;
        }
        self.weights.push((Weight::new(no), Weight::new(yes)));
        self.dd.var()
    }

    /// A fresh trial from a law of booleans (or a probability, drawn as
    /// `bernoulli(p)`).
    fn draw_law(&mut self, law: &Value, span: Span) -> Result<Bdd, Fallback> {
        let d = match law {
            Value::Prob(p) => Arc::new(Dist::bernoulli(*p)),
            Value::Dist(d) => d.clone(),
            _ => return Err(unsupported("drawing from this", span)),
        };
        if d.missing > 0.0 {
            return Err(unsupported("a distribution with unresolved probability", span));
        }
        let (mut no, mut yes) = (0.0, 0.0);
        for (v, p) in &d.outcomes {
            match v {
                Value::Bool(false) => no = *p,
                Value::Bool(true) => yes = *p,
                _ => return Err(unsupported("drawing values other than booleans", span)),
            }
        }
        Ok(self.trial(no, yes))
    }

    /// `x ~ d`: in each set of worlds, a fresh trial of its law.
    fn draw(&mut self, d: Sym, span: Span) -> Result<Sym, Fallback> {
        match d {
            Sym::Values(laws) => {
                let mut b = Bdd::FALSE;
                for (g, law) in laws {
                    let t = self.draw_law(&law, span)?;
                    let here = self.dd.and(g, t);
                    b = self.dd.or(b, here);
                }
                Ok(Sym::Bool(b))
            }
            // A value that isn't a distribution is its only outcome.
            Sym::Bool(_) => Ok(d),
            _ => Err(unsupported(
                "drawing from a variable whose value differs between paths",
                span,
            )),
        }
    }

    /// The worlds where a condition holds. A probability, or a law of
    /// booleans, is a fresh trial each time (docs/semantics.md, section 3).
    fn condition(&mut self, c: Sym, span: Span) -> Result<Bdd, Fallback> {
        match c {
            Sym::Bool(b) => Ok(b),
            Sym::Values(vs) => {
                let mut b = Bdd::FALSE;
                for (g, v) in vs {
                    let c = ops::condition(&v).map_err(|_| unsupported("this condition", span))?;
                    if c.missing > 0.0 {
                        return Err(unsupported("a condition with unresolved probability", span));
                    }
                    let t = self.trial(c.no, c.yes);
                    let here = self.dd.and(g, t);
                    b = self.dd.or(b, here);
                }
                Ok(b)
            }
            _ => Err(unsupported("a variable whose value differs between paths", span)),
        }
    }

    /// `observe b from d`: the worlds a trial of `d` agrees with `b` in,
    /// which keep that part of their weight, and those it never could,
    /// which are ruled out.
    fn likelihood(&mut self, b: Bdd, d: Sym, span: Span) -> Result<(Bdd, Bdd), Fallback> {
        let Sym::Values(laws) = d else {
            return Err(unsupported("observing a value from this", span));
        };
        let (mut kept, mut ruled_out) = (Bdd::FALSE, Bdd::FALSE);
        for (g, law) in laws {
            if matches!(law, Value::Prob(_)) {
                // An error, as enumeration says.
                return Err(unsupported("observing a value from a probability", span));
            }
            let t = self.draw_law(&law, span)?;
            let agrees = self.dd.xor(t, b).not();
            let here = self.dd.and(g, agrees);
            kept = self.dd.or(kept, here);
            // A certain law rules out the worlds that disagree; an
            // uncertain one only weighs them.
            if t == Bdd::TRUE || t == Bdd::FALSE {
                let here = self.dd.and(g, agrees.not());
                ruled_out = self.dd.or(ruled_out, here);
            }
        }
        Ok((kept, ruled_out))
    }

    /// The worlds on the path keep only `kept`.
    fn observe(&mut self, guard: Bdd, kept: Bdd, ruled_out: Bdd, span: Span) {
        let reached = self.dd.and(guard, self.evidence);
        if reached == Bdd::FALSE {
            return;
        }
        self.observed = true;
        if self.dd.and(reached, ruled_out) != Bdd::FALSE {
            self.last_ruling_out = Some(span);
        }
        let others = self.dd.or(guard.not(), kept);
        self.evidence = self.dd.and(self.evidence, others);
    }

    /// Add what the worlds on this path report.
    fn report(&mut self, site: usize, v: Sym, guard: Bdd, span: Span) -> Result<(), Fallback> {
        let reached = self.dd.and(guard, self.evidence);
        if reached == Bdd::FALSE {
            return Ok(());
        }
        // The worlds that reach it with each value, counted.
        let parts = match v {
            Sym::Bool(b) => vec![(b, Value::Bool(true)), (b.not(), Value::Bool(false))],
            Sym::Values(vs) => vs,
            _ => {
                return Err(unsupported(
                    "reporting a variable whose value differs between paths",
                    span,
                ));
            }
        };
        for (worlds, value) in parts {
            let mut pairs = FxHashMap::default();
            let weight = self
                .dd
                .count_and(reached, worlds, &self.weights, &mut self.counts, &mut pairs);
            if weight.is_zero() && !matches!(value, Value::Bool(_)) {
                continue;
            }
            let sink = &self.sinks[site];
            let valid = sink
                .validate_analytic(&Value::Unit, &value)
                .and_then(|()| sink.validate_structure(&Value::Unit, &value));
            if valid.is_err() {
                return Err(unsupported("this report", span));
            }
            self.sinks[site].add(Value::Unit, &value, weight, None);
        }
        Ok(())
    }

    fn eval(&mut self, e: &Expr) -> Result<Sym, Fallback> {
        let span = e.span;
        let varies = || unsupported("a variable whose value differs between paths", span);
        Ok(match &e.kind {
            ExprKind::Lit(l) => match literal(l, &mut self.budget) {
                Ok(Value::Bool(b)) => Sym::fact(b),
                Ok(v) => Sym::constant(v),
                Err(_) => return Err(unsupported("this literal", span)),
            },
            ExprKind::Slot(s) => match &self.slots[*s as usize] {
                Sym::Unset | Sym::Mixed => return Err(varies()),
                v => v.clone(),
            },
            ExprKind::Unary(op, x) => match (op, self.eval(x)?) {
                (UnOp::Not, Sym::Bool(b)) => Sym::Bool(b.not()),
                (op, Sym::Values(vs)) => self.map(vs, span, |v, budget| ops::unary(*op, v, budget))?,
                _ => return Err(unsupported("this operation", span)),
            },
            ExprKind::Binary(op, a, b) => {
                let a = self.eval(a)?;
                let b = self.eval(b)?;
                match (op, a, b) {
                    (BinOp::And, Sym::Bool(x), Sym::Bool(y)) => Sym::Bool(self.dd.and(x, y)),
                    (BinOp::Or, Sym::Bool(x), Sym::Bool(y)) => Sym::Bool(self.dd.or(x, y)),
                    (BinOp::Eq, Sym::Bool(x), Sym::Bool(y)) => Sym::Bool(self.dd.xor(x, y).not()),
                    (BinOp::Ne, Sym::Bool(x), Sym::Bool(y)) => Sym::Bool(self.dd.xor(x, y)),
                    (BinOp::And | BinOp::Or, _, _) => return Err(unsupported("this operation", span)),
                    (op, Sym::Values(xs), Sym::Values(ys)) => {
                        let mut parts = Vec::with_capacity(xs.len() * ys.len());
                        for (g, x) in &xs {
                            for (h, y) in &ys {
                                let both = self.dd.and(*g, *h);
                                if both == Bdd::FALSE {
                                    continue;
                                }
                                match ops::binary(*op, x, y, &mut self.budget) {
                                    Ok(v) => parts.push((both, v)),
                                    Err(_) => return Err(unsupported("an operation that fails", span)),
                                }
                            }
                        }
                        self.values(parts, span)?
                    }
                    _ => return Err(unsupported("this operation", span)),
                }
            }
            ExprKind::Builtin { func, args, .. } => {
                let x = self.eval(&args[0])?;
                match (func, x) {
                    (Builtin::IsTrue, Sym::Bool(b)) => Sym::Bool(b),
                    (Builtin::IsFalse, Sym::Bool(b)) => Sym::Bool(b.not()),
                    (Builtin::IsTrue | Builtin::IsFalse, Sym::Values(vs)) => {
                        let truth = *func == Builtin::IsTrue;
                        self.map(vs, span, |v, _| Ok(Value::Bool(ops::is_certain(v, truth))))?
                    }
                    (Builtin::Bernoulli | Builtin::ScoreLaw, Sym::Values(vs)) => {
                        self.map(vs, span, |p, _| Ok(Dist::bernoulli(ops::to_prob(p)?).into_value()))?
                    }
                    (Builtin::BooleanLaw, Sym::Values(vs)) => self.map(vs, span, |v, _| ops::boolean_law(v))?,
                    (Builtin::BooleanLaw, Sym::Bool(b)) => {
                        // A fact's law is certain: a trial of it is the fact.
                        let certain = |b| Value::Dist(Arc::new(Dist::bernoulli(if b { 1.0 } else { 0.0 })));
                        let parts = vec![(b, certain(true)), (b.not(), certain(false))];
                        Sym::Values(self.gather(parts))
                    }
                    (_, Sym::Unset | Sym::Mixed) => return Err(varies()),
                    _ => return Err(unsupported("this operation", span)),
                }
            }
            _ => unreachable!("checked before running"),
        })
    }
}

/// A literal's value, as the engine makes it.
fn literal(l: &Lit, budget: &mut Budget) -> OpResult<Value> {
    Ok(match l {
        Lit::Builtin(b) => Value::Builtin(*b),
        Lit::Unit => Value::Unit,
        Lit::Bool(b) => Value::Bool(*b),
        Lit::Int(i) => {
            budget.integer_bits(i.bits())?;
            Value::Int(i.clone())
        }
        Lit::Float(x) | Lit::FloatConstant(x) => Value::Float(*x),
        Lit::Prob(p) => Value::Prob(*p),
        Lit::Str(s) => crate::text::value(s, budget)?,
        Lit::Dice { .. } | Lit::Enum { .. } => unreachable!("checked before running"),
    })
}

/// Whether diagrams can carry a value: not a continuous value, a function
/// or a law too broad to list.
fn plain(v: &Value) -> bool {
    match v {
        Value::Unit | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Prob(_) | Value::Str(_) => true,
        Value::Builtin(_) => true,
        Value::Dist(d) => d.missing == 0.0 && d.outcomes.iter().all(|(v, _)| plain(v)),
        _ => false,
    }
}
