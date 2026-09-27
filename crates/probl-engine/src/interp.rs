//! The world-set interpreter.
//!
//! Every statement maps a set of worlds to a set of worlds. All the worlds in
//! a set are at the same point of the program, so control flow is shared and
//! only the data differs. Splitting happens in statements; expressions are
//! evaluated in one world at a time. See docs/semantics.md for the rules this
//! implements.

use crate::builtins;
use crate::chain::{Chain, Solution};
use crate::conjugate::{self, Seen};
use crate::continuous::{Family, Rng};
use crate::dist::{Budget, Counts, Dist};
use crate::error::{ErrorKind, OpError, OpResult, Result, RuntimeError};
use crate::ops::{self, Truth};
use crate::report::Sink;
use crate::value::{Closure, Delayed, Value, fmt_prob};
use crate::weight::Weight;
use crate::world::{Flow, World, clear, clear_dead, live_slots, merge, merge_values, state_hash, total_weight};
use probl_sema::builtins::Lifting;
use probl_sema::conjugate::{Conjugacy, Likelihood, Update};
use probl_sema::ir::*;
use probl_sema::{Builtin, Liveness};
use probl_syntax::Span;
use probl_syntax::ast::BinOp;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Largest number of worlds any statement ran on.
    pub peak_worlds: usize,
    /// Statement executions, counting each world separately.
    pub world_steps: u64,
    pub calls: u64,
    pub memo_hits: u64,
    /// Loops solved as Markov chains, and their states (section 10).
    pub solved_loops: u64,
    pub chain_states: u64,
    /// When sampling: what happened to each variable whose draws may be
    /// delayed (`Conjugacy::variables`).
    pub updates: Vec<Updates>,
}

/// How often a variable's draws were delayed and updated exactly
/// (docs/semantics.md, section 14), counted over all runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Updates {
    /// Draws delayed.
    pub delayed: u64,
    /// Observations that updated it exactly.
    pub exact: u64,
    /// Times it was drawn when first needed, from its updated distribution.
    pub drawn: u64,
}

impl Stats {
    /// Add up another batch's statistics.
    pub fn absorb(&mut self, other: &Stats) {
        self.peak_worlds = self.peak_worlds.max(other.peak_worlds);
        self.world_steps += other.world_steps;
        self.calls += other.calls;
        self.memo_hits += other.memo_hits;
        self.solved_loops += other.solved_loops;
        self.chain_states += other.chain_states;
        if self.updates.len() < other.updates.len() {
            self.updates.resize(other.updates.len(), Updates::default());
        }
        for (mine, theirs) in self.updates.iter_mut().zip(&other.updates) {
            mine.delayed += theirs.delayed;
            mine.exact += theirs.exact;
            mine.drawn += theirs.drawn;
        }
    }
}

/// How the engine runs, with the host's limits already applied.
#[derive(Clone, Debug)]
pub struct Config {
    pub epsilon: f64,
    pub merging: bool,
    pub memoizing: bool,
    pub max_worlds: usize,
    pub max_iterations: u64,
    pub max_call_depth: usize,
    pub max_cached_calls: usize,
    pub max_output: usize,
    pub budget: Budget,
    pub cancel: Option<Arc<AtomicBool>>,
    /// Sample with this seed, instead of enumerating (docs/semantics.md,
    /// section 14).
    pub sample_seed: Option<u64>,
    /// When sampling, delay the draws of conjugate priors and update them
    /// exactly (section 14).
    pub conjugate: bool,
    /// When enumerating, solve loops that cycle as Markov chains (section 10).
    pub solving: bool,
    /// The most states a loop's chain may have to be solved.
    pub max_chain_states: usize,
}

/// The most steps eliminating one loop's chain may take before the loop is
/// unrolled instead.
const MAX_ELIMINATION: u64 = 200_000_000;

/// Runs are executed this many at a time, and each batch has a random
/// stream of its own. The size is fixed: changing it changes the output for
/// a given seed.
pub const BATCH: u64 = 1000;

/// What a batch of sampled runs produced, to be combined with the other
/// batches in order.
#[derive(Debug)]
pub struct Batch {
    pub sinks: Vec<Sink>,
    pub totals: SampleTotals,
    pub observed: bool,
    pub densities: bool,
    pub unresolved: Weight,
    pub last_ruling_out: Option<Span>,
    pub stats: Stats,
}

/// Lines printed by `print(…)`, with where.
pub type Printed = Vec<(Span, String)>;

/// The final weights of all the runs, when sampling.
#[derive(Clone, Copy, Debug)]
pub struct SampleTotals {
    pub weight: Weight,
    /// Sum of the squared weights, for the effective sample size.
    pub squares: Weight,
}

/// The distribution of a function's return value, unnormalized.
#[derive(Debug)]
pub struct CallResult {
    pub outcomes: Vec<(Value, Weight)>,
    /// Weight the call left unresolved.
    pub unresolved: Weight,
    /// Weight its observations ruled out, when enumerating.
    pub lost: Weight,
    /// Whether the call ran `observe` (outside any `simulate`).
    pub observed: bool,
}

type CallKey = (FnId, Vec<Value>);

pub struct Engine<'p> {
    prog: &'p Program,
    live: &'p Liveness,
    /// Which draws may be delayed, and what must draw them.
    conj: &'p Conjugacy,
    config: Config,
    budget: Budget,
    memo: FxHashMap<CallKey, Arc<CallResult>>,
    active: FxHashSet<CallKey>,
    depth: usize,
    /// Weight left unaccounted for (loops cut short, truncated tails).
    pub unresolved: Weight,
    /// When enumerating: weight ruled out by observations, which solving a
    /// loop counts as a way out of it.
    lost: Weight,
    /// For each statement: whether it's an unbounded loop that may be
    /// solved as a Markov chain (section 10).
    solvable: Vec<bool>,
    /// Loops whose chains were found too large to solve: they're unrolled
    /// from then on.
    too_large: FxHashSet<StmtId>,
    /// Whether the current inference scope has run `observe`.
    pub observed: bool,
    /// Whether it has observed a value with a density, which makes the
    /// evidence a density too (docs/semantics.md, section 14).
    pub densities: bool,
    /// The last observation that ruled out a world, for impossible evidence.
    pub last_ruling_out: Option<Span>,
    pub sinks: Vec<Sink>,
    dice: FxHashMap<(u32, u32), Value>,
    pools: FxHashMap<(u32, Value), Value>,
    record_names: Vec<Arc<str>>,
    enum_values: Vec<Vec<Value>>,
    print: &'p mut (dyn FnMut(&str) + Send),
    /// The values of the program's inputs (`read`).
    inputs: &'p [Value],
    /// Bytes printed, counting the batches printed before this one.
    printed: usize,
    /// When sampling a batch: what it printed, kept to be printed in batch
    /// order.
    lines: Option<Printed>,
    pub stats: Stats,
    /// When sampling: the random numbers. `None` when enumerating, including
    /// inside `simulate` while sampling.
    sampler: Option<Rng>,
    /// How many `simulate` blocks are being enumerated inside a sampled run.
    nested: usize,
}

impl<'p> Engine<'p> {
    pub fn new(
        prog: &'p Program,
        live: &'p Liveness,
        conj: &'p Conjugacy,
        config: Config,
        inputs: &'p [Value],
        print: &'p mut (dyn FnMut(&str) + Send),
    ) -> Engine<'p> {
        let sampler = config.sample_seed.map(Rng::new);
        Engine {
            prog,
            live,
            conj,
            budget: config.budget.clone(),
            config,
            memo: FxHashMap::default(),
            active: FxHashSet::default(),
            depth: 0,
            unresolved: Weight::ZERO,
            lost: Weight::ZERO,
            solvable: probl_sema::effects::solvable_loops(prog),
            too_large: FxHashSet::default(),
            observed: false,
            densities: false,
            last_ruling_out: None,
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
            inputs,
            printed: 0,
            lines: None,
            stats: Stats::default(),
            sampler,
            nested: 0,
        }
    }

    /// Run the top level. Returns the total weight of the worlds that finish.
    pub fn run_main(&mut self) -> Result<Weight> {
        let prog = self.prog;
        let main = prog.main();
        let world = World {
            slots: vec![Value::Dead; main.n_slots()],
            weight: Weight::ONE,
            run: 0,
        };
        let flow = self.exec_block(MAIN, &main.body, vec![world])?;
        Ok(total_weight(&flow.next))
    }

    /// Sample the runs `first..first + n` with the random numbers `rng`
    /// (docs/semantics.md, section 14), after earlier batches printed
    /// `printed` bytes. A batch starts afresh: its reports and counters are
    /// its own, so batches can run on any thread; only caches carry over.
    /// What it prints comes back with its result, even when it fails.
    pub fn run_batch(&mut self, rng: Rng, first: u64, n: u64, printed: usize) -> (Result<Batch>, Printed) {
        self.sampler = Some(rng);
        self.sinks = vec![Sink::default(); self.prog.reports.len()];
        self.observed = false;
        self.densities = false;
        self.unresolved = Weight::ZERO;
        self.last_ruling_out = None;
        self.stats = Stats::default();
        self.printed = printed;
        self.lines = Some(Vec::new());
        self.budget = self.config.budget.clone();
        let result = self.sample_runs(first, n);
        self.budget.give_back();
        (result, self.lines.take().unwrap_or_default())
    }

    fn sample_runs(&mut self, first: u64, n: u64) -> Result<Batch> {
        let main = self.prog.main();
        let worlds = (first..first + n)
            .map(|run| World {
                slots: vec![Value::Dead; main.n_slots()],
                weight: Weight::ONE,
                run: run as u32,
            })
            .collect();
        let flow = self.exec_block(MAIN, &main.body, worlds)?;
        let mut totals = SampleTotals {
            weight: Weight::ZERO,
            squares: Weight::ZERO,
        };
        for w in &flow.next {
            totals.weight += w.weight;
            totals.squares += w.weight * w.weight;
        }
        for sink in &mut self.sinks {
            sink.end_batch();
        }
        Ok(Batch {
            sinks: std::mem::take(&mut self.sinks),
            totals,
            observed: self.observed,
            densities: self.densities,
            unresolved: self.unresolved,
            last_ruling_out: self.last_ruling_out,
            stats: std::mem::take(&mut self.stats),
        })
    }

    /// When sampling: one draw from a distribution. A finite distribution
    /// gives one of its outcomes, chosen with its probability (missing mass
    /// is ignored); a continuous one, a number.
    fn sample(&mut self, v: &Value) -> Value {
        let rng = self.sampler.as_mut().expect("only called when sampling");
        match v {
            Value::Dist(d) if !d.outcomes.is_empty() => {
                let i = rng.choose(d.outcomes.iter().map(|(_, p)| *p)).unwrap_or(0);
                match &d.outcomes[i].0 {
                    Value::Continuous(f) => Value::Float(f.sample(rng)),
                    x => x.clone(),
                }
            }
            Value::Continuous(f) => Value::Float(f.sample(rng)),
            other => other.clone(),
        }
    }

    /// When sampling, a value to report: continuous parts are drawn; a
    /// finite distribution is kept whole, so each outcome counts with its
    /// probability.
    fn sample_continuous(&mut self, v: Value) -> Value {
        match &v {
            Value::Continuous(_) => self.sample(&v),
            Value::Dist(d) if d.outcomes.iter().any(|(x, _)| matches!(x, Value::Continuous(_))) => {
                let pairs = d.outcomes.iter().map(|(x, p)| (self.sample(x), *p)).collect();
                Dist::from_pairs(pairs, d.missing).into_value()
            }
            _ => v,
        }
    }

    /// When sampling, `binomial(…)`, `poisson(…)` or `geometric(…)` with plain
    /// arguments is drawn from, or observed, without listing its outcomes:
    /// the distribution is the same, and it's much faster.
    fn direct_counts(&mut self, f: FnId, e: &'p Expr, w: &World) -> Result<Option<Counts>> {
        let ExprKind::Builtin {
            func: b @ (Builtin::Binomial | Builtin::Poisson | Builtin::Geometric),
            args,
            ..
        } = &e.kind
        else {
            return Ok(None);
        };
        if self.sampler.is_none() {
            return Ok(None);
        }
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.eval(f, a, w)?);
        }
        let counts = builtins::counts(*b, &values).map_err(|err| err.at(e.span))?;
        Ok(counts.filter(Counts::direct))
    }

    fn continuous_draw(&self, span: Span) -> RuntimeError {
        let err = RuntimeError::new(
            span,
            "can't draw from a continuous distribution when enumerating: its outcomes can't be listed",
        );
        if self.nested > 0 {
            err.with_note("`simulate` is computed by enumeration, even in sample mode")
                .with_help("sampling inside `simulate` isn't supported yet; draw the value outside the block")
        } else {
            err.with_help(
                "sample instead, with `@mode sample(runs: 10_000)`, or compare it with a number, like `if x > 5`, which enumeration can compute",
            )
        }
    }

    /// Whether draws of conjugate priors are delayed now: when sampling,
    /// but not inside `simulate` (section 14).
    fn delaying(&self) -> bool {
        self.config.conjugate && self.sampler.is_some()
    }

    fn updates(&mut self, variable: u32) -> &mut Updates {
        let i = variable as usize;
        if self.stats.updates.len() <= i {
            let n = self.conj.variables.len().max(i + 1);
            self.stats.updates.resize(n, Updates::default());
        }
        &mut self.stats.updates[i]
    }

    /// Draw a delayed variable from its distribution, updated by the
    /// observations so far, now that its value is needed.
    fn draw_delayed(&mut self, w: &mut World, slot: SlotId) {
        if let Value::Delayed(d) = &w.slots[slot as usize] {
            let d = **d;
            let x = d
                .family
                .sample(self.sampler.as_mut().expect("delayed only when sampling"));
            w.slots[slot as usize] = Value::Float(x);
            self.updates(d.variable).drawn += 1;
        }
    }

    /// `observe` of a delayed variable through a conjugate form: the log of
    /// the observation's probability (or density), with the variable
    /// updated to its distribution after it. `None` when it isn't an exact
    /// update: the variable isn't delayed, or doesn't have the right family,
    /// or the rest of the observation isn't plain. Then the variable is
    /// drawn, and the observation is made as usual. `from` is where
    /// problems with the observed distribution's parameters are reported,
    /// as when it's evaluated.
    fn observe_exactly(&mut self, f: FnId, u: &Update<'p>, w: &mut World, from: Span) -> Result<Option<f64>> {
        let Value::Delayed(d) = &w.slots[u.slot as usize] else {
            return Ok(None);
        };
        let d = **d;
        let pair = matches!(
            (d.family, u.likelihood),
            (
                Family::Beta { .. },
                Likelihood::Binomial { .. } | Likelihood::Bernoulli { .. }
            ) | (Family::Gamma { .. }, Likelihood::Poisson { .. })
                | (Family::Normal { .. }, Likelihood::Normal { .. })
        );
        let seen = if pair { self.seen(f, u, w, from)? } else { None };
        let Some(seen) = seen else {
            self.draw_delayed(w, u.slot);
            return Ok(None);
        };
        let (ln, posterior) = conjugate::update(&d.family, seen).expect("a conjugate pair");
        w.slots[u.slot as usize] = Value::Delayed(Arc::new(Delayed {
            family: posterior,
            variable: d.variable,
        }));
        self.updates(d.variable).exact += 1;
        self.densities |= matches!(seen, Seen::Normal { .. });
        Ok(Some(ln))
    }

    /// What a conjugate observation saw, checked as when its variable is
    /// drawn; `None` if a part of it isn't a plain value.
    fn seen(&mut self, f: FnId, u: &Update<'p>, w: &World, from: Span) -> Result<Option<Seen>> {
        // A count, as observing one from a drawn distribution reads it:
        // anything but a whole number of 0 or more is impossible.
        let count = |v: &Value| match v {
            Value::Bool(_) => f64::NAN,
            _ => v.as_f64().unwrap_or(f64::NAN),
        };
        Ok(match u.likelihood {
            Likelihood::Binomial { value, trials } => {
                let v = self.eval(f, value, w)?;
                let n = self.eval(f, trials, w)?;
                match builtins::counts(Builtin::Binomial, &[n, Value::Prob(0.5)]).map_err(|e| e.at(from))? {
                    Some(Counts::Binomial { n, .. }) => Some(Seen::Binomial {
                        trials: n,
                        k: count(&v),
                    }),
                    _ => None,
                }
            }
            Likelihood::Bernoulli { value: None } => Some(Seen::Bernoulli(true)),
            Likelihood::Bernoulli { value: Some(value) } => match self.eval(f, value, w)? {
                Value::Bool(b) => Some(Seen::Bernoulli(b)),
                _ => None,
            },
            Likelihood::Poisson { value } => {
                let v = self.eval(f, value, w)?;
                Some(Seen::Poisson { k: count(&v) })
            }
            Likelihood::Normal { value, sd } => {
                let v = self.eval(f, value, w)?;
                let sd = self.eval(f, sd, w)?;
                if sd.is_uncertain() {
                    return Ok(None);
                }
                let checked = builtins::call_plain(Builtin::Normal, &[Value::Float(0.0), sd], &mut self.budget)
                    .map_err(|e| e.at(from))?;
                let Value::Continuous(family) = checked else {
                    unreachable!("`normal` gives a continuous distribution")
                };
                let Family::Normal { sd, .. } = *family else {
                    unreachable!("`normal` gives a normal distribution")
                };
                match v {
                    Value::Int(_) | Value::Float(_) | Value::Prob(_) => Some(Seen::Normal {
                        y: v.as_f64().expect("a number"),
                        sd,
                    }),
                    _ => None,
                }
            }
        })
    }

    fn merge(&self, worlds: Vec<World>, stmt: StmtId) -> Vec<World> {
        merge(worlds, &self.live.after[stmt as usize], self.merging())
    }

    /// Whether equal worlds merge: never when sampling, since every run
    /// counts separately (section 14), but inside `simulate` they do.
    fn merging(&self) -> bool {
        self.config.merging && self.sampler.is_none()
    }

    /// Stop if a statement produced more worlds than allowed. (When sampling,
    /// worlds never multiply: there's one per run in the batch.)
    fn check_worlds(&self, n: usize, span: Span) -> Result<()> {
        if n > self.config.max_worlds && self.sampler.is_none() {
            return Err(RuntimeError::limit(
                span,
                format!("more than {} worlds are too many to follow", self.config.max_worlds),
            )
            .with_help("simplify the model, raise the limit, or wait for sample mode (v0.2)"));
        }
        Ok(())
    }

    fn spend(&mut self, n: u64, span: Span) -> Result<()> {
        self.budget.work(n).map_err(|e| e.at(span))?;
        if let Some(cancel) = &self.config.cancel {
            if cancel.load(Ordering::Relaxed) {
                return Err(RuntimeError::limit(span, "the run was cancelled"));
            }
        }
        Ok(())
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
            clear(&mut flow.next, &self.live.dies[stmt.id as usize]);
            flow.broke.extend(out.broke);
            flow.continued.extend(out.continued);
            flow.returned.extend(out.returned);
        }
        Ok(flow)
    }

    fn exec_stmt(&mut self, f: FnId, stmt: &'p Stmt, worlds: Vec<World>) -> Result<Flow> {
        let span = stmt.span;
        let n = worlds.len();
        self.stats.world_steps += n as u64;
        self.stats.peak_worlds = self.stats.peak_worlds.max(n);
        self.spend(n as u64, span)?;
        self.check_worlds(n, span)?;
        let mut worlds = worlds;
        if self.delaying() {
            // Delayed variables this statement reads are drawn first.
            let conj = self.conj;
            let first = &conj.draws_first[stmt.id as usize];
            if !first.is_empty() {
                for w in &mut worlds {
                    for &slot in first {
                        self.draw_delayed(w, slot);
                    }
                }
            }
        }
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
                // A conjugate prior's draw may be delayed (section 14).
                let delay = if self.delaying() {
                    self.conj.delays[stmt.id as usize]
                } else {
                    None
                };
                let mut out = Vec::with_capacity(worlds.len());
                for w in worlds {
                    if self.sampler.is_some() {
                        if let Some(counts) = self.direct_counts(f, dist, &w)? {
                            let k = counts.sample(self.sampler.as_mut().expect("sampling"));
                            let mut w = w;
                            self.assign(f, place, Value::Int(k), &mut w, dist.span)?;
                            out.push(w);
                            continue;
                        }
                    }
                    let d = self.eval(f, dist, &w)?;
                    if let (Some(variable), Value::Continuous(family)) = (delay, &d) {
                        if conjugate::is_prior(family) {
                            let mut w = w;
                            w.slots[place.slot as usize] = Value::Delayed(Arc::new(Delayed {
                                family: **family,
                                variable,
                            }));
                            self.updates(variable).delayed += 1;
                            out.push(w);
                            continue;
                        }
                    }
                    self.split_by(f, place, d, w, dist.span, &mut out)?;
                    self.check_worlds(out.len(), span)?;
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
                    let total: u128 = cards.values().map(|n| *n as u128).sum();
                    if total == 0 {
                        return Err(RuntimeError::new(span, "can't take a card from an empty bag"));
                    }
                    let chosen = match &mut self.sampler {
                        Some(rng) => rng.choose(cards.values().map(|n| *n as f64)),
                        None => None,
                    };
                    for (i, (card, count)) in cards.iter().enumerate() {
                        if self.sampler.is_some() && chosen != Some(i) {
                            continue;
                        }
                        let rest = cards.without_nth(i);
                        let mut nw = if self.sampler.is_some() {
                            w.clone()
                        } else {
                            w.clone().scaled(*count as f64 / total as f64)
                        };
                        self.assign(f, bag, Value::multiset(rest), &mut nw, span)?;
                        self.assign(f, place, card.clone(), &mut nw, span)?;
                        out.push(nw);
                    }
                    self.check_worlds(out.len(), span)?;
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
                    self.lost += w.weight * result.lost;
                    let last = result.outcomes.len().saturating_sub(1);
                    let mut w = Some(w);
                    for (i, (v, p)) in result.outcomes.iter().enumerate() {
                        let mut nw = if i == last {
                            w.take().unwrap()
                        } else {
                            w.clone().unwrap()
                        };
                        nw.weight = nw.weight * *p;
                        self.assign(f, dest, v.clone(), &mut nw, span)?;
                        out.push(nw);
                    }
                    self.check_worlds(out.len(), span)?;
                }
                Ok(Flow::next(self.merge(out, stmt.id)))
            }
            StmtKind::If { cond, then, otherwise } => {
                let (mut yes, mut no) = (Vec::new(), Vec::new());
                for w in worlds {
                    let c = self.eval_condition(f, cond, &w)?;
                    if let Some(rng) = &mut self.sampler {
                        // One branch, chosen with its probability.
                        match rng.choose([c.yes, c.no].into_iter()) {
                            Some(0) => yes.push(w),
                            Some(_) => no.push(w),
                            None => self.unresolved += w.weight,
                        }
                        continue;
                    }
                    let (p, q) = (c.yes, c.no);
                    if c.missing > 0.0 {
                        self.unresolved += w.weight.scale(c.missing);
                    }
                    if q <= 0.0 {
                        yes.push(w.scaled(p));
                    } else if p <= 0.0 {
                        no.push(w.scaled(q));
                    } else {
                        no.push(w.clone().scaled(q));
                        yes.push(w.scaled(p));
                    }
                }
                self.check_worlds(yes.len() + no.len(), span)?;
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
                    // A weight is a condition: its missing mass could go to
                    // any arm, so it's unresolved (docs/semantics.md, section 3).
                    let mut chances = Vec::with_capacity(arms.len());
                    let mut missing = 0.0;
                    for (weight, _) in arms {
                        let c = self.eval_condition(f, weight, &w)?;
                        chances.push(c.yes);
                        missing += c.missing;
                    }
                    if missing > 0.0 && self.sampler.is_none() {
                        self.unresolved += w.weight.scale(missing.min(1.0));
                    }
                    let sum: f64 = chances.iter().sum::<f64>() + missing;
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
                    if let Some(rng) = &mut self.sampler {
                        // One arm, chosen with its probability.
                        let options = chances.iter().copied().chain(std::iter::once(remainder));
                        match rng.choose(options) {
                            Some(i) if i < arms.len() => buckets[i].push(w),
                            Some(_) => rest.push(w),
                            None => self.unresolved += w.weight,
                        }
                        continue;
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
                self.check_worlds(buckets.iter().map(Vec::len).sum::<usize>() + rest.len(), span)?;
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
            StmtKind::Loop { body, bounded } => self.exec_loop(f, stmt, body, *bounded, worlds),
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
                let exact = if self.delaying() {
                    probl_sema::conjugate::update(value, from.as_ref())
                } else {
                    None
                };
                let mut out = Vec::with_capacity(worlds.len());
                for mut w in worlds {
                    if let Some(u) = &exact {
                        let at = from.as_ref().map_or(span, |d| d.span);
                        if let Some(ln) = self.observe_exactly(f, u, &mut w, at)? {
                            w.weight = w.weight * Weight::from_ln(ln);
                            if w.weight.is_zero() {
                                self.last_ruling_out = Some(span);
                            } else {
                                out.push(w);
                            }
                            continue;
                        }
                    }
                    let (factor, missing, ruled_out) = match from {
                        None => {
                            let c = self.eval_condition(f, value, &w)?;
                            (c.yes, c.missing, c.no)
                        }
                        Some(d) => {
                            let v = self.eval(f, value, &w)?;
                            match self.direct_counts(f, d, &w)? {
                                Some(counts) if self.sampler.is_some() => {
                                    let x = match v {
                                        Value::Bool(_) => None,
                                        _ => v.as_f64(),
                                    };
                                    (x.map_or(0.0, |x| counts.pmf(x)), 0.0, 0.0)
                                }
                                _ => {
                                    let dist = self.eval(f, d, &w)?;
                                    self.densities |= is_density(&dist);
                                    likelihood(&dist, &v, self.sampler.is_some()).map_err(|e| e.at(span))?
                                }
                            }
                        }
                    };
                    if missing > 0.0 && self.sampler.is_none() {
                        self.unresolved += w.weight.scale(missing);
                    }
                    if ruled_out > 0.0 && self.sampler.is_none() {
                        self.lost += w.weight.scale(ruled_out);
                    }
                    if factor > 0.0 {
                        w.weight = w.weight.scale(factor);
                        out.push(w);
                    } else {
                        self.last_ruling_out = Some(span);
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
                            if k_value.is_uncertain() {
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
                    let (v, run) = if self.sampler.is_some() {
                        (self.sample_continuous(v), Some(w.run))
                    } else {
                        if matches!(&v, Value::Continuous(_))
                            || matches!(&v, Value::Dist(d) if d.outcomes.iter().any(|(x, _)| matches!(x, Value::Continuous(_))))
                        {
                            return Err(RuntimeError::new(
                                value.span,
                                "can't report a continuous distribution when enumerating: its outcomes can't be listed",
                            )
                            .with_help("sample instead, with `@mode sample(runs: 10_000)`, or report a comparison, like `x > 5`"));
                        }
                        (v, None)
                    };
                    self.sinks[*site as usize].add(k, &v, w.weight, run);
                }
                Ok(Flow::next(worlds))
            }
            StmtKind::Fail { message } => Err(RuntimeError::new(span, message.clone())),
            StmtKind::Check { slot, ty } => {
                for w in &mut worlds {
                    // A delayed variable is drawn only if some of its values
                    // could fail the check.
                    if let Value::Delayed(d) = &w.slots[*slot as usize] {
                        let always = match ty {
                            TypeSpec::Float => true,
                            TypeSpec::Prob => matches!(d.family, Family::Beta { .. }),
                            _ => false,
                        };
                        if always {
                            continue;
                        }
                        self.draw_delayed(w, *slot);
                    }
                    let v = &w.slots[*slot as usize];
                    if !self.conforms(v, ty) {
                        let name = &self.prog.functions[f as usize].slots[*slot as usize].name;
                        let what = if name == "(temporary)" {
                            "the result".to_string()
                        } else {
                            format!("`{name}`")
                        };
                        return Err(RuntimeError::new(
                            span,
                            format!(
                                "{what} should be {}, but it's {}",
                                ops::article(&ty.describe(self.prog)),
                                ops::article(&v.kind())
                            ),
                        ));
                    }
                }
                Ok(Flow::next(worlds))
            }
        }
    }

    fn exec_loop(
        &mut self,
        f: FnId,
        stmt: &'p Stmt,
        body: &'p Block,
        bounded: bool,
        worlds: Vec<World>,
    ) -> Result<Flow> {
        let live = self.live;
        let span = stmt.span;
        let entered = total_weight(&worlds);
        let cutoff = entered.scale(self.config.epsilon);
        let mut inside = worlds;
        let mut out = Flow::default();
        let mut iterations: u64 = 0;
        // The states met at the loop's start in earlier iterations, while it
        // may be solved. Once one comes back, the loop cycles, and it's
        // solved as a Markov chain from there (section 10).
        let mut met = self.may_solve(stmt, bounded).then(FxHashSet::<u64>::default);
        while !inside.is_empty() {
            let cycles = match &mut met {
                Some(met) => {
                    let head = live_slots(&live.loop_head[stmt.id as usize], inside[0].slots.len());
                    let hashes: Vec<u64> = inside.iter().map(|w| state_hash(w, &head)).collect();
                    let again = hashes.iter().any(|h| met.contains(h));
                    met.extend(hashes);
                    again
                }
                None => false,
            };
            if cycles {
                met = None;
                if let Some(solved) = self.solve_loop(f, stmt, body, &inside)? {
                    out.join(solved);
                    break;
                }
            }
            let mass = total_weight(&inside);
            if !bounded && mass < cutoff && self.sampler.is_none() {
                self.unresolved += mass;
                break;
            }
            iterations += 1;
            if iterations > self.config.max_iterations {
                return Err(RuntimeError::limit(
                    span,
                    format!("this loop ran {} times without finishing", self.config.max_iterations),
                )
                .with_note(format!(
                    "worlds still inside the loop weigh {} of what entered it",
                    fmt_prob(mass.ratio(entered))
                ))
                .with_help("check that every world can leave the loop"));
            }
            let mut flow = self.exec_block(f, body, inside)?;
            clear_dead(&mut flow.broke, &live.after[stmt.id as usize]);
            clear_dead(&mut flow.continued, &live.loop_head[stmt.id as usize]);
            out.next.extend(flow.broke);
            out.returned.extend(flow.returned);
            let mut again = flow.next;
            again.extend(flow.continued);
            inside = merge(again, &live.loop_head[stmt.id as usize], self.merging());
        }
        out.next = self.merge(out.next, stmt.id);
        Ok(out)
    }

    /// Whether a loop may be solved as a Markov chain: an unbounded loop
    /// whose body doesn't report or print, when enumerating, unless its
    /// chain was already found too large.
    fn may_solve(&self, stmt: &Stmt, bounded: bool) -> bool {
        !bounded
            && self.config.solving
            && self.sampler.is_none()
            && self.solvable[stmt.id as usize]
            && !self.too_large.contains(&stmt.id)
    }

    /// Solve a loop as an absorbing Markov chain, from the worlds `inside`
    /// at its start (section 10). Its states are the worlds at its start,
    /// told apart by the variables read later. Running the body once from
    /// each state reachable from `inside` gives the chance of going on to
    /// each state, and the worlds that leave. The expected number of visits
    /// to each state then says how much of each leaves. `None` if the chain
    /// is too large, and the loop should be unrolled instead.
    fn solve_loop(&mut self, f: FnId, stmt: &'p Stmt, body: &'p Block, inside: &[World]) -> Result<Option<Flow>> {
        let head_set = &self.live.loop_head[stmt.id as usize];
        let after = &self.live.after[stmt.id as usize];
        let n = inside[0].slots.len();
        let head = live_slots(head_set, n);
        let dead: Vec<SlotId> = head_set.iter_missing(n).collect();
        // Each state: its world, with dead slots cleared, found by its live
        // slots.
        let mut states: Vec<Vec<Value>> = Vec::new();
        let mut index: FxHashMap<Vec<Value>, usize> = FxHashMap::default();
        let mut intern = |mut slots: Vec<Value>, states: &mut Vec<Vec<Value>>| -> usize {
            for &d in &dead {
                slots[d as usize] = Value::Dead;
            }
            let key: Vec<Value> = head.iter().map(|&i| slots[i].clone()).collect();
            *index.entry(key).or_insert_with(|| {
                states.push(slots);
                states.len() - 1
            })
        };
        let total = total_weight(inside);
        let mut start: Vec<f64> = Vec::new();
        for w in inside {
            let i = intern(w.slots.clone(), &mut states);
            if start.len() <= i {
                start.resize(i + 1, 0.0);
            }
            start[i] += w.weight.ratio(total);
        }
        let mut chain = Chain::default();
        let mut exits: Vec<Vec<World>> = Vec::new();
        let mut returns: Vec<Vec<(Value, Weight)>> = Vec::new();
        let mut unresolved: Vec<Weight> = Vec::new();
        let mut k = 0;
        while k < states.len() {
            if states.len() > self.config.max_chain_states {
                self.too_large.insert(stmt.id);
                return Ok(None);
            }
            let world = World {
                slots: states[k].clone(),
                weight: Weight::ONE,
                run: 0,
            };
            let saved_unresolved = std::mem::replace(&mut self.unresolved, Weight::ZERO);
            let saved_lost = std::mem::replace(&mut self.lost, Weight::ZERO);
            let flow = self.exec_block(f, body, vec![world]);
            let left = std::mem::replace(&mut self.unresolved, saved_unresolved);
            let lost = std::mem::replace(&mut self.lost, saved_lost);
            let mut flow = flow?;
            clear_dead(&mut flow.broke, after);
            // Leaving: by `break` or `return`, unresolved, or ruled out.
            let mut leave = left + lost;
            for w in &flow.broke {
                leave += w.weight;
            }
            for (_, w) in &flow.returned {
                leave += *w;
            }
            let mut next: FxHashMap<usize, f64> = FxHashMap::default();
            for w in flow.next.into_iter().chain(flow.continued) {
                let p = w.weight.to_f64();
                if p == 0.0 {
                    // Too small for the chain's arithmetic.
                    self.too_large.insert(stmt.id);
                    return Ok(None);
                }
                let j = intern(w.slots, &mut states);
                *next.entry(j).or_insert(0.0) += p;
            }
            let mut next: Vec<(usize, f64)> = next.into_iter().collect();
            next.sort_by_key(|&(j, _)| j);
            chain.next.push(next);
            chain.leave.push(leave.to_f64());
            exits.push(flow.broke);
            returns.push(flow.returned);
            unresolved.push(left);
            k += 1;
        }
        start.resize(states.len(), 0.0);
        let mut steps = MAX_ELIMINATION;
        let solution = chain.visits(&start, &mut steps);
        self.spend(MAX_ELIMINATION - steps, stmt.span)?;
        let visits = match solution {
            Solution::Visits(v) => v,
            Solution::TooBig => {
                self.too_large.insert(stmt.id);
                return Ok(None);
            }
            Solution::Stuck(s) => return Err(self.never_leaves(f, stmt, &states[s], &head)),
        };
        self.stats.solved_loops += 1;
        self.stats.chain_states += states.len() as u64;
        let mut flow = Flow::default();
        let mut left = Weight::ZERO;
        for (k, v) in visits.into_iter().enumerate() {
            if v == 0.0 {
                continue;
            }
            let times = total.scale(v);
            for mut w in std::mem::take(&mut exits[k]) {
                w.weight = w.weight * times;
                flow.next.push(w);
            }
            for (value, w) in std::mem::take(&mut returns[k]) {
                flow.returned.push((value, w * times));
            }
            left += unresolved[k] * times;
        }
        self.unresolved += left;
        Ok(Some(flow))
    }

    /// The error for a loop that some worlds can never leave, with one of
    /// them.
    fn never_leaves(&self, f: FnId, stmt: &Stmt, slots: &[Value], head: &[usize]) -> RuntimeError {
        let names = &self.prog.functions[f as usize].slots;
        let parts: Vec<String> = head
            .iter()
            .filter(|&&i| !names[i].name.starts_with('$') && names[i].name != "(temporary)")
            .map(|&i| format!("`{}` is {:?}", names[i].name, slots[i]))
            .collect();
        let err = RuntimeError::new(stmt.span, "some worlds can never leave this loop")
            .with_help("check that every world can leave the loop");
        if parts.is_empty() {
            err
        } else {
            err.with_note(format!("for example, the worlds where {}", parts.join(" and ")))
        }
    }

    /// Store each possible value of `d` into `place`, one world per outcome.
    fn split_by(&mut self, f: FnId, place: &Place, d: Value, w: World, span: Span, out: &mut Vec<World>) -> Result<()> {
        match d {
            Value::Dist(_) | Value::Continuous(_) if self.sampler.is_some() => {
                let v = self.sample(&d);
                let mut w = w;
                self.assign(f, place, v, &mut w, span)?;
                out.push(w);
            }
            Value::Continuous(_) => return Err(self.continuous_draw(span)),
            Value::Dist(dist) if dist.outcomes.iter().any(|(v, _)| matches!(v, Value::Continuous(_))) => {
                return Err(self.continuous_draw(span));
            }
            Value::Dist(dist) => {
                self.unresolved += w.weight.scale(dist.missing);
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
            Value::Prob(p) => {
                return Err(RuntimeError::new(
                    span,
                    format!(
                        "can't draw from {}: it's a probability, not a distribution",
                        fmt_prob(p)
                    ),
                )
                .with_help("to draw a fact that's true with this probability, write `bernoulli(p)`"));
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

    /// Does `v` have the declared type?
    fn conforms(&self, v: &Value, ty: &TypeSpec) -> bool {
        match (ty, v) {
            (TypeSpec::Int, Value::Int(_)) => true,
            (TypeSpec::Float, Value::Float(_) | Value::Int(_)) => true,
            (TypeSpec::Prob, Value::Prob(_)) => true,
            (TypeSpec::Prob, Value::Float(x)) => (0.0..=1.0).contains(x),
            (TypeSpec::Bool, Value::Bool(_)) => true,
            (TypeSpec::Str, Value::Str(_)) => true,
            (TypeSpec::Date, Value::Date(_)) => true,
            (TypeSpec::Unit, Value::Unit) => true,
            (TypeSpec::Function, Value::Closure(_)) => true,
            (TypeSpec::List(t), Value::List(items)) => items.iter().all(|x| self.conforms(x, t)),
            (TypeSpec::List(t), Value::Range(..)) => **t == TypeSpec::Int,
            (TypeSpec::Map(k, t), Value::Map(m)) => m.iter().all(|(a, b)| self.conforms(a, k) && self.conforms(b, t)),
            (TypeSpec::Bag(t), Value::Bag(b)) => b.keys().all(|x| self.conforms(x, t)),
            (TypeSpec::Dist(t), Value::Dist(d)) => d.outcomes.iter().all(|(x, _)| match x {
                Value::Continuous(_) => matches!(**t, TypeSpec::Float | TypeSpec::Prob),
                _ => self.conforms(x, t),
            }),
            (TypeSpec::Dist(t), Value::Continuous(_)) => matches!(**t, TypeSpec::Float | TypeSpec::Prob),
            (TypeSpec::Record(r), Value::Record(rec)) => {
                let declared = &self.prog.records[*r as usize];
                rec.ty.as_deref() == Some(declared.name.as_str())
                    && rec.fields.len() == declared.fields.len()
                    && declared
                        .fields
                        .iter()
                        .all(|f| rec.get(&f.name).is_some_and(|x| self.conforms(x, &f.ty)))
            }
            (TypeSpec::Enum(e), Value::Enum(x)) => x.ty == *e,
            (TypeSpec::AnonRecord(fields), Value::Record(rec)) => {
                rec.fields.len() == fields.len()
                    && rec
                        .fields
                        .iter()
                        .zip(fields)
                        .all(|((n, x), (m, t))| &**n == m.as_str() && self.conforms(x, t))
            }
            _ => false,
        }
    }

    // ── Calls ────────────────────────────────────────────────────────────

    /// Run a function as a sub-simulation and return the unnormalized
    /// distribution of its result. Results are memoized unless the function
    /// prints: its behaviour depends only on its arguments and the outside
    /// values it reads, which are all in `key`.
    fn call(&mut self, func: FnId, key: Vec<Value>, span: Span) -> Result<Arc<CallResult>> {
        self.stats.calls += 1;
        let prog = self.prog;
        let fun = &prog.functions[func as usize];
        // When sampling, every call makes its own choices (section 14).
        let sampling = self.sampler.is_some();
        let memoizable = self.config.memoizing && !sampling && !fun.effects.prints;
        let call_key = (func, key);
        if memoizable {
            if let Some(r) = self.memo.get(&call_key) {
                self.stats.memo_hits += 1;
                self.observed |= r.observed;
                return Ok(r.clone());
            }
        }
        if !sampling && self.active.contains(&call_key) {
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
            .with_note("enumeration can't solve recursion that loops back to the same call yet")
            .with_help("write it as a loop instead"));
        }
        if self.depth >= self.config.max_call_depth {
            return Err(RuntimeError::limit(
                span,
                format!("calls are nested more than {} deep", self.config.max_call_depth),
            )
            .with_help("check for recursion that doesn't stop, or write it as a loop"));
        }
        let mut slots = vec![Value::Dead; fun.n_slots()];
        let n = fun.n_params as usize;
        for (i, v) in call_key.1[..n].iter().enumerate() {
            slots[i] = v.clone();
        }
        for (cap, v) in fun.captures.iter().zip(&call_key.1[n..]) {
            slots[cap.slot as usize] = v.clone();
        }
        if !sampling {
            self.active.insert(call_key.clone());
        }
        let saved_unresolved = std::mem::replace(&mut self.unresolved, Weight::ZERO);
        let saved_lost = std::mem::replace(&mut self.lost, Weight::ZERO);
        let saved_observed = std::mem::replace(&mut self.observed, false);
        self.depth += 1;
        let flow = self.exec_block(
            func,
            &fun.body,
            vec![World {
                slots,
                weight: Weight::ONE,
                run: 0,
            }],
        );
        self.depth -= 1;
        let unresolved = std::mem::replace(&mut self.unresolved, saved_unresolved);
        let lost = std::mem::replace(&mut self.lost, saved_lost);
        let observed = std::mem::replace(&mut self.observed, saved_observed);
        self.observed |= observed;
        if !sampling {
            self.active.remove(&call_key);
        }
        let flow = flow.map_err(|e| match fun.kind {
            FnKind::Named => e.with_note(format!("in a call to `{}`", fun.name)),
            FnKind::Simulate => e.with_note("inside a `simulate` block"),
            FnKind::Lambda => e.with_note("inside a lambda"),
            FnKind::Main => e,
        })?;
        let result = Arc::new(CallResult {
            outcomes: merge_values(flow.returned),
            unresolved,
            lost,
            observed,
        });
        if memoizable {
            if self.memo.len() >= self.config.max_cached_calls {
                self.memo.clear();
            }
            self.memo.insert(call_key, result.clone());
        }
        Ok(result)
    }

    /// `simulate { … }`: run a block as a separate model and return its
    /// normalized distribution (docs/semantics.md, section 8).
    fn simulate(&mut self, func: FnId, key: Vec<Value>, span: Span) -> Result<Value> {
        let saved_observed = self.observed;
        let saved_densities = self.densities;
        // Enumerated, even when sampling (section 14).
        let sampler = self.sampler.take();
        self.nested += sampler.is_some() as usize;
        let result = self.call(func, key, span);
        self.nested -= sampler.is_some() as usize;
        self.sampler = sampler;
        // Observations inside `simulate` condition its result only.
        self.observed = saved_observed;
        self.densities = saved_densities;
        let result = result?;
        let resolved = Weight::sum(result.outcomes.iter().map(|(_, w)| *w));
        if resolved.is_zero() {
            let message = if result.unresolved.is_zero() {
                "every world in this `simulate` was ruled out by `observe`"
            } else {
                "this `simulate` left all of its weight unresolved"
            };
            return Err(RuntimeError::new(span, message));
        }
        let denom = resolved + result.unresolved;
        let pairs = result
            .outcomes
            .iter()
            .map(|(v, w)| (v.clone(), w.ratio(denom)))
            .collect();
        ops::combine(pairs, result.unresolved.ratio(denom), &mut self.budget).map_err(|e| e.at(span))
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
            [(v, p)] if (p.to_f64() - 1.0).abs() < 1e-12 && result.unresolved.is_zero() => Ok(v.clone()),
            _ => Err(RuntimeError::new(
                span,
                format!("the function given to `{what}` can't branch on chances, draw values or observe"),
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
                PathElem::Field(name) => ops::field(&v, name, &mut self.budget),
                PathElem::Index(e) => {
                    let i = self.eval(f, e, w)?;
                    ops::index(&v, &i, &mut self.budget)
                }
            }
            .map_err(|e| e.at(span))?;
        }
        Ok(v)
    }

    fn slot(&self, f: FnId, s: SlotId, w: &World, span: Span) -> Result<Value> {
        match &w.slots[s as usize] {
            Value::Dead => Err(self.no_value(f, s, span)),
            Value::Delayed(_) => {
                let name = &self.prog.functions[f as usize].slots[s as usize].name;
                Err(OpError {
                    message: "internal error: a variable was read before it was drawn".into(),
                    help: Some("this is a bug in Probl; please report it with the program that caused it".into()),
                    kind: ErrorKind::Internal,
                }
                .at(span)
                .with_note(format!(
                    "`{name}`'s draw was delayed for an exact update (docs/semantics.md, section 14)"
                )))
            }
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

    fn eval_condition(&mut self, f: FnId, e: &Expr, w: &World) -> Result<ops::Condition> {
        let v = self.eval(f, e, w)?;
        ops::condition(&v).map_err(|err| err.at(e.span))
    }

    fn eval(&mut self, f: FnId, e: &Expr, w: &World) -> Result<Value> {
        let span = e.span;
        let at = |err: OpError| err.at(span);
        match &e.kind {
            ExprKind::Lit(l) => self.literal(l).map_err(at),
            ExprKind::Slot(s) => self.slot(f, *s, w, span),
            ExprKind::Unary(op, x) => {
                let v = self.eval(f, x, w)?;
                ops::unary(*op, &v, &mut self.budget).map_err(at)
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                let and = *op == BinOp::And;
                let word = if and { "and" } else { "or" };
                let va = self.eval(f, a, w)?;
                let ta = ops::truth(&va, word).map_err(|err| err.at(a.span))?;
                if let Truth::Fact(x) = ta {
                    if x != and {
                        // `false and …` is false; `true or …` is true.
                        return Ok(Value::Bool(x));
                    }
                }
                let vb = self.eval(f, b, w)?;
                let tb = ops::truth(&vb, word).map_err(|err| err.at(b.span))?;
                ops::logic(and, ta, tb, &mut self.budget).map_err(at)
            }
            ExprKind::Binary(op, a, b) => {
                let va = self.eval(f, a, w)?;
                let vb = self.eval(f, b, w)?;
                ops::binary(*op, &va, &vb, &mut self.budget).map_err(at)
            }
            ExprKind::List(items) => {
                self.budget.collection(items.len() as u128).map_err(at)?;
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
                    if key.is_uncertain() {
                        return Err(RuntimeError::new(k.span, "map keys can't be distributions"));
                    }
                    let value = self.eval(f, v, w)?;
                    map.insert(key, value);
                }
                Ok(Value::map(map))
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
                ops::field(&v, name, &mut self.budget).map_err(at)
            }
            ExprKind::Index(x, i) => {
                let v = self.eval(f, x, w)?;
                let i = self.eval(f, i, w)?;
                ops::index(&v, &i, &mut self.budget).map_err(at)
            }
            ExprKind::With(x, fields) => {
                let base = self.eval(f, x, w)?;
                let mut updates = Vec::with_capacity(fields.len());
                for (name, v) in fields {
                    updates.push((Arc::from(name.as_str()), self.eval(f, v, w)?));
                }
                ops::lift1(&base, &mut self.budget, |b| ops::with_fields(b, &updates)).map_err(at)
            }
            ExprKind::Builtin { func, args, .. } => self.builtin(f, *func, args, w, span),
            ExprKind::Closure { func, capture_args } => Ok(Value::Closure(Arc::new(Closure {
                func: *func,
                captured: capture_args.iter().map(|&s| w.slots[s as usize].clone()).collect(),
            }))),
            ExprKind::Simulate { func, capture_args } => {
                let key = capture_args.iter().map(|&s| w.slots[s as usize].clone()).collect();
                self.simulate(*func, key, span)
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
            ExprKind::Input(i) => Ok(self.inputs[*i as usize].clone()),
        }
    }

    fn literal(&mut self, l: &Lit) -> OpResult<Value> {
        Ok(match l {
            Lit::Unit => Value::Unit,
            Lit::Bool(b) => Value::Bool(*b),
            Lit::Int(i) => Value::Int(*i),
            Lit::Float(x) => Value::Float(*x),
            Lit::Prob(p) => Value::Prob(*p),
            Lit::Str(s) => Value::str(s),
            Lit::Dice { count, sides } => {
                if let Some(d) = self.dice.get(&(*count, *sides)) {
                    return Ok(d.clone());
                }
                let d = Dist::dice(*count, *sides, &mut self.budget)?.into_value();
                self.dice.insert((*count, *sides), d.clone());
                d
            }
            Lit::Enum { ty, variant } => self.enum_values[*ty as usize][*variant as usize].clone(),
        })
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
                let line = if w.weight == Weight::ONE {
                    text
                } else {
                    format!("[{}] {text}", fmt_weight(w.weight))
                };
                self.printed += line.len() + 1;
                if self.printed > self.config.max_output {
                    return Err(too_much_output(span));
                }
                match &mut self.lines {
                    Some(lines) => lines.push((span, line)),
                    None => (self.print)(&line),
                }
                Ok(Value::Unit)
            }
            Builtin::Map | Builtin::Filter | Builtin::Reduce => self.higher_order(b, &values, span),
            Builtin::Count if values.len() == 2 => self.higher_order(b, &values, span),
            Builtin::Roll => self.roll(&values).map_err(at),
            Builtin::Take => Err(RuntimeError::new(span, "`take()` can only be used with `~`")),
            _ if b.lifting() == Lifting::Raw => builtins::call_raw(b, &values, &mut self.budget).map_err(at),
            _ if values.iter().any(|v| matches!(v, Value::Continuous(_))) => {
                let kind = values
                    .iter()
                    .find(|v| matches!(v, Value::Continuous(_)))
                    .unwrap()
                    .kind();
                Err(
                    RuntimeError::new(span, format!("`{}` needs a value, not a {kind}", b.name()))
                        .with_help("draw a value first, like `let x ~ normal(0, 1)`"),
                )
            }
            _ => ops::lift_n(&values, &mut self.budget, &|a, budget| {
                builtins::call_plain(b, a, budget)
            })
            .map_err(at),
        }
    }

    fn roll(&mut self, values: &[Value]) -> OpResult<Value> {
        let count = match &values[0] {
            Value::Int(n) if (0..=1000).contains(n) => *n as u32,
            Value::Int(_) => return Err(OpError::new("roll needs between 0 and 1000 dice")),
            v if v.is_uncertain() => {
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
            Value::Int(sides) if (1..=u32::MAX as i64).contains(sides) => {
                Dist::dice(1, *sides as u32, &mut self.budget)?
            }
            Value::Dist(d) => (**d).clone(),
            v => {
                return Err(OpError::new(format!(
                    "roll needs a die, like `d6`, found {}",
                    ops::article(&v.kind())
                )));
            }
        };
        let key = (count, die.clone().into_value());
        if let Some(pool) = self.pools.get(&key) {
            return Ok(pool.clone());
        }
        let pool = Dist::pool(count, &die, &mut self.budget)?.into_value();
        self.pools.insert(key, pool.clone());
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
            return ops::combine(results, d.missing, &mut self.budget).map_err(|e| e.at(span));
        }
        let items = builtins::items(&values[0], b.name(), &mut self.budget).map_err(|e| e.at(span))?;
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
                    match test {
                        Value::Bool(true) => kept.push(x),
                        Value::Bool(false) => {}
                        other => {
                            return Err(RuntimeError::new(
                                span,
                                format!("the test given to `{}` must give a fact (true or false)", b.name()),
                            )
                            .with_help(format!("it gave {}", ops::article(&other.kind()))));
                        }
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

/// The error when `print` at `span` goes over the output limit.
pub fn too_much_output(span: Span) -> RuntimeError {
    RuntimeError::limit(span, "the program printed more than the output limit")
}

/// A weight as a percentage, even when it's too small for an `f64`.
pub fn fmt_weight(w: Weight) -> String {
    let x = w.to_f64();
    if x == 0.0 && !w.is_zero() {
        let l = w.log10() + 2.0;
        return format!("{:.1}e{}%", 10f64.powf(l - l.floor()), l.floor());
    }
    fmt_prob(x)
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
            let k = ops::as_index(i, items.len() as u128)? as usize;
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

/// Whether observing a value from `d` uses a density.
fn is_density(d: &Value) -> bool {
    match d {
        Value::Continuous(_) => true,
        Value::Dist(dist) => dist.outcomes.iter().any(|(x, _)| matches!(x, Value::Continuous(_))),
        _ => false,
    }
}

/// The probability of observing `v` from `d`, the probability that is
/// missing from `d` (so the true value may be up to that much higher), and
/// the probability of the other outcomes, which the observation rules out.
/// When sampling, a continuous `d` gives its density instead (section 13).
fn likelihood(d: &Value, v: &Value, sampling: bool) -> OpResult<(f64, f64, f64)> {
    let continuous = match d {
        Value::Continuous(f) => Some(vec![(**f, 1.0)]),
        Value::Dist(dist) if dist.outcomes.iter().any(|(x, _)| matches!(x, Value::Continuous(_))) => {
            let parts: Option<Vec<_>> = dist
                .outcomes
                .iter()
                .map(|(x, p)| match x {
                    Value::Continuous(f) => Some((**f, *p)),
                    _ => None,
                })
                .collect();
            let parts = parts.ok_or_else(|| {
                OpError::new("`observe … from` can't mix a density with the probabilities of single values")
            })?;
            Some(parts)
        }
        _ => None,
    };
    if let Some(parts) = continuous {
        if !sampling {
            return Err(OpError::new("observing a value from a continuous distribution needs sample mode")
                .help("its density isn't a probability, so enumeration can't use it; sample with `@mode sample(runs: 10_000)`"));
        }
        let x = match v {
            Value::Bool(_) => None,
            _ => v.as_f64(),
        }
        .ok_or_else(|| {
            OpError::new(format!(
                "a continuous distribution can't produce {}",
                ops::article(&v.kind())
            ))
        })?;
        return Ok((parts.iter().map(|(f, p)| p * f.pdf(x)).sum(), 0.0, 0.0));
    }
    match d {
        Value::Dist(dist) => {
            let (mut seen, mut other) = (0.0, 0.0);
            for (x, p) in &dist.outcomes {
                if ops::equals(x, v) {
                    seen += p;
                } else {
                    other += p;
                }
            }
            Ok((seen, dist.missing, other))
        }
        Value::Prob(_) => Err(OpError::new("`observe … from` needs a distribution, not a probability")
            .help("to observe that a fact with probability p is true, write `observe true from bernoulli(p)`")),
        other => Ok(if ops::equals(other, v) {
            (1.0, 0.0, 0.0)
        } else {
            (0.0, 0.0, 1.0)
        }),
    }
}
