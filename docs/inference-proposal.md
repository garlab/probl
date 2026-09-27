# Proposal: better inference for forecasts with data

> Revised September 2026 after the [review](inference-proposal-review.md), which found part 1 sound and part 2 underspecified. Part 1, exact updates for conjugate priors, is specified here and built first. Part 2, a general method, is deferred: its specification needs the four things listed in section 2 before anything is built. This is the next step of the [implementation plan](implementation-plan.md) (section 9), and answers the [project audit](project-audit.md)'s finding D4 for what's built.

## The problem

Sampling today is likelihood weighting. Each run draws every unknown from its prior and is weighted by how well that explains the data. As data accumulates, fewer and fewer runs carry the weight:

| Model | Runs | Worth (effective sample size) |
|---|--:|--:|
| `benches/ab_test`, 30 days of data | 100,000 | 852 |
| the same, 120 days | 100,000 | 211 |
| example 08, 14 days of sign-ups | 200,000 | 30,933 |

With more unknowns it collapses exponentially. The evidence estimate stays unbiased, but posterior probabilities and means are ratios of weighted sums, which are biased at any finite number of runs. When few runs carry the weight, both the estimates and their standard errors become unreliable (semantics §14), and the tails drift first: in `ab_test`, the 5% quantile of the lift is 5.92 against the exact 6.34. Now that models read real data (`read`), this is the main limit on forecasting with Probl.

## Decisions

The review settled the first draft's open questions:

1. **Exact updates are on by default when sampling**, once differential tests pass, with an explicit opt-out for diagnosis and benchmarking: `probl run --no-conjugate`. `--stats` shows, for each variable, how many of its draws were delayed, how many observations updated it exactly, and how many times it was drawn.
2. **The general method waits for a concrete benchmark.** A non-conjugate model is chosen first, and the method is specified against it, whether trace Metropolis–Hastings, particles with moves, or something else. Particles wait for a model with a continuous hidden state that changes over time.
3. **When it comes, the mode is called `mcmc`**, and its output names the actual algorithm. Warmup and retained draws are separate settings with explicit counts, and all tuning stops after warmup. Default counts are starting settings, not evidence of convergence.
4. **Part 1 ships alone**, without committing to a design that combines it with MCMC.

## 1. Exact updates for conjugate priors

### What happens

Take the A/B test:

```probl
let a ~ beta(2, 50)
for k in a_signups {
  observe k from binomial(visitors, a)
}
report (b - a) / a * 100 as "B's lift over A (%)"
```

When sampling, `let a ~ beta(2, 50)` doesn't give `a` a number yet. Internally, `a` keeps the distribution `beta(2, 50)` as a *delayed* value. Then:

- **A conjugate observation updates it.** `observe k from binomial(visitors, a)` doesn't draw `a`. It multiplies the run's weight by the probability of `k` with `a` unknown, the beta-binomial probability. It then replaces `a`'s distribution with the posterior, `beta(2 + k, 50 + visitors - k)`. Both are given by formulas, exact up to rounding.
- **Anything else draws it.** The first time the program needs `a`'s value for anything else, like `(b - a) / a` here, the run draws `a` from its current distribution, the posterior so far. From then on `a` is an ordinary number.

In `ab_test`, every run then ends with the same weight: the probability of the data, which doesn't depend on the run. The effective sample size is the number of runs, 100,000 instead of 852. Every run's `a` and `b` are independent draws from the exact posterior, and the evidence estimate is the exact evidence. The reports still have Monte Carlo error: they average over 100,000 random draws of `a` and `b`.

This is *delayed sampling* ([Murray et al., 2018](https://proceedings.mlr.press/v84/murray18a.html)), limited to one level: a delayed variable's parameters are numbers.

### Conjugate pairs

With Probl's parameters, `beta(α, β)`, `gamma(shape, scale)` and `normal(mean, sd)`:

| Prior of `x` | Observation | Posterior | The weight multiplies by |
|---|---|---|---|
| `beta(α, β)` | `observe k from binomial(n, x)` | `beta(α + k, β + n − k)` | the beta-binomial probability C(n, k) B(α + k, β + n − k) / B(α, β) |
| `beta(α, β)` | `observe v from bernoulli(x)`, or `observe bernoulli(x)` (v is `true`) | `beta(α + 1, β)` if v is `true`, `beta(α, β + 1)` if `false` | α / (α + β), or β / (α + β) |
| `gamma(s, θ)` | `observe k from poisson(x)` | `gamma(s + k, θ / (1 + θ))` | the negative-binomial probability Γ(s + k) / (Γ(s) k!) · (θ / (1 + θ))ᵏ · (1 + θ)⁻ˢ |
| `normal(μ, σ)` | `observe y from normal(x, τ)` | `normal(μ + g (y − μ), σ τ / √(σ² + τ²))`, with g = σ² / (σ² + τ²) | the normal density of `y`, with mean μ and standard deviation √(σ² + τ²) |

Other pairs can come later (gamma–exponential, Dirichlet–categorical, normal with unknown variance), each with its tests.

### The rules

- **Which draws are delayed.** When sampling, outside `simulate`, a draw `x ~ D` into a whole variable is delayed when two conditions hold:
  - the same function has an observation of `x` in one of the forms above, which the compiler finds;
  - `D`'s value is a single beta, gamma or normal distribution. A mixture, or another family, is drawn as before.

  A program without such observations runs exactly as before, with the same random numbers.
- **Delayed values are internal.** A delayed variable isn't a `dist` value, and no program can see one. The compiler lists the statements that read each variable that may be delayed, and the engine draws the variable before running any of them.
- **Its parameters are fixed at the draw.** `beta(α, β)` is evaluated and checked where it's drawn. A later change to `α` doesn't matter, and an invalid parameter is an error there, as when drawing.
- **What counts as a conjugate observation.** The variable is the parameter in the table, written as the plain variable: `binomial(n, a)` counts, `binomial(n, a * 1)` doesn't. It appears nowhere else in the observation. The rest of the observation evaluates to plain values, and they're checked as when drawing: `binomial`'s number of trials must be a whole number of 0 or more, `normal`'s standard deviation a finite number above 0, with the same errors. An observed value that the distribution can't produce, like a count above `n` or a fraction, makes the run impossible, as it would when drawing.
- **Everything else draws the variable first**, and it stays drawn:
  - arithmetic, comparisons, conditions, reports and `print`;
  - copying it (`let c = a`), or putting it in a collection;
  - passing it to a function, or capturing it in a lambda or `simulate`. A function can't update its caller's variables, so a delayed variable is drawn before any call that can read it;
  - an observation that isn't conjugate for its current distribution, like `observe k from binomial(n, a * b)`, or a normal prior observed through a binomial.
- **A type annotation doesn't draw it** when every value of the distribution has the declared type: after `let a: prob ~ beta(2, 50)`, `a` stays delayed. Otherwise, as in `let r: prob ~ gamma(2, 1)`, it's drawn and checked right away, as before.
- **A variable that's assigned or goes out of use while delayed is dropped.** Nothing needed its value.

### What stays the same, and what changes

- **The model is the same.** Drawing `a` from its posterior after the observations gives the same joint distribution as drawing it from the prior and weighting by them. Every report and the evidence estimate the same quantities, with section 14's estimators and standard-error formulas: runs are still independent, and nothing merges.
- **The random numbers change.** For a program with delayed draws, a seed gives different random numbers than it did before.
- **The estimates' variance and finite-sample bias change.** Exact updates preserve the target distribution and reduce the reliance on importance weights. When every observation is an exact update and nothing else random affects the weights, every run ends with the same weight. The effective sample size is then the number of runs, and the evidence estimate is exact: its standard error is 0, up to rounding. Report estimates still have Monte Carlo error, estimated per run as before.
- **Some limits of drawing don't apply.** An exact update doesn't draw `x` or list the observed distribution's outcomes. So it never reaches the limits that drawing would, such as `poisson`'s rate above 10¹⁵, or listing a binomial too spread out to draw directly.

### Numbers

- **Logarithms.** Marginal probabilities and densities are computed as logarithms and applied to the weight in that form. A weight has an extended exponent, but a factor that underflowed to 0 as a floating-point number couldn't be recovered. For `p ~ beta(1000, 1000)` observed as `0 from binomial(100_000, p)`, the log probability is −4234.10, which a floating-point number can't hold once exponentiated. The run keeps its tiny weight, and the posterior is `beta(1000, 101_000)`.
- **Stable formulas.**
  - Log beta functions use the Stirling series for large arguments, not differences of log gammas, which cancel.
  - The normal update uses √(σ² + τ²) computed without overflow, and the gain form of the posterior mean.
- **Every constant counts.** Observations update one at a time, never aggregated, so the evidence keeps every normalizing constant, like the binomial coefficients.

### Building it

- **`probl-sema`:** an analysis of the lowered program. It finds the variables whose draws may be delayed, the conjugate observations, and for every statement the delayable variables it reads.
- **`probl-engine`:**
  - the delayed value, internal to the engine;
  - drawing it before a statement that reads it, and at a type check it may fail;
  - the updates, in log form, with their own module and tests;
  - statistics per variable;
  - the option to turn it off.
- **`probl-cli`:** `--no-conjugate`, and the statistics in `--stats`.

### Tests

The review's acceptance checks, for part 1:

- **Formulas:** posteriors and marginals against closed forms and numerical integration, including extremes: the −4234.10 case, long sequences of observations, impossible counts, and densities above 1.
- **Evidence:** a model whose observations are all exact updates gives the analytic evidence of the whole data set, with every run weighted the same.
- **When variables are drawn:** each use in the rules draws the variable, and an annotation doesn't. Parameters are fixed at the draw. Invalid parameters and observations give the same errors as without exact updates.
- **Agreement:** the same estimates as `--no-conjugate`, within their standard errors, on models where both work.
- **Calibration of the reports:** over many seeds, the reports' errors against the exact posterior are calibrated (mean z² near 1), although every run has the same weight.
- **Simulation-based calibration:** for data simulated from the prior, the engine's estimate of the posterior probability that the parameter is below its true value is uniform. This checks the whole posterior, not a few numbers.

## 2. A general method: deferred

Models that aren't conjugate still need a method whose runs aren't independent draws from the prior. They include lognormal priors, `a to b` estimates, hierarchical models and regressions. The first draft proposed lightweight Metropolis–Hastings over a run's random choices ([Wingate et al., 2011](https://proceedings.mlr.press/v15/wingate11a.html)), in an `@mode mcmc(…)`. The review found that it described a family of algorithms rather than one. Before a general method is built, its specification needs:

1. **A target and every term of the acceptance ratio** (F1):
   - the unnormalized density of a run's choices and observations;
   - site selection when the number of sites changes;
   - value proposals, with their Jacobians on transformed scales;
   - regenerated choices, and reused choices rescored when their distribution changes.

   Every source of randomness needs an address that stays the same through loops, calls, recursion and moved draws. That covers `~`, but also probabilistic `if`, `chance`, bag draws and direct count draws. Detailed balance is checked exactly on tiny models whose runs make different numbers of choices.
2. **Its state, with exact updates** (F2). Either MCMC doesn't use exact updates at first, or its target collapses them explicitly. The collapsed target must say how a delayed variable is drawn for output, including in models with no other choices and variables drawn on one branch only.
3. **Reachability and initialization** (F3):
   - Single-choice moves can't get from `(false, false)` to `(true, true)` under `observe a == b`, so hard constraints are either rejected or given moves that connect them, such as whole-run proposals.
   - Finding a first run with positive weight needs a budget, and a diagnostic that tells failing to start apart from impossible evidence.
   - R̂ isn't a substitute for reachability.
4. **Report estimators and their errors** (F4):
   - **Estimators:** the ratio estimators Σaₜ / Σbₜ over retained runs, with a rejection repeating the previous run, and output buffered per run and committed when accepted.
   - **Standard errors:** a long-run variance estimator with minimum batch lengths, not a fixed 20 batches.
   - **Diagnostics tied to quantities:** rank-normalized split R̂, bulk and tail effective sample sizes. They're tested on slow-mixing, short, constant and multimodal cases.

Particles (sequential Monte Carlo) would need the same kind of contract: where to resample when branches observe different things, and errors that account for shared ancestry.

## The audit's checklist (D4), for what's built

1. **Merged samples need statistical bookkeeping.** Exact updates don't merge runs. Runs stay independent, so section 14's per-run standard errors apply.
2. **Nested `simulate` in decisions.** Unchanged: `simulate` is computed exactly, by enumeration, in every mode. A delayed variable is drawn before a `simulate` block captures it.
3. **Resampling and mode changes need boundaries.** Nothing resamples, and there's no `auto` mode. A delayed variable is updated at the observation itself.
4. **An inference context.** The batch runner owns the randomness (a stream per batch), the budgets, and the results' metadata (the effective sample size, the evidence and its error). Exact updates add a second way for an observation to change a run: a weight and a posterior, instead of a weight alone. A general method will get its own context when it's specified.
