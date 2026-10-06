# Original language design and architecture audit

> Historical review of the revision named below. Findings and reproductions describe that revision, not the current release. The [resolution map](../architecture.md#original-audit-resolution-map) records the resulting changes; consult the [current semantics](../semantics.md) for the language contract. This document preserves design rationale and prior art, and is not a fresh security assessment. Source links locate files in the current tree; inspect the reviewed implementation with `git show 6d0557a:<path>`.

Reviewed on 26 September 2026 against commit `6d0557a` (workspace version `0.0.1`). The review covers the language proposal, implementation plan, examples, compiler pipeline, runtime, reporting, and tests. Design findings come first; implementation findings are restricted to semantic architecture, numerical reliability, and security boundaries.

**Assessment:** Probl has a useful core: imperative models over weighted states, explicit draws, value semantics, and liveness-based state merging. Keep those. Before expanding into forecasting, strengthen the distinction between events and probabilities, specify inference and reporting scopes, and make accuracy claims enforceable. Those decisions affect the language's meaning and the runtime's data structures; postponing them until tooling or sampling will make them expensive to change.

`cargo test --all` passed: **56 passed, 3 ignored**. The ignored cases are the forecasting examples. Small, isolated CLI probes confirmed the behaviors reproduced below. “Confirmed” means executed against this checkout; “source finding” means established by inspection; “proposal risk” concerns a documented feature that is not implemented yet. High priority means a credible path to misleading results, unstable semantics, or host denial of service. Medium priority means a substantial limitation or missing contract.

| ID | Priority | Finding | Basis |
|---|---|---|---|
| D1 | High | Events, probabilities, and distributions of probabilities collapse into one another | Design + confirmed |
| D2 | High | Unresolved prior mass is not a posterior or expectation error bound | Design + confirmed |
| D3 | High | Evidence and report scopes need explicit contracts | Design + confirmed |
| D4 | High before sampling | Sampling, nested inference, and particle uncertainty need more than a split policy | Proposal risk |
| D5 | Medium | State merging is useful but does not solve general state explosion or recurrent loops | Design + source finding |
| D6 | Medium | Interval estimates hide consequential distribution assumptions | Proposal risk |
| I1 | High | “Exact” fractions and missing-mass accounting are not supported by the numerical architecture | Confirmed + source finding |
| I2 | High | Lowering and memoization do not consistently preserve effects and evaluation order | Confirmed |
| I3 | High for untrusted models | Resource limits and runtime error containment do not form a security boundary | Confirmed + source finding |
| I4 | Medium | The validation architecture cannot independently check key semantic claims | Source finding |

**D1 — Separate events, probability parameters, and distributions.**

The [chances-and-facts design](../semantics.md#2-events-and-identity) deliberately gives `and` and `or` the algebra of independent trials. This is internally understandable, but familiar logical syntax now accepts formulas with very unfamiliar meanings:

```probl
let p = 30%
report p and not p       # confirmed: 21.00%
report p or not p        # confirmed: 79.00%
let happened ~ p
report happened and not happened  # confirmed: 0.00%
```

These are specified behaviors, not arithmetic bugs. The design flaw is that a number describing an event has no event identity, while the syntax strongly suggests one. A repeated-name lint cannot protect against aliases, function calls, or two expressions describing the same underlying cause. Ordinary logical rewrites such as idempotence and excluded middle are invalid on chances.

The deeper problem is loss of uncertainty about probability parameters. The special rule that a probability-valued `simulate` returns a probability averages away the parameter distribution:

```probl
let p ~ simulate { if 50% { 10% } else { 90% } }
report p and p as "percent rates"  # confirmed: 50.00%

let q ~ simulate { if 50% { 0.1 } else { 0.9 } }
report q and q as "float rates"    # confirmed: 41.00%
```

For a model that first chooses a 10% or 90% success rate and then performs two independent trials with that shared rate, the answer is `0.5 × 0.1² + 0.5 × 0.9² = 0.41`. In the first version, `simulate` collapses to 50%, and `~` then draws a Boolean fact. Merely writing percentages changes the model. Likewise, `let x ~ (30% * 1); report x and x` produces 9%, because arithmetic returns a float and drawing a float leaves it unchanged; drawing `30%` produces a fact and gives 30%.

This ambiguity also escapes into pattern matching. A `match` over `let x = d6` with arms for all six faces fails with “no arm ... matches”. Adding a wildcard gives that impossible “other” outcome **33.49%**: each successive equality compares a fresh independent die. Pattern alternatives use probabilistic `or` as well. This is a missing semantic boundary, not just a missing exhaustiveness check. See [pattern lowering](../../crates/probl-sema/src/lower.rs).

**Suggested improvement:** introduce distinct `bool`, `prob`, and `dist[T]` meanings now. Make `simulate` preserve its result type, including `dist[prob]`; make Bernoulli conversion explicit, for example `bernoulli(p)`. Reserve logical operators and pattern tests for facts. Probl can retain convenient probabilistic branching through `chance` or explicitly specified `if prob` sugar. If independent-chance algebra remains, expose it through deliberately named operations and reject its accidental use in patterns. Require `match` subjects to be settled values, or define one explicit draw for the whole match.

Move the minimal type/effect checks needed for those rules ahead of the full type-checker milestone. Currently even `let p: prob = "not a probability"` passes `probl check`; annotations are discarded during [binding lowering](../../crates/probl-sema/src/lower.rs). A complete type system can wait, but silently accepting contracts the language does not enforce makes these mistakes harder to detect.

Relevant prior art: [ProbLog's coin model](https://dtai.cs.kuleuven.be/problog/tutorial/basic/01_coins.html) gives random facts explicit identities; [WebPPL conditioning](https://webppl.readthedocs.io/en/master/inference/conditioning.html) distinguishes sampled values, conditions, and likelihood factors. These offer useful semantic distinctions without requiring Probl to copy their syntax.

**D2 — Define accuracy at the query, after conditioning.**

The [execution-mode table](../language-overview.md#8-execution-modes) promises exact answers with unresolved mass and bounds from beam pruning. A bound on discarded *prior* weight does not generally bound the error in a normalized posterior. Evidence can make a discarded rare world dominate the answer.

Confirmed example, with a deliberately visible tolerance:

```probl
@epsilon 0.01
var win = false
if 0.5% { repeat 1 { win = true } }
observe if win { true } else { 0.00001 }
report win
```

The engine reports `win 0.00%` and `unresolved < 1e-2`. Without truncation, the posterior is

```text
P(win | evidence) = 0.005 / (0.005 + 0.995 × 0.00001)
                  ≈ 99.80%
```

Thus the unresolved number must not be interpreted as ±1 percentage point on the result. The same issue exists at the default tolerance for sufficiently rare evidence. The engine checks absolute weight before executing a [loop body](../../crates/probl-engine/src/interp.rs), even for bounded loops. `observe 1e-13; repeat 0 { }; report true` reports “never reached”, whereas removing the empty loop reports 100%. A harmless structural edit can discard the entire posterior.

A small missing mass also cannot bound a mean or variance without bounds on the omitted values. A very rare, very large loss can dominate expected cost. Future continuous likelihoods make the issue harder: densities can exceed one, so a bound on prior mass need not bound discarded evidence weight at all.

**Suggested improvement:** distinguish termination mass, numerical error, and query error. Never truncate a known finite loop merely because its input evidence weight is small. Normalize/rescale retained weights while maintaining a separate evidence normalizer. Allocate a run-wide approximation budget and attach bounds to each result. For bounded likelihoods, if retained event weight is `a`, retained evidence weight is `Z`, and omitted evidence weight is at most `U`, a conservative event interval is `[a/(Z+U), (a+U)/(Z+U)]` when the denominator is positive. Without a justified `U`, disclose that the posterior error is unbounded. Means need tail-moment bounds or bounded payoff ranges, not just an epsilon.

Treat beam results the same way: posterior and expected-value guarantees need their own derivation. A single global `unresolved` counter cannot express all these contracts.

**D3 — Make inference scopes and report denominators visible.**

The proposal usefully chooses snapshot reports, but leaves several related distinctions implicit: ordinary calls carry observation weights into the caller; `simulate` locally normalizes them; branch reports condition on reaching the report; loop reports accumulate visits. These need to be first-class parts of the model.

Confirmed behaviors:

- `if 1% { report true as "win" }` prints `win 100.00%` without the 1% reach weight. The conditional answer is correct, but the overview promises to show the share reaching the branch. Its omission invites a serious interpretation error.
- `let long ~ 50%; repeat if long { 9 } else { 1 } { report long by "all" }` prints 90%. The engine measures report visits: long runs contribute nine times. Requiring a `by` clause does not ensure one observation per trajectory and key.
- `let d = simulate { observe 10%; 1 }; report d` prints `evidence 100.00%`. The nested observation is locally normalized, but sets a global “observed” flag. The output does not identify which evidence it describes.
- `observe false; report true` exits successfully with “never reached”. There is no posterior under impossible evidence; this needs a distinct result from ordinary unreachable control flow.

The relevant architecture is the [global observation flag and observation execution](../../crates/probl-engine/src/interp.rs), [local normalization in `simulate`](../../crates/probl-engine/src/interp.rs), [header construction](../../crates/probl-engine/src/lib.rs), and [report accumulators](../../crates/probl-engine/src/report.rs).

**Suggested improvement:** define a model execution as an unnormalized measure plus an evidence normalizer, termination information, and diagnostics. Define exactly where normalization occurs. Ordinary calls should compose the unnormalized measure; an inference operation should explicitly return a normalized distribution and its own metadata. Scope observations and unresolved mass to that operation.

Every report should identify its evidence snapshot, conditional denominator, and reach weight. Specify whether repeated reports estimate trajectories, visits, or one value per time key; require an explicit aggregation policy when keys repeat. Impossible evidence should produce a clear diagnostic, distinct from numerical underflow and unresolved evidence.

For forecasting, replace the universal phrase “probability of the evidence” with a normalizing-constant or log-evidence contract that distinguishes discrete mass from continuous density. A density is not a percentage and changes with the measurement units. WebPPL's separate [conditioning and log-score primitives](https://webppl.readthedocs.io/en/master/inference/conditioning.html) are relevant here.

**D4 — Specify estimator semantics before adding sampling and particles.**

This is a risk in the [planned policies and reporting architecture](../architecture.md#sampling-and-reproducibility), not a claim that an existing sampler is wrong. Exact state merging preserves a weighted measure; preserving Monte Carlo error estimates needs additional information.

Three decisions need to precede implementation:

1. **Merged samples need statistical bookkeeping.** Runs that reach equal program states may have different importance weights or shared particle ancestry. Total weight and a run count alone do not preserve weight dispersion or dependence. At minimum, preserve the information required by the chosen variance estimator; resampled particles may require ancestry or independent batches. Weight-based ESS is a diagnostic, not a universal standard-error formula. See [Elvira, Martino, and Robert, *Rethinking the Effective Sample Size*](https://arxiv.org/abs/1809.04129).
2. **Nested `simulate` can affect decisions.** An estimated `mean(simulate { ... })` used in an `if` introduces an inner estimation error that an outer sample count does not eliminate. Memoizing an empirical result can also share that error across callers. Finite inner budgets and nonlinear decisions require a stated approximation contract. This is a documented issue in [Rainforth et al., *On Nesting Monte Carlo Estimators*](https://proceedings.mlr.press/v80/rainforth18a.html).
3. **Resampling and mode changes need explicit execution boundaries.** “After each `observe`” is ambiguous when branches encounter different observations or different numbers of them. An `auto` fallback also needs to say whether it restarts, samples a retained frontier, or uses another estimator. Restarting must handle reports and debug output consistently; a frontier conversion must preserve its weights and control locations.

**Suggested improvement:** ship straightforward independent sampling first, using the exact engine as an oracle on small discrete models. Add sample merging only with demonstrated preservation of the estimator and uncertainty calculations. Give nested inference separate budgets and diagnostics; initially restrict approximate nested inference in decision-making contexts if necessary. Specify particle synchronization, log normalizers, and ancestry before implementing resampling. Promise the same *target model* across modes, with explicit estimator limitations, rather than implying interchangeable finite-run results.

The planned `split/draw/after_join` interface does not yet expose enough context for all of these responsibilities. Establish an inference context that owns observations, budgets, randomness, and result metadata. [WebPPL's inference methods](https://webppl.readthedocs.io/en/master/inference/methods.html) are a useful comparison because inference algorithms have distinct options and restrictions.

**D5 — Narrow the scalability claim and distinguish loop solving from unrolling.**

The README's suggestion that games stay small because worlds merge is too broad. Merging helps only when histories become observationally indistinguishable for the remaining computation. Keeping a hand, a trajectory, or many independent latent variables can preserve exponentially many states. Requesting a distribution of duration can make the relevant state unbounded even when the board itself is finite.

The current [loop interpreter](../../crates/probl-engine/src/interp.rs) unrolls until an epsilon or iteration limit; it does not solve recurrent transition equations. [Function calls](../../crates/probl-engine/src/interp.rs) reject re-entry with identical arguments. Consequently, the almost-surely terminating function `fn f() { if 50% { 1 } else { f() } }` is unsupported even though a corresponding loop can be approximated. Rational arithmetic alone would not make an infinitely unrolled craps computation return a mathematically exact finite fraction.

**Suggested improvement:** describe performance in terms of live state, requested queries, and support size. Expose estimates of distribution support, merge ratios, and which values keep states apart. Keep enumeration as the initial backend. For a supported finite-state subset, consider solving strongly connected components as absorbing Markov chains, including reachability and expected rewards. Use a symbolic backend only where benchmark structure justifies it.

This direction has close prior art: [PRISM](https://www.prismmodelchecker.org/manual/ConfiguringPRISM/ComputationEngines) offers explicit, sparse, and symbolic representations, with numerical and arbitrary-precision modes; its [reward queries](https://www.prismmodelchecker.org/manual/PropertySpecification/Reward-basedProperties) distinguish expected accumulated quantities from terminal-state distributions. [Dice](https://arxiv.org/abs/2005.09089) demonstrates factorization through weighted model counting. Neither makes general exact inference cheap, but both give more precise architectural comparisons than describing state merging itself as the distinctive innovation.

**D6 — Make interval-estimate assumptions inspectable.**

The planned `a to b` syntax chooses a lognormal distribution for positive endpoints and a normal distribution otherwise. Two quantiles do not uniquely specify a distribution, and changing an endpoint across zero changes the family. A normal estimate can assign probability to negative durations or counts; a lognormal tail can materially affect expected cost despite the same central 90% interval.

This is a modeling risk rather than an implementation defect: the constructors are currently unsupported. The overview's “as in Squiggle” attribution also needs qualification. Squiggle's currently documented [`to` constructor](https://www.squiggle-language.com/docs/Api/Dist#to) uses lognormal quantiles and rejects nonpositive endpoints; it does not document Probl's normal fallback.

**Suggested improvement:** keep the shorthand, but document one stable interpretation and expose its family, quantiles, support, and tail assumptions in model inspection. Offer explicit constructors for normal, positive, and bounded estimates; require a deliberate choice when constraints such as nonnegative duration matter. Make dependence between estimates easy to express using shared sampled parameters. This is more valuable to forecasting reliability than adding many distribution names without guidance.

**I1 — Replace the numerical “exactness” claim with an enforceable contract.**

All current [world weights](../../crates/probl-engine/src/world.rs) and [distribution weights](../../crates/probl-engine/src/dist.rs) are `f64`. `--fractions` changes rendering only: [fraction reconstruction](../../crates/probl-engine/src/report.rs) searches for a nearby fraction with denominator at most one million. It is not the `BigRational` engine described in the plan.

Confirmed example:

```probl
report 33.33333333333%
```

Running with `--fractions` prints `33.33% (1/3)`. That finite decimal is not exactly one third. Recovering the familiar craps answer from a close float is likewise not a proof of exact computation.

Missing-mass bookkeeping also fails to compose consistently. Source findings:

- [`ops::combine`](../../crates/probl-engine/src/ops.rs) discards its `missing` argument when results are probabilities. Thus comparing a truncated distribution can produce a plain probability with no associated uncertainty.
- [`Dist::pool`](../../crates/probl-engine/src/dist.rs) returns the input die's missing mass for any roll count. For independent draws with missing mass `m`, the omitted pool mass is `1 - (1-m)^count`, not `m`.
- [`Dist::mean` and `quantile`](../../crates/probl-engine/src/dist.rs) normalize retained support; [report accumulation](../../crates/probl-engine/src/report.rs) expands a distribution without recording its missing mass. Different query paths therefore hide or handle tails differently.
- [`simulate`](../../crates/probl-engine/src/interp.rs) normalizes return weights and constructs the result with zero missing mass, while adding truncation to a global counter. This detaches the distribution value from the approximation that created it.

Repeated observations also multiply ordinary floating-point weights directly. Underflow can erase evidence even when no approximation was intended. Scheduling log-space weights only for future sampling leaves the present conditioned enumeration path vulnerable.

**Suggested improvement:** distinguish enumerated, truncated, and sampled results; label reconstructed fractions as approximations until a rational backend exists. If exact fractions remain a product promise, preserve exact literal weights and rational arithmetic through the supported model subset, and solve supported recurrent systems or explicitly retain truncation. Introduce a common result/measure abstraction carrying retained weight, missing-mass bounds, normalization scope, and numerical status through draws, queries, and reports. Use scaled or log weights for likelihood accumulation, with explicit finite-value checks. Add mass-conservation and tail-composition tests before expanding distribution support.

**I2 — Make evaluation order and effects explicit in the IR.**

The compiler moves statements generated by subexpressions ahead of the enclosing expression, but leaves earlier variable reads unevaluated. This makes ordinary refactoring change a deterministic program's result:

```probl
var x = 1
report x + if true { x = 2; 0 } else { 0 }
# confirmed: 2
```

Compare this separate program:

```probl
var x = 1
fn read_x() { x }
report read_x() + if true { x = 2; 0 } else { 0 }
# confirmed: 1
```

The first operand is delayed when it is a slot read but evaluated earlier when it is a call. Similarly, `[x, { x = 2; x }]` produces `[2, 2]`. The proposal does not specify a complete evaluation-order contract, so the immediate design obligation is to choose one; the current mixture is a compiler architecture problem regardless of whether left-to-right order is chosen. See [binary lowering](../../crates/probl-sema/src/lower.rs) and [collection expression lowering](../../crates/probl-sema/src/lower.rs).

Memoization has the related assumption that every function is observationally pure. Yet `print` is allowed inside functions:

```probl
fn f() { print("called"); 1 }
repeat 2 { let x = f() }
```

This prints once normally and twice with `--no-memo`. Read-only captures prevent external variable mutation, but do not eliminate diagnostic, observation, or inference effects. The engine's [memoized call summaries](../../crates/probl-engine/src/interp.rs) contain return weights and unresolved mass, while [printing](../../crates/probl-engine/src/interp.rs) happens immediately.

**Suggested improvement:** specify operand order, then lower operands to stable temporaries whenever later evaluation can change what they read. Use effect information for state mutation, sampling, scoring, nested inference, and diagnostics. Memoize only the effects represented faithfully by the call summary; either replay debug events with caller weights or explicitly define debug output as cache-dependent. Do not infer “pure” from read-only captures or from a call happening to return one outcome. This also provides the foundation for the restrictions recommended in D1 and the inference scopes in D3.

**I3 — Enforce host-owned resource budgets and contain model errors.**

The current language exposes no general file/network/process operations, which limits the attack surface. The material security concern is availability when executing an untrusted model in a future playground, service, or embedded host. A local user can already accidentally exhaust the process.

The [world limit](../../crates/probl-engine/src/interp.rs) is checked at statement entry. Expensive distributions, cross-products, and outgoing world vectors can be constructed before that check. [`Dist::dice` and `pool`](../../crates/probl-engine/src/dist.rs) have no shared allocation/work budget; Poisson construction walks upward from zero even for enormous rates; range helpers can eagerly materialize a large range. Memo tables, report sinks, and caches have no aggregate quota. Model pragmas can raise the available limits, so they are preferences rather than host protection.

A bounded probe demonstrates the gap without exhausting memory:

```probl
@max_worlds 1
report len(support(d100000))  # confirmed: 100,000
```

This is valid under a world-only limit, but shows why that limit is not a total resource budget. Larger literals and combinatorial pools follow the same unchecked construction paths; destructive resource-exhaustion probes were not run.

Error containment is also incomplete. `report roll(1, 1)` causes a Rust panic and exits with code 101. [`Dist::into_value`](../../crates/probl-engine/src/dist.rs) collapses a one-outcome distribution to a scalar, but [`roll`](../../crates/probl-engine/src/interp.rs) asserts that the result must be a distribution. A separate boundary-integer range probe also panicked in a debug build. These demonstrate an inconsistent runtime invariant and unchecked arithmetic reaching host failure, rather than ordinary language diagnostics.

Every execution additionally requests a [512 MiB thread stack](../../crates/probl-engine/src/lib.rs). This is a large stack reservation, not necessarily 512 MiB of resident memory, but is a poor default for concurrent embedding and a sign that recursion needs an explicit execution strategy. Panics are propagated back to the caller, and the CLI terminates.

**Suggested improvement:** give the host immutable upper bounds for source size, syntax nesting, instructions/work, live and intermediate outcomes, bytes, recursion, caches, and output. Check budgets before allocations and inside distribution construction; support cancellation. Model pragmas may lower host limits, never raise them. Keep singleton distributions type-stable or normalize all consumers through a safe distribution interface. Make language errors return diagnostics, use checked size arithmetic, and replace deep Rust recursion with an explicit stack where needed. Run untrusted hosted models in a cancellable isolated worker/process as a second boundary. Catching panics alone cannot contain out-of-memory aborts or stack overflow.

This review did not establish a code-execution or data-exfiltration vulnerability. It was not a dependency-advisory audit or a sustained fuzzing campaign.

**I4 — Add a genuinely independent semantic oracle.**

The existing examples and known-answer tests are useful. However, the [differential test](../../crates/probl-engine/tests/semantics.rs) compares seven handwritten programs through the same compiler and interpreter with merging and memoization toggled. Even with merging disabled, [`world::merge`](../../crates/probl-engine/src/world.rs) still clears dead slots before its `enabled` check. Thus this comparison cannot independently validate lowering or dead-slot elimination, two central correctness assumptions.

The plan calls for an independent enumerator, generated programs, and fuzzing; those are not present in the reviewed implementation. The green suite did not catch the evaluation-order, rate-collapse, evidence/truncation, or runtime-panic examples above.

**Suggested improvement:** implement a deliberately small AST interpreter that evaluates operands in the specified order, keeps all bindings, does not memoize, and uses rational weights on a bounded discrete subset. Compare full measures and evidence normalizers, not only normalized report probabilities. Generate programs covering mutation inside expressions, function extraction, aliases, repeated observations, impossible evidence, finite loops, and singleton distributions. Add separate properties for mass conservation, bounds, and supported refactorings. Fuzz parsing and execution under strict budgets. Keep unsupported-example allowlists explicit so new “unsupported” regressions cannot silently become ignored tests.

**Prior art most relevant to Probl's decisions**

The following comparisons are architectural recommendations from this audit, not claims that these systems are drop-in backends or have identical semantics.

| Language/platform | Relevant established work | What Probl should study |
|---|---|---|
| [PRISM — computation engines](https://www.prismmodelchecker.org/manual/ConfiguringPRISM/ComputationEngines) and [reward properties](https://www.prismmodelchecker.org/manual/PropertySpecification/Reward-basedProperties) | Explicit and symbolic probabilistic model checking; separate numerical and exact modes; expected reward queries | Finite-state recurrent loops, query-specific guarantees, state-space diagnostics, honest precision terminology |
| [Dice — paper](https://arxiv.org/abs/2005.09089) and [implementation](https://github.com/SHoltzen/dice) | Exact discrete inference through weighted model counting and structural factorization | A possible future backend for structured discrete models that enumerate poorly; benchmarks before committing to it |
| [WebPPL — inference](https://webppl.readthedocs.io/en/master/inference/methods.html) and [conditioning](https://webppl.readthedocs.io/en/master/inference/conditioning.html) | Explicit inference operations, enumeration and sampling algorithms, conditions and log factors | Boundaries between model execution and inference, algorithm restrictions, observation effects |
| [ProbLog — coin tutorial](https://dtai.cs.kuleuven.be/problog/tutorial/basic/01_coins.html) | Named probabilistic facts, deterministic rules, queries, and evidence | Event identity and why repeated references should be distinguishable from independent trials |
| [AnyDice — functions](https://anydice.com/docs/functions/) and [dice](https://anydice.com/docs/dice/) | Dice distributions and parameter-dependent function evaluation over possible outcomes | Distribution/value boundaries and approachable exact game-odds modeling; compare actual function semantics rather than treating it as only a calculator |
| [Squiggle — distributions](https://www.squiggle-language.com/docs/Api/Dist) and [gotchas](https://www.squiggle-language.com/docs/Guides/Gotchas) | Distribution arithmetic, quantile estimates, multiple representations and documented lossy conversions | Forecasting ergonomics, explicit assumptions, representation changes, and faithful attribution of `to` syntax |
| [PSI — project](https://psisolver.org/) and [language implementation](https://github.com/eth-sri/psi) | Symbolic inference for probabilistic programs with discrete and continuous variables | An additional exact-inference comparison; “continuous” need not universally mean sampling, though supported program classes matter |
| [UPPAAL — statistical query semantics](https://docs.uppaal.org/language-reference/query-semantics/smc_queries/) | Statistical model checking with explicit query bounds and confidence-interval settings | Distinguishing a modeled probability from the statistical uncertainty of its estimate |

Probl's strongest positioning is an approachable imperative interface to game and forecast models with automatic inference and transparent diagnostics. Weighted states, probabilistic branching, enumeration, and state-space analysis all have substantial prior art. The opportunity is their integration and usability, backed by a precise semantic contract.

**Recommended order of work**

1. Write the missing reference semantics: event identity and types; evaluation order; ordinary-call versus `simulate` normalization; observation scope; report denominators; termination and approximation. Resolve D1–D3 before syntax or representation becomes harder to change.
2. Repair effect-preserving lowering and the numerical result contract. Correct the “exact fraction” claim immediately; implement rational support only with an explicitly supported scope. Carry evidence and uncertainty metadata through the engine.
3. Add host budgets and close panic paths before accepting untrusted models. Add the independent oracle and promote the confirmed probes into regression tests for the agreed semantics.
4. Deliver basic sampling with verified estimates and uncertainty. Add merged sampling, particles, nested approximate inference, and auto fallback only after their estimator contracts are defined.
5. Use realistic game and forecast benchmarks to choose further work: Markov-chain solving, symbolic inference, persistent collections, or a faster interpreter. The current frontend/IR/runtime separation is a useful base for that evolution.

To reproduce the code examples, put each independent snippet in a `.probl` file and run `cargo run -p probl-cli -- run <file>`; add `--fractions` or `--no-memo` where indicated. The audit changed no implementation files.
