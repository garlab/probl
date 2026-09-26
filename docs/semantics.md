# Probl reference semantics

> Version 0.2, September 2026. This document is normative for the engine. Where it disagrees with the [language overview](language-overview.md), this document wins. It resolves findings D1–D3, I1 and I2 of the [project audit](project-audit.md). Continuous distributions (D6) and basic sampling (D4) are in sections 13 and 14, and data read from files in section 15; what is still open, such as particles and solving recursive equations (D5), is listed in section 16.

## 1. Values and types

| Type | Values | Notes |
|---|---|---|
| `bool` | `true`, `false` | **Facts.** Comparisons of settled values, `and`/`or`/`not`, `in`, pattern tests |
| `prob` | a number from 0 to 1 | **Probabilities.** Percentage literals from `0%` to `100%`. A probability is a parameter, not an event |
| `int`, `float` | numbers | Arithmetic on probabilities gives floats |
| `str`, `date`, `list[T]`, `map[K, V]`, `bag[T]`, records, enums, functions | | |
| `dist[T]` | a distribution over `T` | Dice, `one_of`, `bernoulli`, counts, `roll`, `simulate`, continuous distributions (section 13), and operations on distributions |

All values are immutable: assigning or passing a collection gives an independent copy.

Wherever a probability is expected, a `float` from 0 to 1 is accepted too. Nothing else converts implicitly: in particular, a `prob` never turns into an event, and an `int` is never a condition.

## 2. Events and identity

A **distribution is a recipe**. Each occurrence of a distribution in an expression is an independent draw: `d6 + d6` is two dice, and `let die = d6` followed by `die + die` is also two dice.

A **drawn value is a fact**. `let x ~ d6` gives `x` one value in each world, and every use of `x` refers to that value.

Operators and built-in functions applied to distributions act on every outcome and return a distribution. Comparing a distribution gives a `dist[bool]`: `d6 > 4` is the distribution of the fact "this roll is above 4", which is true a third of the time. A distribution stays a distribution even when only one outcome is possible (so `d1` is `dist[int]`, not `int`).

A distribution never has distributions as outcomes: where one would, they are mixed in. `one_of([d2, 10])` is 1 or 2 a quarter of the time each and 10 half the time, so a value drawn from it is settled. The same holds for the results of operators and of `simulate` (section 8).

**Logical operators work on facts.** `and`, `or` and `not` accept `bool` operands. They also accept a `dist[bool]`, but `and` and `or` accept at most one uncertain operand: with two, Probl can't know whether they describe the same event (`e and e`) or two independent ones, so it reports an error. Probabilities are never accepted: `30% and 30%` is an error, because a probability has no identity.

To combine uncertain events, give them identities by drawing them:

```probl
let rain ~ bernoulli(30%)     # a fact: true in 30% of worlds
let late ~ bernoulli(20%)
report rain and late          # 6%
report rain and rain          # 30%: the same event
```

**Drawing needs a distribution.** `x ~ D` splits each world by the outcomes of `D`. Drawing from a probability is an error (write `bernoulli(p)`). Drawing from any other plain value leaves it unchanged, so that `x ~ if c { d6 } else { 0 }` works.

**`match` needs a settled subject.** Matching a distribution is an error; draw it first. Patterns are tests on a single value, and `|` between patterns is an ordinary `or` on facts.

## 3. Conditions and branching

`if c`, `while c`, the weights of `chance`, and `observe c` accept:

| Condition | Meaning |
|---|---|
| `bool` | no split: the world goes one way |
| `prob` (or a float from 0 to 1) | a fresh, independent trial with that probability |
| `dist[bool]` | a trial with the probability that it's true |

`if c { A } else { B }` sends each world into `A` with its weight multiplied by p and into `B` multiplied by 1 − p, where p is the condition's probability in that world. Branches with weight zero are skipped.

`chance { p₁ => A₁ … pₙ => Aₙ else => B }` evaluates every weight first, then splits. Each weight is a condition from the table above, and counts with the probability that it holds. The weights must not add up to more than 1 (a tolerance of 10⁻⁹ allows for rounding). The remainder goes to `else`. Without `else`, the remainder continues after the statement; when the `chance` is used as a value, a remainder above 10⁻⁹ is an error.

## 4. Evaluation order

Evaluation is **left to right**, and every operand is evaluated exactly once:

- operands of binary operators, arguments of calls, elements of lists, maps and records, and the parts of string interpolation are evaluated in the order they're written;
- `place = value` evaluates `value`, then the indices in `place`, then assigns;
- `place += value` (and `-=`, `*=`, `/=`) reads the current value of `place` first, then evaluates `value`;
- `and` doesn't evaluate its right side when the left side is certainly false, and `or` when it's certainly true.

An assignment made while evaluating an operand (for example inside a block expression) is visible to the operands evaluated after it, and not to those evaluated before it:

```probl
var x = 1
report x + { x = 2; 0 }       # 1
report [x, { x = 3; x }]      # [2, 3]
```

## 5. Worlds, merging and liveness

A running program is a finite set of worlds, each a program state σ with a weight w. A statement maps each world to a set of worlds:

| Statement | From (σ, w) |
|---|---|
| `x = e` | (σ[x ↦ e], w) |
| `x ~ D` | (σ[x ↦ v], w · P(D = v)) for every outcome v of D |
| `if c { A } else { B }` | A on (σ, w · p), B on (σ, w · (1 − p)) |
| `observe c` | (σ, w · p) |
| `observe v from D` | (σ, w · P(D = v)) |

Where branches rejoin, worlds with equal states are merged by adding their weights. Before comparing, variables that can't be read again are cleared (liveness analysis), so worlds that differ only in such variables merge. Merging changes nothing but rounding: a set of worlds denotes the sum of its weighted states, and equal states add.

Weights are unnormalized: they include every observation so far. Nothing is normalized except where section 8 and section 9 say so.

## 6. Functions and effects

A function may read any variable in scope; the values are copied in when it's called. It may assign only its own variables. A call runs the function's body as a set of worlds of its own and returns the distribution of results, **unnormalized**: splits and observations inside the function become part of the caller's weights. An ordinary call is not an inference boundary.

Because a function's result depends only on its arguments and the values it reads, the engine may compute it once and reuse it (memoization) when enumerating. It does so only when the function has no debug output: a function that calls `print`, directly or through other functions, runs every time it's called. When sampling, every call runs (section 14).

`print` is debug output, not part of the model. It prints once for each world that executes it; worlds that have merged count once, so the number of lines depends on how the engine merges.

## 7. Evidence

`observe c` multiplies each world's weight by the probability of `c` (section 3). `observe v from D` multiplies it by P(D = v), or, when sampling, by the density of a continuous `D` at `v` (section 13). Every factor is between 0 and 1, except densities.

The **evidence** of a program is the total final weight of its worlds: the probability of all its observations, including those made inside ordinary function calls. Observations inside `simulate` are not program evidence (section 8).

If the evidence is zero and no weight is unresolved, the evidence is **impossible**: the run fails with an error pointing at an `observe`. This is different from a report that is never reached, which is ordinary control flow.

**No `observe` may follow a `report`.** The compiler rejects a program in which an `observe`, or a call to a function that observes, can run after a `report` on the same path, including through a loop. So every report sees all the evidence its worlds will ever get, and describes the posterior. (Reports that update as evidence arrives, as in filtering, are future work.)

## 8. `simulate`

`simulate { B }` runs `B` as a separate model, starting from a copy of the current world with weight 1. It returns the distribution of `B`'s value, **normalized**: conditioned on the observations made inside `B`. Where `B`'s value is itself a distribution, its outcomes are mixed in (section 2). It is an inference boundary:

- the current world doesn't split;
- the result is always a `dist[T]`, even when only one outcome is possible, and a distribution over probabilities stays a distribution over probabilities (it is never averaged into a single probability);
- observations inside `B` condition the result and don't count as program evidence;
- if all of `B`'s weight is ruled out by its observations, it's an error;
- weight that `B` leaves unresolved (section 10) becomes the result's **missing mass**.

## 9. Reports

`report` is only allowed at the top level of the program. When a world reaches a report, the report records the world's value (and key, with `by`) and its weight.

- The **denominator** of a report is the weight that reached it: the result is conditional on reaching it.
- The **reach** of a report is that weight divided by the program's total weight. When it is below 100%, the output says so ("reached in 1.00% of worlds").
- A `bool` or `dist[bool]` value is reported as the probability that it's true. Numbers are summarized (mean, standard deviation, quantiles); other values are listed with their probabilities. A `dist[T]` value contributes each of its outcomes.

**Repeated reports.** A report outside any loop runs at most once per world, so it describes a distribution over worlds. A report inside a loop must have `by`. If the key is the variable of the innermost enclosing `for` loop, each world reports at most once per key. Otherwise a world may report the same key several times; every visit counts, and the output labels the report "per visit".

## 10. Termination and approximation

**Unbounded loops.** `while` and `loop` repeat until no world is left inside. A loop stops early once the weight still inside is less than ε times the weight that entered it (ε is 10⁻¹² by default, set with `@epsilon`). The weight left inside is **unresolved**. `for` and `repeat` loops are never cut short.

**Infinite supports.** Distributions with infinitely many outcomes (`poisson`, `geometric`) drop outcomes whose probability is below 10⁻¹⁸. The dropped probability is the distribution's missing mass; drawing from it, or using it as a condition, adds (weight × missing mass) to the unresolved weight, because the missing outcomes could go either way. Combining distributions combines their missing mass (for independent draws, 1 − Π(1 − mᵢ)).

**Bounds.** Every observation factor is at most 1, so unresolved weight U can only shrink with further observations. For an event with weight a among the weight Z that reached a report, the true probability therefore lies in

  [ a / (Z + U), (a + U) / (Z + U) ]

where U is all the weight left unresolved by the program. Reports print this range when it is visible at two decimals, and a single number otherwise. Means and quantiles are computed on the resolved part; when unresolved or missing weight is visible, the output says so, but no bound is claimed for them: a rare, very large value can move a mean arbitrarily.

**Numbers.** Weights are binary floating-point numbers with an extended exponent, so repeated observations can't underflow to zero. Sums are subject to rounding (about 10⁻¹⁵ relative). `--fractions` shows the simplest fraction within 10⁻¹³ of a probability, marked "≈": it's a hint for recognizing an answer, not a proof that the answer is exact. The engine's mode is called **enumeration**, not "exact", for these reasons.

## 11. Resource limits

The host running a program sets upper limits on: worlds per statement (when enumerating; sampled runs don't multiply), outcomes per distribution, total work, loop iterations, call depth, cached results, length of materialized collections, and output. A program's `@max_worlds` and `@max_iterations` can lower these limits, never raise them. Exceeding a limit, or cancelling a run, stops it with an error; so do all language errors. The engine never crashes on a program: a crash is a bug in Probl, and it is reported as an internal error.

## 12. How this document is checked

The crate `probl-oracle` is a second implementation of sections 1–9, for the discrete part of the language whose loops end. It shares only the parser with the engine and is deliberately naive: it follows every path on its own (no merging, no liveness analysis, no memoization), evaluates operands left to right one world at a time, and computes weights as exact fractions.

Its tests generate thousands of programs from that part of the language, run each one through both implementations, and require the same evidence, the same unnormalized weight for every value at every report, or an error from both. The engine runs with merging and memoization on and off. A hand-written corpus covers each rule above, and a fuzzing test feeds mutated programs to the parser, the compiler and the engine under small limits, which must never crash (section 11).

Sampling (section 14) is checked against enumeration on the same generated programs: every estimate must be within six standard errors of the exact value, and the standard errors must be calibrated (across thousands of estimates, the mean squared error in standard errors is close to 1). The samplers of section 13 are checked against their CDFs with Kolmogorov–Smirnov and chi-square tests.

## 13. Continuous distributions

`normal(mean, sd)`, `lognormal(mu, sigma)`, `uniform(lo, hi)`, `beta(a, b)`, `gamma(shape, scale)`, `exponential(rate)`, `triangular(lo, mode, hi)` and `pert(lo, mode, hi)` are continuous distributions. Two more describe an estimate by its 90% interval:

- `a to b` is the lognormal whose 5% and 95% quantiles are `a` and `b`. Both must be positive, as in Squiggle.
- `normal_range(lo, hi)` is the normal whose 5% and 95% quantiles are `lo` and `hi`, for quantities that can be negative.

A continuous distribution is a `dist[float]`, and a recipe like any other distribution (section 2). What can be done with one:

- **Draw from it** with `~`, when sampling (section 14). Enumeration can't list its outcomes, so a continuous draw is an error there.
- **Compare it with a number.** `normal(0, 1) > 1.96` is a `dist[bool]`, true with probability 1 − F(1.96), where F is the distribution's CDF; this works in both modes. `==` is never true.
- **Ask about it.** `mean`, `sd`, `variance`, `median`, `quantile`, `cdf` and `pdf` use the formulas.
- **Choose among them.** A choice with continuous options, such as `one_of([normal(0, 1), 5])` or `if 35% { 1 to 3 } else { 0 }` used as a value, is a mixture: drawing from it chooses an option with its probability, then draws from that option.
- **Use it as evidence.** When sampling, `observe v from D` with a continuous `D` multiplies the weight by D's density at `v`, which can be more than 1.

Anything else, such as arithmetic (`normal(0, 1) * 2`) or comparing two continuous distributions, is an error for now: draw a value first (`let x ~ normal(0, 1)`), then compute with it.

## 14. Sampling

`@mode sample(runs: n, seed: s)` estimates the same model as sections 1–13 by following `n` random paths through the program, called **runs**, instead of every path.

- **A run is a single world.** Where enumeration would split a world (`if`, `chance`, `~`, taking a card, calling a function), a run takes one branch, chosen with that branch's probability, and its weight doesn't change. A `dist[bool]` condition picks a branch with the probability that it's true, without drawing the distribution.
- **Weights come from evidence.** `observe` multiplies a run's weight as in section 7 (likelihood weighting). A run that is ruled out has weight zero.
- **Runs are independent.** They don't merge, and calls aren't memoized, so every call makes its own choices. A function may call itself with the same arguments: each call chooses its own path. Unbounded loops aren't cut short; the iteration limit still applies.
- **`simulate` is enumerated.** Inside each run, a `simulate` block is computed exactly, by enumeration, as in section 8, so its result has no sampling error. A block that enumeration can't compute (because it draws from a continuous distribution, say) is an error, even when sampling: estimates inside estimates aren't supported yet.
- **Reproducible.** The same program, seed and version of Probl give the same output on any machine, with any number of threads. Runs go in batches of 1,000, each with a random stream of its own, derived from the seed and the batch's number. Batches may run at the same time, but they're combined in order: the estimates, what `print` shows and the first error are those of running them one after another. Only whether a run reaches a host's limit on work, which the threads share, can depend on timing.
- The missing mass of infinite discrete distributions (below 10⁻¹⁸, section 10) is ignored.

**Estimates.** Let the runs end with weights w₁…wₙ. A run's weight when it reaches a report is its final weight, since no observation may follow a report (section 7). A report's estimates are averages over the runs, weighted and normalized:

- A **probability** is estimated as p̂ = Σ wᵢ aᵢ / Σ wᵢ bᵢ, where bᵢ is the number of times run i reached the report (0 or 1, except for per-visit reports), and aᵢ adds up, over those times, the probability that the reported fact was true: 1 or 0 for a fact, P(true) for a `dist[bool]`. A reported finite distribution isn't drawn: each of its outcomes counts with its probability. (A continuous one can't list its outcomes, so it's drawn once.) The probabilities of other values are estimated the same way.
- Its **standard error** is √(Σ wᵢ² (aᵢ − p̂ bᵢ)²) / Σ wᵢ bᵢ (the delta method; with equal weights, this is √(p̂(1 − p̂)/n)). Probabilities are printed with it, rounded to its precision: `46.1% ± 0.3%`.
- A **mean** is estimated the same way, with aᵢ adding up the reported values, and printed with its standard error when that shows at the printed precision. Standard deviations and quantiles are those of the weighted runs, with no error estimate.
- A report's **reach** is the weight of the runs that reached it, divided by the weight of all runs.
- The **effective sample size** (Σ wᵢ)² / Σ wᵢ² is printed when observations made the weights unequal. It says how many equally weighted runs the estimates are worth; when it's small, the estimates and their standard errors are unreliable.

Sampling doesn't estimate the evidence yet. If every run is ruled out, it's an error: the evidence is impossible, or too unlikely for this number of runs.

## 15. Data

`let name: T = read(path)` binds data read from outside the program, such as a CSV or JSON file ([reading data](data-input.md)).

- **Before running.** The data is read before the program runs, as a value of the declared type `T`. Data that doesn't fit `T` is an error, and the program doesn't run.
- **A constant.** The value is the same in every world and every run, as if it had been written in the program. Reading it splits nothing and weighs nothing: it isn't evidence. To condition on data, `observe` it.
- **Part of the input.** The same program, data and seed give the same output.

## 16. Not specified yet

- **Particles, beam search and merged runs** (audit D4): how merged samples keep their statistical bookkeeping, and when particles resample.
- **Nested estimates** (D4): `simulate` blocks that must be sampled, and how their error affects decisions.
- **Evidence estimates** (D3): the evidence when sampling, and log-evidence for densities.
- **Arithmetic on continuous distributions**, beyond comparing them with numbers.
- **Recursion that returns to the same call, when enumerating** (D5): currently an error; solving such systems as Markov chains is future work.
- **Reports that update with later evidence** (filtering and smoothing).
