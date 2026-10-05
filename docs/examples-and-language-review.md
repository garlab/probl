**Examples and language review**

Follow-up, 1 October 2026: R1's repeated-key classification and R2's sparse-report certainty/precision issues are fixed, as is D2's mode-dependent callback restriction. See the [current semantics](semantics.md) and [regression tests](../crates/probl-engine/tests/reporting_regressions.rs). Other findings below remain a record of the reviewed version and are not all resolved by that change.

Reviewed on 27 September 2026 against `2650c18`. Scope: all ten programs in `examples/`, the current [semantics](semantics.md), and implementation paths relevant to the findings below. This is a design review, with targeted execution checks, not a full security audit. No example or engine code was changed.

**Assessment: Probl now has a coherent core and a useful niche. Keep it. The next work should make composition and reported answers trustworthy before adding more inference algorithms.**

The strongest examples are the games: ordinary stateful code describes the process, value semantics makes branching understandable, and merging/memoization recover dynamic programming without requiring users to write it. The forecasting examples also read well. Separating facts, probabilities and distribution recipes was the right decision. Explicit evidence boundaries, an independent oracle, approximation accounting, reproducible sampling, and conjugate updates are substantial improvements over the first audit.

Its present strengths are finite probabilistic processes, forward simulation, and a useful subset of Bayesian updating. It is not yet a general statistical modeling environment: realistic non-conjugate inference, continuous distribution composition, model checking, and reusable model/query interfaces remain limited. That is an acceptable scope if stated clearly.

**What to fix in the examples**

All ten programs pass the CLI example tests, including their documented output checks. I found no wrong headline calculation in the examples as written. Several explanations and modeling assumptions should change.

| Example | Assessment and suggested change |
|---|---|
| [01 — Tour](../examples/01_tour.probl) | Good introduction to recipes versus draws. Add a correlated-record example: the distinction becomes much less obvious when extracting fields from a distribution. Say that “every branch runs” describes enumeration; sampling follows one path per run. |
| [02 — Craps](../examples/02_craps.probl) | The rules and result are consistent. The roll counter deliberately prevents a finite cyclic state, so retaining the unresolved-tail explanation is appropriate even now that the engine can solve cyclic loops. No substantive fix found. |
| [03 — RPG duel](../examples/03_rpg_duel.probl) | The attack rules and first-strike advantage are explicit. Fix the comment saying a yes/no `simulate` result “is simply its probability”: it is `dist[bool]`; `P(...)` extracts its probability and `report` summarizes it. Qualify the memoization comment as enumeration behavior. |
| [04 — Risk](../examples/04_risk_battle.probl) | Correct for ten armies in the attacking territory, one reserved, and defenders rolling the maximum number of dice. State that convention in the opening sentence: “ten attacking armies” can also mean ten available to attack. The code already explains it. |
| [05 — Snakes and ladders](../examples/05_snakes_and_ladders.probl) | Good use of independent races and tie advantage. The overshoot and noninteraction assumptions are stated. No substantive fix found. |
| [06 — Blackjack](../examples/06_blackjack_dealer.probl) | Coherent dealer-only model. Clarify that it includes dealer naturals in the `21` category, is not conditioned on a peek ruling out blackjack, and has no player cards removed. Its table should not be compared directly with tables using those different conditions. |
| [07 — Launch forecast](../examples/07_launch_forecast.probl) | Useful illustration, but the churn prior has support outside valid probabilities. Its particular invalid tail is negligible; the modeling pattern is still unsafe to teach without explanation. State that churn is one uncertain constant for the whole trajectory, and that the reported bands are pointwise monthly quantiles, not a band containing 90% of whole trajectories. |
| [08 — Signup forecast](../examples/08_signup_forecast.probl) | The strongest data example. The posterior is `beta(120, 3422)`, with mean 3.3879%; independent calculation gives evidence approximately `3.59794e-15`, matching the output. Explain the fixed conversion-rate/independent-visitor assumption and that `daily_visitors` is one uncertain average for the month, not thirty independent daily draws. Add a posterior predictive check or a link to a companion example. |
| [09 — Roadmap](../examples/09_roadmap.probl) | Stale as a forecast of the current project: its future work includes major features already implemented. Rename it as an illustrative estimate made before development, or replace it with a fictional project's roadmap. Most phase durations are independent; shared overhead introduces some dependence, but not a common productivity factor in the phase estimates themselves. Teach that distinction rather than presenting these dates as calibrated promises. |
| [10 — Quantum key](../examples/10_quantum_key.probl) | The calculations are correct for the specified intercept–resend toy model. Its prose overstates what that model establishes; revise as detailed below. |

**The quantum example needs an explicit scope statement.** Suggested opening: “An ideal, noiseless BB84 model under independent intercept–resend attacks; this computes error and detection probabilities, not a general security proof.” Replace “Eve … has to measure … in bases she guesses” with “In this attack, Eve measures …”. That is one attack strategy, not a consequence that excludes other quantum attacks. State that the public classical channel is authenticated; authentication is part of the QKD setup, not something the photon model supplies. [Scarani et al., *The Security of Practical Quantum Key Distribution*](https://arxiv.org/abs/0802.4155).

Rename “bits of the key Eve has right” to “Eve's guesses correct in the sifted key.” The computed 75% is guessing accuracy before privacy amplification. It is not the fraction of final secret bits known to Eve. Within this model, she learns the bit exactly when her basis matches Alice's, half the time; otherwise her recorded bit is a random guess. The code initializes `eve_right = false` for unmeasured photons, so extending that report to partial interception would not count uninformed guesses consistently.

The 11% comment also needs qualification: it comes from a particular asymptotic security analysis, not a universal operating threshold for real systems. Shor–Preskill obtain the threshold where `1 − 2H(q)` vanishes; other postprocessing protocols have different tolerances. [Shor–Preskill](https://arxiv.org/abs/quant-ph/0003004), [Gottesman–Lo](https://arxiv.org/abs/quant-ph/0105121).

The toy model's numerical results check out: sifted-key error rate is 25%, detection after `m` independent checked bits is `1 − (3/4)^m`, and the final Bayesian table uses `0.1(3/4)^m / (0.9 + 0.1(3/4)^m)`. Label that posterior as evidence about **this attack under the stated prior**, not all possible eavesdropping.

**Two report issues to fix now**

**R1 — A loop variable is not proof that report keys are unique. High priority; reproduced contract defect.**

[Report classification](../crates/probl-sema/src/lower.rs#L1592) treats `report ... by key` as at most once per world/key whenever `key` names the innermost `for` variable. Collections can contain duplicate values, and enclosing loops can revisit the same key.

```probl
let doubled ~ bernoulli(50%)
let keys = if doubled { [1, 1] } else { [1] }
for key in keys {
  report doubled by key as "doubled"
}
```

Current output is **66.67%, without a “per visit” label**. The probability of `doubled` over worlds is 50%; the result is 66.67% because true worlds contribute twice. The accumulator is doing a valid per-visit calculation, but the promised classification is wrong. Nested loops reproduce the classification problem even with a unique inner collection.

Fix the contract before users build tables around it. Conservatively label a report per visit unless uniqueness across the entire execution is established, or explicitly require/enforce one contribution per world/key for that report kind. For nested indices, a composite key must include every dimension needed for uniqueness. Do not silently deduplicate: choosing the first, last, or an aggregate value changes the question being answered.

**R2 — Zero empirical error is presented as certainty, including on almost unreached reports. High priority; reproduced statistical presentation limitation.**

```probl
@mode sample(runs: 1000, seed: 1)
let visited ~ bernoulli(0.1%)
if visited {
  let outcome ~ bernoulli(50%)
  report outcome as "rare branch"
}
```

Current output:

```text
rare branch    100.00% ± 0.00% (reached in 0.2% ± 0.1% of runs)
```

Only two sampled runs reached the report. The true conditional probability is 50%. The delta-method formula gives zero empirical standard error when all observed outcomes agree; that does not establish zero uncertainty. The implementation follows its formula, but the answer needs a reliability qualification.

Show report/key-specific contributing-run counts and effective sample sizes, not only the program's weight ESS. Distinguish analytically evaluated probabilities from empirical proportions. For ordinary independent Bernoulli reports, use a boundary-aware interval or an explicit “too few events to estimate error” status. Zero successes in 1,000 trials gives a one-sided 95% upper bound of about 0.299%, not certainty of zero. [NIST's exact binomial confidence limits](https://itl.nist.gov/div898/software/dataplot/refman2/auxillar/exacbici.htm) are one reference. Weighted, conditional, per-visit, and integrated reports need their own treatment; do not apply a binomial interval indiscriminately.

Also preserve useful numeric precision. A sampled `p ~ beta(1,1)` updated by `observe 0 from binomial(100000,p)` currently prints its mean, SD and all quantiles as `0.00`. Its mean is approximately `1e-5`. Scientific notation or significant digits should survive into every output format. This matters to forecasts and rare-event models, not just formatting preferences.

**Design decisions worth settling before libraries grow**

**D1 — Preserve recipe semantics, but make correlation harder to discard accidentally.**

This executed example captures the most important remaining teaching hazard:

```probl
let pair = simulate { let x ~ d6; { a: x, b: x } }
report pair.a == pair.b       # 16.67%: separate draws from the marginals
let p ~ pair
report p.a == p.b             # 100%: one joint draw
```

The behavior follows today's rules. It is nevertheless surprising that projecting two fields from an explicitly joint distribution loses their relationship when combined. The same distinction applies to `let x = f()` when `f` draws internally: `x` is then a settled result, while `let x = d6` binds a recipe. “`=` keeps a distribution” is only a rule about a right-hand side that returns a distribution, not a rule about assignment generally.

Keep `~` as the explicit identity boundary. Add joint records to the tour, make editor hovers show `dist[Record]` versus `Record`, and warn on obvious repeated recipe use such as `d == d` or separate projections from the same joint recipe. Provide a clear joint transformation operation/pattern that binds an outcome once before applying a function. Do not change correlation implicitly according to the representation or the inference engine. [Squiggle's documented representation-dependent correlation pitfalls](https://www.squiggle-language.com/docs/Guides/Gotchas) are a useful cautionary comparison.

This is not a reason to ban convenient independent arithmetic such as `a.dice + a.dice` in the RPG example. It is a reason to make the two meanings easy to see and to express.

**D2 — Give functions an explicit effect contract. The current higher-order boundary already behaves differently by mode.**

This program fails in enumeration but succeeds with `--mode sample`:

```probl
let values = [1, 2].map(x -> { let roll ~ d6; x + roll })
report values
```

Enumeration reports that the callback cannot branch, draw or observe. Sampling prints a distribution of lists. The [callback check](../crates/probl-engine/src/interp.rs#L1665) infers purity from the executed result: one outcome with approximately unit weight. A sampled random call naturally has that shape. This is an inconsistent restriction on a finite model, not a continuous-enumeration limitation.

Choose whether collection callbacks are deterministic-only or propagate probabilistic effects. Enforce that choice consistently in both engines; a realized result is not an effect check. If probabilistic collection traversal is supported, specify its joint outcomes and evidence propagation as clearly as an ordinary loop. A separate traversal operation is an option if keeping ordinary `map` pure is valuable.

The [existing effect analysis](../crates/probl-sema/src/effects.rs) also unions all lambdas' effects for an unknown closure call. I reproduced an unused observing lambda causing a later call to an unrelated identity lambda to be rejected after a report. That conservative rule is understandable in a prototype, but will make libraries interfere with one another.

Before stabilizing function types/modules, define how function signatures carry or infer draw, observe and debug effects; distinguish a settled result from a returned distribution. This can be incremental. It need not make every function declaration verbose or require a whole new type system immediately. Even simple known return-type mismatches currently escape `check` and fail at runtime, so avoid presenting the current checks as a complete static type checker.

**D3 — Make inference performance explainable under ordinary refactoring.**

With 5,000 runs and seed 17, I compared:

```probl
let p ~ beta(1, 1)
observe 0 from binomial(100000, p)
report p
```

with the same observation extracted into a helper:

```probl
fn evidence(p: float) { observe 0 from binomial(100000, p) }
let p ~ beta(1, 1)
evidence(p)
report p
```

The inline version gives evidence approximately `1e-5` and ESS 5,000. The helper gives approximately `8.31e-20`, relative error 100%, and ESS 1 for that run. The helper forces the parameter and loses the exact update. Both target the same model; their inference quality is radically different. This limitation is documented, so it is not an undisclosed implementation bug.

The immediate improvement is a source-level explanation: “exact updates available here,” “forced by this call,” and a prominent low-ESS reliability status. Longer term, decide how reusable observation helpers can preserve delayed parameters: safe inlining, an internal symbolic parameter representation, or another deliberately scoped mechanism. Do not make users reason about compiler temporaries to obtain useful Bayesian inference. Keep the opt-out and differential checks.

**D4 — Separate a model, its distribution, and an approximate inference result.**

`simulate` is currently an eager enumeration-and-normalization boundary. That is a sound restriction, but it limits composition: a continuous forecast cannot be packaged inside it, even under `@mode sample`. Arithmetic on continuous distribution values is also unavailable, although analogous finite-distribution expressions work.

Before expanding this area, decide whether a distribution can represent a generative computation lazily and which operations require enumeration, sampling, or a density. These are different capabilities. A sampleable transformed distribution does not automatically have an evaluable density, and a finite approximate sample set is not an exact distribution.

Keep existing `simulate` semantics stable. If approximate inference becomes a value, give its result an explicit contract for error, evidence, provenance and permitted reuse. Do not silently place estimates inside estimates. [Gen's generative-function and trace interface](https://www.gen.dev/docs/stable/ref/core/gfi/) is useful prior art for separating model execution from inference machinery; it is not a prescription to expose all that machinery to beginners.

The current “no observe after report” rule is a good guardrail. Future filtering/smoothing should introduce an explicit observation/query scope, not weaken that rule without replacing its meaning. [Stan's model and generated-quantities blocks](https://mc-stan.org/docs/reference-manual/blocks.html) demonstrate a useful separation of fitting from prediction, even though Probl should retain its much lighter syntax.

**D5 — Make support constraints easy to model, not just easy to violate.**

`3% to 7%` is a lognormal with positive, unbounded support, not a probability-bounded distribution. In example 07 its mass above one is approximately `2.56e-33`, so this is not a practical failure of that particular run. However, `50% to 95%` puts about **2.82% above one**. I reproduced a runtime failure when drawing it into `p: prob`.

Preserve the documented meaning of `to`; changing its family when endpoints have percentage syntax would introduce another hidden rule. Add an explicitly bounded probability-interval constructor, or make beta/logit-normal calibration straightforward. Warn when a known distribution's support conflicts with an annotation or a probability argument. Similarly, a normal growth rate can fall below −100%; a positive multiplicative growth factor often expresses the intended support more directly. Clamping is not a neutral fix because it changes the model.

The forecasts should also demonstrate dependence and model adequacy, not only marginal intervals. Example 08 assumes a common conversion rate and conditionally independent visitors. Real variation across days or campaigns would need a richer model; it is not established by the pilot example either way. A small replicated-data check is a better next teaching example than another isolated distribution family. [Stan's posterior predictive checks](https://mc-stan.org/docs/stan-users-guide/posterior-predictive-checks.html) give the relevant workflow.

**D6 — Stabilize structured report results before charts and integrations depend on text.**

The Rust engine already retains report accumulators in `Outcome.reports`; this is not a request to start that separation from scratch. The [WASM success response](../crates/probl-wasm/src/lib.rs#L284), however, exposes formatted output plus summary statistics rather than a structured report contract.

Define report identity, key type, contribution unit (world or visit), reach, estimate kind, uncertainty kind, reliability status and precision in a versioned result schema. Preserve seed/version/input provenance with exported results. The same data should drive CLI text, browser charts and files.

Separate model spread (SD and posterior/predictive quantiles) from Monte Carlo error and unresolved bounds. A 90% monthly band in example 07 is not a 90% simultaneous trajectory band; combining separately reported marginal quantiles also cannot recover joint risk. Support explicitly requested joint outputs without retaining every trajectory by default, which would defeat merging and increase memory use.

**What I would prioritize**

1. **Fix current contracts:** repeated-key classification, the mode-dependent callback restriction, and the presentation of unreliable/rare sampled reports.
2. **Repair the teaching surface:** scope BB84 accurately, relabel the roadmap, clarify forecast assumptions, and teach correlated records and bounded probabilities.
3. **Write the function/effect and structured-result contracts before expanding libraries.** Add diagnostics explaining lost conjugacy and inference limitations.
4. **Add one demanding, representative forecasting case:** correlated inputs, replicated-data checking, and a non-conjugate component. Use its failure mode to choose the next inference method.
5. **Keep major engine changes benchmark-driven.** Merging is valuable but still depends on how much state remains live. [Dice](https://arxiv.org/abs/2005.09089) shows a different approach based on factoring discrete inference; it is worth understanding, not a reason to replace the current engine without a suitable workload. Likewise, general MCMC should follow its revised correctness contract and an actual model that needs it.

I would not redesign the core syntax, remove mutation, change the `~` identity rule, or rush to add every inference backend. The costly future mistakes would be freezing ambiguous correlation, callback effects, report contribution units, or approximate-result semantics into a public ecosystem. Those boundaries deserve attention now; the existing model underneath them is worth preserving.

**Validation**

- `cargo test -p probl-cli --test examples`: all 11 checks passed (ten examples plus the comparator test).
- Read every example and the relevant semantics, lowering, effect analysis, interpreter, reporting, and WASM result code.
- Executed small temporary programs for joint projections, function/recipe identity, duplicate and nested report keys, rare conditional reports, bounded probability priors, helper-induced loss of conjugacy, callback behavior in both modes, conservative closure effects, and continuous `simulate` rejection.
- Independently calculated example 08's beta posterior/evidence, example 10's detection/Bayes formulas, and the lognormal support tails discussed above.
- Example output tests verify execution and numerical agreement with their references. They do not establish that a forecast's assumptions fit reality, or that a toy security model covers all attacks. No full test-suite or deployment/security review was performed for this report.
