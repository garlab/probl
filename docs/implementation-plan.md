# Probl: implementation plan

> Draft 0.2, September 2026, revised after the [project audit](project-audit.md). Companion to the [language overview](language-overview.md), which describes what is being built, and the [reference semantics](semantics.md), which defines it precisely. This document covers how, in what order, and what could go wrong.

## 0. Status

The engine enumerates and samples. All nine examples run: 01–06 print their documented output exactly, as checked against independent calculations, and the sampled 07–09 agree with an independent reference simulation within their sampling error. The audit of commit `a37c0b6` found that several of the language's promises weren't well defined, and that the tests couldn't catch it. All five of its recommended steps are done:

1. **Reference semantics.** [docs/semantics.md](semantics.md) is normative for the engine: types and event identity, evaluation order, calls versus `simulate`, evidence, reports, termination and approximation, and resource limits.
2. **Lowering and the numerical contract.** Operands are evaluated left to right, effects decide what can be memoized, weights can't underflow, and fractions are labelled as approximations.
3. **Budgets, crashes and an independent oracle.** The host sets limits that a program can only lower, panics become internal errors, and a second, deliberately naive interpreter checks the engine on generated programs.
4. **Basic sampling, with verified estimates and uncertainty.** The estimators were specified first (semantics §14): independent runs weighted by the evidence, self-normalized estimates with delta-method standard errors, and the effective sample size. Continuous distributions came with them (§13). Sampling is checked against enumeration on generated programs, and its standard errors are calibrated. Merged runs, particles, beam search and nested estimates wait for their own contracts.
5. **Benchmarks.** Twelve realistic game and forecast models, run with the examples by `probl-bench`, chose what comes next: [docs/benchmarks.md](benchmarks.md) has the measurements, profiles and reasons, and §9 the resulting order.

How each finding was answered:

| Finding | What changed | Where |
|---|---|---|
| D1: events, probabilities and distributions collapse into one another | `bool` (facts), `prob` (parameters) and `dist[T]` are distinct. `and`/`or` take facts, and at most one uncertain fact. `bernoulli(p)` makes an event. `simulate` keeps distributions of probabilities. `match` needs a settled value. Type annotations are checked. | semantics §1–3; `ops.rs`, `lower.rs` |
| D2: dropped prior weight doesn't bound a posterior | `for` and `repeat` are never cut short. `while` and `loop` stop at ε *relative to the weight that entered*. Event probabilities are printed as the range [a/(Z+U), (a+U)/(Z+U)] when unresolved weight U is visible. Means claim no bound. | semantics §10; `interp.rs`, `report.rs` |
| D3: evidence and report scopes | Evidence is the program's; observations inside `simulate` are local. Impossible evidence is an error. An `observe` that can run after a `report` is a compile error. Reports show their reach, and reports that can count a world twice are labelled "per visit". | semantics §7–9; `effects.rs` |
| D4: sampling needs estimator contracts | Specified before building (semantics §14), and only the simplest estimator is built: independent runs that never merge, likelihood weighting, standard errors on every printed probability, and the effective sample size. `simulate` is enumerated even when sampling, so no estimate hides inside another. Particles, beam search and merged runs stay unspecified and unbuilt. Exact updates for conjugate priors came later, after a [proposal and its review](inference-proposal.md); they keep runs independent. | semantics §14; `interp.rs`, `report.rs`, `conjugate.rs` |
| D5: merging isn't a general answer to state explosion | The docs no longer claim it is. A loop whose states come back is solved as an absorbing Markov chain; the others are unrolled. A call that comes back to itself is solved by iteration. | overview §11; semantics §6 and §10; `chain.rs`, `interp.rs` |
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
- **Collections are `Arc`-shared and copied on write**, not `im` persistent structures. No benchmark spends significant time copying collections (docs/benchmarks.md), so persistent collections wait for one that does.
- **The engine thread's stack is 64 MiB** (and so is each sampling thread's), with a call-depth limit of 500. Replacing recursion in the interpreter with an explicit stack waits for a model that needs deeper calls.

## 1. Scope

**Goals for the 0.x releases**

- Answers for discrete models (dice, cards, boards) computed over every possibility, fast enough to explore interactively.
- Sampling for everything else (continuous estimates, huge state spaces), with honest error bars.
- One semantics and several engines: switching mode changes speed and accuracy, never the model.
- Error messages that teach the model: *"`d6` is a distribution; did you mean `let n ~ d6`?"*
- Reproducible runs: the same program, seed and version give the same output on any machine and with any number of threads.
- Safe to run untrusted models inside a host's limits.

**Out of scope for now:** gradient-based inference (HMC, variational inference), GPUs, general I/O beyond reading data files, concurrency, a foreign-function interface and a package manager.

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
  probl-wasm/           the engine as a WebAssembly module, for the playground
web/                    the playground: the module's loader and its tests
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
- **Loops that cycle are solved** (semantics §10, `chain.rs`). While an unbounded loop runs, the engine hashes the states at its head. When one comes back, it stops unrolling:
  - It finds every state reachable from the worlds inside, and runs the body once from each.
  - It solves the resulting absorbing Markov chain one strongly connected group at a time, in the order weight flows. Within a group, it eliminates states as Grassmann, Taksar and Heyman do: only nonnegative numbers are added, so rare exits keep their precision.
  - The worlds that leave each state are scaled by its expected visits.
  - A state that can never be left is an error.
  - A chain with more states than the host allows, or too costly to eliminate, is unrolled after all.
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

- **Enumerate:** a call is a sub-simulation that returns an unnormalized distribution of results (semantics §6). The engine runs the body once per distinct combination of arguments and top-level values read, and reuses the result in every world: dynamic programming without the programmer asking for it. This is sound because functions can't assign outside variables, and it's only done for functions that don't print (the effect analysis in `effects.rs` follows calls). A call that comes back to itself with identical arguments is solved by iteration (semantics §6):
  - **Rounds.** The call it came back to reruns round after round. The call that comes back gets the previous round's result, which starts with nothing resolved.
  - **Stopping.** It stops when the weight still waiting on it is below ε.
  - **Which calls rerun.** Calls that wait on one further down the stack don't iterate on their own. What waits on them is counted as waiting on that call. Their results are reused until its next round, so a round runs each call of the cycle at most once.
  - **Settled results.** When it's solved, the results that are as settled become final, and are memoized.
- **Sample:** the callee runs with the caller's run, taking its own random path; its result's weight is the observations it made. Nothing is memoized, and a call may return to itself with the same arguments.

### 3.7 Liveness and merging

A standard backward liveness analysis on the IR gives, for every join point, the slots that may still be read. Worlds merge when they agree on those, so worlds that differ only in stale variables are the same world. In Risk, the dice pools die at the end of each round, and 56 × 21 dice outcomes collapse back to a few (attackers, defenders) pairs. In snakes and ladders, `roll` dies at once, so a game is a walk over at most 100 squares.

The analysis also gives, for every statement, the slots that die in it: read or written there, and never read again. They're cleared (set to `Value::Dead`) after the statement, which frees their values at once. Worlds that jump out with `break` or `continue` skip some of those statements, so they're cleared of every dead slot where they land.

Merging hashes the live slots of every world at every join point, although most collections haven't changed since the last one. So lists, maps, bags and records keep their hash once computed, and forget it when they change. Hashing a world costs a step per slot instead of one per element, and collections with different hashes are unequal without comparing them. Bags are sorted vectors, which compare fastest. A bag's hash is a sum over its entries, so `take` updates it without going through the rest of the bag.

A compiler pass moves each draw of a distribution written out (dice, `bernoulli` of a probability, `one_of` a list of values) down its block, to just before the first statement that uses its variable. A model that draws every component's state first then splits worlds only when a statement needs them, after the facts combined earlier have died and merged. Such draws can't fail and add up to 1, so moving them changes nothing but the number of worlds. They stop at anything that prints or can leave the block (docs/benchmarks.md, "Since: moving draws").

Merging helps only when histories stop mattering. Two refinements come later: hoisting values that are the same in every world out of the worlds, and hash-consing large values.

### 3.8 Distributions

Finite distributions are sorted `(value, probability)` pairs plus a *missing mass*: the probability of outcomes too unlikely to keep (below 10⁻¹⁸, for `poisson` and `geometric`). Operators apply to every combination of outcomes and combine missing mass as for independent draws. Options that are distributions are mixed in, so a distribution never has distributions as outcomes.

Continuous distributions (`continuous.rs`) are their family and parameters: density, CDF, quantiles and moments come from the formulas (with the incomplete beta and gamma functions, and `libm` for `erfc` and `lgamma`), and draws from exact samplers. Comparing one with a number gives a distribution of facts through the CDF, in both modes; a finite distribution whose outcomes include continuous ones is a mixture. Arithmetic on them waits for a representation of the result (closed forms, or empirical distributions).

### 3.9 Reports and output

Each `report` site owns an accumulator for each value of its `by` key: the weight that reached it, the weight of each value, and the missing mass of reported distributions. When sampling, it also keeps what each run of the current batch reported, and folds that into running sums at the end of the batch, for the standard errors; so that part doesn't grow with the number of runs. The distinct reported values do: exact quantiles need them all, and a report of a continuous quantity keeps one per run. A quantile sketch (such as a t-digest) will bound that. Rendering adds the reach ("reached in 1.00% of worlds"), the "per visit" label, ranges when unresolved weight is visible, and standard errors when sampling (semantics §9–10, §14). Output is text; `--format json` and HTML come in phase 5.

### 3.10 Randomness and reproducibility

Runs go in batches of a fixed size, 1,000, and each batch draws from a xoshiro256++ stream of its own, seeded through SplitMix64 from the seed and the batch's number. Every floating-point function goes through `libm`, not the platform's math library, so a program prints the same on every platform, WebAssembly included.

Batches run on up to `max_threads` threads (all the cores by default, `--threads` on the command line), each with an engine of its own. Batches are combined in order, as they arrive:

- Each batch's reports are added in batch order, so the sums are the same whatever thread ran which batch.
- What a batch prints is kept, with where it was printed, and printed in batch order. The output limit counts in that order too.
- The first error in batch order is the one reported. Once a batch fails, no later batch starts.

A batch starts only if it's fewer than two per thread past the batch being combined, which bounds what finished batches hold in memory. So a program and a seed give the same output on every machine and with any number of threads, whatever the engine's other settings.

The threads share one work budget, which they take in chunks of 16,384 units. The total stays within `max_work`. Only whether a run reaches that limit can depend on timing, when it comes within a few chunks of it.

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

### Phase 4: sampling and continuous distributions, release v0.2 "Forecasts" (done)

- Done: the estimators, specified first (semantics §14): independent runs, likelihood weighting, standard errors, the effective sample size; `simulate` enumerated inside runs.
- Done: independent sampling, checked against enumeration on generated programs with fixed seeds, including the calibration of its standard errors.
- Done: continuous families with pdf, cdf, quantile and sampling; `a to b`, `normal_range`, `pert`, `triangular`; `observe … from` with densities; `--mode`, `--runs` and `--seed`.
- **Exit reached:** examples 07 to 09 match their reference outputs within tolerance, and every sampler passes its statistical tests.
- Done: parallel batches, each with a random stream of its own. The output is the same with any number of threads, and sampled models run 6–8× faster on 12 cores (docs/benchmarks.md).
- Done: reading data from files and stdin: CSV, JSON and lines, with declared types that decide how the data is read, `probl schema`, host limits and resolvers ([reading data](data-input.md)). Record types keep their fields' types.
- Done: the evidence when sampling (semantics §14): the average final weight, with its standard error from the effective sample size, or its logarithm when an observation uses a density. It's checked against enumeration's exact evidence, standard errors included.
- **Exit reached:** everything planned for v0.2 is built; releasing it is a matter of packaging.
- Only if benchmarks call for them, each with its contract first: arithmetic on continuous distributions, merged sampling, particles, beam, nested estimates, and an `auto` mode that says what it chose.
- Done since: exact updates for conjugate priors (semantics §14, [inference proposal](inference-proposal.md)). When sampling, draws of beta, gamma and normal priors are delayed. Binomial, Bernoulli, Poisson and normal observations update them exactly, and they're drawn from the result when first needed. `ab_test`'s 100,000 runs are worth 100,000 instead of 863, and its evidence is exact.

### Phase 5: inspection (2–4 weeks)

- Per-world traces, and `probl explain`, which shows the most likely path to an outcome (*"how does the attacker usually lose?"*).
- Runtime errors that report the probability of the failing worlds and an example path.
- `--format json`, an HTML report with charts, and a REPL prompt that shows the world count.
- Support sizes, merge ratios, and which variables keep worlds apart (D5).

### Phase 6: types and tooling (5–8 weeks), release v0.3

- The static type checker (section 8: static, with inference), growing out of today's checks. It works out every expression's type before the program runs; annotations stay optional, except for data. Values and distributions have different types, so its errors tell a distribution from a value and suggest the fix, such as `~` for `=`. `read` takes its type from the declaration.
- `probl fmt`, a language server, and a VS Code extension.
- **Exit:** release **v0.3**.

### Phase 7: reach (open-ended), v0.4 and later

- A WebAssembly build and a browser playground ([its plan](playground-plan.md), which could come earlier); Python bindings.
- Performance, if benchmarks call for it after the work in §9: a bytecode VM that runs each instruction over all worlds at once, persistent collections, a parallel enumerator.
- Research track: compiling to decision diagrams for exact inference (as the Dice language does), and reports conditioned on later evidence (smoothing). The benchmarks' blow-ups are fixed more cheaply by moving draws, or need sampling.

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
- **Sampling against enumeration.** For generated programs, every estimate sampling prints (probabilities, means, reach) must be within six standard errors of the exact value from enumeration, and across all of them the standard errors must be calibrated: few estimates more than four standard errors off, and a mean squared z-score near 1. In the mean, each z counts as at most 4. An estimate whose standard error comes from the few runs that matter (a rare event, or a few heavy weights) can be dozens of standard errors off, and one of them would otherwise dwarf the rest; the count beyond 4 bounds them instead. On 2,272 programs and 9,320 estimates, the mean was 0.94, with 2 beyond 4. `PROBL_SAMPLING_CASES` runs more.
- **Threads.** Sampled programs give the same output, the same printed lines and the same first error on 1, 2, 3 and 8 threads, and the threads share the work budget.
- **Data.** The readers are tested on:
  - every type and error;
  - collisions, limits, cancellation, refusing hosts and failing streams;
  - snapshots, and the command line;
  - round trips of random tables written as CSV, JSON and lines;
  - mutated files, which must never make them panic (`PROBL_DATA_CASES` runs more).

  Example 08 reads its data from a CSV file ([reading data](data-input.md#tests)).
- **Loops solved as Markov chains.** `probl-engine/tests/chains.rs` and the unit tests of `chain.rs` check:
  - closed forms: gambler's ruin, a tennis game with deuce, a race to a 6 or a 1, evidence from observations inside a loop;
  - loops some worlds can never leave, loops that end once in 10⁹ rounds, and chains too large to solve;
  - 3,000 random loops that cycle, against unrolling: 7,875 probabilities agreed. `PROBL_CHAIN_CASES` runs more.
- **Recursion.** `probl-engine/tests/recursion.rs` checks:
  - closed forms: the audit's `f()`, exploding dice, mutual recursion, a recursive tennis game, observations in each round;
  - calls that never return, and ones that come back and print;
  - 3,000 random processes written both ways, as recursion and as a loop. Half of them add 1 at each step, so their results have infinitely many values. All 176,491 probabilities agreed. `PROBL_RECURSION_CASES` runs more.
- **Samplers.** Kolmogorov–Smirnov tests for every continuous family, chi-square tests for the direct count samplers, and closed-form checks of CDFs, quantiles and densities. `probl-engine/tests/sampling.rs` covers the rules of semantics §13–14.
- **Exact updates.** `probl-engine/tests/conjugate.rs` and the unit tests of `conjugate.rs` check:
  - the formulas, against closed forms and numerical integration, including probabilities far below the smallest `f64`;
  - the evidence of whole data sets against mpmath;
  - which uses draw a delayed variable, and that errors are the same with and without exact updates;
  - agreement with `--no-conjugate`, and calibration against exact posteriors (a mean squared z-score near 1 over 1,800 estimates, with every run weighted the same);
  - simulation-based calibration for each pair;
  - random programs that mix exact updates with every other use of the variables. They never read a delayed variable, and estimate the same posteriors as without exact updates. On 3,000 of them, 4,863 estimates agreed within their standard errors (mean z² 0.78). `PROBL_CONJUGATE_CASES` runs more.

  Planting an error in a posterior's formula makes at least three of them fail.
- **Sampled examples.** Examples 07–09 are compared with outputs from an independent reference simulation, token by token: estimates within five standard errors, other numbers within 4%, dates within three days. The comparator has its own test.
- **Benchmarks.** `cargo run --release -p probl-bench` runs the models in `benches/` and `examples/` and prints their time, worlds, world-steps, calls, peak heap, and cost without merging; `cargo test` checks that they compile. They aren't run in the test suite (they take about 3½ minutes), and nothing tracks them over time yet.

## 6. Performance targets

- Enumeration: examples 02 to 06 each finish in under one second on a laptop, and craps in under 50 ms. (Met.)
- Sample mode: example 07 (50,000 runs × 18 months) finishes in under two seconds on eight cores. (Met: 0.28 seconds on 8 threads, 1.8 seconds on one.)
- If a target is missed by more than 10×, the vectorized VM from phase 7 moves earlier.

## 7. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Enumeration blows up on real models | high | high | Liveness-based merging; comparing distributions directly instead of drawing; limits that fail clearly; statistics on what keeps worlds apart; loops that cycle solved as Markov chains |
| Users confuse `=` and `~` | high | medium | Errors with suggested fixes, `match` and `and`/`or` refusing distributions, documentation that leads with the rule |
| Sampling estimates are wrong in subtle ways | medium | high | Estimators specified before being built; independent runs only; every estimate tested against enumeration, and the standard errors' calibration too |
| Rounding in floating-point weights | medium | medium | An extended exponent (no underflow); distributions renormalized to 1 − missing; fractions marked ≈; the oracle's exact comparison |
| Likelihood weighting degenerates (low effective sample size) | high for Bayesian models | medium | Always print the effective sample size; exact updates for conjugate priors (done); a general method, specified first, for the rest |
| Float-valued state never merges | medium | medium | Warn when float slots keep worlds apart; recommend integers or rounding |
| Untrusted models exhaust a host | medium | high | Host limits checked before allocation, cancellation, no panics; a separate worker process for hosted use. Sampled reports of continuous values still keep every distinct value: a quantile sketch is next |
| Scope creep (units, plotting, modules, …) | high | medium | Phase exit criteria; features no example needs wait |

## 8. Decisions

Settled by the audit, and in review since:

| Decision | Chosen | Instead of |
|---|---|---|
| `and`/`or` on probabilities | facts only, at most one uncertain; `bernoulli(p)` makes an event | independent events (p·q) |
| Reports and evidence | an `observe` after a `report` is an error | snapshots of the evidence so far |
| Evaluation order | left to right, each operand once | unspecified |
| `a to b` | lognormal, positive ends only; `normal_range` for normal | normal when an end is negative |
| Name of the mode | enumeration | exact |
| Typing | static, with inference: every expression's type is known before running, and values and distributions have different types. Annotations are optional, except for data read from outside | dynamic, with annotations checked as the program runs (what exists today) |
| Types of data | declared in the program, and they decide how data is read; `probl schema` suggests them | guessed from the data when running |
| Conjugate priors when sampling | updated exactly by default, with `--no-conjugate` to compare | opt-in, as `@mode sample(…, exact: true)` |
| A general method for other models | waits for a concrete benchmark, and a specification of its target, moves and report estimators | lightweight Metropolis–Hastings now, as first proposed |

Still open:

| Decision | Proposal | Alternative |
|---|---|---|
| Block syntax | braces | significant indentation, as in Python |
| Functions and outside variables | read-only, which enables memoization | explicit `inout` parameters |
| Dice literals | `2d6` is syntax, so names like `d6` are reserved | only `dice(2, 6)` |
| Debug output from merged worlds | printed once per merged world | replayed once per path |

## 9. Next steps

The order the benchmarks recommend (docs/benchmarks.md). The first three are done: parallel sampling batches (6–8× for sampled models on 12 cores), cheaper merging (1.4–2.9× for enumeration), and moving draws to their first use (the reliability model follows 256 worlds instead of 2²⁰). So are reading data ([reading data](data-input.md)) and the evidence when sampling, which completes v0.2. So are exact updates for conjugate priors, the first part of better inference: an A/B test's 100,000 runs are worth 100,000 instead of 863 with 30 days of data, and instead of 204 with 120. And so is solving loops and recursion that cycle (semantics §6 and §10).

1. **A playground in the browser** ([its plan](playground-plan.md)). Phase 1 is built: the engine runs in WebAssembly, and prints exactly what the command line prints. Next is the page.
2. **A general method for models that aren't conjugate** (lognormal priors, `a to b` estimates, hierarchical models, regressions). First a benchmark that needs it, then a specification: the review of the first proposal lists what it must contain ([inference proposal](inference-proposal.md), section 2).
