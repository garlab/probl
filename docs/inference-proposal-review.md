**Review of the inference proposal**

Reviewed on 27 September 2026 against `cf771d2`. This reviews [inference-proposal.md](inference-proposal.md), the current reference semantics, and the interpreter/reporting interfaces. The proposed inference methods are not implemented; findings concern their contracts and implementation plan, not reproduced defects in a new sampler.

**Recommendation: ship part 1 first, with an opt-out for validation. Defer part 2 until its trace target, transition kernel, and report estimators are specified.** The conjugate updates address the measured bottleneck directly. The MCMC proposal currently describes a family of possible algorithms, but not enough detail to establish that the chosen algorithm targets Probl's posterior or produces trustworthy error estimates.

The beta–binomial and gamma–Poisson posterior parameters are correct under Probl's shape/scale convention. The normal predictive variance and posterior precision are also correct. Keeping independent runs for delayed sampling, freezing adaptation after warmup, using independent streams per chain, and declining to promise an MCMC evidence estimate are sound choices. The remaining issues are substantial rather than stylistic.

| ID | Priority | Finding |
|---|---|---|
| F1 | High, before MCMC | Specify a complete trace target and forward/reverse proposal probabilities |
| F2 | High, before combining the methods | Delayed sampling cannot be plugged into replay without defining its latent state and proposal densities |
| F3 | High, before MCMC | Single-choice moves can be reducible; initialization and hard constraints need a contract |
| F4 | High, before MCMC | Preserve conditional/per-visit report estimators and justify their Monte Carlo errors |
| F5 | High, before conjugate updates | Marginal likelihoods need a numerical representation that survives underflow |
| F6 | Medium | Correct the claims about unbiasedness, zero variance, and exactness |

**F1 — The MH rule needs a full trace contract, not just weights and priors.**

The [execution description](inference-proposal.md#L92) mentions weights, prior densities, and appearing/disappearing choices. It leaves unspecified the forward and reverse proposal probabilities that determine acceptance. For a trace `t`, a useful starting contract is:

```text
π̃(t) = product of conditional random-choice densities/masses
        × product of observation factors

accept(t → t') = min(1, π̃(t') q(t | t') / (π̃(t) q(t' | t)))
```

The implementation must specify all of `q`, including the probability of selecting a site, its value proposal, and every regenerated choice. Uniform selection among a changing number of sites generally contributes an old/new site-count correction. Reused choices must be rescored when their conditional distributions change. A log-space or logit-space random walk also needs the appropriate Jacobian or the equivalent proposal-density correction in the original coordinates. A symmetric walk after transformation is not generally symmetric before transformation.

The cited [Wingate et al. trace-MH paper](https://proceedings.mlr.press/v15/wingate11a.html) is the relevant foundation, but the proposal should give the concrete specialization for Probl and test its balance equations.

There is an additional Probl-specific requirement: random choices are not confined to `~`. For example:

```probl
let x = if 50% { true } else { false }
observe if x { 90% } else { 10% }
report x
```

This model has a 90% posterior probability for `x`, despite containing no draw statement. The current interpreter samples [probabilistic `if`](../crates/probl-engine/src/interp.rs#L485), `chance`, bag draws, and direct count draws through separate paths. All inference-relevant stochastic choices need addresses and replay/scoring behavior. Merely addressing “the statement that drew them” risks missing some paths. Reporting also introduces auxiliary randomness for continuous distribution values, which needs a deliberate treatment separate from latent model choices.

**Suggested change:** define a trace-choice interface with a stable address, distribution/base measure, current value, conditional score, and forward/reverse proposal scores. Specify address identity through loops, repeated calls, recursion, and compiler draw movement. Specify behavior when a reused address changes distribution family or support. Route all model randomness through this interface. Initially restrict unsupported mixtures or dynamic cases explicitly rather than calling the first implementation general-purpose.

Before statistical tests, check detailed balance exactly on tiny finite models with changing trace lengths and parameter-dependent choice probabilities. This will catch acceptance-ratio errors more directly than approximate agreement on a few posterior summaries.

**F2 — Define the collapsed MCMC state before retaining conjugate updates inside it.**

The [conjugacy rules](inference-proposal.md#L60) materialize a delayed variable at its first ordinary use. The [MCMC section](inference-proposal.md#L100) then says conjugate updates still apply and the chain moves only what cannot be integrated. Those statements leave open whether a subsequently materialized variable belongs to the chain state, is regenerated during replay, or is drawn only to produce output.

Consider:

```probl
let p ~ beta(1, 1)
observe true from bernoulli(p)
report p
```

The conjugate observation contributes marginal likelihood `1/2`; the materialized report value comes from `beta(2, 1)`, whose density is `2p`. This is a fully collapsed model with no latent MH site left before reporting. Treating the report draw as though it were generated from the original uniform prior would score the wrong proposal or target. Doing nothing because there are no sites would fail to produce the intended posterior samples.

A harder case forces `p` before a later non-conjugate observation, or forces it only on one side of a branch. A trace can then contain an explicit `p` on one execution and integrate it out on another. The generic rule that new choices come from their priors is no longer a complete description: a delayed variable can be generated from an updated conditional distribution.

**Suggested change:** choose one of two staged implementations. The simplest first trace-MH engine can disable delayed sampling, while `sample` retains it. Alternatively, define a collapsed trace target explicitly: integrate eligible variables in the target, reconstruct output-only variables conditionally after each retained transition, and treat earlier forced values with their correct conditional proposal and target terms. Include fully collapsed models, branch-dependent materialization, and rejected transitions in the derivation and tests.

Keep delayed state internal. A delayed `float` or `prob` must not become a language-visible `dist` recipe, which would reintroduce the event/distribution confusion already resolved in the semantics. Annotation checks should validate its eventual scalar type without accidentally forcing every annotated parameter immediately. The [delayed-sampling paper](https://proceedings.mlr.press/v84/murray18a.html) motivates the analytical updates; it does not by itself supply the proposed hybrid MH kernel.

**F3 — Single-choice MH needs a reachability and initialization strategy.**

The [proposed single-choice moves](inference-proposal.md#L92) can fail more severely than slow exploration of separate modes:

```probl
let a ~ bernoulli(50%)
let b ~ bernoulli(50%)
observe a == b
report a
```

The posterior has two equally probable states: `(false, false)` and `(true, true)`. Changing just one choice makes the observation false, so every move between them is rejected. Each chain stays in its initial state forever. More steps and random-walk tuning cannot fix this reducible kernel. Averaging independent initializations also does not generally recover posterior mode weights; those initializations are drawn from the prior, not the posterior.

In addition, [starting from one likelihood-weighted run](inference-proposal.md#L98) can produce a zero-weight trace. The proposal needs to define how a valid initial state is obtained and what happens when none is found within the host's budget.

**Suggested change:** explicitly support or reject hard-constrained models in the first release. If supported, add a kernel with broader reach, such as occasional correctly scored whole-trace prior proposals or suitable blocked moves, and test that the combined kernel connects the supported finite examples. Whole-trace proposals provide a useful correctness fallback where prior mass on valid traces is positive, but can be very inefficient for rare evidence; do not promise performance from that alone. Initialization should use bounded attempts to obtain positive finite target density, with a diagnostic that distinguishes initialization failure from proven impossible evidence.

R̂ is not a substitute for reachability. Chains can all miss the same mode, and no finite diagnostic can guarantee that a deliberately constructed two-mode posterior will always trigger a warning. Change the [test requiring that warning](inference-proposal.md#L128) into specified diagnostic fixtures and probabilistic coverage tests, including chains deliberately initialized in different modes.

**F4 — MCMC reports are ratios with correlated observations; fixed batches are not a complete error contract.**

The [proposal](inference-proposal.md#L104) describes unweighted averages over retained runs. That is sufficient for a scalar reported exactly once on every trace. Probl also supports branch-local reports, variable `by` keys, per-visit reports, and finite-distribution reports. Their current meaning is defined by [semantics §9](semantics.md#9-reports) and [§14](semantics.md#14-sampling).

For each retained state and report key, define `a_t` as its summed reported contribution and `b_t` as its number of visits. Preserve the estimator `μ̂ = Σ a_t / Σ b_t`. Unreached sites have both values zero. Reports of finite distributions contribute their probabilities, not an arbitrary extra latent draw. Averaging per-state or per-batch ratios instead would change the estimator whenever visit counts differ. For example, batches with `(a, b) = (1, 1)` and `(0, 9)` have a pooled estimate of 10%, not the average batch ratio of 50%.

Rejected proposals must repeat the previous chain state in the retained sequence; warmup and rejected candidate outputs must not enter the posterior accumulator. The current [interpreter writes reports immediately](../crates/probl-engine/src/interp.rs#L658), so replay needs per-trace output buffers and an explicit commit step. Define separately whether debug prints describe proposals or retained states.

For Monte Carlo standard errors, a ratio requires the long-run variance of `a_t − μ̂ b_t`, scaled by the mean denominator. It is not covered by blindly taking the standard deviation of 20 batch ratios. Even for an ordinary mean, dividing every chain into [exactly 20 batches](inference-proposal.md#L105) does not ensure batches are long enough to remove material dependence. A fixed number of batches also does not make the variance estimate increasingly precise as chains grow. Batch means can be used, but their batch-length and reliability conditions need specification. See [Flegal and Jones, *Batch means and spectral variance estimators in MCMC*](https://arxiv.org/abs/0811.1729).

**Suggested change:** specify retained-state accounting first, then select a validated long-run variance estimator with minimum-length checks. Store enough ordered chain output to compute it; the current unordered distribution accumulators and independent-run sums are insufficient. Bound that storage under host limits.

Diagnostics must be tied to quantities. Use rank-normalized split/folded R̂ and distinguish bulk from tail ESS; state what the single summary number aggregates and provide details for the worst quantities. Include reported events and derived quantities such as lift, not only the raw latent parameters. Define unavailable diagnostics for short, constant, or rarely reached series. A low R̂ is evidence of agreement, not proof of convergence. [Stan's diagnostic definitions](https://mc-stan.org/rstan/reference/Rhat.html) give a concrete reference; [Vehtari et al.](https://arxiv.org/abs/1903.08008) explain the motivation.

The proposed simulation-based calibration tests also need a dependence policy: ordinary uniform-rank tests assume sufficiently independent posterior draws. Do not feed all correlated MH iterations into an IID rank test. [Stan's SBC guidance](https://mc-stan.org/docs/stan-users-guide/simulation-based-calibration.html) explicitly addresses this distinction. Thinning for calibration is separate from discarding otherwise useful samples in ordinary inference.

**F5 — Conjugate likelihoods must reach `Weight` without underflowing first.**

The [implementation plan for marginal probabilities](inference-proposal.md#L73) needs to specify their numerical interface. Probl's extended-exponent `Weight` protects products of likelihoods; it cannot recover a positive marginal likelihood that was already rounded to zero as an ordinary `f64` factor. The current [observation path](../crates/probl-engine/src/interp.rs#L613) obtains a floating-point factor before scaling the weight.

A concrete conjugate case is `p ~ beta(1000, 1000)` followed by `observe 0 from binomial(100000, p)`. Its log marginal likelihood is approximately `-4234.10`. The evidence is positive and the posterior is the well-defined `beta(1000, 101000)`, but exponentiating that log probability into binary64 gives zero. A naïve implementation would discard every run and report impossible evidence, precisely where conjugate inference should work well.

**Suggested change:** compute beta-binomial, gamma–Poisson, and normal marginal scores in log or extended-exponent form, and update `Weight` without an intermediate underflowing exponentiation. Specify stable posterior-parameter updates, support validation, and the normal posterior mean as well as its precision. Validate constructor parameters at the original draw even if realization is delayed, preserving the language's error behavior.

Test rare-but-possible data, impossible counts, long observation sequences, and likelihood densities above one. For evidence tests, retain all observation normalizing constants. Summing binomial successes is enough for the posterior parameters, but replacing several observations with one aggregate binomial generally changes the evidence unless the combinatorial factors are preserved.

**F6 — Distinguish evidence unbiasedness from posterior estimation and sampling error.**

The claim that likelihood-weighted [estimates remain unbiased](inference-proposal.md#L7) is false for the normalized posterior estimators Probl reports. The current semantics correctly identify the **evidence estimator** as unbiased; posterior probabilities and means use self-normalized importance sampling, a ratio of random sums that is generally biased at finite sample size.

A direct counterexample uses `x ~ bernoulli(50%)` and an observation with likelihood 90% for `x=true`, 10% otherwise. The posterior probability is 90%. With one likelihood-weighted run, normalization cancels its positive weight, leaving the sampled Boolean. Its expectation is the prior probability, 50%, not 90%.

Consequently, “changes nothing but the variance” is too strong as a statement about finite-run estimators. Delayed sampling preserves the target model; it can change finite-sample bias as well as variance. Likewise, equal run weights do not eliminate uncertainty in reported posterior quantities. Independent draws of `a` and `b` still give Monte Carlo error in their lift's mean, probability, and quantiles. What can have zero Monte Carlo variance in the fully conjugate examples is the evidence estimate, assuming deterministic input data and no remaining random dependence in its normalizer.

**Suggested change:** use “preserves the target distribution and reduces reliance on importance weights.” Call the conjugate formulas analytically exact, subject to numerical rounding. Reserve “zero standard error” for the quantities actually computed without Monte Carlo variation. Test posterior-report error calibration even when the weight ESS equals the number of runs.

**Decisions on the open questions**

1. **Conjugate updates by default:** yes, after differential tests, with an explicit opt-out for diagnosis and benchmarking. Keep per-variable update/materialization statistics. An annotation or harmless compiler temporary should not silently defeat the optimization.
2. **MCMC or particles next:** do not choose solely by nominal generality. Pick a concrete non-conjugate benchmark and define the needed kernel against it. Trace MH is a reasonable next experiment; particles with rejuvenation do not remove the need for sound moves or accurate uncertainty calculations. Deferring particles until a sequential-state workload needs them is reasonable.
3. **Name and defaults:** keep `mcmc` as the user-facing family, but report the actual algorithm. Expose warmup separately from retained draws and make their counts explicit. Twenty thousand transitions can mean very few updates per latent site in a large model; fixed defaults and a quarter for warmup are starting settings, not evidence of convergence. Freeze all tuning after warmup.
4. **Part 1 alone:** yes. It has a clear benchmark, analytic posteriors, and a much smaller correctness surface. Ship it without committing to the current hybrid-MH design.

**Acceptance checks worth adding before implementation**

| Area | Essential check |
|---|---|
| Delayed sampling | Annotation/copy/call/control-flow materialization, immutable parameter snapshots, and constructor failures preserve scalar semantics |
| Evidence | Sequential conjugate updates match analytic evidence, including constants and extreme log likelihoods |
| MH trace machinery | Every source of model randomness replays; tiny dynamic-trace kernels satisfy detailed balance |
| Proposals | Transformed walks include their corrections; changed downstream distributions are rescored |
| Reachability | Hard-constrained finite models expose reducibility; initialization failures terminate under a budget |
| Hybrid inference | Fully collapsed, partially collapsed, and branch-dependent materialization have an explicit joint or marginal target |
| Reports | Rejections, reach, keys, visit counts, finite distributions, and ratio MCSE preserve existing report meaning |
| Diagnostics | Slow-mixing, heavy-tailed, short, constant, and multimodal fixtures exercise failure and unavailable-result paths |

Validation for this review consisted of source/specification inspection, primary-source research, and small independent calculations of the finite-sample bias, disconnected support example, and beta-binomial log likelihood. No new inference implementation or full test-suite run was performed. The proposal itself is unchanged.
