# Architecture and development priorities

This describes the implementation as of October 2026. The [language overview](language-overview.md) introduces Probl; the [reference semantics](semantics.md) defines its behavior. Future work below is a set of priorities, not a release schedule.

## What exists

Probl runs discrete models by enumeration and larger or continuous models by independent sampling. Enumeration also supports a restricted analytic representation of continuous draws. The CLI, Rust library and browser playground share compilation, execution and report calculations.

The language includes arbitrary-precision integers, finite floats, complex scalars, probabilities, immutable Unicode strings and dates, collections, functions and distribution values. Typed CSV, JSON and line-based input is loaded by the host before execution. The compiler resolves names, checks declared boundaries and some expression types, and analyzes effects and liveness. It is not yet a complete static type checker: some errors are found only when a program runs.

## Crates and pipeline

```text
source → lexer/parser → AST → lowering → IR and analyses → engine → report results
                                                                        ↓
                                                               library / CLI / web
```

| Component | Responsibility |
|---|---|
| [`probl-number`](../crates/probl-number) | Integer representation and numeric support |
| [`probl-syntax`](../crates/probl-syntax) | Hand-written lexer, Pratt/recursive-descent parser, AST, spans and diagnostics |
| [`probl-sema`](../crates/probl-sema) | Name resolution, lowering, checks, liveness, effects, conjugate-update analysis and editor symbols |
| [`probl-engine`](../crates/probl-engine) | Values, distributions, weights, interpreter, inference, data decoding, limits and reports |
| [`probl`](../crates/probl) | Supported embedding API: compile, load, run and inspect results |
| [`probl-cli`](../crates/probl-cli) | The `probl` executable: run, check, schema, IR inspection and REPL |
| [`probl-wasm`](../crates/probl-wasm) and [`web`](../web) | Browser adapter, workers, editor and playground |
| [`probl-oracle`](../crates/probl-oracle) | Independent rational-arithmetic interpreter and generated comparisons |
| [`probl-bench`](../crates/probl-bench) | Benchmark runner for models in `benches/` and `examples/` |

The internal crates are implementation details. Embedders should use [`probl`](library.md); the workspace tools also use internal APIs for editor support, schema suggestions and debugging.

## Values, worlds and identity

A world contains a frame of values and a weight. Values and collections are shared with reference counting and copied on write, so forking does not eagerly duplicate all their contents. Collections cache structural hashes to make repeated state comparisons cheaper.

Storage identity is typed and structural: integer `1` and float `1.0` are distinct map keys and PMF outcomes. Language numeric equality and ordering can compare across numeric types. These contracts must not be collapsed into one implementation comparator; see [semantics §1](semantics.md#1-values-and-types).

Integers grow beyond machine size within host limits. Probabilities and weights use floating-point arithmetic; `Weight` extends the exponent range to preserve extremely small likelihoods. This avoids ordinary floating-point underflow, but does not make inference rational or eliminate rounding. `--fractions` displays an approximation.

## Enumeration

The interpreter maps a set of worlds through each statement. Lowering extracts operations that can split worlds into statements and preserves left-to-right evaluation. Separate flow results carry normal continuation, `break`, `continue` and return values.

At joins, loop heads and function returns, worlds that agree on live values merge and their weights add. Backward liveness analysis lets the engine discard values that can no longer affect the result. This is effective for a board position or remaining hit points; retaining a whole history, or many distinct continuous values, can prevent merging.

Several optimizations preserve this model:

- Eligible literal draws move to their first use, within effect and control-flow barriers. This can avoid constructing independent combinations that never need to coexist.
- Calls that do not print can reuse results for the same arguments and captured values. Functions cannot assign outside variables.
- Unbounded loops whose states recur can be solved as absorbing Markov chains. Chains outside the solver's supported size or work budget fall back to unrolling.
- Recursive calls that revisit the same state can be solved by successive rounds, retaining unresolved mass until the stopping condition is met.

Unbounded processes can finish with unresolved weight. Reports preserve that information and give probability bounds where available; a small unresolved tail does not establish a small error for an unbounded mean. See [termination and approximation](semantics.md#10-termination-and-approximation).

## Distributions and continuous outcomes

Finite distributions store typed outcomes, weights and missing mass. Distribution algebra combines independent recipes; reusing a drawn outcome preserves its identity. Draw a joint record once before reading multiple correlated fields.

Continuous distributions retain their family and parameters, with analytic densities, CDFs, quantiles and moments. Enumeration can carry one continuous draw through affine arithmetic and threshold conditioning while preserving its identity. Nonlinear uses, combinations of independent continuous draws and continuous draws inside `simulate` are outside that analytic subset. Sampling supports broader computations. [Example 19](../examples/19_analytic_continuous.probl) and [the continuous semantics](semantics.md#13-continuous-distributions) define the boundary.

`simulate` always enumerates its block, including inside a sampled program. A sampleable computation and an approximate inference result need separate contracts before nested inference can be added.

## Sampling and reproducibility

Sampling uses independent runs, without merging them. Observations weight each run; reports retain per-run contributions for uncertainty estimates. Conjugate beta, gamma and normal priors can remain delayed and update analytically until their values are needed. See [inference](inference.md) for the supported pairs and the effects of refactoring an observation into a helper.

Runs are grouped into batches of 1,000. Each batch has its own xoshiro256++ stream, seeded with SplitMix64 from the run seed and batch number. Floating-point functions use `libm`. Batches combine in a fixed order, so successful runs with the same program, data, date, seed and engine version have the same output across supported native/WASM hosts and thread counts. No cross-version random-stream guarantee is made.

Parallel workers share a work budget. Near that limit, scheduling can affect whether the run finishes. Tests compare outputs and errors across thread counts; host limits and platform resources still affect which programs can run.

## Reports and the library boundary

Each report site accumulates values by its optional key, with reach and contribution metadata. A report reached repeatedly can count visits rather than worlds. Distinct numeric outcomes remain stored for quantiles; large sampled populations can therefore consume substantial memory.

[`report::results`](../crates/probl-engine/src/report/results.rs) computes the quantities shared by the renderer and the library. Formatting does not define their meaning. Results distinguish resolved point estimates, incompleteness, unresolved bounds and Monte Carlo uncertainty. Evidence stays in log space and distinguishes probabilities from densities. The [library guide](library.md) explains the public accessors and their limits.

The CLI and playground currently present text. The Rust library exposes structured report numbers, but typed outcome export, a versioned JSON result schema and charts remain future work.

The deferred [portal proposal](design/portals-and-effects.md) explores explicit access to a joint population, with host effects confined to a pre-inference prologue or portal scopes. Portals could return shared values and permit midway data acquisition; their synchronization, evidence and sampled-feedback contracts remain open. Improve reporting and population queries before implementing this boundary.

## Host boundaries

The engine has no ambient filesystem or network access. A host supplies data through a resolver, and loading validates the declared schemas before simulation. `LocalFiles` deliberately follows local CLI filesystem rules; it is not a directory sandbox. Hosted applications should grant only the inputs they intend to expose, for example through `MemoryFiles`.

Hosts set budgets for worlds, work, outcomes, integers, collections, calls, input and output, plus cancellation. Program pragmas can lower host limits, never raise them. The native library executes on a configured thread and converts caught panics into internal errors. Out-of-memory failures, stack overflow and WASM traps are not all recoverable this way. The playground runs in replaceable workers; services running untrusted models should also use process isolation and host-enforced resource limits.

## Priorities

1. **Maintainability.** 0.1.0 is published. Next: automate the existing WASM/browser tests, measure native coverage, and check the `probl` API against its published version. See [library packaging](library.md#packaging-and-publication) and [testing](testing.md).
2. **Static checking and explanations.** Catch distribution/value mistakes before execution, make effects visible, and explain lost conjugate updates and state growth. A formatter and standalone language server can build on the existing parser and editor symbols.
3. **Reusable models and results.** Specify sampleable computations, joint transformations, optional data and modules; extend structured results to browser/JSON consumers. Keep inference uncertainty explicit. The [use cases](use-cases.md) motivate these gaps.
4. **Inference guided by models.** Choose a difficult non-conjugate benchmark before specifying MCMC or particles. Evaluate a finite discrete symbolic backend separately, following the [research plan](design/symbolic-inference.md). Neither is implemented.

Bytecode execution, persistent collections, parallel enumeration and quantile sketches should follow measured bottlenecks. The [benchmarks](benchmarks.md) retain dated measurements and the optimizations they motivated. [Example 09](../examples/09_roadmap.probl) is an illustrative forecast of the original project plan, not the current release schedule.

## Original audit: resolution map

The [September design and architecture audit](design/project-audit.md) is retained as historical rationale. Its findings describe the reviewed commit; this map records the resulting work, not a new security certification.

| Finding | Current contract or implementation |
|---|---|
| D1: facts, probabilities and distributions | Distinct types; explicit draws preserve identity; conditions and evidence have documented conversions |
| D2: unresolved weight and posterior accuracy | Unresolved mass is retained; probability bounds use report denominators; means do not acquire unsupported bounds |
| D3: evidence/report scopes | Local evidence inside `simulate`; no observation after a report in the same scope; per-visit reports identified |
| D4: inference estimators | Independent sampling, per-run uncertainty and exact conjugate updates; more advanced methods deferred pending contracts |
| D5: state explosion | Liveness, deferred draws and cyclic-state solvers help supported models; sampling remains necessary for larger spaces |
| D6: interval assumptions | Positive `a to b` estimates are lognormal; `normal_range` is explicit |
| I1: numerical accuracy | Extended-exponent weights, approximation labels and retained missing mass |
| I2: evaluation order | Left-to-right lowering and effect-aware optimization |
| I3: resource limits and crashes | Host budgets, cancellation, parser limits and native panic handling; isolation is still a host concern |
| I4: independent verification | Rational oracle, generated programs, robustness tests and regressions |
