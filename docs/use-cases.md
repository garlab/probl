# Use cases and remaining design gaps

Updated October 2026. The models below exercise decisions, operations, model checking, monitoring and correlated risks. Their reference calculations were recorded on 30 September 2026; their output blocks remain executable regressions. The [language overview](language-overview.md#12-examples) lists the full example collection.

Probl's strongest fit is a finite probabilistic process or a forward simulation written as ordinary stateful code. Supported conjugate updates also make some Bayesian forecasts practical. General non-conjugate inference and reusable statistical-model libraries remain less developed.

## Models to try

These five models run with the current language. Each has documented assumptions and an output block checked by the example tests, and is bundled in the playground.

| Example | Question and result | What its next realistic extension needs |
|---|---|---|
| [14 — Stock decision](../examples/14_stock_decision.probl) | How much should we order before demand is known? Candidate orders 20/40/60 have expected profits 80/120/80. Choose 40; perfect demand information would be worth 40. | Reusable expected-utility queries for continuous models; convenient selection by a score; useful scalar tables. |
| [15 — Service queue](../examples/15_service_queue.probl) | How much waiting would a second agent save on the same jobs? About 1.73 minutes per job in this short batch model. | Sampleable model composition; explicit sharing of scenarios across policies; eventually event scheduling and resource libraries. |
| [16 — Predictive check](../examples/16_predictive_check.probl) | Can one fitted conversion rate reproduce the variation between eight days? A replicated range at least as large occurs about 10.6% of the time. | Reusable fitted models, replicated datasets, diagnostics and export. Hierarchical day/campaign effects would put pressure on inference. |
| [17 — Sensor tracking](../examples/17_sensor_tracking.probl) | Given noisy alarms, how likely is a machine to have failed? Successive filtered probabilities are 1.56%, 37.19%, 81.52%, 17.83%; next-step risk is 18.37%. | Returning local evidence alongside the posterior; reusable filtering/smoothing interfaces; inference for continuous hidden states. |
| [18 — Correlated losses](../examples/18_correlated_losses.probl) | Does dependence matter if individual sites have the same risk? Both models have mean loss 8.94, but multiple-site loss probabilities are 2.03% versus 0.26%; their 99th percentiles are 300 versus 100. | Dependable rare-event reporting, explicit tail queries and correlation-preserving transformations of joint outcomes. |

Several modeling choices matter more than adding syntax. The stock policy is selected **before** drawing demand; choosing separately in each future would give a clairvoyant answer. Queue policies receive the same arrival and service vectors, so the reported change is paired. Replicated pilot days share one posterior rate. The sensor example conditions each historical estimate only on the corresponding prefix. Site losses share one common-hazard draw.

The predictive check is deliberately modest evidence: a 10.6% tail probability does not establish that the common-rate model is wrong, and a single range statistic cannot establish that it is right. Probl should help users check several substantively chosen aspects of a model and inspect replications. This follows the workflow in [Stan's predictive-checking guide](https://mc-stan.org/docs/stan-users-guide/posterior-predictive-checks.html).

## Current gaps

### Reusable computations and correlation

A distribution recipe, one drawn outcome and an inference result are different things. Reusing a recipe makes independent draws; draw a joint outcome once to preserve relationships between its fields:

```probl
let pair = simulate { let x ~ d6; { a: x, b: x } }
report pair.a == pair.b       # 16.67%: independent marginal recipes
let p ~ pair
report p.a == p.b             # 100%: one joint outcome
```

This contract is deliberate, but editor type information and a joint transformation API could make mistakes easier to avoid. Policy comparisons should draw shared exogenous inputs once, as the queue example does. Equal random seeds alone do not ensure shared scenarios when policies make different random calls.

`simulate` always enumerates, including inside sampling. Analytic continuous outcomes now support some top-level affine models, but continuous draws inside `simulate` remain unsupported. A sampleable model abstraction needs explicit capabilities and an inference-result type that carries approximation metadata. Modules/imports are not implemented yet. Specify these contracts before encouraging reusable modeling packages; [Gen's generative-function interface](https://www.gen.dev/docs/stable/ref/core/gfi/) is relevant prior art.

### Results and model checking

The [Rust library](library.md) now exposes structured probabilities, numeric summaries, uncertainty, evidence and provenance. This resolves the former need for Rust callers to parse report text. Typed outcomes, report reach accessors, a versioned JSON schema and browser charts remain open. The WASM page still presents text rather than a complete structured result interface.

Small scalar tables and explicit selection of reported statistics would improve decision and tracking models. A fitted-model interface should preserve parameter uncertainty when generating replicated datasets or predictions for new inputs. [PyMC's posterior-predictive API](https://www.pymc.io/projects/docs/en/stable/api/generated/pymc.sample_posterior_predictive.html) illustrates that distinction.

Tail queries need definitions before convenient names. For a discrete expected-shortfall function, probability mass at the quantile must be split according to the desired tail fraction; simply averaging all losses at or above the quantile can select too much mass. Example 18 uses the already-defined expected excess `mean(max(loss - reserve, 0))` instead.

### Explain inference changes under refactoring

Conjugate updates operate within the compiler's supported observation forms. Passing a delayed parameter into an ordinary observation helper materializes it and can turn an efficient exact update into inefficient likelihood weighting. The target model stays the same, but finite-run quality can change dramatically.

`--stats` and `--no-conjugate` help inspect this today. Explaining why a draw became necessary, and preserving eligible updates through reusable observation helpers, would make model refactoring safer. General hierarchical and non-conjugate models also need stronger inference, selected against concrete benchmarks. See [inference](inference.md) for the existing boundaries and the requirements for another method.

### Filtering and local evidence

The sensor example already performs exact incremental finite-state filtering: each `simulate` update returns the next belief distribution. No new filtering syntax is needed. Its carried belief has two states rather than a history of all observations.

The remaining gap is packaging local evidence and provenance alongside that posterior. Local normalization remains inside `simulate`; the outer run does not automatically report the history's likelihood. Distinguish filtered, smoothed and predictive queries, and carry uncertain transition parameters jointly with state rather than recombining their marginals independently. Larger or continuous hidden states motivate particle inference only after its uncertainty contract is defined.

### Data, collections and static checks

Comparator-based `sort`, `minimum` and `maximum` now support records and complex values ordered by a chosen score. Extrema have named empty-input defaults; `reduce` accepts an optional seed and function values. These address the earlier sorting and best-candidate gaps. The current API is in the [language guide](language-overview.md).

Missing data is still more consequential than another collection helper. There is no optional/null value type: a missing or null field cannot be read as an ordinary `int`. Coordinate optional/tagged values, exhaustive matching and data validation rather than introducing sentinel zeros. Grouping, joins and priority queues could begin as libraries once module contracts exist.

Collection callbacks now enforce the same draw/observation restrictions in both inference modes. The broader effect contract and complete static checking remain priorities: compilation still cannot detect every invalid call before execution.

### Modeling assumptions worth keeping visible

Forecast examples are illustrative assumptions, not calibrated predictions. Choose probability priors with valid support, distinguish one shared uncertain parameter from fresh daily variation, and distinguish pointwise interval bands from intervals for whole trajectories. Choosing a policy after seeing a simulated future is clairvoyance, not an implementable decision.

[Example 09](../examples/09_roadmap.probl) forecasts an early plan, not current release dates. [Example 10](../examples/10_quantum_key.probl) computes classical probabilities for ideal BB84 under an intercept–resend attack. It is neither a quantum-amplitude simulator nor a general security proof; its guessing accuracy and detection rates apply to that specified model.

## Fixed issues retained as regressions

Earlier reviews found mode-dependent callback checks, repeated-key report classification, sparse sampled probabilities shown with misleading certainty, and tiny values rounded into apparent zero. Those behaviors were corrected; [reporting_regressions.rs](../crates/probl-engine/tests/reporting_regressions.rs) preserves the cases. The later API consistency fixes are summarized in [testing](testing.md#api-regression-contracts). The removed reviews' old reproductions should not be treated as current language behavior.

## Further scenarios

| Scenario | Why Probl fits | Next feature or contract to investigate |
|---|---|---|
| Retention, warranties and repair lifetimes | Combine uncertainty about a rate with observed event times and deadlines. | Censoring: a still-active customer contributes survival probability, not an event at the cutoff. Stable survival/log-likelihood operations and explicit observation status; optional fields when importing incomplete records. See [Stan's survival models](https://mc-stan.org/docs/stan-users-guide/survival.html). |
| Campaign or regional demand models | Share information across small groups while preserving group variation. | Grouping/joining data; hierarchical inference and diagnostics; reusable predictive queries. |
| Appointment scheduling and logistics | Simulate delays, contention, deadlines and staffing policies. | Event queues, priorities, cancellation and resources as libraries. Elapsed durations already work as numbers; only add civil timestamps/timezones if real schedules require them. [SimPy's resource abstractions](https://simpy.readthedocs.io/en/latest/topical_guides/resources.html) are relevant prior art. |
| Capacity planning and reliability targets | Compare policies under shared shocks and ask about small failure probabilities. | Tail reliability diagnostics and, later, explicit importance proposals with correct weighting. Extra runs alone can be impractical for rare events. |

## Reference calculations and verification

Run a model from the repository root, for example:

```sh
cargo run --release -p probl-cli -- run examples/14_stock_decision.probl
```

Examples 15–16 specify sampling mode and seed; examples 14, 17 and 18 enumerate. These five do not depend on `today` or external files.

Independent calculations were performed outside the engine:

- **Stock:** enumerate the 3×3 payoff table. Expected profits are 80/120/80; expected clairvoyant profit is 160; expected regret for ordering 40 is 40.
- **Queue:** 100,000 independent Python batches, seed `20260930`, using Lindley's recursion for one agent and a completion-time heap for two. Mean waits were 1.88920 and 0.15775 minutes; mean saving 1.73145. Any two-agent wait over two minutes occurred in 15.354% of batches. Probl's 2,000-run result is 14.6% ± 0.8%, consistent within sampling error. The example retains Probl's seeded output as its regression baseline.
- **Predictive check:** beta-binomial calculations give posterior mean rate 30.4878%, replicated-total mean 24.39024 and SD 5.75251. Expanding the eight binomial event polynomials and integrating against `beta(25, 57)` gives mean range 4.01036 and `P(range >= 6) = 0.1058967462`; Probl reports 10.7% ± 0.3%. This validates an actual joint replicated-data query, not only the fitted mean.
- **Sensor:** exact rational forward recursion gives 0.015625, 0.3718619247, 0.8151826352, 0.1783240933, then prediction 0.1837430700. At each step predict `q = 0.8*p + 0.05*(1-p)`, then apply Bayes' rule using alarm likelihoods 0.9 and 0.1.
- **Losses:** the common-hazard mixture is `0.02` at loss 300 plus `0.98` times a scaled `binomial(3, 0.01)`. Compare with scaled `binomial(3, 0.0298)`. Multiple-site probabilities are 0.02029204 and 0.002611192816; expected excesses above 100 are 4.029302 and 0.2637656408.

Current regression entry points are the CLI's example suite, parser/lowering snapshots and actual native/WASM output parity. See [testing](testing.md) for commands. The independent calculations above validate selected quantities; they do not establish that every modeling assumption is appropriate for another application.
