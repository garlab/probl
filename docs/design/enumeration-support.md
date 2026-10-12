# Extending enumeration beyond explicit worlds

> October 12, 2026, at `0.2.2`. What enumeration computes without listing a world for every outcome, how, where it stops, and what's next. It replaces the survey and phased plan of October 9 (revision `f12e583`), which are in the history; the inventory keeps its letters, A–R, which the [capability corpus](../../crates/probl-engine/tests/capabilities.rs) uses.

## Where it stands

Enumeration is exact, and doesn't need a world for each outcome of a continuous draw, a count law, a loop's rounds or a set of live facts:

- A continuous draw is an **analytic outcome**: a function of the draw, piecewise and monotone, that arithmetic, comparisons, `abs`, `min`, `max`, rounding, squares, roots, exponentials and logarithms keep exact. Events of independent draws combine, sums of normal draws are joint normals, and observations update them in closed form.
- **Conjugate observations** update a draw's posterior, with its identity, through aliases and branches. With `@integrate`, other observations of one draw are integrated numerically, to a stated tolerance.
- **Densities** weigh finite worlds, with evidence in log units.
- **`simulate`** blocks hold continuous draws, and give distributions whose outcomes own their draws.
- **Count laws** answer `pmf` and `cdf` exactly, and laws too broad to list can be sampled.
- **Loops** that cycle are solved as Markov chains, also when they count their rounds or add to a total.
- An experimental **symbolic backend**, off by default, answers Boolean programs with decision diagrams.

The [capability corpus](../../crates/probl-engine/tests/capabilities.rs) runs 79 programs in both modes: 12 controls, 3 programs the language rejects, 4 recipe operations and built-ins that aren't implemented in either mode, and 60 programs that each name the inventory item below that changes them. Of those 60, enumeration runs 35. The other 25 are the next steps, and sampling runs 23 of them. `PROBL_CAPABILITIES=print cargo test -p probl-engine --test capabilities` prints what each does now.

There's still no universal extension: infinite output, arbitrary loops, difficult integrals and exponentially large dependencies remain possible. The objective stays to support useful classes of models and queries, with explicit boundaries.

## Contracts

Agreed before the first phase, and kept by every phase since:

- **A capability that isn't built is an `Unsupported` error.** It's never a fault: no `catch` takes it, and partial mode doesn't end only its world, so the run stops. Its help suggests sampling only where sampling runs the program, and enumeration never falls back to sampling by itself.
- **Faults on part of a domain fault that part.** An operation whose input has a region of positive probability where it faults fails that region as a world would fail: the run stops in total mode, the region's probability is failed weight in partial mode, and a matching `catch` continues the region. An operation that doesn't separate its fault region yet stays `Unsupported` for inputs that reach it. A region of probability zero doesn't fault, by the equality-to-a-point convention.
- **Analytic results are exact up to floating point.** Formulas and numerical CDF inversion are reported as complete. A method that approximates beyond rounding, like quadrature, reports its error status and is opt-in; it's never shown as an exact answer.
- **Densities keep their units.** Evidence that includes a density is labelled as one and kept as its logarithm. Unresolved-weight bounds hold only for probability likelihoods: a capability that applies densities where weight is unresolved must bound the missing likelihood, or reject the program.

## How it works

| Item | What enumeration does | Code |
|---|---|---|
| A | Densities weigh finite worlds | `likelihood` in [interp.rs](../../crates/probl-engine/src/interp.rs) |
| B–E, N | Analytic outcomes of one draw: pieces and kernels | [analytic.rs](../../crates/probl-engine/src/analytic.rs) |
| F, I | Events of independent draws; linear forms of normal latents | [analytic.rs](../../crates/probl-engine/src/analytic.rs) |
| G, H | The posterior context; conjugate updates, and branches on a drawn probability | [analytic.rs](../../crates/probl-engine/src/analytic.rs), [conjugate.rs](../../crates/probl-engine/src/conjugate.rs) |
| J | Opt-in numerical integration of one draw's likelihood | [likelihood.rs](../../crates/probl-engine/src/likelihood.rs), [quadrature.rs](../../crates/probl-engine/src/quadrature.rs) |
| K, L | Joint recipes from `simulate`; structured reports | [joint.rs](../../crates/probl-engine/src/joint.rs), [report/results.rs](../../crates/probl-engine/src/report/results.rs) |
| O | Count laws | [dist.rs](../../crates/probl-engine/src/dist.rs) |
| P | Rewards of loop chains | [rewards.rs](../../crates/probl-sema/src/rewards.rs), [chain.rs](../../crates/probl-engine/src/chain.rs) |
| Q | Decision diagrams (experimental) | [symbolic.rs](../../crates/probl-engine/src/symbolic.rs) |

**Analytic outcomes (B–E).** An outcome is a function of one draw, in pieces over the draw's CDF coordinates; each piece is `outer(kernel(inner(x)))`, two affine functions around one of six monotone kernels: identity, square, square root, exponential, logarithm, reciprocal. Squares split at their vertex and reciprocals at their pole, so every piece is monotone, and comparing one with a number inverts its kernel. `abs`, `min`, `max`, `clamp` and the extrema of a list are one partition: one function where an event holds, another elsewhere. A constant piece is an atom, which CDFs, quantiles and moments include. Rounding splits pieces at the steps and gives an int; assigned, it splits the world, a world per value with the draw restricted to its part. Built-ins that move values without looking at them, `sum` and `mean` of a list of outcomes of one draw, and `get` with a settled key keep outcomes. Moments come from closed forms per family and kernel, or with `@integrate` from quadrature, or are unavailable; quantiles always come from inverse images. An outcome has at most 100,000 pieces.

**The posterior context (G, H).** A world's restriction on a draw is a `Latent`: the draw's distribution in that world and the intervals in its coordinates. Values carry the coordinates they were made in, and reading one moves it to the world's, so stored events and restrictions keep meaning the same values after an update. The compiler's recognition of conjugate observations serves enumeration as well as the sampler: `binomial`, `bernoulli` and `score` with a beta, `poisson` with a gamma, `normal` with a normal mean, including a mean affine in the draw. A restricted draw stays restricted, and the evidence includes the ratio of the updated family's mass to the old one's. `if p`, `~bernoulli(p)` and `~binomial(n, p)` on a beta `p` split by outcome, each with its weight and its posterior.

**Events and joint normals (F, I).** An event is a union of disjoint boxes over independent axes, at most 10,000 of them; observing or branching on one splits a world per box. An axis can be a linear form of independent normal latents, so sums and differences of normal draws, and draws whose mean is one, are normal outcomes that the one-draw machinery works on unchanged. A normal observation of a form updates its latents jointly: each becomes its posterior mean plus a form of fresh standard normal latents, from a closed-form square root of the posterior covariance, so the world still holds only independent latents.

**Densities (A).** `observe v from D` with a continuous `D` multiplies a world's weight by its density, kept in logarithms. Each world counts its densities, worlds merge only with as many, and finished worlds and reports must agree, or the program is rejected; unresolved weight with densities is rejected too.

**Numerical integration (J).** With `@integrate` (`--integrate`, `Options::integrate`; tolerance 1e-9 by default), an observation of one draw that isn't conjugate becomes a likelihood factor over the draw's coordinates, and every probability, quantile and moment goes through one weighted measure, integrated by adaptive Gauss–Kronrod. An integral that misses the tolerance is `Unsupported` for an observation and unavailable for a moment. The summary says `integrated to 1e-9`, and `Outcome::integration()` gives the tolerance.

**Local models and structured reports (K, L).** Continuous draws work inside `simulate`. A block of events gives a distribution of truth values; a block whose values hold outcomes gives a `Value::Joint`, whose outcomes own their latents and get fresh ones at each draw, so draws are independent and one outcome's fields stay correlated. Blocks can observe densities, normalized away. Reports of records and lists of outcomes summarize each field (`Group::fields`).

**Count laws (O).** A listed `binomial`, `poisson` or `geometric` keeps its law, which `pmf` and `cdf` use. A law too broad to list is a `Value::Counts`: sampled directly, with exact moments, and exact queries for a geometric.

**Rewards (P).** A variable that an unbounded loop only adds to, and reads nowhere else, is a reward: the chain is solved over the other variables, and carries how much each step adds. Results are unrolling's, much faster: example 05 in 18 ms instead of 159 ms. A loop whose worlds can never leave is an error at once.

**Decision diagrams (Q).** Experimental, off by default: [stage 1 of symbolic inference](symbolic-inference.md#stage-1-as-built). Boolean programs, with draws from constant laws or laws chosen by facts, `if`, `observe`, `score` and reports without `by`, are answered by weighted model counts over binary decision diagrams, filled into the same report sinks, so results are enumeration's. Facts that stay live through evidence cost diagram nodes rather than worlds: 18 faults in 0.37 ms instead of 796 ms.

## What's left

Each row is a gap the corpus records, with a direction and a first estimate in person-days, a judgment rather than a measurement.

| Item | Not yet | Direction | Effort |
|---|---|---|--:|
| A | Densities inside functions and `simulate`; densities with unresolved weight | Carry the density count in call results; a likelihood envelope to bound unresolved weight | 4–8 |
| D | Rounding a draw without bounds (`floor` of a normal); `round(x, digits)`; a rounded outcome used before it's assigned; quantiles of a Poisson or binomial too broad to list | A lazy discrete law, or an unresolved tail; accurate incomplete gamma and beta functions | 5–10 |
| E | Trigonometry; powers other than 2, ½ and −1; compositions like `exp(x * x)` | More kernels, and pieces that compose; periodic functions on bounded ranges first | 8–15 |
| F | Events of draws that depend on each other (`x > 0 and x + y > 0`) | Joint normal probabilities of rectangles | 10–15 |
| G, H | Affine parameters of other likelihoods without `@integrate` (`bernoulli(1 - p)`); a drawn weight in `chance`; `prob(p)` as a value | Run-time recognition of affine likelihoods, as mixtures of updates | 6–10 |
| I | Conditions on sums (`observe x + y > 0`); products of draws; sums with other families; normal observations inside functions; a drawn standard deviation | Truncated multivariate normals; closures for gamma sums and uniform convolutions | 15–30 |
| J | Several dimensions; `~bernoulli(p)` for a weighted `p`; error estimates per quantity; integration when sampling | Low-dimensional cubature, after factorization | 20–40 |
| K | `simulate` capturing an outer analytic draw; arithmetic between `simulate` distributions; `support` and `pmf` of them | A conditional-model contract, then capture-dependent normalization | 15–25 |
| L | Records of outcomes reported with other values | A result schema for mixed structured groups | 3–5 |
| M | Sorting, membership and aggregate equality on outcomes | Finite partitions by comparisons, within budgets | 8–15 |
| N | `str` and `print` of outcomes; grouping by a continuous outcome | A debug rendering policy; explicit bins | 2–4 |
| O | Enumerating a draw from a law too broad to list; `median` of one | Analytic count outcomes with thresholds and closed convolutions | 8–15 |
| P | Rewards that read rewards or themselves; exact reward moments; phase-type queries; loops with analytic state; recursion that prints | `(I − Q)m = r` for the moments a report asks for; phase-type laws | 15–40 |
| Q | Diagrams beyond Booleans: finite values, loops, calls, faults; handing over to worlds; automatic selection | [Symbolic inference](symbolic-inference.md), stages 2 and 3 | 40–80 |
| R | Faults on part of an analytic domain (`sqrt` of a draw that can be negative), with `catch` | Split the fault region, as the contract says | 4–8 |

Four recipe operations and built-ins aren't implemented in either mode, and aren't enumeration features: arithmetic on an undrawn continuous recipe (`normal(0, 1) * 2`; an affine wrapper is 4–7 days), `mixture` (1–3 days once its contract is chosen, since `one_of` and `chance` already make mixtures), `truncate` (3–6 days for supported families) and `bins` (3–5 days for explicit finite bins). Some restrictions stay on purpose: `support` of a continuous law, `mean(x)` of a drawn scalar, stochastic collection callbacks, and scalar queries of a distribution with an unresolved tail.

## How it's checked

- **The corpus** classifies each program in both modes, and checks that no error suggests sampling for a program sampling can't run. Building a capability flips its rows to `Works`, with the capability's own tests.
- **Independent references:** closed forms and separately derived formulas for analytic results ([analytic.rs](../../crates/probl-engine/tests/analytic.rs), [joint.rs](../../crates/probl-engine/tests/joint.rs)); a 400,000-point midpoint rule for integration ([integrate.rs](../../crates/probl-engine/tests/integrate.rs)); the rational oracle for finite programs, including generated Boolean programs answered by diagrams ([differential.rs](../../crates/probl-oracle/tests/differential.rs)); sampled estimates as a calibrated cross-check.
- **Identity, order and faults:** aliases in lists, records and closures; two draws against one; observations in either order; faults in total and partial mode and in catches; merging, memoization and loop solving off.
- **No regression,** in correctness and in time and memory. Each phase was compared with the commit before it: `probl-bench --json` twice for each build, alternating, compared by `probl-bench compare` at a 5% threshold above noise floors of 2 ms and 64 KB, and from phase 2 on with one codegen unit and fat LTO.

| | Outputs and world steps | Total time |
|---|---|---|
| Phase 0: the corpus, reclassified errors | identical for all 31 models | 66.93 s → 66.97 s (+0.1%) |
| Phase 1: A, B, C and event keys (N) | identical | 67.01 s → 67.01 s (−0.0%) |
| Phase 2: G and H | identical | 67.41 s → 66.85 s (−0.8%) |
| Phase 3: D and O | identical | 66.90 s → 66.93 s (+0.0%) |
| Phase 4: E | identical | 66.96 s → 67.06 s (+0.1%) |
| Phase 5: F and I | identical | 67.01 s → 67.00 s (−0.0%) |
| Phase 6: K and L | identical | 66.89 s → 66.78 s (−0.2%) |
| J | identical | 67.11 s → 67.08 s (−0.0%) |
| P | identical; fewer world steps for the 4 examples whose loops count | 67.51 s → 66.84 s (−1.0%) |
| Q, off by default | identical; 3 new models | 66.82 s → 66.81 s (−0.0%) |

What the comparisons taught, for the next ones:

- **Compare with one codegen unit and fat LTO.** With the release profile's 16 units, how the compiler splits the engine depends on its types: adding a field to a type the models never use made sampled models 5% slower in the same instructions.
- **Keep rare paths out of the hot ones.** Code inlined into `exec_stmt_kind` grew the frame that every statement runs in; generic lifting functions that recursed were copied out of line 14 times; arithmetic tested each operand for three unusual kinds one by one. Each cost a few percent, and was fixed by cold functions and one test for all the unusual kinds.
- **A slowdown in cycles but not in instructions is layout.** Sampled models on 12 threads in the benchmark binary moved by 6–8% between builds that retire the same instructions within 0.5% on one thread, and didn't move through the command line. Check with `/usr/bin/time -l` before changing code.
- **Small models show memory per statement.** A table per statement raised the smallest models' heap 1–4%; a map of the few statements that need it didn't.

## Relevant prior art

These are directions to borrow, not evidence that another implementation can be imported unchanged.

| Work | Relevance |
|---|---|
| [Delayed sampling](https://proceedings.mlr.press/v84/murray18a.html) | Analytic substructure and conjugacy tracked through a graph (G, H); enumeration also has to continue analytically where a sampler would draw. |
| [PSI](https://psisolver.org/) | Symbolic inference for mixed discrete and continuous programs; its finite-loop assumption is a boundary to keep in mind. |
| [Hakaru](https://hakaru-dev.github.io/) | Program transformations for expectations, normalization and disintegration; relevant to conditional local models (K). |
| [Genfer](https://arxiv.org/abs/2305.17058) and [generating functions for probabilistic programs](https://arxiv.org/abs/2007.06327) | Exact queries over infinite-support discrete models, and phase-type laws (O, P). |
| [PRISM rewards](https://www.prismmodelchecker.org/manual/PropertySpecification/Reward-basedProperties) | Accumulated costs and hitting times; an expected reward isn't a full law (P). |
| [Weighted model integration](https://proceedings.mlr.press/v115/zeng20a.html) | Structure in mixed discrete and continuous integration, beyond independent intervals and Gaussian closures. |
| [Dice](https://arxiv.org/abs/2005.09089) | Weighted model counting for discrete structure (Q), assessed in [symbolic inference](symbolic-inference.md). |
| [GSL quadrature](https://www.gnu.org/software/gsl/doc/html/integration.html) | Adaptive quadrature and its error-estimate contracts (J). |
