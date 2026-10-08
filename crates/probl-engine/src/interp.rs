//! The world-set interpreter.
//!
//! Every statement maps a set of worlds to a set of worlds. All the worlds in
//! a set are at the same point of the program, so control flow is shared and
//! only the data differs. Splitting happens in statements; expressions are
//! evaluated in one world at a time. See docs/semantics.md for the rules this
//! implements.

use crate::analytic::{self, Analytic};
use crate::builtins;
use crate::chain::{Chain, Solution};
use crate::conjugate::{self, Seen};
use crate::continuous::{Family, Rng};
use crate::dist::{Budget, Counts, Dist};
use crate::error::{Fault, OpError, OpResult, Result, RuntimeError};
use crate::failure::Failures;
use crate::ops::{self, Truth};
use crate::report::Sink;
use crate::value::{Closure, Delayed, Value, fmt_prob};
use crate::weight::Weight;
use crate::world::Returned;
use crate::world::{Flow, World, clear, clear_dead, live_slots, merge, merge_values, state_hash, total_weight};
use probl_sema::builtins::Lifting;
use probl_sema::conjugate::{Conjugacy, Likelihood, Update};
use probl_sema::effects::EvidenceOrder;
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
    /// Calls that came back to themselves, solved by iteration, and the
    /// rounds that took (section 6).
    pub solved_calls: u64,
    pub call_rounds: u64,
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
        self.solved_calls += other.solved_calls;
        self.call_rounds += other.call_rounds;
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
    pub today: Option<i32>,
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
    /// A fault ends only the world it happens in (docs/semantics.md,
    /// section 11), instead of the run.
    pub partial: bool,
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
    pub failures: Failures,
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
#[derive(Clone, Debug)]
pub struct CallResult {
    pub outcomes: Vec<Returned>,
    /// Weight the call left unresolved.
    pub unresolved: Weight,
    /// Weight its observations ruled out, when enumerating.
    pub lost: Weight,
    /// Whether the call ran `observe` (outside any `simulate`).
    pub observed: bool,
    /// The part of `unresolved` waiting on calls that came back to
    /// themselves and are still being solved (section 6), for each by its
    /// depth in the stack. A result waiting on one isn't final.
    pub pending: Vec<(usize, Weight)>,
    /// In partial mode: its worlds that failed.
    pub failures: Failures,
}

impl CallResult {
    /// The result of a call that came back to itself, before the first
    /// round: nothing resolved, all of it waiting on that call.
    fn waiting(depth: usize) -> CallResult {
        CallResult {
            outcomes: Vec::new(),
            unresolved: Weight::ONE,
            lost: Weight::ZERO,
            observed: false,
            pending: vec![(depth, Weight::ONE)],
            failures: Failures::default(),
        }
    }

    /// Take out the weight waiting on the call at `depth`.
    fn take_pending(&mut self, depth: usize) -> Weight {
        match self.pending.iter().position(|&(d, _)| d == depth) {
            Some(i) => self.pending.remove(i).1,
            None => Weight::ZERO,
        }
    }
}

/// A call's result so far, while the call it waits on is being solved
/// (section 6).
struct Approx {
    result: Arc<CallResult>,
    /// The depth of that call, and its frame and round when this was worked
    /// out. It stands for the call as long as that frame runs, and can be
    /// reused as it is until that frame's next round.
    head: usize,
    frame: u64,
    round: u64,
}

/// Add `weight` times what's waiting in `from` to `into`.
fn add_pending(into: &mut Vec<(usize, Weight)>, from: &[(usize, Weight)], weight: Weight) {
    for &(depth, w) in from {
        match into.iter_mut().find(|(d, _)| *d == depth) {
            Some((_, total)) => *total += weight * w,
            None => into.push((depth, weight * w)),
        }
    }
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
    /// The calls running, with their depth in the stack.
    active: FxHashMap<CallKey, usize>,
    /// Results so far of calls that wait on a call being solved by
    /// iteration (section 6).
    approx: FxHashMap<CallKey, Approx>,
    /// For each depth in the stack: the frame running there, and its round,
    /// each a number never used before.
    frames_at: Vec<u64>,
    rounds_at: Vec<u64>,
    last_number: u64,
    /// Weight waiting on calls being solved, in the current call.
    pending: Vec<(usize, Weight)>,
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
    /// Nesting depth of local `simulate` inference (in either outer mode).
    nested: usize,
    next_latent: u64,
    /// Dynamic effect boundary for collection callbacks. Effects inside an
    /// explicit `simulate` are local to that computation instead.
    callback: Option<&'static str>,
    /// In partial mode: the worlds that failed, in the current call.
    pub failures: Failures,
    /// The `try`s running, innermost last, each with the depth of the call
    /// it's in, and its catches.
    handlers: Vec<(usize, &'p [Catch])>,
    /// Worlds of the statement running that faulted inside a `try` of this
    /// call that catches the fault, on their way to it.
    faulted: Vec<(World, RuntimeError)>,
    /// Where evidence can still be applied, to tell whether a failed
    /// world's weight can be added to the finished worlds'.
    evidence: EvidenceOrder,
}

/// Where a world was when it failed.
#[derive(Clone, Copy)]
struct At {
    f: FnId,
    stmt: StmtId,
    weight: Weight,
    run: u32,
}

impl At {
    fn new(f: FnId, stmt: &Stmt, w: &World) -> At {
        At {
            f,
            stmt: stmt.id,
            weight: w.weight,
            run: w.run,
        }
    }
}

/// The value of `$e` in one world. If it fails with a fault that a `catch`
/// or partial mode takes, the world leaves the statement: `$undo` runs, and
/// the statement goes on with the next world (see `Engine::fail`). A catch
/// gets the world as its statement began: `world w` when `$e` leaves `w` as
/// it was, or `saved s` with a copy `s` taken before `$e` changed it.
macro_rules! each {
    ($self:ident, $at:expr, world $w:expr, $e:expr $(, $undo:block)?) => {
        match $e {
            Ok(v) => v,
            Err(e) => {
                let saved = $self.snapshot(&$w);
                $self.fail(e, $at, saved)?;
                $($undo)?
                continue;
            }
        }
    };
    ($self:ident, $at:expr, saved $saved:ident, $e:expr $(, $undo:block)?) => {
        match $e {
            Ok(v) => v,
            Err(e) => {
                $self.fail(e, $at, $saved.take())?;
                $($undo)?
                continue;
            }
        }
    };
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
            active: FxHashMap::default(),
            approx: FxHashMap::default(),
            frames_at: Vec::new(),
            rounds_at: Vec::new(),
            last_number: 0,
            pending: Vec::new(),
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
            next_latent: 0,
            callback: None,
            failures: Failures::default(),
            evidence: probl_sema::effects::evidence_order(prog),
            handlers: Vec::new(),
            faulted: Vec::new(),
        }
    }

    /// Run the top level. Returns the total weight of the worlds that finish.
    pub fn run_main(&mut self) -> Result<Weight> {
        let prog = self.prog;
        let main = prog.main();
        let world = World {
            slots: vec![Value::Dead; main.n_slots()],
            constraints: Default::default(),
            inherited: Default::default(),
            weight: Weight::ONE,
            run: 0,
        };
        let flow = self.exec_block(MAIN, &main.body, vec![world])?;
        uncaught(&flow)?;
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
        self.failures = Failures::default();
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
                constraints: Default::default(),
                inherited: Default::default(),
                weight: Weight::ONE,
                run: run as u32,
            })
            .collect();
        let flow = self.exec_block(MAIN, &main.body, worlds)?;
        uncaught(&flow)?;
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
            failures: std::mem::take(&mut self.failures),
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
        let counts = builtins::counts(*b, &values, &mut self.budget).map_err(|err| err.at(e.span))?;
        Ok(counts.filter(Counts::direct))
    }

    fn continuous_draw(&self, span: Span) -> RuntimeError {
        analytic::unsupported("a continuous draw inside `simulate`")
            .at(span)
            .with_note("`simulate` is computed by enumeration, even in sample mode")
            .with_help("draw the value outside `simulate`; analytic joint distribution recipes aren't supported yet")
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
                match builtins::counts(Builtin::Binomial, &[n, Value::Prob(0.5)], &mut self.budget)
                    .map_err(|e| e.at(from))?
                {
                    Some(Counts::Binomial { n, .. }) => Some(Seen::Binomial {
                        trials: n,
                        k: count(&v),
                    }),
                    _ => None,
                }
            }
            Likelihood::Bernoulli { value } => match self.eval(f, value, w)? {
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
                    Value::Int(_) | Value::Float(_) | Value::Prob(_) if v.as_f64().is_some() => Some(Seen::Normal {
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

    /// Whether a fault ends only its world: in partial mode, except inside
    /// `simulate` and collection callbacks, which fail as a whole, in the
    /// world that runs them (docs/semantics.md, section 11).
    fn partial(&self) -> bool {
        self.config.partial && self.nested == 0 && self.callback.is_none()
    }

    /// Whether a `try` running in this call catches `fault`.
    fn catches_here(&self, fault: Option<Fault>) -> bool {
        self.handlers
            .iter()
            .any(|(depth, catches)| *depth == self.depth && catches.iter().any(|c| c.catches(fault)))
    }

    /// Whether a `try` running in a call further up catches `fault`.
    fn catches_above(&self, fault: Option<Fault>) -> bool {
        self.handlers
            .iter()
            .any(|(depth, catches)| *depth < self.depth && catches.iter().any(|c| c.catches(fault)))
    }

    /// Inside a `try` of this call: a copy of `w`, for the catch it may
    /// fault to. Elsewhere nothing is copied.
    #[inline]
    fn snapshot(&self, w: &World) -> Option<World> {
        if self.handlers.is_empty() {
            return None;
        }
        self.handlers
            .iter()
            .any(|(depth, _)| *depth == self.depth)
            .then(|| w.clone())
    }

    /// A world at `at` failed with `e`. A fault that a `try` of this call
    /// catches sends `saved`, the world as its statement began, on to the
    /// catch. One that a `try` further up catches, or any fault in partial
    /// mode, ends the world here: it's recorded, for the call's caller to
    /// catch, or as a failure. Either way the statement goes on with the
    /// other worlds. Anything else stops the run.
    fn fail(&mut self, e: RuntimeError, at: At, saved: Option<World>) -> Result<()> {
        if e.fault.is_none() {
            return Err(e);
        }
        if let Some(w) = saved.filter(|_| self.catches_here(e.fault)) {
            self.faulted.push((w, e));
            return Ok(());
        }
        if !self.partial() && !self.catches_above(e.fault) {
            return Err(e);
        }
        let fun = &self.prog.functions[at.f as usize];
        let e = match fun.kind {
            FnKind::Named => e.with_note(format!("in a call to `{}`", fun.name)),
            FnKind::Lambda => e.with_note("inside a lambda"),
            FnKind::Simulate => e.with_note("inside a `simulate` block"),
            FnKind::Main => e,
        };
        let stmt = at.stmt as usize;
        let before_evidence = self.evidence.at[stmt] || self.evidence.after[stmt];
        let run = self.sampler.is_some().then_some(at.run);
        self.failures.record(e, at.weight, run, before_evidence);
        Ok(())
    }

    /// Stop if a statement produced more worlds than allowed. (When sampling,
    /// worlds never multiply: there's one per run in the batch.)
    fn check_worlds(&self, n: usize, span: Span) -> Result<()> {
        if n > self.config.max_worlds && self.sampler.is_none() {
            return Err(RuntimeError::limit(
                span,
                format!("more than {} worlds are too many to follow", self.config.max_worlds),
            )
            .with_help("simplify the model, raise the limit, or sample it with `@mode sample(runs: 10_000)`"));
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
            flow.faulted.extend(out.faulted);
        }
        Ok(flow)
    }

    fn restrict_event(&mut self, event: &analytic::Event, yes: bool, w: &mut World, span: Span) -> Result<f64> {
        self.budget
            .collection((w.constraints.len() + 1) as u128)
            .map_err(|e| e.at(span))?;
        self.budget
            .work((w.constraints.len() + event.yes.0.len() + event.draw.domain.0.len()) as u64)
            .map_err(|e| e.at(span))?;
        Ok(event.restrict(yes, &mut w.constraints))
    }

    fn exec_stmt(&mut self, f: FnId, stmt: &'p Stmt, worlds: Vec<World>) -> Result<Flow> {
        // Outside every `try`, no world can fault to a catch.
        if self.handlers.is_empty() {
            return self.exec_stmt_kind(f, stmt, worlds);
        }
        // The worlds that fault in this statement, on their way to a catch,
        // leave with its flow.
        let outer = std::mem::take(&mut self.faulted);
        let flow = self.exec_stmt_kind(f, stmt, worlds);
        let faulted = std::mem::replace(&mut self.faulted, outer);
        let mut flow = flow?;
        flow.faulted.extend(faulted);
        Ok(flow)
    }

    fn exec_stmt_kind(&mut self, f: FnId, stmt: &'p Stmt, worlds: Vec<World>) -> Result<Flow> {
        let span = stmt.span;
        if matches!(
            stmt.kind,
            StmtKind::Draw { .. } | StmtKind::Take { .. } | StmtKind::Observe { .. } | StmtKind::Chance { .. }
        ) {
            self.check_callback_effect(span)?;
        }
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
                let mut failed = Vec::new();
                for (i, w) in worlds.iter_mut().enumerate() {
                    let mut saved = self.snapshot(w);
                    let v = match self.record_place_type(place, w) {
                        Some(ty) => self.eval_expected(f, value, w, &ty),
                        None => self.eval(f, value, w),
                    };
                    let v = each!(self, At::new(f, stmt, w), saved saved, v, { failed.push(i) });
                    each!(self, At::new(f, stmt, w), saved saved, self.assign(f, place, v, w, span), {
                        failed.push(i)
                    });
                }
                drop_failed(&mut worlds, &failed);
                // When the live values say less than before, as when a
                // call's result is used up (`f = f - $5`), worlds may now be
                // the same: merging spares the statements after from
                // following each of them (a call next would multiply them).
                if self.live.narrows[stmt.id as usize] && self.merging() {
                    return Ok(Flow::next(self.merge(worlds, stmt.id)));
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
                    let at = At::new(f, stmt, &w);
                    let mut saved = self.snapshot(&w);
                    if self.sampler.is_some() {
                        if let Some(counts) = each!(self, at, saved saved, self.direct_counts(f, dist, &w)) {
                            let k = counts.sample(self.sampler.as_mut().expect("sampling"));
                            let mut w = w;
                            each!(self, at, saved saved, self.assign(f, place, Value::Int(k.into()), &mut w, dist.span));
                            out.push(w);
                            continue;
                        }
                    }
                    let d = each!(self, at, saved saved, self.eval(f, dist, &w));
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
                    let mark = out.len();
                    each!(self, at, saved saved, self.split_by(f, place, d, w, dist.span, &mut out), {
                        out.truncate(mark)
                    });
                    self.check_worlds(out.len(), span)?;
                }
                Ok(Flow::next(self.merge(out, stmt.id)))
            }
            StmtKind::Take { place, bag } => {
                let mut out = Vec::new();
                for w in worlds {
                    let at = At::new(f, stmt, &w);
                    let current = each!(self, at, world w, self.read_place(f, bag, &w, span));
                    let Value::Bag(cards) = &current else {
                        return Err(RuntimeError::new(
                            span,
                            format!("`take` needs a bag, found {}", ops::article(&current.kind())),
                        )
                        .with_help("make one with `bag([card: count, …])`"));
                    };
                    let total: u128 = cards.values().map(|n| *n as u128).sum();
                    if total == 0 {
                        let empty = RuntimeError::new(span, "can't take a card from an empty bag")
                            .as_fault(Fault::EmptyCollection);
                        each!(self, at, world w, Err::<(), _>(empty));
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
                    let at = At::new(f, stmt, &w);
                    let mut key = Vec::with_capacity(args.len() + 4);
                    let evaluated: Result<()> = args.iter().try_for_each(|a| {
                        key.push(self.eval(f, a, &w)?);
                        Ok(())
                    });
                    each!(self, at, world w, evaluated);
                    let func = match callee {
                        Callee::Fn { func, capture_args } => {
                            for &s in capture_args {
                                key.push(self.slot(f, s, &w, span)?);
                            }
                            *func
                        }
                        Callee::Value(e, named) => match each!(self, at, world w, self.eval(f, e, &w)) {
                            Value::Builtin(b) => {
                                let value =
                                    each!(self, at, world w, self.builtin_values(b, &key, named, w.weight, span));
                                let mut saved = self.snapshot(&w);
                                let mut nw = w;
                                each!(self, at, saved saved, self.assign(f, dest, value, &mut nw, span));
                                out.push(nw);
                                continue;
                            }
                            Value::Closure(c) => {
                                if !named.is_empty() {
                                    return Err(RuntimeError::new(
                                        span,
                                        "only builtin minimum/maximum accept named defaults",
                                    ));
                                }
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
                    let result = each!(self, at, world w, self.call(func, key, span));
                    self.unresolved += w.weight * result.unresolved;
                    self.lost += w.weight * result.lost;
                    // What faulted in the call faulted in this world: a catch
                    // here takes it with the world as it was, and otherwise
                    // the world fails, for a `try` further up or as a
                    // failure, and may still meet evidence after the call.
                    let run = self.sampler.is_some().then_some(w.run);
                    let after = self.evidence.after[stmt.id as usize];
                    for g in &result.failures.groups {
                        if self.catches_here(g.error.fault) {
                            let mut caught = w.clone();
                            caught.weight = caught.weight * g.weight;
                            self.faulted.push((caught, g.error.clone()));
                        } else if self.partial() || self.catches_above(g.error.fault) {
                            self.failures.absorb_group(g, w.weight, run, after);
                        } else {
                            return Err(g.error.clone());
                        }
                    }
                    add_pending(&mut self.pending, &result.pending, w.weight);
                    let last = result.outcomes.len().saturating_sub(1);
                    let mut w = Some(w);
                    for (i, (v, p, restrictions)) in result.outcomes.iter().enumerate() {
                        let mut nw = if i == last {
                            w.take().unwrap()
                        } else {
                            w.clone().unwrap()
                        };
                        nw.weight = nw.weight * *p;
                        // Only copy the constraints (shared with the other
                        // outcomes' worlds) when the call added some.
                        if !restrictions.is_empty() {
                            Arc::make_mut(&mut nw.constraints)
                                .extend(restrictions.iter().map(|(id, d)| (*id, d.clone())));
                        }
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
                    let condition = each!(self, At::new(f, stmt, &w), world w, self.eval(f, cond, &w));
                    if let Value::Event(event) = condition {
                        self.check_callback_effect(cond.span)?;
                        let mut y = w.clone();
                        let mut n = w;
                        let p = self.restrict_event(&event, true, &mut y, cond.span)?;
                        let q = self.restrict_event(&event, false, &mut n, cond.span)?;
                        if p > 0.0 {
                            yes.push(y.scaled(p));
                        }
                        if q > 0.0 {
                            no.push(n.scaled(q));
                        }
                        continue;
                    }
                    let c = ops::condition(&condition).map_err(|e| e.at(cond.span))?;
                    if (c.yes > 0.0 && c.no > 0.0) || c.missing > 0.0 {
                        self.check_callback_effect(cond.span)?;
                    }
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
                    let at = At::new(f, stmt, &w);
                    // Chance weights are explicit probabilities.
                    let chances: Result<Vec<f64>> = arms
                        .iter()
                        .map(|(weight, _)| {
                            let value = self.eval(f, weight, &w)?;
                            ops::to_prob(&value).map_err(|e| e.at(weight.span))
                        })
                        .collect();
                    let chances = each!(self, at, world w, chances);
                    let sum: f64 = chances.iter().sum();
                    if sum > 1.0 + 1e-9 {
                        let over = RuntimeError::new(
                            span,
                            format!(
                                "the chances in this `chance` add up to {}, more than 100%",
                                fmt_prob(sum)
                            ),
                        );
                        each!(self, at, world w, Err::<(), _>(over.as_fault(Fault::DomainError)));
                    }
                    let remainder = (1.0 - sum).max(0.0);
                    if remainder > 1e-9 && *exhaustive && otherwise.is_none() {
                        let short = RuntimeError::new(
                            span,
                            format!("the chances add up to {} and there's no `else` arm", fmt_prob(sum)),
                        )
                        .with_help("a `chance` used as a value needs its chances to add up to 100%, or an `else`");
                        each!(self, at, world w, Err::<(), _>(short.as_fault(Fault::DomainError)));
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
                    let v = each!(self, At::new(f, stmt, &w), world w, self.eval(f, value, &w));
                    let mut constraints = w.constraints;
                    if constraints.keys().any(|id| !w.inherited.contains(id)) {
                        Arc::make_mut(&mut constraints).retain(|id, _| w.inherited.contains(id));
                    }
                    flow.returned.push((v, w.weight, constraints));
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
                    let here = At::new(f, stmt, &w);
                    let mut saved = self.snapshot(&w);
                    if let Some(u) = &exact {
                        let at = from.as_ref().map_or(span, |d| d.span);
                        if let Some(ln) = each!(self, here, saved saved, self.observe_exactly(f, u, &mut w, at)) {
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
                            let v = each!(self, here, saved saved, self.eval(f, value, &w));
                            if let Value::Event(event) = v {
                                let p = self.restrict_event(&event, true, &mut w, value.span)?;
                                (p, 0.0, 1.0 - p)
                            } else {
                                let b = ops::fact(&v, "observe").map_err(|e| e.at(value.span))?;
                                if b { (1.0, 0.0, 0.0) } else { (0.0, 0.0, 1.0) }
                            }
                        }
                        Some(d) => {
                            let v = each!(self, here, saved saved, self.eval(f, value, &w));
                            match each!(self, here, saved saved, self.direct_counts(f, d, &w)) {
                                Some(counts) if self.sampler.is_some() => {
                                    let x = match v {
                                        Value::Bool(_) => None,
                                        _ => v.as_f64(),
                                    };
                                    (x.map_or(0.0, |x| counts.pmf(x)), 0.0, 0.0)
                                }
                                _ => {
                                    let dist = each!(self, here, saved saved, self.eval(f, d, &w));
                                    if let (Value::Bool(b), Value::Event(event)) = (&v, &dist) {
                                        let p = self.restrict_event(event, *b, &mut w, span)?;
                                        (p, 0.0, 1.0 - p)
                                    } else {
                                        if analytic::contains(&v) || analytic::contains(&dist) {
                                            return Err(analytic::unsupported("this likelihood observation").at(span));
                                        }
                                        self.densities |= is_density(&dist);
                                        each!(
                                            self,
                                            here,
                                            saved saved,
                                            likelihood(&dist, &v, self.sampler.is_some()).map_err(|e| e.at(span))
                                        )
                                    }
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
                let mut failed = Vec::new();
                for (i, w) in worlds.iter().enumerate() {
                    let v = each!(self, At::new(f, stmt, w), world w, self.eval(f, value, w), { failed.push(i) });
                    let k = match key {
                        Some(k) => {
                            let k_value =
                                each!(self, At::new(f, stmt, w), world w, self.eval(f, k, w), { failed.push(i) });
                            if analytic::contains(&k_value) {
                                return Err(analytic::unsupported("grouping by a continuous outcome").at(k.span));
                            }
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
                        if analytic::contains(&v) && !matches!(v, Value::Analytic(_) | Value::Event(_)) {
                            return Err(analytic::unsupported(
                                "reporting an aggregate containing analytic outcomes; report its fields individually",
                            )
                            .at(value.span));
                        }
                        (v, None)
                    };
                    self.sinks[*site as usize]
                        .validate_analytic(&k, &v)
                        .map_err(|e| e.at(value.span))?;
                    self.sinks[*site as usize].add(k, &v, w.weight, run);
                }
                let mut worlds = worlds;
                drop_failed(&mut worlds, &failed);
                Ok(Flow::next(worlds))
            }
            StmtKind::Fail { message } => Err(RuntimeError::new(span, message.clone())),
            StmtKind::Try { body, catches } => {
                self.handlers.push((self.depth, catches.as_slice()));
                let flow = self.exec_block(f, body, worlds);
                self.handlers.pop();
                let mut flow = flow?;
                // Each faulted world goes on in the first catch that takes
                // its fault; the others are for a `try` around this one.
                let mut caught: Vec<Vec<World>> = vec![Vec::new(); catches.len()];
                for (w, e) in std::mem::take(&mut flow.faulted) {
                    match catches.iter().position(|c| c.catches(e.fault)) {
                        Some(i) => caught[i].push(w),
                        None => flow.faulted.push((w, e)),
                    }
                }
                for (c, worlds) in catches.iter().zip(caught) {
                    if !worlds.is_empty() {
                        flow.join(self.exec_block(f, &c.body, worlds)?);
                    }
                }
                flow.next = self.merge(flow.next, stmt.id);
                Ok(flow)
            }
            StmtKind::Check { slot, ty } => {
                for w in &mut worlds {
                    // A delayed variable is drawn only if some of its values
                    // could fail the check.
                    if let Value::Delayed(_) = &w.slots[*slot as usize] {
                        let always = *ty == TypeSpec::Float;
                        if always {
                            continue;
                        }
                        self.draw_delayed(w, *slot);
                    }
                    w.slots[*slot as usize] = self.coerce(w.slots[*slot as usize].clone(), ty, span)?;
                    let v = &w.slots[*slot as usize];
                    if !self.conforms(v, ty) {
                        let name = &self.prog.functions[f as usize].slots[*slot as usize].name;
                        let what = if name == TEMP {
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
            out.faulted.extend(flow.faulted);
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
        if inside
            .iter()
            .any(|w| !w.constraints.is_empty() || w.slots.iter().any(analytic::contains))
        {
            self.too_large.insert(stmt.id);
            return Ok(None);
        }
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
        let mut returns: Vec<Vec<Returned>> = Vec::new();
        let mut unresolved: Vec<Weight> = Vec::new();
        let mut failed: Vec<Failures> = Vec::new();
        let mut faulted: Vec<Vec<(World, RuntimeError)>> = Vec::new();
        // A body that uses a call's result so far can't be solved as a chain:
        // that result changes from round to round.
        let saved_pending = std::mem::take(&mut self.pending);
        let mut k = 0;
        while k < states.len() {
            if !self.pending.is_empty() {
                self.pending = saved_pending;
                return Ok(None);
            }
            if states.len() > self.config.max_chain_states {
                self.pending = saved_pending;
                self.too_large.insert(stmt.id);
                return Ok(None);
            }
            let world = World {
                slots: states[k].clone(),
                constraints: Default::default(),
                inherited: Default::default(),
                weight: Weight::ONE,
                run: 0,
            };
            let saved_unresolved = std::mem::replace(&mut self.unresolved, Weight::ZERO);
            let saved_lost = std::mem::replace(&mut self.lost, Weight::ZERO);
            let saved_failures = std::mem::take(&mut self.failures);
            let flow = self.exec_block(f, body, vec![world]);
            let left = std::mem::replace(&mut self.unresolved, saved_unresolved);
            let lost = std::mem::replace(&mut self.lost, saved_lost);
            let failures = std::mem::replace(&mut self.failures, saved_failures);
            let mut flow = flow?;
            if flow
                .next
                .iter()
                .chain(&flow.continued)
                .chain(&flow.broke)
                .any(|w| !w.constraints.is_empty() || w.slots.iter().any(analytic::contains))
                || flow
                    .returned
                    .iter()
                    .any(|(v, _, c)| !c.is_empty() || analytic::contains(v))
            {
                self.pending = saved_pending;
                self.too_large.insert(stmt.id);
                return Ok(None);
            }
            clear_dead(&mut flow.broke, after);
            // Leaving: by `break` or `return`, unresolved, ruled out,
            // failed, or faulted to a catch outside the loop.
            let mut leave = left + lost + failures.weight;
            let faults = std::mem::take(&mut flow.faulted);
            for (w, _) in &faults {
                leave += w.weight;
            }
            for w in &flow.broke {
                leave += w.weight;
            }
            for (_, w, _) in &flow.returned {
                leave += *w;
            }
            let mut next: FxHashMap<usize, f64> = FxHashMap::default();
            for w in flow.next.into_iter().chain(flow.continued) {
                let p = w.weight.to_f64();
                if p == 0.0 {
                    // Too small for the chain's arithmetic.
                    self.too_large.insert(stmt.id);
                    self.pending = saved_pending;
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
            failed.push(failures);
            faulted.push(faults);
            k += 1;
        }
        let waiting = !self.pending.is_empty();
        self.pending = saved_pending;
        if waiting {
            return Ok(None);
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
            for (value, w, c) in std::mem::take(&mut returns[k]) {
                flow.returned.push((value, w * times, c));
            }
            left += unresolved[k] * times;
            self.failures.absorb(&failed[k], times, None, false);
            for (mut w, e) in std::mem::take(&mut faulted[k]) {
                w.weight = w.weight * times;
                flow.faulted.push((w, e));
            }
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
            .filter(|&&i| names[i].name != TEMP)
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
            Value::Continuous(family) => {
                if self.nested > 0 {
                    return Err(self.continuous_draw(span));
                }
                self.next_latent += 1;
                let v = Analytic::new(self.next_latent, *family)
                    .value()
                    .map_err(|e| e.at(span))?;
                let mut w = w;
                self.assign(f, place, v, &mut w, span)?;
                out.push(w);
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
                    if matches!(v, Value::Continuous(_)) {
                        self.split_by(f, place, v.clone(), nw, span, out)?;
                    } else {
                        self.assign(f, place, v.clone(), &mut nw, span)?;
                        out.push(nw);
                    }
                }
            }
            Value::Prob(p) => {
                return self.split_by(f, place, Dist::bernoulli(p).into_value(), w, span, out);
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

    /// Convert values at declared type boundaries, recursively through
    /// containers. Numeric probability conversions are checked, never clamped.
    fn coerce(&mut self, v: Value, ty: &TypeSpec, span: Span) -> Result<Value> {
        self.coerce_at(v, ty, span, 0)
    }

    fn coerce_at(&mut self, v: Value, ty: &TypeSpec, span: Span, depth: usize) -> Result<Value> {
        self.budget.work(1).map_err(|e| e.at(span))?;
        if depth > 64 {
            return Err(RuntimeError::new(
                span,
                "type conversion nesting exceeds the limit of 64",
            ));
        }
        Ok(match (v, ty) {
            (Value::Analytic(_) | Value::Event(_), TypeSpec::Prob) => {
                return Err(analytic::unsupported("converting this outcome to prob").at(span));
            }
            (v @ Value::Float(_), TypeSpec::Int) => Value::Int(
                ops::integer(&v, "int conversion", &mut self.budget)
                    .map_err(|e| e.at(span))?
                    .into_owned(),
            ),
            (Value::Analytic(_), TypeSpec::Int) => {
                return Err(analytic::unsupported("converting this outcome to int").at(span));
            }
            (Value::Prob(p), TypeSpec::Float) => Value::Float(p),
            // A declared type is a contract: failing it isn't a fault of the
            // world's values, even in partial mode.
            (v @ (Value::Int(_) | Value::Float(_)), TypeSpec::Prob) => {
                ops::make_prob(&v).map_err(|e| OpError { fault: None, ..e }.at(span))?
            }
            (Value::List(xs), TypeSpec::List(t)) => {
                self.budget.collection(xs.len() as u128).map_err(|e| e.at(span))?;
                let mut out = Vec::with_capacity(xs.len());
                for x in xs.iter() {
                    out.push(self.coerce_at(x.clone(), t, span, depth + 1)?);
                }
                Value::list(out)
            }
            (Value::Map(xs), TypeSpec::Map(kt, vt)) => {
                let mut out = BTreeMap::new();
                for (k, v) in xs.iter() {
                    let key = self.coerce_at(k.clone(), kt, span, depth + 1)?;
                    let value = self.coerce_at(v.clone(), vt, span, depth + 1)?;
                    if out.insert(key, value).is_some() {
                        return Err(RuntimeError::new(span, "map key collision during type conversion"));
                    }
                }
                Value::map(out)
            }
            (Value::Bag(xs), TypeSpec::Bag(t)) => {
                let mut out = BTreeMap::<Value, u64>::new();
                for (x, n) in xs.iter() {
                    let key = self.coerce_at(x.clone(), t, span, depth + 1)?;
                    let count = out.entry(key).or_default();
                    *count = count
                        .checked_add(*n)
                        .ok_or_else(|| RuntimeError::new(span, "bag count overflow during type conversion"))?;
                }
                Value::multiset(crate::value::Multiset::new(out))
            }
            (Value::Dist(d), TypeSpec::Dist(t)) => {
                let mut out = Vec::with_capacity(d.outcomes.len());
                for (x, p) in &d.outcomes {
                    out.push((self.coerce_at(x.clone(), t, span, depth + 1)?, *p));
                }
                ops::combine(out, d.missing, &mut self.budget).map_err(|e| e.at(span))?
            }
            (Value::Record(r), TypeSpec::Record(t)) => {
                let types: Vec<_> = self.prog.records[*t as usize]
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect();
                let mut fields = Vec::with_capacity(r.fields.len());
                for (name, value) in &r.fields {
                    let value = match types.iter().find(|(n, _)| n == &**name) {
                        Some((_, t)) => self.coerce_at(value.clone(), t, span, depth + 1)?,
                        None => value.clone(),
                    };
                    fields.push((name.clone(), value));
                }
                ops::make_record(r.ty.clone(), fields)
            }
            (Value::Record(r), TypeSpec::AnonRecord(types)) => {
                let mut fields = Vec::with_capacity(r.fields.len());
                for (name, value) in &r.fields {
                    let value = match types.iter().find(|(n, _)| n == &**name) {
                        Some((_, t)) => self.coerce_at(value.clone(), t, span, depth + 1)?,
                        None => value.clone(),
                    };
                    fields.push((name.clone(), value));
                }
                ops::make_record(r.ty.clone(), fields)
            }
            (v, _) => v,
        })
    }

    /// Does `v` have the declared type?
    fn conforms(&self, v: &Value, ty: &TypeSpec) -> bool {
        match (ty, v) {
            (TypeSpec::Int, Value::Int(_)) => true,
            (TypeSpec::Float, Value::Float(_) | Value::Int(_) | Value::Analytic(_)) => true,
            (TypeSpec::Complex, Value::Complex(_)) => true,
            (TypeSpec::Prob, Value::Prob(_)) => true,
            (TypeSpec::Bool, Value::Bool(_) | Value::Event(_)) => true,
            (TypeSpec::Str, Value::Str(_)) => true,
            (TypeSpec::Date, Value::Date(_)) => true,
            (TypeSpec::Unit, Value::Unit) => true,
            (TypeSpec::Function, Value::Closure(_) | Value::Builtin(_)) => true,
            (TypeSpec::List(t), Value::List(items)) => items.iter().all(|x| self.conforms(x, t)),
            (TypeSpec::List(t), Value::Range(..)) => **t == TypeSpec::Int,
            (TypeSpec::Map(k, t), Value::Map(m)) => m.iter().all(|(a, b)| self.conforms(a, k) && self.conforms(b, t)),
            (TypeSpec::Bag(t), Value::Bag(b)) => b.keys().all(|x| self.conforms(x, t)),
            (TypeSpec::Dist(t), Value::Dist(d)) => d.outcomes.iter().all(|(x, _)| match x {
                Value::Continuous(_) => **t == TypeSpec::Float,
                _ => self.conforms(x, t),
            }),
            (TypeSpec::Dist(t), Value::Continuous(_)) => **t == TypeSpec::Float,
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
    ///
    /// When enumerating, a call can come back to itself: the same function,
    /// with the same arguments, while it's still running. Its result is then
    /// solved by iteration (section 6). The call that comes back gets the
    /// result so far, which starts with nothing resolved, and the call it
    /// came back to runs again with each new result, until the weight still
    /// waiting on it is below ε (`run_call`).
    fn call(&mut self, func: FnId, key: Vec<Value>, span: Span) -> Result<Arc<CallResult>> {
        self.stats.calls += 1;
        let prog = self.prog;
        let fun = &prog.functions[func as usize];
        // When sampling, every call makes its own choices (section 14).
        let sampling = self.sampler.is_some();
        // A cached result does not prove a callback's effects were permitted:
        // a draw or observation can collapse to one outcome of weight one.
        let memoizable = self.config.memoizing && !sampling && !fun.effects.prints && self.callback.is_none();
        let call_key = (func, key);
        if memoizable {
            if let Some(r) = self.memo.get(&call_key) {
                self.stats.memo_hits += 1;
                self.observed |= r.observed;
                return Ok(r.clone());
            }
        }
        if !sampling {
            if let Some(&depth) = self.active.get(&call_key) {
                if fun.effects.prints {
                    return Err(RuntimeError::new(
                        span,
                        format!("`{}` comes back to itself, and prints", describe_call(fun, &call_key.1)),
                    )
                    .with_note("a call that comes back to itself runs several times, until its result settles, and would print each time")
                    .with_help("remove the `print`, or print the result where the call is made"));
                }
                let so_far = match self.approx.get(&call_key) {
                    Some(a) if self.frames_at.get(a.head) == Some(&a.frame) => a.result.clone(),
                    _ => {
                        let result = Arc::new(CallResult::waiting(depth));
                        let approx = Approx {
                            result: result.clone(),
                            head: depth,
                            frame: self.frames_at[depth],
                            round: self.rounds_at[depth],
                        };
                        self.approx.insert(call_key, approx);
                        result
                    }
                };
                return Ok(so_far);
            }
            // Worked out already, in this round of the call it waits on.
            if let Some(a) = self.approx.get(&call_key) {
                if self.frames_at.get(a.head) == Some(&a.frame) && self.rounds_at.get(a.head) == Some(&a.round) {
                    self.stats.memo_hits += 1;
                    return Ok(a.result.clone());
                }
            }
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
        let depth = self.depth + 1;
        if !sampling {
            self.active.insert(call_key.clone(), depth);
            if self.frames_at.len() <= depth {
                self.frames_at.resize(depth + 1, 0);
                self.rounds_at.resize(depth + 1, 0);
            }
            self.last_number += 1;
            self.frames_at[depth] = self.last_number;
        }
        let result = self.run_call(func, &call_key, slots, depth, span);
        if !sampling {
            self.active.remove(&call_key);
            // Results that waited on this frame no longer stand for anything.
            self.frames_at[depth] = 0;
        }
        let result = result?;
        // A result still waiting on a call being solved isn't final.
        if memoizable
            && result.pending.is_empty()
            && result
                .outcomes
                .iter()
                .all(|(v, _, c)| c.is_empty() && !analytic::contains(v))
        {
            if self.memo.len() >= self.config.max_cached_calls {
                self.memo.clear();
            }
            self.memo.insert(call_key, result.clone());
        }
        Ok(result)
    }

    /// Run a call's body from `slots`, at `depth` in the stack, and return
    /// its result (section 6).
    ///
    /// If the call came back to itself, and waits on no call further down
    /// the stack, it's solved here: round after round, each using the result
    /// of the round before, until the weight still waiting on it is below ε.
    /// That weight stays unresolved. If it waits on a call further down,
    /// it's part of that call's solving instead: what waits on it waits on
    /// that call, and its result stands for it until that call's next round.
    fn run_call(
        &mut self,
        func: FnId,
        call_key: &CallKey,
        slots: Vec<Value>,
        depth: usize,
        span: Span,
    ) -> Result<Arc<CallResult>> {
        let fun = &self.prog.functions[func as usize];
        let sampling = self.sampler.is_some();
        let mut inherited = std::collections::BTreeSet::new();
        for v in &slots {
            analytic::collect_ids(v, &mut inherited);
        }
        let inherited = Arc::new(inherited);
        let mut rounds: u64 = 0;
        let mut before = f64::INFINITY;
        loop {
            if !sampling {
                self.last_number += 1;
                self.rounds_at[depth] = self.last_number;
            }
            let saved_unresolved = std::mem::replace(&mut self.unresolved, Weight::ZERO);
            let saved_lost = std::mem::replace(&mut self.lost, Weight::ZERO);
            let saved_observed = std::mem::replace(&mut self.observed, false);
            let saved_pending = std::mem::take(&mut self.pending);
            let saved_failures = std::mem::take(&mut self.failures);
            self.depth += 1;
            let flow = self.exec_block(
                func,
                &fun.body,
                vec![World {
                    slots: slots.clone(),
                    constraints: Default::default(),
                    inherited: inherited.clone(),
                    weight: Weight::ONE,
                    run: 0,
                }],
            );
            self.depth -= 1;
            let unresolved = std::mem::replace(&mut self.unresolved, saved_unresolved);
            let lost = std::mem::replace(&mut self.lost, saved_lost);
            let observed = std::mem::replace(&mut self.observed, saved_observed);
            let pending = std::mem::replace(&mut self.pending, saved_pending);
            let failures = std::mem::replace(&mut self.failures, saved_failures);
            self.observed |= observed;
            let flow = flow.map_err(|e| match fun.kind {
                FnKind::Named => e.with_note(format!("in a call to `{}`", fun.name)),
                FnKind::Simulate => e.with_note("inside a `simulate` block"),
                FnKind::Lambda => e.with_note("inside a lambda"),
                FnKind::Main => e,
            })?;
            uncaught(&flow)?;
            let mut result = CallResult {
                outcomes: merge_values(flow.returned),
                unresolved,
                lost,
                observed,
                pending,
                failures,
            };
            if let Some(head) = result.pending.iter().map(|&(d, _)| d).filter(|&d| d < depth).min() {
                // Part of the solving of the call at `head`.
                let waiting = Weight::sum(result.pending.iter().map(|&(_, w)| w));
                result.pending = vec![(head, waiting)];
                let result = Arc::new(result);
                let approx = Approx {
                    result: result.clone(),
                    head,
                    frame: self.frames_at[head],
                    round: self.rounds_at[head],
                };
                self.approx.insert(call_key.clone(), approx);
                return Ok(result);
            }
            let waiting = result.take_pending(depth);
            if waiting.is_zero() {
                return Ok(Arc::new(result));
            }
            // It came back to itself: solved here.
            if rounds == 0 {
                self.stats.solved_calls += 1;
            }
            let w = waiting.to_f64();
            if w <= self.config.epsilon {
                self.settle(depth);
                return Ok(Arc::new(result));
            }
            rounds += 1;
            self.stats.call_rounds += 1;
            if w >= before {
                return Err(RuntimeError::new(
                    span,
                    format!(
                        "`{}` never returns for some of its worlds",
                        describe_call(fun, &call_key.1)
                    ),
                )
                .with_note(format!("{} of its weight keeps coming back to it", fmt_prob(w)))
                .with_help("check that the recursion can end"));
            }
            if rounds >= self.config.max_iterations {
                return Err(RuntimeError::limit(
                    span,
                    format!(
                        "`{}` came back to itself {} times without settling",
                        describe_call(fun, &call_key.1),
                        self.config.max_iterations
                    ),
                )
                .with_note(format!("{} of its weight is still waiting on it", fmt_prob(w)))
                .with_help("check that the recursion can end, or raise `@epsilon`"));
            }
            before = w;
            result.pending.push((depth, waiting));
            let approx = Approx {
                result: Arc::new(result),
                head: depth,
                frame: self.frames_at[depth],
                round: self.rounds_at[depth],
            };
            self.approx.insert(call_key.clone(), approx);
        }
    }

    /// The call at `depth` is solved: the results that waited on it, from
    /// its last round, are final too if the weight still waiting in them is
    /// below ε. The others are dropped.
    fn settle(&mut self, depth: usize) {
        let frame = self.frames_at[depth];
        let epsilon = self.config.epsilon;
        let settled: Vec<(CallKey, CallResult)> = self
            .approx
            .iter()
            .filter(|(_, a)| a.head == depth && a.frame == frame)
            .filter(|(_, a)| a.result.pending.iter().all(|&(_, w)| w.to_f64() <= epsilon))
            .map(|(key, a)| {
                let mut result = (*a.result).clone();
                // What's still waiting stays unresolved.
                result.pending.clear();
                (key.clone(), result)
            })
            .collect();
        self.approx.retain(|_, a| a.head != depth);
        if !self.config.memoizing {
            return;
        }
        for (key, result) in settled {
            if !self.active.contains_key(&key)
                && !self.prog.functions[key.0 as usize].effects.prints
                && result
                    .outcomes
                    .iter()
                    .all(|(v, _, c)| c.is_empty() && !analytic::contains(v))
            {
                self.memo.insert(key, Arc::new(result));
            }
        }
    }

    /// `simulate { … }`: run a block as a separate model and return its
    /// normalized distribution (docs/semantics.md, section 8).
    fn simulate(&mut self, func: FnId, key: Vec<Value>, span: Span) -> Result<Value> {
        if key.iter().any(analytic::contains) {
            return Err(analytic::unsupported("capturing an analytic draw inside `simulate`").at(span));
        }
        let saved_observed = self.observed;
        let saved_densities = self.densities;
        // Enumerated, even when sampling (section 14).
        let sampler = self.sampler.take();
        let callback = self.callback.take();
        self.nested += 1;
        let result = self.call(func, key, span);
        self.nested -= 1;
        self.sampler = sampler;
        self.callback = callback;
        // Observations inside `simulate` condition its result only.
        self.observed = saved_observed;
        self.densities = saved_densities;
        let result = result?;
        if !result.pending.is_empty() {
            return Err(OpError::unsupported(
                "a `simulate` block that comes back to a call still running isn't supported yet",
            )
            .at(span));
        }
        // It's built whole or not at all: a fault on one of its paths, not
        // caught inside it, fails the world that runs it.
        if let Some(g) = result.failures.groups.first() {
            return Err(g.error.clone());
        }
        let resolved = Weight::sum(result.outcomes.iter().map(|(_, w, _)| *w));
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
            .map(|(v, w, _)| (v.clone(), w.ratio(denom)))
            .collect();
        ops::combine(pairs, result.unresolved.ratio(denom), &mut self.budget).map_err(|e| e.at(span))
    }

    /// Call a closure that must not split worlds (used by `map`, `filter`, …).
    fn call_pure(&mut self, closure: &Value, args: Vec<Value>, what: &'static str, span: Span) -> Result<Value> {
        if let Value::Builtin(b) = closure {
            return self.builtin_values(*b, &args, &[], Weight::ONE, span);
        }
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
        let previous = self.callback.replace(what);
        let result = self.call(c.func, key, span);
        self.callback = previous;
        let result = result?;
        if !result.pending.is_empty() {
            return Err(OpError::unsupported(format!(
                "a function given to `{what}` that comes back to a call still running isn't supported yet"
            ))
            .at(span));
        }
        // A callback that faults fails the whole operation, in the world
        // that runs it.
        if let Some(g) = result.failures.groups.first() {
            return Err(g.error.clone());
        }
        match result.outcomes.as_slice() {
            [(v, p, c)] if c.is_empty() && (p.to_f64() - 1.0).abs() < 1e-12 && result.unresolved.is_zero() => {
                Ok(v.clone())
            }
            _ => Err(RuntimeError::new(
                span,
                format!("the function given to `{what}` can't branch on chances, draw values or observe"),
            )),
        }
    }

    // ── Places ───────────────────────────────────────────────────────────

    fn check_callback_effect(&self, span: Span) -> Result<()> {
        match self.callback {
            Some(what) => Err(RuntimeError::new(
                span,
                format!("the function given to `{what}` can't branch on chances, draw values or observe"),
            )
            .with_help("use a loop for probabilistic traversal, or `simulate` for a local distribution")),
            None => Ok(()),
        }
    }

    fn record_place_type(&self, place: &Place, w: &World) -> Option<TypeSpec> {
        if place.path.is_empty() {
            return None;
        }
        let Value::Record(r) = &w.slots[place.slot as usize] else {
            return None;
        };
        let id = self
            .prog
            .records
            .iter()
            .position(|t| Some(t.name.as_str()) == r.ty.as_deref())?;
        let mut ty = TypeSpec::Record(id as u32);
        for element in &place.path {
            ty = match (element, ty) {
                (PathElem::Field(name), TypeSpec::Record(id)) => self.prog.records[id as usize]
                    .fields
                    .iter()
                    .find(|f| f.name == *name)?
                    .ty
                    .clone(),
                (PathElem::Field(name), TypeSpec::AnonRecord(fields)) => fields.into_iter().find(|(n, _)| n == name)?.1,
                (PathElem::Index(_), TypeSpec::List(t) | TypeSpec::Map(_, t)) => *t,
                _ => return None,
            };
        }
        Some(ty)
    }

    fn assign(&mut self, f: FnId, place: &Place, v: Value, w: &mut World, span: Span) -> Result<()> {
        if place.path.is_empty() {
            w.slots[place.slot as usize] = v;
            return Ok(());
        }
        let v = match self.record_place_type(place, w) {
            Some(ty) => {
                let v = self.coerce(v, &ty, span)?;
                if !self.conforms(&v, &ty) {
                    return Err(RuntimeError::new(
                        span,
                        format!(
                            "assigned field should be {}, found {}",
                            ty.describe(self.prog),
                            v.kind()
                        ),
                    ));
                }
                v
            }
            None => v,
        };
        let mut keys = Vec::with_capacity(place.path.len());
        for elem in &place.path {
            keys.push(match elem {
                PathElem::Field(name) => PathKey::Field(name.as_str()),
                PathElem::Index(e) => {
                    let key = self.eval(f, e, w)?;
                    if analytic::contains(&key) {
                        return Err(analytic::unsupported("an analytic assignment index").at(e.span));
                    }
                    PathKey::Index(key)
                }
            });
        }
        let target = &mut w.slots[place.slot as usize];
        if matches!(target, Value::Dead) {
            return Err(self.no_value(f, place.slot, span));
        }
        update(target, &keys, v, &mut self.budget).map_err(|e| e.at(span))
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

    fn slot(&mut self, f: FnId, s: SlotId, w: &World, span: Span) -> Result<Value> {
        match &w.slots[s as usize] {
            Value::Dead => Err(self.no_value(f, s, span)),
            Value::Delayed(_) => {
                let name = &self.prog.functions[f as usize].slots[s as usize].name;
                Err(
                    OpError::internal("internal error: a variable was read before it was drawn")
                        .at(span)
                        .with_note(format!(
                            "`{name}`'s draw was delayed for an exact update (docs/semantics.md, section 14)"
                        )),
                )
            }
            v => analytic::resolve(v, &w.constraints, &mut self.budget).map_err(|e| e.at(span)),
        }
    }

    fn no_value(&self, f: FnId, s: SlotId, span: Span) -> RuntimeError {
        let name = &self.prog.functions[f as usize].slots[s as usize].name;
        RuntimeError::new(span, format!("`{name}` has no value here")).with_help(
            "it's used before it's given a value; if a function reads it, call the function after the variable is set",
        )
    }

    // ── Expressions ──────────────────────────────────────────────────────

    fn eval_expected(&mut self, f: FnId, e: &Expr, w: &World, ty: &TypeSpec) -> Result<Value> {
        match (&e.kind, ty) {
            (ExprKind::List(xs), TypeSpec::List(t)) => {
                self.budget.collection(xs.len() as u128).map_err(|err| err.at(e.span))?;
                let mut out = Vec::with_capacity(xs.len());
                for x in xs {
                    out.push(self.eval_expected(f, x, w, t)?);
                }
                Ok(Value::list(out))
            }
            (ExprKind::Map(xs), TypeSpec::Map(kt, vt)) => {
                let mut out = BTreeMap::new();
                for (k, v) in xs {
                    let key = self.eval_expected(f, k, w, kt)?;
                    if analytic::contains(&key) {
                        return Err(analytic::unsupported("analytic map keys").at(k.span));
                    }
                    if key.is_uncertain() {
                        return Err(RuntimeError::new(k.span, "map keys can't be distributions"));
                    }
                    out.insert(key, self.eval_expected(f, v, w, vt)?);
                }
                Ok(Value::map(out))
            }
            (ExprKind::Record { ty: None, fields }, TypeSpec::AnonRecord(types)) => {
                let mut out = Vec::with_capacity(fields.len());
                for (n, e) in fields {
                    let v = match types.iter().find(|(name, _)| name == n) {
                        Some((_, ty)) => self.eval_expected(f, e, w, ty)?,
                        None => self.eval(f, e, w)?,
                    };
                    out.push((Arc::from(n.as_str()), v));
                }
                Ok(ops::make_record(None, out))
            }
            _ => {
                let v = self.eval(f, e, w)?;
                self.coerce(v, ty, e.span)
            }
        }
    }

    /// A shared field type supplies conversion context even when the record is
    /// represented by a finite distribution. Heterogeneous records have no
    /// shared context, and their individual declarations are checked below.
    fn record_field_type(&self, value: &Value, name: &str) -> Option<TypeSpec> {
        match value {
            Value::Record(r) => self
                .prog
                .records
                .iter()
                .find(|t| Some(t.name.as_str()) == r.ty.as_deref())?
                .fields
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.ty.clone()),
            Value::Dist(d) => {
                let first = self.record_field_type(&d.outcomes.first()?.0, name)?;
                d.outcomes
                    .iter()
                    .all(|(v, _)| self.record_field_type(v, name).as_ref() == Some(&first))
                    .then_some(first)
            }
            _ => None,
        }
    }

    fn check_updated_records(&mut self, value: Value, span: Span) -> Result<Value> {
        match value {
            Value::Record(ref r) => {
                let Some(id) = self
                    .prog
                    .records
                    .iter()
                    .position(|t| Some(t.name.as_str()) == r.ty.as_deref())
                else {
                    return Ok(value);
                };
                let ty = TypeSpec::Record(id as u32);
                let value = self.coerce(value, &ty, span)?;
                if !self.conforms(&value, &ty) {
                    return Err(RuntimeError::new(
                        span,
                        format!("updated record doesn't conform to {}", ty.describe(self.prog)),
                    ));
                }
                Ok(value)
            }
            Value::Dist(d) => {
                let mut outcomes = Vec::with_capacity(d.outcomes.len());
                for (v, p) in &d.outcomes {
                    outcomes.push((self.check_updated_records(v.clone(), span)?, *p));
                }
                ops::combine(outcomes, d.missing, &mut self.budget).map_err(|e| e.at(span))
            }
            v => Ok(v),
        }
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
                    if analytic::contains(&key) {
                        return Err(analytic::unsupported("analytic map keys").at(k.span));
                    }
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
                    let field_ty = ty
                        .and_then(|t| self.prog.records[t as usize].fields.iter().find(|d| d.name == *name))
                        .map(|d| d.ty.clone());
                    let value = match field_ty {
                        Some(t) => {
                            let value = self.eval(f, v, w)?;
                            let value = self.coerce(value, &t, v.span)?;
                            if !self.conforms(&value, &t) {
                                return Err(RuntimeError::new(
                                    v.span,
                                    format!(
                                        "field `{name}` should be {}, found {}",
                                        t.describe(self.prog),
                                        value.kind()
                                    ),
                                ));
                            }
                            value
                        }
                        None => self.eval(f, v, w)?,
                    };
                    values.push((Arc::from(name.as_str()), value));
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
                    let field_ty = self.record_field_type(&base, name);
                    let value = match field_ty {
                        Some(ty) => {
                            let value = self.eval_expected(f, v, w, &ty)?;
                            if !self.conforms(&value, &ty) {
                                return Err(RuntimeError::new(
                                    v.span,
                                    format!(
                                        "field `{name}` should be {}, found {}",
                                        ty.describe(self.prog),
                                        value.kind()
                                    ),
                                ));
                            }
                            value
                        }
                        None => self.eval(f, v, w)?,
                    };
                    updates.push((Arc::from(name.as_str()), value));
                }
                let value = ops::lift1(&base, &mut self.budget, |b, _| ops::with_fields(b, &updates)).map_err(at)?;
                self.check_updated_records(value, span)
            }
            ExprKind::Builtin { func, args, named } => self.builtin(f, *func, args, named, w, span),
            ExprKind::Closure { func, capture_args } => Ok(Value::Closure(Arc::new(Closure {
                func: *func,
                captured: capture_args
                    .iter()
                    .map(|&s| self.slot(f, s, w, span))
                    .collect::<Result<_>>()?,
            }))),
            ExprKind::Simulate { func, capture_args } => {
                let key = capture_args
                    .iter()
                    .map(|&s| self.slot(f, s, w, span))
                    .collect::<Result<_>>()?;
                self.simulate(*func, key, span)
            }
            ExprKind::Interp(parts) => {
                let mut text = String::new();
                for part in parts {
                    match part {
                        InterpPart::Lit(s) => crate::text::push(&mut text, s, &mut self.budget).map_err(at)?,
                        InterpPart::Expr(e) => {
                            let value = self.eval(f, e, w)?;
                            crate::text::push_value(&mut text, &value, &mut self.budget).map_err(at)?;
                        }
                    }
                }
                Ok(Value::str(&text))
            }
            ExprKind::Input(i) => Ok(self.inputs[*i as usize].clone()),
        }
    }

    fn literal(&mut self, l: &Lit) -> OpResult<Value> {
        Ok(match l {
            Lit::Builtin(b) => Value::Builtin(*b),
            Lit::Unit => Value::Unit,
            Lit::Bool(b) => Value::Bool(*b),
            Lit::Int(i) => {
                self.budget.integer_bits(i.bits())?;
                self.budget.work(i.bits().div_ceil(64).max(1))?;
                Value::Int(i.clone())
            }
            Lit::Float(x) | Lit::FloatConstant(x) => Value::Float(*x),
            Lit::Prob(p) => Value::Prob(*p),
            Lit::Str(s) => crate::text::value(s, &mut self.budget)?,
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

    fn builtin(
        &mut self,
        f: FnId,
        b: Builtin,
        args: &[Expr],
        named: &[(String, Expr)],
        w: &World,
        span: Span,
    ) -> Result<Value> {
        if b == Builtin::Typeof {
            // A direct inspection knows a delayed variable's outcome type
            // without drawing it and losing subsequent conjugate updates.
            let v = match args[0].kind {
                ExprKind::Slot(s) if matches!(w.slots[s as usize], Value::Delayed(_)) => w.slots[s as usize].clone(),
                _ => self.eval(f, &args[0], w)?,
            };
            return crate::type_name::of(&v, &self.prog.enums, &mut self.budget).map_err(|e| e.at(span));
        }
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.eval(f, a, w)?);
        }
        let mut names = Vec::with_capacity(named.len());
        for (name, value) in named {
            names.push(name.clone());
            values.push(self.eval(f, value, w)?);
        }
        self.builtin_values(b, &values, &names, w.weight, span)
    }

    fn check_function_arity(&self, value: &Value, given: usize, span: Span) -> Result<()> {
        match value {
            Value::Closure(c) => self.check_arity(c, given, span),
            Value::Builtin(b) => {
                let (min, max) = b.arity();
                if given < min || given > max || (*b == Builtin::Date && given == 2) {
                    return Err(RuntimeError::new(
                        span,
                        format!(
                            "`{}` takes {}, but {given} were given",
                            b.name(),
                            if *b == Builtin::Date {
                                "one ISO string or three integers".into()
                            } else if min == max {
                                format!("{min} argument(s)")
                            } else if max == usize::MAX {
                                format!("at least {min} arguments")
                            } else {
                                format!("{min} to {max} arguments")
                            }
                        ),
                    ));
                }
                Ok(())
            }
            _ => Err(RuntimeError::new(
                span,
                "expected a comparator function, like `(a, b) -> a - b`",
            )),
        }
    }

    /// Shared dispatch for direct calls and first-class builtin values.
    fn builtin_values(
        &mut self,
        b: Builtin,
        values: &[Value],
        named: &[String],
        weight: Weight,
        span: Span,
    ) -> Result<Value> {
        let extrema = matches!(b, Builtin::Minimum | Builtin::Maximum);
        if !named.is_empty() && (!extrema || named != ["default"]) {
            return Err(RuntimeError::new(
                span,
                format!(
                    "`{}` does not accept these named arguments; only minimum/maximum support `default`",
                    b.name()
                ),
            ));
        }
        let positional = values.len() - named.len();
        if !named.is_empty() && positional == 0 {
            return Err(RuntimeError::new(
                span,
                "a collection argument is required before `default`",
            ));
        }
        if !named.is_empty() && positional >= 3 {
            return Err(RuntimeError::new(
                span,
                "the default was supplied both positionally and by name",
            ));
        }
        self.check_function_arity(&Value::Builtin(b), values.len(), span)?;
        let (values, default) = if extrema && (!named.is_empty() || values.len() == 3) {
            (&values[..values.len() - 1], values.last())
        } else {
            (values, None)
        };
        let at = |err: OpError| err.at(span);
        builtins::check_query_input(b, values).map_err(at)?;
        if values.iter().any(analytic::contains)
            && !matches!(
                b,
                Builtin::Map
                    | Builtin::Filter
                    | Builtin::Reduce
                    | Builtin::Len
                    | Builtin::IterItems
                    | Builtin::Settled
                    | Builtin::BooleanLaw
            )
        {
            return Err(analytic::unsupported(&format!("`{}` on this outcome", b.name())).at(span));
        }
        match b {
            Builtin::Typeof => crate::type_name::of(&values[0], &self.prog.enums, &mut self.budget).map_err(at),
            Builtin::RunDate => self.config.today.map(Value::Date).ok_or_else(|| {
                RuntimeError::new(span, "the host did not supply an execution date for `today`")
                    .with_help("set Options.today, or supply today in the WASM request")
            }),
            Builtin::Print => {
                let mut text = String::new();
                for (i, value) in values.iter().enumerate() {
                    if i != 0 {
                        crate::text::push(&mut text, " ", &mut self.budget).map_err(at)?;
                    }
                    crate::text::push_value(&mut text, value, &mut self.budget).map_err(at)?;
                }
                let line = if weight == Weight::ONE {
                    text
                } else {
                    format!("[{}] {text}", fmt_weight(weight))
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
            Builtin::Map | Builtin::Filter | Builtin::Reduce => self.higher_order(b, values, span),
            Builtin::Count if values.len() == 2 => self.higher_order(b, values, span),
            Builtin::Minimum | Builtin::Maximum => {
                if values.len() == 2 {
                    self.collection_extreme(b, values, default, span)
                } else {
                    builtins::population_extreme(&values[0], b == Builtin::Maximum, default, &mut self.budget)
                        .map_err(at)
                }
            }
            Builtin::Sort | Builtin::SortDesc if values.len() == 2 => self.higher_order(b, values, span),
            Builtin::Highest | Builtin::Lowest if values.len() == 3 => self.higher_order(b, values, span),
            Builtin::Min | Builtin::Max => builtins::call_plain(b, values, &mut self.budget).map_err(at),
            Builtin::Roll => self.roll(values).map_err(at),
            Builtin::Take => Err(RuntimeError::new(
                span,
                "use `deck.take()` to draw and remove an item from a mutable bag",
            )),
            _ if b.lifting() == Lifting::Raw => builtins::call_raw(b, values, &mut self.budget).map_err(at),
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
            _ => ops::lift_n(values, &mut self.budget, &|a, budget| {
                builtins::call_plain(b, a, budget)
            })
            .map_err(at),
        }
    }

    fn roll(&mut self, values: &[Value]) -> OpResult<Value> {
        if values[0].is_uncertain() {
            return Err(OpError::new("the number of dice to roll must be a plain number")
                .help("draw it first, like `let n ~ d4`"));
        }
        let count = ops::integer(&values[0], "roll count", &mut self.budget)?;
        let count = count
            .to_u64()
            .filter(|n| *n <= 1000)
            .ok_or_else(|| OpError::new("roll needs between 0 and 1000 dice"))? as u32;
        let die = match &values[1] {
            v @ (Value::Int(_) | Value::Float(_)) => {
                let sides = ops::integer(v, "roll sides", &mut self.budget)?;
                let sides = sides
                    .to_u64()
                    .filter(|n| *n >= 1 && *n <= u32::MAX as u64)
                    .ok_or_else(|| OpError::new("roll needs between 1 and 4294967295 sides"))?;
                Dist::dice(1, sides as u32, &mut self.budget)?
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

    fn compare_callback(
        &mut self,
        comparator: &Value,
        a: &Value,
        other: &Value,
        b: Builtin,
        span: Span,
    ) -> Result<std::cmp::Ordering> {
        self.budget.work(1).map_err(|e| e.at(span))?;
        match self.call_pure(comparator, vec![a.clone(), other.clone()], b.name(), span)? {
            Value::Int(n) => Ok(n.cmp(&probl_number::Integer::ZERO)),
            Value::Float(x) if x.is_finite() => Ok(x.partial_cmp(&0.0).expect("finite comparator result")),
            other => Err(RuntimeError::new(
                span,
                format!(
                    "the comparator given to `{}` must return a finite int or float (negative, zero, or positive), found {}",
                    b.name(),
                    ops::article(&other.kind())
                ),
            )),
        }
    }

    fn collection_extreme(
        &mut self,
        b: Builtin,
        values: &[Value],
        default: Option<&Value>,
        span: Span,
    ) -> Result<Value> {
        if matches!(values[0], Value::Dist(_) | Value::Continuous(_)) {
            return Err(RuntimeError::new(
                span,
                "a comparator is supported only for collection extrema, not distribution support bounds",
            ));
        }
        let comparator = &values[1];
        self.check_function_arity(comparator, 2, span)?;
        let items = builtins::items(&values[0], b.name(), &mut self.budget).map_err(|e| e.at(span))?;
        let mut iter = items.into_iter();
        let Some(mut best) = iter.next() else {
            return builtins::empty_extreme(b.name(), default).map_err(|e| e.at(span));
        };
        for x in iter {
            let order = self.compare_callback(comparator, &x, &best, b, span)?;
            if (b == Builtin::Maximum && order.is_gt()) || (b == Builtin::Minimum && order.is_lt()) {
                best = x;
            }
        }
        Ok(best)
    }

    /// Collection operations which call a function, including custom sorting.
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
        if matches!(b, Builtin::Highest | Builtin::Lowest) {
            if let Value::Dist(d) = &values[1] {
                let mut results = Vec::with_capacity(d.outcomes.len());
                for (count, p) in &d.outcomes {
                    let mut args = values.to_vec();
                    args[1] = count.clone();
                    results.push((self.higher_order(b, &args, span)?, *p));
                }
                return ops::combine(results, d.missing, &mut self.budget).map_err(|e| e.at(span));
            }
        }
        let items = builtins::items(&values[0], b.name(), &mut self.budget).map_err(|e| e.at(span))?;
        match b {
            Builtin::Sort | Builtin::SortDesc | Builtin::Highest | Builtin::Lowest => {
                let comparator = values.last().unwrap();
                // Validate even empty/singleton collections without invoking the callback.
                self.check_function_arity(comparator, 2, span)?;
                let count = if matches!(b, Builtin::Highest | Builtin::Lowest) {
                    Some(builtins::extreme_count(&values[1], &mut self.budget).map_err(|e| e.at(span))?)
                } else {
                    None
                };
                crate::ordering::reserve_sort(items.len(), &mut self.budget).map_err(|e| e.at(span))?;
                let mut items = items;
                crate::ordering::try_sort_by(&mut items, |a, other| {
                    let order = self.compare_callback(comparator, a, other, b, span)?;
                    Ok(if matches!(b, Builtin::SortDesc | Builtin::Highest) {
                        order.reverse()
                    } else {
                        order
                    })
                })?;
                if let Some(n) = count {
                    items.truncate(n);
                }
                Ok(Value::list(items))
            }
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
                    Value::Int((kept.len() as i64).into())
                } else {
                    Value::list(kept)
                })
            }
            Builtin::Reduce => {
                let callback = &values[1];
                if !matches!(callback, Value::Closure(_) | Value::Builtin(_)) {
                    return Err(
                        RuntimeError::new(span, "`reduce` needs a function as its second argument")
                            .with_help("write `reduce(xs, f)` or `reduce(xs, f, initial)`"),
                    );
                }
                self.check_function_arity(callback, 2, span)?;
                let mut items = items.into_iter();
                let mut acc = match values.get(2) {
                    Some(initial) => initial.clone(),
                    None => items.next().ok_or_else(|| {
                        RuntimeError::new(span, "`reduce` of an empty collection needs an initial value")
                            .with_help("supply an initial value: `reduce(xs, f, initial)`")
                    })?,
                };
                for x in items {
                    acc = self.call_pure(callback, vec![acc, x], "reduce", span)?;
                }
                Ok(acc)
            }
            _ => unreachable!(),
        }
    }
}

/// A call as written: `f(3, true)`.
fn describe_call(fun: &Function, key: &[Value]) -> String {
    let args: Vec<String> = key[..fun.n_params as usize].iter().map(|v| format!("{v:?}")).collect();
    format!("{}({})", fun.name, args.join(", "))
}

/// The error when `print` at `span` goes over the output limit.
/// A world faults to a catch only inside a `try` of its own call, which takes
/// it: none can leave the call that way.
fn uncaught(flow: &Flow) -> Result<()> {
    match flow.faulted.first() {
        Some((_, e)) => Err(OpError::internal("a fault went past the `try` meant to catch it").at(e.span)),
        None => Ok(()),
    }
}

/// Remove the worlds at the indices `failed`, which are in increasing order.
fn drop_failed(worlds: &mut Vec<World>, failed: &[usize]) {
    if failed.is_empty() {
        return;
    }
    let mut failed = failed.iter().copied().peekable();
    let mut i = 0;
    worlds.retain(|_| {
        let drop = failed.next_if_eq(&i).is_some();
        i += 1;
        !drop
    });
}

pub fn too_much_output(span: Span) -> RuntimeError {
    RuntimeError::limit(span, "the program printed more than the output limit")
}

/// A weight as a percentage, even when it's too small for an `f64`.
pub fn fmt_weight(w: Weight) -> String {
    let x = w.to_f64();
    if x == 0.0 && !w.is_zero() {
        let l = w.log10() + 2.0;
        return format!("{:.1}e{}%", libm::pow(10.0, l - l.floor()), l.floor());
    }
    fmt_prob(x)
}

enum PathKey<'a> {
    Field(&'a str),
    Index(Value),
}

fn update(target: &mut Value, keys: &[PathKey], v: Value, budget: &mut Budget) -> OpResult<()> {
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
            update(slot, rest, v, budget)
        }
        (PathKey::Index(i), Value::List(items)) => {
            let items = Arc::make_mut(items);
            let k = ops::as_index(i, items.len() as u128, budget)? as usize;
            update(&mut items[k], rest, v, budget)
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
            update(slot, rest, v, budget)
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
