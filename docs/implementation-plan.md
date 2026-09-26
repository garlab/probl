# Probl: implementation plan

> Draft 0.2, September 2026, revised after the [project audit](project-audit.md). Companion to the [language overview](language-overview.md), which describes what is being built, and the [reference semantics](semantics.md), which defines it precisely. This document covers how, in what order, and what could go wrong.

## 0. Status

The engine enumerates and samples. All nine examples run: 01–06 print their documented output exactly, as checked against independent calculations, and the sampled 07–09 agree with an independent reference simulation within their sampling error. The audit of commit `a37c0b6` found that several of the language's promises weren't well defined, and that the tests couldn't catch it. Its first four recommended steps are done:

1. **Reference semantics.** [docs/semantics.md](semantics.md) is normative for the engine: types and event identity, evaluation order, calls versus `simulate`, evidence, reports, termination and approximation, and resource limits.
2. **Lowering and the numerical contract.** Operands are evaluated left to right, effects decide what can be memoized, weights can't underflow, and fractions are labelled as approximations.
3. **Budgets, crashes and an independent oracle.** The host sets limits that a program can only lower, panics become internal errors, and a second, deliberately naive interpreter checks the engine on generated programs.
4. **Basic sampling, with verified estimates and uncertainty.** The estimators were specified first (semantics §14): independent runs weighted by the evidence, self-normalized estimates with delta-method standard errors, and the effective sample size. Continuous distributions came with them (§13). Sampling is checked against enumeration on generated programs, and its standard errors are calibrated. Merged runs, particles, beam search and nested estimates wait for their own contracts.

What's left of the audit's order is step 5: benchmarks of realistic models, to choose what to build next.

How each finding was answered:

| Finding | What changed | Where |
|---|---|---|
| D1: events, probabilities and distributions collapse into one another | `bool` (facts), `prob` (parameters) and `dist[T]` are distinct. `and`/`or` take facts, and at most one uncertain fact. `bernoulli(p)` makes an event. `simulate` keeps distributions of probabilities. `match` needs a settled value. Type annotations are checked. | semantics §1–3; `ops.rs`, `lower.rs` |
| D2: dropped prior weight doesn't bound a posterior | `for` and `repeat` are never cut short. `while` and `loop` stop at ε *relative to the weight that entered*. Event probabilities are printed as the range [a/(Z+U), (a+U)/(Z+U)] when unresolved weight U is visible. Means claim no bound. | semantics §10; `interp.rs`, `report.rs` |
| D3: evidence and report scopes | Evidence is the program's; observations inside `simulate` are local. Impossible evidence is an error. An `observe` that can run after a `report` is a compile error. Reports show their reach, and reports that can count a world twice are labelled "per visit". | semantics §7–9; `effects.rs` |
| D4: sampling needs estimator contracts | Specified before building (semantics §14), and only the simplest estimator is built: independent runs that never merge, likelihood weighting, standard errors on every printed probability, and the effective sample size. `simulate` is enumerated even when sampling, so no estimate hides inside another. Particles, beam search and merged runs stay unspecified and unbuilt. | semantics §14; `interp.rs`, `report.rs` |
| D5: merging isn't a general answer to state explosion | The docs no longer claim it is; loops are unrolled, not solved. Solving finite-state loops as Markov chains is on the list for phase 7. | overview §11 |
| D6: `a to b` hides assumptions | `a to b` is a lognormal with positive ends, as in Squiggle; `normal_range(lo, hi)` says "normal" explicitly. `mean`, `quantile`, `cdf` and the other queries expose each distribution's assumptions. | semantics §13; `continuous.rs` |
| I1: "exact" fractions | The mode is called *enumeration*. `--fractions` prints "≈ 244/495". Weights have an extended exponent, so repeated observations can't underflow. Missing mass composes, and distributions keep their probabilities adding up to 1 − missing. | semantics §10; `weight.rs`, `dist.rs` |
| I2: evaluation order and effects | Lowering saves earlier operands in temporaries when a later operand assigns variables. An effect analysis finds functions that print (never memoized) or observe. | semantics §4 and §6; `lower.rs`, `effects.rs` |
| I3: limits and crashes | Host limits on worlds, outcomes, work, iterations, call depth, cache, collections, output and stack; cancellation and `--timeout`. The parser limits nesting and source size. A panic in the engine comes back as an internal error. | semantics §11; `lib.rs`, `parser.rs` |
| I4: no independent check | `probl-oracle`: an exact, path-by-path interpreter of the AST, a program generator and a fuzzer. It found four engine bugs, all fixed (§5). | semantics §12; `crates/probl-oracle` |

The implementation differs from the original plan in a few places, each deliberately:

- **The lexer is written by hand**, not generated with `logos`: percentages, dice and interpolated strings were easier to get right directly.
- **Distributions live in the engine crate** (`dist.rs` and `continuous.rs`), not a separate `probl-dist`.
- **Random numbers and samplers are written by hand** (xoshiro256++, Box–Muller, Marsaglia–Tsang, inversion), with `libm` for the special functions, instead of `rand` and `statrs`: a seed must give the same numbers on every platform and in every version of the dependencies. They're tested against their distributions with Kolmogorov–Smirnov and chi-square tests.
- **Weights are `f64` with an extended exponent** (`Weight`), not a generic parameter. Exact rational weights were dropped: they would have made `--fractions` a proof only for programs without loops or observations of floats, which is too narrow to promise (audit I1). If exact answers come back, it will be for an explicitly supported subset.
- **Collections are `Arc`-shared and copied on write**, not `im` persistent structures. That's enough for the examples; persistent collections wait for a benchmark that needs them.
- **The engine thread's stack is 64 MiB**, with a call-depth limit of 500. Replacing recursion in the interpreter with an explicit stack waits for a model that needs deeper calls.

## 1. Scope

**Goals for the 0.x releases**

- Answers for discrete models (dice, cards, boards) computed over every possibility, fast enough to explore interactively.
- Sampling for everything else (continuous estimates, huge state spaces), with honest error bars.
- One semantics and several engines: switching mode changes speed and accuracy, never the model.
- Error messages that teach the model: *"`d6` is a distribution; did you mean `let n ~ d6`?"*
- Reproducible runs: the same program, seed and version give the same output on any machine and with any number of threads.
- Safe to run untrusted models inside a host's limits.

**Out of scope for now:** gradient-based inference (HMC, variational inference), GPUs, general I/O (files, network), concurrency, a foreign-function interface and a package manager.

## 2. Technology

**Rust**, for the reasons that still hold:

- The engine does a lot of hashing, copying and arithmetic for every world, so native speed and control over memory layout matter.
- It compiles to WebAssembly, which makes a browser playground realistic.
- In use: `ariadne` (diagnostics), `clap`, `rustc-hash`, `libm` (special functions), `insta` (snapshot tests), `libtest-mimic` (golden tests), and `num-rational` in the oracle only. Planned: `rayon`, `rustyline`, `criterion`, `tower-lsp`, and later `pyo3`.

## 3. Architecture

### 3.1 Pipeline

```
source ─► lexer ─► parser ─► AST ─► lowering ─► IR ─► analyses ─► engine ─► reports ─► text
          by hand   Pratt +         scopes, slots,     liveness,    world-set
                    recursive       left-to-right      effects      interpreter
                    descent         operands, checks
```

### 3.2 Repository layout

```
Cargo.toml              workspace
crates/
  probl-syntax/         lexer, parser, AST, spans, diagnostics
  probl-sema/           name resolution, lowering to IR, liveness and effect analysis, checks
  probl-engine/         values, distributions, weights, worlds, the interpreter, reports, limits
  probl-cli/            probl run | check | repl; golden tests over examples/
  probl-oracle/         the independent reference interpreter, program generator and fuzzer
examples/               sample programs; their "Output" blocks are golden tests
docs/
```

### 3.3 Values and worlds

```rust
enum Value {
    Dead, Unit, Bool(bool), Int(i64), Float(f64), Prob(f64), Str(Arc<str>), Date(i32),
    List(Arc<Vec<Value>>), Map(Arc<BTreeMap<Value, Value>>), Bag(Arc<BTreeMap<Value, u64>>),
    Range(i64, i64), Record(Arc<Record>), Enum(Arc<EnumValue>), Dist(Arc<Dist>), Closure(Arc<Closure>),
}

struct World {
    slots: Arc<[Value]>,         // variables of the current frame, copied on write
    weight: Weight,              // f64 mantissa and i64 exponent: can't underflow
}
```

- Value semantics plus shared, copy-on-write collections make forking a world a reference-count increment.
- `Hash` and `Eq` are structural, so an `int` 1 and a `float` 1.0 are different values; the language's `==` compares numbers across types. Floats compare by bit pattern, with −0.0 and NaN normalized.
- `Bool` (facts), `Prob` (probabilities) and `Dist` are separate variants, as the semantics requires, and reports print them differently.
- A distribution's probabilities add up to exactly 1 − missing: constructing one rescales away the rounding of the sums that produced it.

### 3.4 The world-set interpreter

The engine is a tree-walking interpreter in which **every statement maps a set of worlds to a set of worlds**. All worlds in a set are at the same point in the program, so control flow is shared and only the data differs.

```rust
/// Worlds leaving a statement, grouped by how they left it.
struct Flow {
    next: Vec<World>,               // fell through to the next statement
    broke: Vec<World>,              // `break`
    continued: Vec<World>,          // `continue`
    returned: Vec<(World, Value)>,  // `return`
}

fn exec_loop(&mut self, body: &Block, bounded: bool, worlds: Vec<World>) -> Result<Flow> {
    let cutoff = total_weight(&worlds).scale(self.epsilon);   // relative to what entered
    let mut inside = worlds;
    let mut out = Flow::default();
    while !inside.is_empty() {
        let mass = total_weight(&inside);
        if !bounded && mass < cutoff {                         // `for` and `repeat` never stop early
            self.unresolved = self.unresolved.add(mass);       // unresolved, not dropped
            break;
        }
        let flow = self.exec_block(body, inside)?;
        out.next.extend(flow.broke);
        out.returned.extend(flow.returned);
        inside = self.merge(chain(flow.next, flow.continued), &self.live_at_head[s.id]);
    }
    Ok(out)
}
```

The interpreter follows these rules:

- **Expressions never split.** Lowering moves every construct that can split into a statement of its own (A-normal form). When an operand's statements assign a variable, the operands before it are saved in temporaries first, so evaluation is left to right (semantics §4).
- **Join points merge.** After `if`, `chance` and `match`, at the head of every loop iteration and at function return, worlds are grouped by their live slots, and their weights are summed.
- **`break`, `continue` and `return`** are just other outputs of `Flow`.
- **Every step is budgeted.** World counts, outcomes, collection sizes and work are checked before the memory is allocated, and the cancellation flag is polled as work is counted.

### 3.5 Policies: one interpreter, several engines

The original design had execution modes differ only in how worlds split and what happens after a join:

| Policy | `split(w, p)` | `draw(w, D)` | `after_join` |
|---|---|---|---|
| Enumerate | two worlds, with weights w·p and w·(1 − p) | one world per outcome; infinite supports are cut at 10⁻¹⁸ | nothing |
| Beam(k) | as Enumerate | as Enumerate | keep the k heaviest worlds |
| Sample(n) | the world's r runs split into Binomial(r, p) and the rest | the r runs spread over sampled outcomes | nothing |
| Particles(n) | as Sample | as Sample | resample when the effective sample size falls below n/2 |

The audit (D4) showed this interface isn't enough. Merged samples need statistical bookkeeping beyond a run count; a `simulate` estimated inside a decision carries its own error; resampling needs a precise synchronization point; and dropped prior weight doesn't bound a posterior, so Beam needs its own accuracy contract.

What exists is the simplest sampler, specified first (semantics §14). A batch of 1,000 runs is a set of worlds that never merge; at every split, each run takes one branch at random and keeps its weight, which only observations change. Each world carries its run's number, so that reports can add up what each run reported and compute delta-method standard errors, and the effective sample size comes from the runs' final weights. `simulate` switches back to enumeration for its block, so estimates never nest. When sampling, `x ~ binomial(…)`, `poisson(…)` or `geometric(…)` is drawn directly, by inversion outward from the mode, instead of listing the distribution's outcomes, and `observe … from` them uses their formulas. Merged runs, particles and beam search will each need their contract in the semantics first; an *inference context* that owns observations, budgets, randomness and result metadata is the likely shape.

### 3.6 Function calls

- **Enumerate:** a call is a sub-simulation that returns an unnormalized distribution of results (semantics §6). The engine runs the body once per distinct combination of arguments and top-level values read, and reuses the result in every world: dynamic programming without the programmer asking for it. This is sound because functions can't assign outside variables, and it's only done for functions that don't print (the effect analysis in `effects.rs` follows calls). A call that re-enters itself with identical arguments is an error for now; solving such recursions is phase 7 work (D5).
- **Sample:** the callee runs with the caller's run, taking its own random path; its result's weight is the observations it made. Nothing is memoized, and a call may return to itself with the same arguments.

### 3.7 Liveness and merging

A standard backward liveness analysis on the IR gives, for every join point, the slots that may still be read. Dead slots are overwritten with `Value::Dead`, so worlds that differ only in stale variables become identical. In Risk, the dice pools die at the end of each round, and 56 × 21 dice outcomes collapse back to a few (attackers, defenders) pairs. In snakes and ladders, `roll` dies at once, so a game is a walk over at most 100 squares.

Merging helps only when histories stop mattering. Two refinements come later: hoisting values that are the same in every world out of the worlds, and hash-consing large values.

### 3.8 Distributions

Finite distributions are sorted `(value, probability)` pairs plus a *missing mass*: the probability of outcomes too unlikely to keep (below 10⁻¹⁸, for `poisson` and `geometric`). Operators apply to every combination of outcomes and combine missing mass as for independent draws. Options that are distributions are mixed in, so a distribution never has distributions as outcomes.

Continuous distributions (`continuous.rs`) are their family and parameters: density, CDF, quantiles and moments come from the formulas (with the incomplete beta and gamma functions, and `libm` for `erfc` and `lgamma`), and draws from exact samplers. Comparing one with a number gives a distribution of facts through the CDF, in both modes; a finite distribution whose outcomes include continuous ones is a mixture. Arithmetic on them waits for a representation of the result (closed forms, or empirical distributions).

### 3.9 Reports and output

Each `report` site owns an accumulator for each value of its `by` key: the weight that reached it, the weight of each value, and the missing mass of reported distributions. When sampling, it also keeps what each run of the current batch reported, and folds that into running sums at the end of the batch, for the standard errors; so that part doesn't grow with the number of runs. The distinct reported values do: exact quantiles need them all, and a report of a continuous quantity keeps one per run. A quantile sketch (such as a t-digest) will bound that. Rendering adds the reach ("reached in 1.00% of worlds"), the "per visit" label, ranges when unresolved weight is visible, and standard errors when sampling (semantics §9–10, §14). Output is text; `--format json` and HTML come in phase 5.

### 3.10 Randomness and reproducibility

Random numbers come from one xoshiro256++ stream, seeded through SplitMix64, used by the batches in order; the batch size is fixed at 1,000. The samplers use `libm`, not the platform's math library. So a program and a seed give the same output on every machine, whatever the engine's other settings.

Running batches in parallel will need a stream per batch, derived from `(seed, batch index)`, combined in batch order so that the output doesn't depend on the number of threads. That changes the numbers a seed gives, which is fine between versions.

### 3.11 Errors

Every problem is a diagnostic with a source span: a language error, an unsupported feature, a limit (including cancellation), or an internal error. The engine runs on its own thread and catches panics, so a crash is reported as a bug in Probl instead of taking the host down; the CLI exits with status 70 for those. Out-of-memory and stack overflow can't be caught this way, which is what the limits are for; a hosted playground should also run models in a separate, cancellable worker.

Planned for phase 5: a runtime error that reports the probability of the worlds that hit it and one path to it: *"index 7 out of range in 2.3% of worlds; for example: come_out = 4 → r = 9 → r = 12 → …"*.

## 4. Phases

The effort ranges assume one developer working full time, and they are 90% confidence ranges. [`examples/09_roadmap.probl`](../examples/09_roadmap.probl) turns them into dates.

### Phases 0–3: groundwork, front end, enumeration, discrete distributions (done)

- Workspace, golden tests over `examples/` with an explicit list of examples waiting for features.
- Lexer, parser, diagnostics, lowering, liveness, the world-set interpreter, merging, memoized calls.
- Finite distributions, dice pools, bags and `take`, `binomial`, `poisson`, `geometric`, queries, `simulate`, `observe`, reports.
- **Exit reached:** examples 01 to 06 reproduce their documented output.

### Phase 3½: the audit (done)

- The reference semantics, and the engine brought in line with it (§0).
- Host limits, cancellation, no panics.
- The oracle, the program generator and fuzzing; regression tests for every confirmed probe.
- **Exit reached:** the oracle and the engine agree on 50,000 generated programs (36,356 compared value by value, 12,192 rejected by both, the rest too big for the oracle), and 30,000 fuzzed programs never crash.

### Phase 4: sampling and continuous distributions, release v0.2 "Forecasts" (mostly done)

- Done: the estimators, specified first (semantics §14): independent runs, likelihood weighting, standard errors, the effective sample size; `simulate` enumerated inside runs.
- Done: independent sampling, checked against enumeration on generated programs with fixed seeds, including the calibration of its standard errors.
- Done: continuous families with pdf, cdf, quantile and sampling; `a to b`, `normal_range`, `pert`, `triangular`; `observe … from` with densities; `--mode`, `--runs` and `--seed`.
- **Exit reached:** examples 07 to 09 match their reference outputs within tolerance, and every sampler passes its statistical tests.
- Left before the release: parallel batches with reproducible seeding; an estimate of the evidence (log-evidence with densities).
- Only if benchmarks call for them, each with its contract first: arithmetic on continuous distributions, merged sampling, particles, beam, nested estimates, and an `auto` mode that says what it chose.

### Phase 5: inspection (2–4 weeks)

- Per-world traces, and `probl explain`, which shows the most likely path to an outcome (*"how does the attacker usually lose?"*).
- Runtime errors that report the probability of the failing worlds and an example path.
- `--format json`, an HTML report with charts, and a REPL prompt that shows the world count.
- Support sizes, merge ratios, and which variables keep worlds apart (D5).

### Phase 6: types and tooling (5–8 weeks), release v0.3

- A static type checker with local inference, growing out of today's checks; errors that tell a distribution from a value, with suggested fixes.
- `probl fmt`, a language server, and a VS Code extension.
- **Exit:** release **v0.3**.

### Phase 7: reach (open-ended), v0.4 and later

- Benchmarks of realistic game and forecast models, to choose among the next items (audit, step 5).
- Solving finite-state loops and recursions as absorbing Markov chains, instead of unrolling them (D5).
- A WebAssembly build and a browser playground; Python bindings.
- Performance: a bytecode VM that runs each instruction over all worlds at once, hash-consing, persistent collections, a parallel enumerator.
- Research track: compiling to decision diagrams for exact inference (as the Dice language does), and reports conditioned on later evidence (smoothing).

### Forecast

The forecast made when the plan was first written, by running [`examples/09_roadmap.probl`](../examples/09_roadmap.probl) with three risks, 10–30% overhead and two weeks of holidays, starting on 28 September 2026:

| Milestone | 5% | Median | 95% |
|---|---|---|---|
| v0.1 Games | 2027-01-26 | 2027-02-18 | 2027-03-24 |
| v0.2 Forecasts | 2027-03-07 | 2027-04-05 | 2027-05-13 |
| v0.3 Types and tooling | 2027-05-17 | 2027-06-21 | 2027-08-05 |

## 5. Testing

`cargo test --all` runs all of these:

- **Golden tests.** Each example's Output block is its expected output. An example that can't run yet must be on an explicit list; an unsupported example that isn't listed, or a listed one that now runs, fails the suite.
- **Known answers.** The 2d6 distribution, craps (244/495), Monty Hall, gambler's ruin, the audit's posterior (99.80%), a shared rate (41%), and more in `probl-engine/tests/semantics.rs`.
- **Audit regressions.** Every confirmed probe from the audit, as the semantics now defines it, in `probl-engine/tests/audit.rs`.
- **The oracle.** `probl-oracle` interprets the AST directly with exact fractions, following every path separately: no merging, no liveness, no memoization, operands left to right. Its tests generate programs (mutation inside expressions, calls with splits and evidence, recursion, aliases, repeated and impossible observations, loops that break and continue, `simulate`, distributions with one outcome) and require the same evidence and the same unnormalized weight for every reported value, or an error from both; impossible evidence must be reported by both. The engine runs with merging and memoization on and off. A hand-written corpus covers each rule of the semantics. `PROBL_ORACLE_CASES` runs more programs.

  It found four engine bugs: `one_of` kept distributions as outcomes, so a drawn value could still be a distribution; `chance` weights didn't accept facts and uncertain facts, as the semantics says they should; floating-point rounding left branches of weight 10⁻¹⁶ behind certain conditions, reported as unresolved weight or as evidence where there was none; and an operand that can fail or print (such as `simulate { … }` in `simulate { … } == f(x)`) ran after a later operand's call, not before it.
- **Fuzzing.** Mutated examples, mutated generated programs and random fragments go through parsing, compiling, rendering diagnostics and running under small limits. Nothing may panic, hang or return an internal error. `PROBL_FUZZ_CASES` runs more.
- **Snapshots.** The AST and the IR of every example.
- **Sampling against enumeration.** For generated programs, every estimate sampling prints (probabilities, means, reach) must be within six standard errors of the exact value from enumeration, and across all of them the standard errors must be calibrated: few estimates more than four standard errors off, and a mean squared z-score near 1. On 2,271 programs and 9,316 estimates it was 0.99. `PROBL_SAMPLING_CASES` runs more.
- **Samplers.** Kolmogorov–Smirnov tests for every continuous family, chi-square tests for the direct count samplers, and closed-form checks of CDFs, quantiles and densities. `probl-engine/tests/sampling.rs` covers the rules of semantics §13–14.
- **Sampled examples.** Examples 07–09 are compared with outputs from an independent reference simulation, token by token: estimates within five standard errors, other numbers within 4%, dates within three days. The comparator has its own test.
- **Planned:** `criterion` benchmarks tracking worlds per second, merge ratio and peak world count.

## 6. Performance targets

- Enumeration: examples 02 to 06 each finish in under one second on a laptop, and craps in under 50 ms. (Met.)
- Sample mode: example 07 (50,000 runs × 18 months) finishes in under two seconds on eight cores. (It takes 2.3 seconds on one core; parallel batches should meet the target.)
- If a target is missed by more than 10×, the vectorized VM from phase 7 moves earlier.

## 7. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Enumeration blows up on real models | high | high | Liveness-based merging; comparing distributions directly instead of drawing; limits that fail clearly; statistics on what keeps worlds apart; Markov-chain solving later |
| Users confuse `=` and `~` | high | medium | Errors with suggested fixes, `match` and `and`/`or` refusing distributions, documentation that leads with the rule |
| Sampling estimates are wrong in subtle ways | medium | high | Estimators specified before being built; independent runs only; every estimate tested against enumeration, and the standard errors' calibration too |
| Rounding in floating-point weights | medium | medium | An extended exponent (no underflow); distributions renormalized to 1 − missing; fractions marked ≈; the oracle's exact comparison |
| Likelihood weighting degenerates (low effective sample size) | high for Bayesian models | medium | Always print the effective sample size and warn when it's low; resample-move and conjugate updates later |
| Float-valued state never merges | medium | medium | Warn when float slots keep worlds apart; recommend integers or rounding |
| Untrusted models exhaust a host | medium | high | Host limits checked before allocation, cancellation, no panics; a separate worker process for hosted use. Sampled reports of continuous values still keep every distinct value: a quantile sketch is next |
| Scope creep (units, plotting, modules, …) | high | medium | Phase exit criteria; features no example needs wait |

## 8. Decisions

Settled by the audit:

| Decision | Chosen | Instead of |
|---|---|---|
| `and`/`or` on probabilities | facts only, at most one uncertain; `bernoulli(p)` makes an event | independent events (p·q) |
| Reports and evidence | an `observe` after a `report` is an error | snapshots of the evidence so far |
| Evaluation order | left to right, each operand once | unspecified |
| `a to b` | lognormal, positive ends only; `normal_range` for normal | normal when an end is negative |
| Name of the mode | enumeration | exact |

Still open:

| Decision | Proposal | Alternative |
|---|---|---|
| Block syntax | braces | significant indentation, as in Python |
| Functions and outside variables | read-only, which enables memoization | explicit `inout` parameters |
| Dice literals | `2d6` is syntax, so names like `d6` are reserved | only `dice(2, 6)` |
| Debug output from merged worlds | printed once per merged world | replayed once per path |

## 9. Next steps

1. Write benchmarks from realistic game and forecast models (the audit's step 5), before choosing between Markov-chain solving, a faster interpreter, parallel batches and persistent collections.
2. Finish v0.2: parallel batches with a stream per batch, and an estimate of the evidence.
3. Specify the next inference methods before building any (particles, beam search, nested estimates), with the audit's D4 checklist.
