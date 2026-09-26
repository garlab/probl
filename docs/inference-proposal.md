# Proposal: better inference for forecasts with data

> Draft, September 2026, for review. Nothing here is implemented. It's the next step in the [implementation plan](implementation-plan.md) (section 9), and answers the checklist of the [project audit](project-audit.md)'s finding D4. The open questions at the end need a decision before it's built.

## The problem

Sampling today is likelihood weighting. Each run draws every unknown from its prior and is weighted by how well that explains the data. It's unbiased, but as data accumulates, fewer and fewer runs carry the weight:

| Model | Runs | Worth (effective sample size) |
|---|--:|--:|
| `benches/ab_test`, 30 days of data | 100,000 | 852 |
| the same, 120 days | 100,000 | 211 |
| example 08, 14 days of sign-ups | 200,000 | 31,244 |

With more unknowns it collapses exponentially. The estimates stay unbiased, but their standard errors stop being reliable (semantics §14), and the tails drift first: in `ab_test`, the 5% quantile of the lift is 5.92 against the exact 6.34. Now that models read real data (`read`), this is the main limit on forecasting with Probl.

The proposal has two parts, to be built in order:

1. **Exact updates for conjugate priors.** No new mode or syntax, and no change to what sampling estimates. Only the variance changes, and in the common cases it goes to nothing.
2. **Markov chain Monte Carlo (MCMC)** for the other models, as a new mode.

Particles (sequential Monte Carlo) wait for a model that needs them.

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

When sampling, `let a ~ beta(2, 50)` doesn't give `a` a number yet: `a` holds the distribution `beta(2, 50)`, as a *delayed* value. Then:

- **A conjugate observation updates it.** `observe k from binomial(visitors, a)` is conjugate to a beta, so it doesn't draw `a`. It multiplies the run's weight by the probability of `k` with `a` unknown, the beta-binomial probability. It then replaces `a`'s distribution with the posterior, `beta(2 + k, 50 + visitors - k)`. Both are exact.
- **Anything else draws it.** The first time the program needs `a`'s value for anything else, like `(b - a) / a` here, the run draws `a` from its current distribution, the posterior so far. From then on `a` is an ordinary number.

In `ab_test`, every run ends with the same weight: the probability of the data, which doesn't depend on the run. So the effective sample size is the number of runs, 100,000 instead of 852, and every run's `a` and `b` are exact draws from the posterior. The evidence estimate is exact too (standard error 0). Example 08 becomes the same.

This is *delayed sampling* (Murray et al., 2018, as in the Birch language), limited to one level: a delayed variable's parameters are numbers.

### Conjugate pairs

| Prior of `x` | Observation | Posterior | The weight multiplies by |
|---|---|---|---|
| `beta(α, β)` | `observe k from binomial(n, x)` | `beta(α + k, β + n − k)` | the beta-binomial probability of `k` |
| `beta(α, β)` | `observe bernoulli(x)`, and `observe v from bernoulli(x)` | `beta(α + 1, β)`, or `beta(α + v, β + 1 − v)` | α / (α + β), or its complement |
| `gamma(s, θ)` | `observe k from poisson(x)` | `gamma(s + k, θ / (1 + θ))` | the negative-binomial probability of `k` |
| `normal(μ, σ)` | `observe y from normal(x, τ)`, with `τ` a number | normal, with precision 1/σ² + 1/τ² | the normal density of `y`, with mean μ and variance σ² + τ² |

Other pairs can come later (gamma–exponential, Dirichlet–categorical, normal with unknown variance), each with its test.

### The rules

- **When it applies.** Only when sampling. A draw is delayed when its distribution is one of the priors above, with numbers as parameters.
- **What counts as conjugate.** An observation is conjugate only when the variable appears directly as the parameter in the table, like `binomial(n, a)`, and the observation's other parameters are numbers.
- **Everything else is a use.** Any other read of a delayed variable draws it first, and it stays drawn: arithmetic, comparisons, `if`, reports, passing it to a function or a lambda, capturing it in `simulate`, copying it (`let c = a`), or a non-conjugate observation (`observe k from binomial(n, a * b)`). A function can't update its caller's variables, so a delayed variable is drawn before it goes into one.
- **It changes nothing but the variance.** Drawing `a` from its posterior after the observations has the same distribution as drawing it from the prior and weighting by them. Runs stay independent, and nothing merges, so section 14's estimators and standard errors apply unchanged. What changes is which random numbers a seed gives.
- **The evidence** gets the exact marginal probability, or density, of each conjugate observation. Section 14's estimate keeps its meaning, with less variance.
- **Visible in `--stats`:** how many observations were exact updates, and which variables were delayed. It doesn't appear in the summary line, since the estimate means the same thing either way.

### Building it

- `probl-engine`:
  - a delayed value, the family and its parameters, held in the run's variable;
  - the conjugate observation, which updates the variable in place;
  - drawing a delayed variable before any statement reads it, found as the draw-moving pass finds reads;
  - the marginal probabilities and densities, in `continuous.rs`.
- **Tests:**
  - posteriors in closed form for every pair;
  - the effective sample size equal to the runs when everything is conjugate;
  - agreement with plain likelihood weighting on a model where both are feasible;
  - every "use" in the rules drawing the variable;
  - *simulation-based calibration*: draw parameters from the prior, simulate data, infer, and check the ranks of the true parameters are uniform. This checks the whole posterior, not a few numbers.
- About 700 lines with the tests.

## 2. MCMC for the other models

Models that aren't conjugate, like lognormal priors, `a to b` estimates, hierarchical models or regressions, need a method whose runs aren't independent draws from the prior. The proposal is *lightweight Metropolis–Hastings* (Wingate et al., 2011), the general method of WebPPL and early Anglican, in a new mode:

```probl
@mode mcmc(steps: 20_000, chains: 8, seed: 1)
```

### How it runs

- **A chain** is a sequence of runs. Each run is the previous one with one random choice changed. The choice is picked at random, and its new value comes from:
  - a random walk, for a continuous value, on a scale where it's unbounded (log for a rate, logit for a probability), tuned during warmup;
  - its prior, for a discrete value.

  The run is kept or rejected by the Metropolis–Hastings rule, from both runs' weights and prior densities.
- **Addresses.** To change one choice and keep the others, a run's choices have addresses: the statement that drew them, the iteration of each loop around it, and the calls it's in. A run that takes a different path gets new choices from their priors, and the acceptance rule accounts for the choices that appear or disappear.
- **Warmup.** Each chain starts from a run of plain likelihood weighting. Its first steps (a quarter, by default) tune the random walks and are discarded.
- **Chains and threads.** Chains are independent. Each has a random stream of its own, from the seed and the chain's number, so the output is the same with any number of threads, as for batches now.
- **Conjugate updates** (part 1) still apply inside each run, so the chain only moves what can't be integrated exactly.

### What it estimates, and its errors

- **Estimates.** Every run after warmup, in every chain, counts once, with no weights. A report's estimates are averages over those runs.
- **Standard errors** come from batch means. Each chain's runs after warmup are cut into 20 consecutive batches. A probability's or mean's standard error is the standard deviation of the batch averages over √(chains × 20). This accounts for runs within a chain being correlated, which the per-run formula of section 14 doesn't.
- **Diagnostics in the summary line:**
  - R̂ compares the chains' split halves (Vehtari et al., 2021). It must be close to 1: above 1.01, the output says the chains disagree and the estimates can't be trusted.
  - The effective sample size is computed from autocorrelation.
  - The acceptance rate.

  For example: `mcmc · 8 chains × 20,000 steps · seed 1 · R̂ 1.00 · effective sample size 6,480 · accepted 31%`.
- **No evidence estimate.** MCMC doesn't estimate the evidence, and the summary line says so if the program observes.
- **Its limits.** MCMC explores one region at a time. Posteriors with separate modes are the classic failure, and R̂ across chains that started apart is the defence. Discrete choices that many others depend on move slowly.

### Building it

- **`probl-engine`:**
  - addresses for random choices;
  - running a single run with some choices fixed;
  - the proposals and the acceptance rule;
  - the chains' runner, from the batch runner;
  - report accumulators with batch means;
  - R̂ and the effective sample size.
- **Tests:**
  - MCMC against enumeration on the generated programs the oracle checks, with estimates within their batch-mean errors and those errors calibrated, as sampling is checked now;
  - closed-form posteriors;
  - simulation-based calibration;
  - a two-mode posterior, where R̂ must warn.
- About 2,000 lines with the tests.

## Deferred

- **Particles (sequential Monte Carlo).** They suit hidden states that change over time, but discrete ones already work exactly by enumeration: `benches/regimes` is the forward algorithm. They need the synchronization D4 asks for: where to resample when branches observe different things. Particle ancestry also breaks the independence that section 14's errors assume, so their errors would come from independent batches. That waits for a model with a continuous hidden state.
- **Merged sampling, nested estimates and `@mode auto`**, as the plan says: each needs its contract first.

## The audit's checklist (D4)

1. **Merged samples need statistical bookkeeping.** Neither part merges runs. With exact updates, runs stay independent, and section 14's per-run errors apply. MCMC's runs are correlated within a chain, so its errors come from batch means, and its effective sample size from autocorrelation, not from weights.
2. **Nested `simulate` in decisions.** Unchanged: `simulate` is computed exactly, by enumeration, in every mode. A delayed variable is drawn before it's captured.
3. **Resampling and mode changes need boundaries.** Neither part resamples. A chain's step is a whole run, and a delayed variable is updated at the observation itself. There's no `auto` mode.
4. **An inference context.** Each mode gets one object that owns what the batch runner owns now:
   - its randomness (a stream per batch or chain);
   - its budgets (runs, steps, work);
   - how observations change runs (a weight; a weight and a posterior; a weight and an acceptance);
   - the metadata of its results (effective sample size, R̂, acceptance, evidence or not).

## Open questions

1. **Exact updates by default?** The proposal applies them whenever sampling, since they change nothing but the variance. The alternative is to ask for them, with `@mode sample(runs: …, exact: true)`.
2. **The general method: MCMC first (proposed), or particles with MCMC moves?** Particles also estimate the evidence, and cope better with several modes, but they need the synchronization rules above and MCMC moves anyway.
3. **The mode's name and defaults:** `@mode mcmc(steps: 20_000, chains: 8)` (proposed), with a quarter of the steps for warmup. Or name it after the algorithm, `@mode mh(…)`?
4. **Part 1 alone first?** It fixes `ab_test` and example 08, and most forecasts of rates and counts, for a fraction of part 2's work. Part 2 could wait for a model that needs it.
