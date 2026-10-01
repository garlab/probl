# New use cases and what to build next

Reviewed 30 September 2026 against `a0c0255`, before adding examples 14–18. The examples were written and executed first; the recommendations below come from using them and trying natural extensions. This review adds examples, documentation and playground integration. It does not change language semantics or inference algorithms.

**Follow-up, 1 October 2026:** callback restrictions now apply in both modes, sparse or constant sampled probabilities no longer print zero error as certainty, report/key support is shown when low, and tiny nonzero summaries retain their scale. Repeated-key classification from the earlier review is also corrected. These fixes have [regression tests](../crates/probl-engine/tests/reporting_regressions.rs); the findings below describe the reviewed version. Observation helpers still force delayed parameters, and the model/result-interface recommendations remain open.

**Focus next on trustworthy, reusable models and useful query results.** Probl now expresses much more than dice games and marginal forecasts: decisions, operations, model checking, monitoring and correlated risk all fit its core. Draw identity, ordinary stateful code and immutable values work well together. The limits become visible when turning a small, self-contained program into a reusable model, extending its evidence, or consuming its answers elsewhere.

The most consequential design decision is the boundary between **a probabilistic computation, a distribution value and an inference result**. Settle that boundary, along with function effects and correlation, before modules and reusable modeling libraries grow around accidental behavior. More scalar math is a lower priority for these use cases.

## The new examples

All five run with the existing language. Each has documented assumptions and an output block checked by the example tests, and is bundled in the playground.

| Example | Question and result | What its next realistic extension needs |
|---|---|---|
| [14 — Stock decision](../examples/14_stock_decision.probl) | How much should we order before demand is known? Candidate orders 20/40/60 have expected profits 80/120/80. Choose 40; perfect demand information would be worth 40. | Reusable expected-utility queries for continuous models; convenient selection by a score; useful scalar tables. |
| [15 — Service queue](../examples/15_service_queue.probl) | How much waiting would a second agent save on the same jobs? About 1.73 minutes per job in this short batch model. | Sampleable model composition; explicit sharing of scenarios across policies; eventually event scheduling and resource libraries. |
| [16 — Predictive check](../examples/16_predictive_check.probl) | Can one fitted conversion rate reproduce the variation between eight days? A replicated range at least as large occurs about 10.6% of the time. | Reusable fitted models, replicated datasets, diagnostics and export. Hierarchical day/campaign effects would put pressure on inference. |
| [17 — Sensor tracking](../examples/17_sensor_tracking.probl) | Given noisy alarms, how likely is a machine to have failed? Successive filtered probabilities are 1.56%, 37.19%, 81.52%, 17.83%; next-step risk is 18.37%. | Returning local evidence alongside the posterior; reusable filtering/smoothing interfaces; inference for continuous hidden states. |
| [18 — Correlated losses](../examples/18_correlated_losses.probl) | Does dependence matter if individual sites have the same risk? Both models have mean loss 8.94, but multiple-site loss probabilities are 2.03% versus 0.26%; their 99th percentiles are 300 versus 100. | Dependable rare-event reporting, explicit tail queries and correlation-preserving transformations of joint outcomes. |

Several modeling choices matter more than adding syntax. The stock policy is selected **before** drawing demand; choosing separately in each future would give a clairvoyant answer. Queue policies receive the same arrival and service vectors, so the reported change is paired. Replicated pilot days share one posterior rate. The sensor example conditions each historical estimate only on the corresponding prefix. Site losses share one common-hazard draw.

The predictive check is deliberately modest evidence: a 10.6% tail probability does not establish that the common-rate model is wrong, and a single range statistic cannot establish that it is right. Probl should help users check several substantively chosen aspects of a model and inspect replications. This follows the workflow in [Stan's predictive-checking guide](https://mc-stan.org/docs/stan-users-guide/posterior-predictive-checks.html).

## Priorities supported by the examples

**1. Fix effect consistency and misleading reliability displays first.** These are existing contracts that new model libraries would depend on. Two findings from the [earlier review](examples-and-language-review.md) still reproduce.

```probl
report [1, 2].map(x -> { let r ~ d6; x + r })
```

Enumeration rejects the callback because it draws; `--runs 100 --seed 19` succeeds. The [callback implementation](../crates/probl-engine/src/interp.rs) checks the realized number and weight of returned outcomes, which cannot establish purity in sampling. Decide whether ordinary collection callbacks must be deterministic, then enforce the same rule in both modes using effects. A loop already expresses random traversal correctly, as example 16 demonstrates. If a separate probabilistic traversal is added later, define its joint result and observation propagation explicitly.

```probl
@mode sample(runs: 1000, seed: 1)
let visited ~ bernoulli(0.1%)
if visited {
  let outcome ~ bernoulli(50%)
  report outcome as "rare branch"
}
```

This prints `100.00% ± 0.00%`, with reach `0.2% ± 0.1%`: just two runs contributed. A rare-loss or service-failure calculation must not present that as reliable certainty. Expose contributing-run counts and report-specific effective sample sizes, and distinguish unavailable error estimates from zero error. For ordinary independent Bernoulli proportions, use a boundary-aware interval; weighted and per-visit estimators need their own treatment. [NIST documents exact binomial confidence limits](https://itl.nist.gov/div898/software/dataplot/refman2/auxillar/exacbici.htm), a useful reference for the simple case.

**2. Make report results useful as data, and let users ask for the statistic they need.** Example 14's deterministic expected-profit table repeats each number in five quantile columns. Example 17's probability table does the same after converting probabilities to percentages. These are small examples of a larger interface gap: the [WASM response](../crates/probl-wasm/src/lib.rs) exposes formatted output and aggregate run statistics, but no structured report schema. The Rust engine already has report accumulators; build on those.

Define a versioned result containing report identity, key/value types, contribution unit, reach, estimates, uncertainty, reliability and run provenance. Support a scalar table for already-computed numbers and explicit selections of means, probabilities, quantiles and tail summaries. Preserve the difference between model spread and Monte Carlo error. Use the same result contract for CLI text, browser views and export.

For discrete loss distributions, a proposed expected-shortfall function needs a precise convention for probability mass at the quantile. Averaging every loss greater than or equal to that quantile can include more than the intended tail fraction. Example 18 deliberately uses the unambiguous expected excess `mean(max(loss - reserve, 0))`, which already works.

**3. Design reusable probabilistic computations without silently changing `simulate`.** The stock decision can compute an expectation inside the program because its model is finite. The continuous queue can be written as an ordinary function and called in a sampled run, but this natural attempt to package a distribution fails even in sample mode:

```probl
@mode sample(runs: 100, seed: 19)
let duration = simulate { let t ~ exponential(1); t }
report duration
```

`simulate` always enumerates. That is a coherent contract; preserve it. Design a separate way to describe a sampleable computation, and a separate inference result when an approximation is computed. Specify capabilities: sampling, enumerating support and evaluating a density are different promises. So are an exact expectation and an estimated expectation with error.

Specify how a single joint outcome is transformed without independently redrawing its fields. For policy comparisons, the user must be able to draw exogenous inputs once and evaluate each policy on those inputs, as example 15 does. Matching seeds alone is insufficient: policies can make different numbers of random calls. Do not change recipe independence implicitly to make a particular example convenient.

For continuous expected-utility decisions, a practical first interface could live at the host level: evaluate a fixed candidate set over shared scenarios, return estimates and uncertainty, then evaluate the selected policy on fresh scenarios. Choosing the largest noisy estimate introduces selection bias; an estimated winner is not automatically the true best policy. [Stan's decision-analysis chapter](https://mc-stan.org/docs/stan-users-guide/decision-analysis.html) provides the expected-utility foundation. [Gen's generative-function interface](https://www.gen.dev/docs/stable/ref/core/gfi/) provides prior art for separating probabilistic computations from inference operations; Probl need not expose its full trace machinery to beginners.

**4. Keep inference behavior understandable under refactoring.** Example 16 obtains the exact `beta(25, 57)` posterior through the existing conjugate update. Extending it to reusable observation helpers can lose that optimization. I reran this smaller diagnostic with 5,000 runs and seed 17:

```probl
let p ~ beta(1, 1)
observe 0 from binomial(100000, p)
report p
```

The inline form has evidence about `1e-5` and effective sample size 5,000. Moving the observation into `fn evidence(p: float) { observe 0 from binomial(100000, p) }` and calling it gives evidence `8.31e-20`, relative error 100%, and effective sample size 1 for this seed. The target model is unchanged; the helper forces the delayed variable. Both forms also print this very small parameter's summaries as `0.00`.

Expose why an exact update was possible or lost, make low effective sample size prominent, and preserve significant digits for small nonzero results. Decide how observation helpers retain delayed parameters before promoting a module ecosystem. The existing [inference proposal](inference-proposal.md) is still the right starting point for a broader algorithm; adding hierarchical syntax alone will not make difficult posteriors tractable.

For model checking, a fitted-result interface should retain parameter uncertainty and permit multiple replications or new inputs without confusing them with new fits. [PyMC's posterior-predictive API](https://www.pymc.io/projects/docs/en/stable/api/generated/pymc.sample_posterior_predictive.html) distinguishes checks on the original inputs from predictions on new inputs.

**5. Build on the conditioning scopes that already work.** Example 17 carries a distribution as its current belief, then assigns the result of one `simulate` update per observation. This is already an exact incremental finite-state filter. Its carried belief has only two possible states; it needs neither history replay nor a change to the no-observation-after-report rule. The first draft replayed every prefix; replacing it reduced the run from 275 to 102 world-steps with identical output. No new filtering primitive was needed.

The remaining gap is packaging and metadata. Local evidence stays inside each simulation, so the outer run does not report the history's likelihood. A reusable inference result could return normalization and provenance alongside its posterior, avoiding separate likelihood calculations. Distinguish filtered `P(state_t | readings_1..t)`, smoothed `P(state_t | readings_1..T)` and future predictive queries. When transition parameters are also uncertain, carry their joint distribution with the state rather than independently recombining marginals. Large finite supports and continuous states still exceed what exact enumeration can do; particle filtering can follow a clear approximation-result contract. This is lower priority than fixing current reports and designing composition.

**6. Extend collections and data handling in response to real models.** The stock example's manual best-candidate loop is reasonable, but `argmin_by`/`argmax_by` with documented tie and empty-input behavior would help. Queue resources want `sort_by` for timestamped records and eventually a priority queue. Cohort forecasts want grouping and indexed joins. These can begin as ordinary library functions once reusable modules are supported; `import` currently produces a compiler error.

Optional data is a more consequential type-design gap. A probe reading `[{"value": null}]` as `list[{ value: int }]` fails, as documented: there is no missing-value type. In real retention or repair data, "not yet observed" must not become zero. Coordinate optional/tagged values, exhaustive matching and data validation with the type checker, rather than inventing sentinels in individual loaders. This is supported by the loader probe and [data-input contract](data-input.md), rather than exercised by the five deliberately self-contained examples.

## Further scenarios worth testing after this batch

| Scenario | Why Probl fits | Next feature or contract to investigate |
|---|---|---|
| Retention, warranties and repair lifetimes | Combine uncertainty about a rate with observed event times and deadlines. | Censoring: a still-active customer contributes survival probability, not an event at the cutoff. Stable survival/log-likelihood operations and explicit observation status; optional fields when importing incomplete records. See [Stan's survival models](https://mc-stan.org/docs/stan-users-guide/survival.html). |
| Campaign or regional demand models | Share information across small groups while preserving group variation. | Grouping/joining data; hierarchical inference and diagnostics; reusable predictive queries. |
| Appointment scheduling and logistics | Simulate delays, contention, deadlines and staffing policies. | Event queues, priorities, cancellation and resources as libraries. Elapsed durations already work as numbers; only add civil timestamps/timezones if real schedules require them. [SimPy's resource abstractions](https://simpy.readthedocs.io/en/latest/topical_guides/resources.html) are relevant prior art. |
| Capacity planning and reliability targets | Compare policies under shared shocks and ask about small failure probabilities. | Tail reliability diagnostics and, later, explicit importance proposals with correct weighting. Extra runs alone can be impractical for rare events. |

I would implement priorities 1–2 next, specify the computation/result boundary in priority 3 alongside them, and let that design guide modules and inference improvements. Optional data should be part of the planned type-checker work. Event scheduling, richer optimization and additional distribution families can follow concrete models that need them.

## Validation and reproduction

Run any new file with `cargo run --release -p probl-cli -- run examples/14_stock_decision.probl` (substitute its filename). Examples 15–16 specify sampling mode and seed; the others enumerate. None of these five depends on `today` or external files.

Independent calculations were performed outside the engine:

- **Stock:** enumerate the 3×3 payoff table. Expected profits are 80/120/80; expected clairvoyant profit is 160; expected regret for ordering 40 is 40.
- **Queue:** 100,000 independent Python batches, seed `20260930`, using Lindley's recursion for one agent and a completion-time heap for two. Mean waits were 1.88920 and 0.15775 minutes; mean saving 1.73145. Any two-agent wait over two minutes occurred in 15.354% of batches. Probl's 2,000-run result is 14.6% ± 0.8%, consistent within sampling error. The example retains Probl's seeded output as its regression baseline.
- **Predictive check:** beta-binomial calculations give posterior mean rate 30.4878%, replicated-total mean 24.39024 and SD 5.75251. Expanding the eight binomial event polynomials and integrating against `beta(25, 57)` gives mean range 4.01036 and `P(range >= 6) = 0.1058967462`; Probl reports 10.7% ± 0.3%. This validates an actual joint replicated-data query, not only the fitted mean.
- **Sensor:** exact rational forward recursion gives 0.015625, 0.3718619247, 0.8151826352, 0.1783240933, then prediction 0.1837430700. At each step predict `q = 0.8*p + 0.05*(1-p)`, then apply Bayes' rule using alarm likelihoods 0.9 and 0.1.
- **Losses:** the common-hazard mixture is `0.02` at loss 300 plus `0.98` times a scaled `binomial(3, 0.01)`. Compare with scaled `binomial(3, 0.0298)`. Multiple-site probabilities are 0.02029204 and 0.002611192816; expected excesses above 100 are 4.029302 and 0.2637656408.

The repository checks include golden CLI outputs, parser/lowering snapshots, and native/WASM output parity. Minimal failure probes above distinguish current limitations from proposed additions; they are not broken programs in the examples menu.

Validation passed: 19 CLI example tests (18 programs plus the sampled-output comparator), 12 WASM API tests, and both all-example parser/lowering snapshot tests. The release CLI and browser/WASM builds succeeded. `node web/test/examples.mjs` matched all 22 native/WASM cases (18 examples and four additional fixtures), plus the execution-date replay check. Formatting and whitespace checks passed.
