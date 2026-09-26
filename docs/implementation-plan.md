# Probl: implementation plan

> Draft 0.1, September 2026. Companion to the [language overview](language-overview.md), which defines what is being built. This document covers how, in what order, and what could go wrong.

## 1. Scope

**Goals for the 0.x releases**

- Exact answers for discrete models (dice, cards, boards), fast enough to explore interactively.
- Sampling for everything else (continuous estimates, huge state spaces), with honest error bars.
- One semantics and several engines: switching mode changes speed and accuracy, never meaning.
- Error messages that teach the model: *"`d6` is a distribution; did you mean `let n ~ d6`?"*
- Reproducible runs: the same program, seed and version give the same output on any machine and with any number of threads.

**Out of scope for now:** gradient-based inference (HMC, variational inference), GPUs, general I/O (files, network), concurrency, a foreign-function interface and a package manager.

## 2. Technology

**Rust is the recommended implementation language.**

- The engine does a lot of hashing, copying and arithmetic for every world, so native speed and control over memory layout matter.
- It compiles to WebAssembly, which makes a browser playground realistic. That matters for sharing game odds and forecasts with people who won't install anything.
- It has strong crates for building languages: `logos` (lexer), `ariadne` (diagnostics), `im` (persistent collections), `rayon` (parallel sampling), `rand` and `rand_xoshiro`, `statrs` (distribution functions), `num-rational` (exact fractions), `clap`, `rustyline`, `insta` (snapshot tests), `proptest`, `criterion`, `tower-lsp`, and later `pyo3` for Python bindings.

Alternatives considered:

- **TypeScript**: the quickest route to a playground, but slower, and its numeric support is weaker.
- **Python**: the quickest prototype, but too slow for exact enumeration.
- **OCaml**: excellent for compilers, but a smaller ecosystem and a harder path to a playground.

## 3. Architecture

### 3.1 Pipeline

```
source ─► lexer ─► parser ─► AST ─► resolver ─► lowering ─► IR ─► analyses ─► engine ─► reports ─► text / JSON
          logos     Pratt +          scopes,      splitting         liveness,     world-set
                    recursive        slots,       constructs        effects,      interpreter
                    descent          rules        become            mode choice   + policy
                                                  statements
```

### 3.2 Repository layout

```
Cargo.toml              workspace
crates/
  probl-syntax/         lexer, parser, AST, spans, diagnostics
  probl-sema/           resolver, lowering to IR, liveness and effect analysis, lints (types later)
  probl-dist/           distribution values: finite, parametric, empirical; lifted operators; queries
  probl-engine/         values, worlds, the interpreter, policies, report sinks
  probl-cli/            probl run | check | repl
  probl-wasm/           playground bindings (phase 7)
std/                    standard library written in Probl
examples/               sample programs; their "Output" blocks are golden tests
tests/                  golden, differential and statistical tests
docs/
```

### 3.3 Values and worlds

```rust
enum Value {
    Int(i64), Float(f64), Prob(f64), Str(Arc<str>), Date(i32),
    List(im::Vector<Value>), Map(im::OrdMap<Value, Value>), Bag(Arc<Bag>),
    Record(Arc<Record>), Enum(TypeId, u32), Dist(Arc<Dist>), Func(Arc<Closure>),
    Dead,                        // a slot whose variable is never read again
}

struct World {
    slots: Arc<[Value]>,         // variables of the current frame, copied on write
    weight: W,                   // f64 by default; log-space or exact rationals on request
    runs: u32,                   // sample modes: how many runs share this world
}
```

- Value semantics plus persistent collections make forking a world a reference-count increment. Changing a list in one world copies only the changed path.
- `Hash` and `Eq` are structural. Floats compare by bit pattern, with −0.0 and NaN normalized. Large values cache their hash so that merging stays O(1) per world.
- `Prob` is a separate variant from `Float` so that reports can tell a chance (printed as one percentage) from a number that happens to lie between 0 and 1 (printed as a distribution).
- The weight type is a generic parameter: `f64` by default, log-space `f64` when sampling with many observations, and `BigRational` for `--fractions`.

### 3.4 The world-set interpreter

The engine is a tree-walking interpreter in which **every statement maps a set of worlds to a set of worlds**. All worlds in a set are at the same point in the program, so control flow is shared and only the data differs. The program effectively runs once over a batch of states, like SIMD with worlds as the lanes.

```rust
/// Worlds leaving a statement, grouped by how they left it.
struct Flow {
    next: Vec<World>,               // fell through to the next statement
    broke: Vec<World>,              // `break`
    continued: Vec<World>,          // `continue`
    returned: Vec<(World, Value)>,  // `return`
}

fn exec_if(&mut self, s: &If, worlds: Vec<World>) -> Result<Flow> {
    let (mut yes, mut no) = (Vec::new(), Vec::new());
    for w in worlds {
        let p = self.eval(&s.cond, &w)?.as_prob()?;    // expressions are pure: nothing splits here
        self.policy.split(w, p, &mut yes, &mut no);    // exact: both halves; sample: one side per run
    }
    let flow = self.exec_block(&s.then, yes)?.join(self.exec_block(&s.otherwise, no)?);
    Ok(self.merge(flow, &self.live_after[s.id]))       // join point: identical worlds merge
}

fn exec_while(&mut self, s: &While, worlds: Vec<World>) -> Result<Flow> {
    let mut inside = worlds;
    let mut out = Flow::default();
    for iteration in 0.. {
        let (stay, leave) = self.split_on(&s.cond, inside)?;
        out.next.extend(leave);
        if total_weight(&stay) < self.epsilon {         // the rest is "unresolved", not dropped
            self.unresolved += total_weight(&stay);
            break;
        }
        if iteration == self.max_iterations {
            return Err(self.no_progress_error(s, &stay));
        }
        let body = self.exec_block(&s.body, stay)?;
        out.next.extend(body.broke);
        out.returned.extend(body.returned);
        inside = self.merge_worlds(chain(body.next, body.continued), &self.live_at_head[s.id]);
    }
    Ok(self.merge(out, &self.live_after[s.id]))
}
```

The interpreter follows these rules:

- **Expressions never split.** Lowering moves every construct that can split into a statement of its own: `~`, calls to functions that may split, and `if`, `chance` or `match` used as values (A-normal form). Expressions are then evaluated per world as plain functions. Arithmetic on distributions is fine here, because it produces a new distribution value without splitting anything.
- **Join points merge.** After `if`, `chance` and `match`, at the head of every loop iteration and at function return, worlds are grouped in a hash map keyed by their live slots, and their weights are summed.
- **Loops** keep iterating over the set of worlds still inside. Worlds that exit accumulate, and when the weight inside drops below ε it moves to the *unresolved* counter.
- **`break`, `continue` and `return`** are just other outputs of `Flow`.

The phase 1 interpreter is built in this shape from day one, as a set that happens to hold a single world, so that phase 2 extends it rather than replacing it.

### 3.5 Policies: one interpreter, four engines

The execution modes differ only in how worlds split and what happens after a join:

```rust
trait Policy {
    /// `if p`: send world `w` into the yes and/or no branch.
    fn split(&mut self, w: World, p: f64, yes: &mut Vec<World>, no: &mut Vec<World>);
    /// `x ~ d`: the worlds (with their value of x) that come out of one world.
    fn draw(&mut self, w: World, d: &Dist) -> Result<Vec<(World, Value)>>;
    /// Called after merging at a join point: prune (beam) or resample (particles).
    fn after_join(&mut self, worlds: &mut Vec<World>);
}
```

| Policy | `split(w, p)` | `draw(w, D)` | `after_join` |
|---|---|---|---|
| Exact | two worlds, with weights w·p and w·(1 − p) | one world per outcome; infinite supports are cut at ε; continuous D is an error | nothing |
| Beam(k) | as Exact | as Exact | keep the k heaviest worlds and add the rest to the *pruned* mass |
| Sample(n) | the world's r runs split into Binomial(r, p) and the rest | the r runs spread over sampled outcomes | nothing |
| Particles(n) | as Sample | as Sample | after an `observe`, resample if the effective sample size falls below n/2 |

Sample mode starts from a single world holding all n runs. Because identical runs merge, sampling a discrete model costs about as much as its number of distinct states, not its number of runs. Continuous models degrade gracefully to ordinary Monte Carlo, since their runs rarely coincide.

### 3.6 Function calls

- **Exact:** a call is a sub-simulation. The engine runs the body once per distinct combination of arguments and outside values read, stores the resulting distribution of return values in a memo table, and splits every calling world by it. `attack(hero, goblin)` is computed once however many rounds and worlds use it: dynamic programming without the programmer asking for it. This is sound because functions can't assign outside variables (see the overview). A call that re-enters itself with identical arguments, which is a cycle, isn't memoized; it is unrolled until the remaining weight falls below ε.
- **Sample:** callee worlds are created from the caller's worlds, tagged with the caller they came from, run through the body, and handed back.

### 3.7 Liveness and merging

A standard backward liveness analysis on the IR gives, for every join point, the slots that may still be read. Dead slots are overwritten with `Value::Dead` as soon as they die, so worlds that differ only in stale variables become identical. In Risk, the dice pools die at the end of each round, and 56 × 21 dice outcomes collapse back to a few (attackers, defenders) pairs. In snakes and ladders, `roll` dies at once, so a game is a walk over at most 100 squares.

Two refinements come later:

- Variables that are the same in every world, such as a `let` computed before anything splits, can be hoisted out of worlds entirely (binding-time analysis).
- Large values can be hash-consed, which makes equality a pointer comparison.

### 3.8 Distributions (`probl-dist`)

```rust
enum Dist {
    Finite(Arc<[(Value, W)]>),           // dice, one_of, binomial, bags, results of lifted operators
    Parametric(Family),                  // normal, lognormal, beta, gamma, poisson, … : pdf, cdf, quantile, sample
    Empirical(Arc<WeightedSamples>),     // simulate { } in sample modes
    Mixture(Arc<[(W, Dist)]>),
}
```

- **Lifted operators:** finite combined with finite uses convolution, merging equal results. Closed forms are used where they exist (sums of normals, scaling). Otherwise sample modes approximate by sampling, and exact mode raises an error that suggests a fix.
- **Comparisons:** exact sums for finite distributions, the CDF for parametric distributions compared with a constant, and sampling otherwise.
- **Draws in exact mode:** a finite distribution gives one world per outcome. An infinite discrete one (Poisson, geometric) is enumerated until the tail weighs less than ε. A continuous one is an error that suggests sample mode or `.bins(n)`.

### 3.9 Reports and output

Each `report` site owns a sink for each value of its `by` key:

- in exact mode, a map from value to weight, plus mixture components for reports whose value is a distribution;
- in sample modes, running moments and a weighted quantile sketch (t-digest), with standard errors computed from the effective sample size.

Output is rendered as text by default (percentages, a statistics line, sparklines, tables), as JSON with `--format json` for tools and notebooks, and later as HTML with charts.

### 3.10 Randomness and reproducibility

Random numbers come from Xoshiro256++ streams derived from `(seed, batch index)`. Sample runs are split into fixed-size batches, processed in parallel with rayon, and combined in batch order. Results therefore don't depend on the number of threads.

### 3.11 Errors

Runtime errors happen in particular worlds. When one does, the run stops with the error message, the probability of the worlds that hit it, and the branch choices that led to one of them: *"index 7 out of range in 2.3% of worlds; for example: come_out = 4 → r = 9 → r = 12 → …"*. Keeping the choices costs a persistent linked list per world, so it's on by default for exact runs and optional when sampling.

## 4. Phases

The effort ranges assume one developer working full time, and they are 90% confidence ranges. [`examples/09_roadmap.probl`](../examples/09_roadmap.probl) turns them into dates.

### Phase 0: Groundwork (1–2 weeks)

- Rust toolchain, Cargo workspace with the crates above, CI (format, lint, test), license, contributing notes.
- A golden-test runner over `examples/` that marks tests as pending until their features exist.
- `docs/semantics.md`: the reference semantics made precise (join points, ε, snapshot reports, errors), to settle arguments about the engine's behavior.
- **Exit:** CI is green and `probl --version` runs.

### Phase 1: Front end and single-world core (3–5 weeks)

- Lexer: numbers, percentages, dice, interpolated strings and the newline rules.
- Parser: Pratt parsing for expressions and recursive descent for the rest, covering the full grammar in the overview, including constructs the engine can't run yet.
- An AST with source spans, and diagnostics with source snippets.
- Resolver: scopes and slots, `let` and `var` checks, *functions don't assign outside variables*, and *`report` only at top level*.
- Lowering to the IR, with splitting constructs moved into statements.
- The interpreter over single-world sets: ints, floats, strings, lists, maps, records, enums, functions and loops.
- `probl run`, `probl check` and a basic `probl repl`.
- **Exit:** all nine examples parse and resolve; deterministic programs (FizzBuzz, sorting, recursion) print correct results; the parser survives an hour of fuzzing.

### Phase 2: Worlds, the exact engine (4–7 weeks)

- World sets, `Flow` and the Exact policy; splitting `if`, `chance` and `match` on probabilities; `prob` values with `and`, `or` and `not`.
- `~` from dice, `one_of` and probabilities.
- Loops over world sets, with ε, unresolved mass and iteration caps.
- Liveness analysis, and merging at join points and loop heads.
- Memoized function summaries.
- `report` with `as` and `by`, snapshot semantics, and text output with statistics and sparklines.
- A naive reference engine for differential tests: a deliberately simple path-by-path enumerator with no merging and no memoization.
- **Exit:** `02_craps` reproduces its documented output; the random walk in `01_tour` never holds more than about 2,000 worlds; differential tests pass on 1,000 generated programs.

### Phase 3: Discrete distributions and evidence (3–5 weeks), release v0.1 "Games"

- `dist` values: finite distributions, lifted operators and comparisons, and queries (`mean`, `quantile` and the rest).
- Dice pools (`roll`), bags and `take`, `binomial`, and `poisson` and `geometric` with tails cut at ε.
- `simulate { }`, and `observe` in exact mode, with the evidence shown in the run summary.
- `--fractions`: exact rational weights.
- **Exit:** examples 01 to 06 reproduce their documented output. Release **v0.1**.

### Phase 4: Continuous distributions and sampling (4–6 weeks), release v0.2 "Forecasts"

- Parametric families with pdf, cdf, quantile and sampling; `a to b`, `pert` and `triangular`.
- The Sample, Particles and Beam policies; `auto` mode and the world budget.
- `observe … from` with log-space weights, effective sample size and standard errors.
- Parallel batches with reproducible seeding.
- `date` values and calendar helpers.
- **Exit:** examples 07 to 09 match their outputs within tolerance, and every sampler passes its statistical tests. Release **v0.2**.

### Phase 5: Inspection (2–4 weeks)

- Per-world traces, and `probl explain`, which shows the most likely path to an outcome (*"how does the attacker usually lose?"*).
- Runtime errors that report the probability of the failing worlds and an example path.
- `--format json`, an HTML report with charts, and a REPL prompt that shows the world count.

### Phase 6: Types and tooling (5–8 weeks), release v0.3

- A static type checker with local inference; errors that tell a distribution from a value, with suggested fixes; the lints listed in the overview.
- `probl fmt`, a language server (diagnostics, and hover showing the type and, in exact mode, the distribution of a variable at that line), and a VS Code extension with syntax highlighting.
- **Exit:** release **v0.3**.

### Phase 7: Reach (open-ended), v0.4 and later

- A WebAssembly build and a browser playground with shareable links.
- Python bindings: run a model from a notebook and get DataFrames back.
- Performance: a bytecode VM that runs each instruction over all worlds at once, hash-consing, and a parallel exact mode.
- Standard library modules written in Probl: cards, calendars, board-game helpers.
- Research track: resample-move MCMC and delayed sampling (conjugate updates) for static parameters, compiling to decision diagrams for exact inference (as the Dice language does), and reports conditioned on later evidence (forward–backward smoothing).

### Forecast

Running [`examples/09_roadmap.probl`](../examples/09_roadmap.probl), which adds three risks, 10–30% overhead and two weeks of holidays to the ranges above, starting on 28 September 2026:

| Milestone | 5% | Median | 95% |
|---|---|---|---|
| v0.1 Games | 2027-01-26 | 2027-02-18 | 2027-03-24 |
| v0.2 Forecasts | 2027-03-07 | 2027-04-05 | 2027-05-13 |
| v0.3 Types and tooling | 2027-05-17 | 2027-06-21 | 2027-08-05 |

## 5. Testing

- **Golden tests.** Each example's Output block is its expected output. Sampled examples are compared within a tolerance, for example ±4 standard errors.
- **Differential tests.** The naive reference engine and the real exact engine must agree to 10⁻¹² on every program small enough for the naive one. Programs are generated with `proptest` from a subset of the grammar: random `if`, `chance`, draws, and loops with bounded iterations.
- **Cross-engine agreement.** Sample mode and exact mode are compared on discrete programs, using chi-square or binomial tests with fixed seeds and a significance level (10⁻⁶) low enough that false alarms are negligible.
- **Known answers.** The 2d6 distribution; craps (244/495); Monty Hall (2/3); the birthday problem; gambler's ruin; beta-binomial conjugacy (posterior mean and variance); normal CDF values; the infinite-deck blackjack dealer (busts 28.16% of the time when standing on soft 17).
- **Samplers.** Kolmogorov–Smirnov or chi-square tests for every distribution family.
- **Invariants.** In debug builds, after every statement: live weight + exited + unresolved + pruned + observed away = 1, up to rounding.
- **Fuzzing.** `cargo fuzz` on the lexer and parser (no panics), and generated programs run through the engine.
- **Benchmarks.** `criterion` benchmarks for examples 02 to 07, tracked in CI to catch regressions in worlds per second, merge ratio and peak world count.

## 6. Performance targets

These are initial targets, to be revisited after profiling in phase 2:

- Exact mode: examples 02 to 06 each finish in under one second on a laptop, and craps in under 50 ms.
- Sample mode: example 07 (50,000 runs × 18 months) finishes in under two seconds on eight cores.
- If a target is missed by more than 10×, the vectorized VM from phase 7 moves earlier.

## 7. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Exact mode blows up on real models | high | high | Liveness-based merging; comparing distributions directly instead of drawing; a world budget that switches to beam or sampling; `--stats` showing which variables keep worlds apart |
| Users confuse `=` and `~` | high | medium | Errors with suggested fixes, lints, a REPL that shows world counts, and documentation that leads with the rule |
| `and`/`or` assuming independence surprises people | medium | medium | A lint for the same chance used twice; the *chances and facts* section of the docs |
| Reports are snapshots and don't see later evidence | medium | medium | A warning when a report comes before an `observe`; smoothing in the research track |
| Likelihood weighting degenerates (low effective sample size) | high for Bayesian models | medium | Always print the effective sample size and warn when it's low; resample-move and conjugate updates later |
| Float-valued state never merges | medium | medium | Warn when float slots keep worlds apart; recommend integers or rounding |
| The tree-walking interpreter is too slow | medium | medium | Profile first; batch operations over worlds; the bytecode VM |
| Scope creep (units, plotting, modules, …) | high | medium | Phase exit criteria; features no example needs wait |

## 8. Decisions to confirm

| Decision | Proposal | Alternative |
|---|---|---|
| Implementation language | Rust | TypeScript (a faster route to a playground) |
| Block syntax | braces | significant indentation, as in Python |
| `and`/`or` on chances | independent events (p·q) | allowed only on settled facts |
| Report semantics | snapshots of the evidence so far | conditioned on all evidence (forward–backward, slower) |
| Functions and outside variables | read-only, which enables memoization | allow writes through explicit `inout` parameters |
| Dice literals | `2d6` is syntax, so names like `d6` are reserved | only `dice(2, 6)` |
| `prob` vs `float` | separate types | one number type, formatted by hand |
| Name and file extension | Probl, `.probl` | |

## 9. The first two weeks

1. Install the Rust toolchain with `rustup` (it isn't on this machine yet), create the Cargo workspace and crate skeletons, and set up CI with GitHub Actions.
2. Write the golden-test runner that reads the Output blocks in `examples/*.probl`, with every test pending for now.
3. Write the lexer, including percentage, dice and interpolated-string tokens, with unit tests.
4. Write the parser: Pratt parsing for expressions, recursive descent for statements. The goal is that all nine examples parse, with snapshot tests of their ASTs (`insta`).
5. Set up diagnostics with `ariadne`, with five deliberately broken programs and the messages they should produce.
6. Draft `docs/semantics.md` from the semantics table in the overview, making join points, ε and snapshot reports precise.
