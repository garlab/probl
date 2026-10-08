# Error handling: local recovery and failed worlds

> Draft proposal, updated 8 October 2026. Intended for near-term language work, independently of [portals and external effects](portals-and-effects.md). The global [failure modes](#failure-modes-total-or-partial) and local recovery with `try`/`catch` are implemented, as the reference semantics specifies them ([failure modes](../semantics.md#failure-modes), [catching faults](../semantics.md#catching-faults)); see [as built](#as-built) for how they differ from this proposal. Binding a fault in a catch (`catch F as err`), `finally`, stage 0 diagnostics and a failure cap are not. Proposed syntax and output elsewhere are illustrative.

## Recommendation

First, make today's behavior, where any fault stops the run, more informative: show which values caused it and, when safely measurable, its share of a clearly identified population ([stage 0](#stage-0-say-how-likely-the-failure-is)). That needs no language change. Then introduce two complementary mechanisms:

1. **Explicit local recovery:** an expression-valued `try` block with catches for named recoverable faults. A handler supplies an ordinary value or executes ordinary code; no nullable value, mandatory result wrapper or monadic API is introduced.
2. **A failure mode for unhandled faults:** *total*, where one world's fault fails the run as today, or *partial*, where the failed world ends and the others finish. Total is the default when enumerating, partial when sampling. A failed world stops reaching later reports, like a world that takes another branch. Each report keeps the contributions of worlds that reached it. Continued results must expose failures and the report's population basis; reach needs additional accounting rather than division by successful-world weight alone.

A failed world does not skip the offending instruction and continue with a missing value. Either a local handler gives it a defined continuation, or that execution path ends. Resource exhaustion, cancellation, unsupported features and implementation bugs remain fatal to the invocation.

This is also a numerical and inference contract. Failures must remain distinct from evidence rejection, unresolved mass and deliberate fallback outcomes.

## The motivating example

This is valid current syntax:

```probl
@mode sample(runs: 5)

let x ~ d6 - 1
print(1 / x)
```

`x` is uniform on 0 through 5. Each independent draw has a 1/6 probability of division by zero. Five draws all avoid zero with probability `(5/6)^5`, about 40.19%; at least one fails with probability about 59.81%. Increasing the run count exposes an existing undefined case rather than making arithmetic less reliable.

A check with the current release executable and default seed 0 completed the five-run program, failed when overridden to 1,000 runs, and failed in enumeration. The five-run success is therefore not evidence that the program is valid on its whole support. Diagnostics may already have been printed before the failure.

When zero is an expected modeled outcome, three different intentions should be expressible:

| Intention | Program behavior |
|---|---|
| Zero is evidence-excluded | `observe x != 0` explicitly changes the model's conditioning |
| Zero has a defined alternative result | Catch division by zero, or use `if x == 0 { fallback } else { 1 / x }` |
| Inspect the valid part of a faulty model | Use partial mode: finish the other worlds, retain the failure mass, and show each report's reach |

These must not be interchangeable spellings for silently discarding data.

## Stage 0: say how likely the failure is

Before any new syntax, the error that stops a run can answer the questions its author has next: how often does this fail, and for which values? Today the reciprocal example stops with only `division by zero` and the source location, in both modes.

When enumerating (`--mode enumerate` for this example), a pure operation may receive already-materialized inputs from a finite set of worlds. On a value-dependent fault, the diagnostic can inspect that operation's inputs across the same set, without continuing the surrounding statement:

```text
Error: division by zero
   ╭─[ model.probl:4:7 ]
   │
 4 │ print(1 / x)
   │       ──┬──
   │         ╰──── here
   │
   │ Note: fails for 16.67% of the weight reaching this operation, for example when x = 0
───╯
```

- **The denominator is explicit.** Let `R` be the weight entering this operation on this visit, and `F` the part for which it faults. `F/R` describes that population under its accumulated evidence, provided the weights share a justified measure and the operation was checked over the whole population. It is not necessarily the probability that the program ever fails. If only resolved inputs are available, label the share as conditional on those resolved inputs. If the denominator or fault region cannot be established, omit the percentage.
- **The boundary is an operation, not arbitrary source text.** A statement can draw, observe, print, mutate, call a function or contain a loop before reaching its fault. Finishing it for other worlds could emit more output, change weights, encounter another error or consume unbounded work. Stage 0 must not do that. Start with an internal allowlist of pure operations whose available inputs can be inspected without replay, new draws, writes, callbacks or evidence. A recipe/`simulate` failure is a failure of its enclosing operation, not automatically a failure of only some outer worlds.
- **The scope is recorded.** A loop visit or one invocation with particular arguments is not every visit/call to that source location. Label such a share with its visit/call scope, or show only the witness if that distinction cannot be explained. Deferred draws, memoization and solver discovery can prevent a complete source-level population from being available; they must not turn a local diagnostic into a claimed global failure probability.
- **The example comes from the actual failing evaluation.** Preserve bounded snapshots of relevant inputs and their source names where available. `x = 0` says more than `1 / 0`, but re-reading `x` after an earlier operand mutated it could be wrong. Fall back to actual operand values when source bindings were cleared or cannot be attributed; do not replay an expression or retain/dump the entire environment for diagnostics.
- **Enrichment stays bounded.** Inspect eligible inputs under a small diagnostic budget within host limits. Group faults by source and message, and bound examples and rendered values. Cancellation, resource exhaustion or an internal error stops enrichment immediately. Never report a complete share from a partly inspected population; retain the original fault with the appropriate incomplete-diagnostic/fatal context.

This is best-effort enrichment for total-mode enumeration, not a promise to compute every fault's frequency. Stopping at the first sampled failure is a stopping rule, so neither `1 / runs_requested` nor `1 / first_failing_run` is a justified fixed-sample failure estimate. Name the failing run and the values instead: `in run 37 of 1,000 (seed 0), when x = 0`. Preserve the deterministic error order within batches and combine batches in order; this does not promise the lowest failing run ID across all source locations. As today, shared limits/cancellation can affect which diagnostic is available.

The invocation still fails with the same language-error exit status when enrichment succeeds, and no additional model effects run. Enrichment can add bounded computation, so it is not cost-free. The library's `Error` can gain optional structured accessors for the share, its scope/basis, coverage and witness values; absent means unavailable, not zero. This stage does not need public catchable fault codes, but it does need an internal eligibility check. Grouping by source/message is diagnostic presentation only, never a recovery rule.

## Local recovery

Proposed syntax:

```probl
let x ~ d6 - 1
let y = try {
  1 / x
} catch DivisionByZero {
  0
}

report y
```

`try` evaluates normally. If a recoverable fault matches a catch, only the affected world enters that handler. Other worlds retain their successful results. The final value follows the ordinary rules for block/conditional expressions and declared types; there is no implicit `nil`, `null` or error-as-value wrapper.

Here the fallback has probability 1/6. The mean is `137/360`, about 0.380556. Continuing only the nonzero worlds instead gives a conditional mean of `137/300`, about 0.456667. That difference is intentional and should appear in documentation and regression tests.

### Catch specific fault codes

Permit multiple specific catches and an optional diagnostic binding:

```probl
# Proposed syntax and fault-code patterns.
let y = try {
  sqrt(values[index])
} catch IndexOutOfBounds {
  0
} catch DomainError as err {
  print(err.message)
  0
}
```

Fault codes are stable language patterns, not English message strings to compare. Their exact qualification/namespacing is to be settled before parsing is implemented; the examples use short names. A bound diagnostic can expose a code, message and source information without making failures ordinary model values everywhere.

Initially omit bare `catch { ... }` (implemented after all: see [as built](#as-built)), wildcard catches, an ignore-errors operator, implicit default values, exception inheritance, user-defined exception classes and `finally`. Specific catches make the fallback's scope reviewable. A value-producing `try` needs an explicit result from the handler. A statement-context handler may intentionally perform no further action, but still names the fault it handles.

Unmatched faults propagate to the nearest matching enclosing handler, including through ordinary function calls. A fault in a handler propagates outward; it is not caught again by the same handler. No automatic retry occurs. Existing narrow APIs such as `get(..., default)` and `maximum(default: ...)` remain useful and need not be rewritten using exceptions.

Prefer this explicit form over a generic `expression otherwise fallback`: an unqualified fallback would conceal which failures the author anticipated, and newly introduced faults could silently select an old default. For a simple known condition such as `x == 0`, an ordinary `if` is still the clearest option. Recovery becomes useful when a domain failure arises inside a helper or a longer calculation.

### Preserve state, choices and evidence

Use ordinary forward exception semantics, not transactional rollback:

- Successful statements and assignments before the fault remain effective in that world.
- Later operands and statements in the failed expression/block do not execute.
- A failed right-hand side does not complete its enclosing assignment. Earlier assignments inside that evaluation remain visible under the normal evaluation-order rules.
- Draws already made are not redrawn and random streams are not rewound.
- The handler receives the failing path's current weight, including earlier observations/scores. It does not receive the weight at entry to `try`.
- Block-local bindings leave scope on unwinding. The handler sees the enclosing bindings, including changes that were already committed there.
- Diagnostics already emitted by `print` are not erased. Future external effects would not be rolled back either.

For example:

```probl
# Proposed recovery syntax.
var marker = 0
let y = try {
  marker = 1
  let x ~ d6 - 1
  1 / x
} catch DivisionByZero {
  marker
}
```

The zero branch returns 1, and all continuing worlds have `marker == 1`. There is no resampling until a nonzero value appears. A `try` containing a split can therefore have both successful and recovered exits, each with its own state and weight.

A rejected observation is not an exception. `observe false` removes that world; it does not run a catch. Proving the whole evidence impossible is an inference failure, not an implicit fallback opportunity.

Handled faults count as recoveries, not failed worlds. They may appear in statistics or traces, but need not emit a warning for every deliberate fallback. If one world recovers several times, its recovery-event count can exceed one; that count is not a probability and must not be added to unhandled failure mass.

## Which failures are recoverable?

Start with a documented allowlist of value-dependent faults, rather than catching every current `Language` error.

| Candidate category | Examples | Proposed treatment |
|---|---|---|
| `DivisionByZero` | Real/complex division or remainder by zero | Catchable; terminates only the affected world in partial mode |
| `DomainError` | Real square root of a negative value; invalid value for a probability/distribution parameter | Catchable when the operation's argument types are otherwise valid |
| `IndexOutOfBounds`, `MissingKey` | A validly typed index/key that is absent | Catchable; invalid index types are a separate error |
| `EmptyCollection` | A minimum, reduction or bag draw requiring an element | Catchable; existing explicit defaults retain their own contract |
| `ConversionError` | An explicit conversion cannot represent/parse the supplied value | Catchable; distinguish this from calling the conversion with an unsupported type |
| `NumericOverflow` | A real numeric result is not finitely representable | Catchable; exhausting the host's bigint/allocation budget is not this category |
| Programming/contract errors | Wrong types or arity, unknown names, forbidden callback effects, incompatible declared shapes | Fatal whether found statically or dynamically |
| Inference/capability errors | Impossible evidence, unsupported analytic operations, unsupported modes, a proven nonterminating process | Fatal initially; do not manufacture a different model through fallback |
| Host and engine failures | Input loading, usage errors, budgets, timeout/cancellation, internal errors/panics | Fatal; not catchable as model faults |

This taxonomy needs an operation-by-operation audit. The current engine's `ErrorKind::Language` includes both domain failures and programming errors; it is too broad to use as a recovery boundary. Unclassified errors should remain fatal until deliberately assigned a code.

Optimization must preserve recoverability. Folding a well-typed constant expression such as `1 / 0` should retain a catchable fault at that expression, not turn a handled case into an unconditional compile error. Conversely, moving a type error between compile time and runtime must not suddenly make it recoverable. Checks involving contextual numeric conversion need explicit classification.

Keep handlers visible to effect and liveness analysis even when a fault seems rare. For example, a catch that observes must make its enclosing call evidence-bearing, and a catch that prints must disable print-unsafe memoization. A failed evaluation must carry the state and analytic constraints at the fault, not reconstruct them from the start of the `try` block.

## Failure modes: total or partial

A world always stops at its first unhandled fault. The failure mode decides what happens to the other worlds:

- **Total:** one world's fault fails the whole invocation, with its source diagnostic, as today.
- **Partial:** the failed world ends there, its fault and weight are recorded, and the other active or scheduled worlds finish. No placeholder value is inserted.

The names describe the scope of a failure. A name like "continue" would suggest that the failing world carries on, which it never does.

Local handlers always get the first opportunity to recover; the failure mode applies only to an unhandled recoverable fault. Fatal infrastructure and contract errors stop the invocation in either mode.

### The default depends on the mode

| Mode | Default | Why |
|---|---|---|
| Enumerate | Total | Enumeration follows represented outcomes deterministically, with floating-point arithmetic and possible unresolved tails. An encountered fault makes the model incomplete; stop and provide [stage 0](#stage-0-say-how-likely-the-failure-is) context where available. Enumeration is not a general exactness or completeness guarantee. |
| Sample | Partial | A sampled run finds a fault only when it draws a failing path, possibly late in a long run. A division by zero after an hour of sampling shouldn't discard the hour: the run finishes, and its breakdown shows that, say, 2% of runs failed and where. |

The default follows the inference mode the run actually uses, including a CLI `--mode` or `--runs` override, not just the mode the source declares. Either failure mode can be chosen explicitly:

```probl
@on_error partial

let x ~ d6 - 1
let y = 1 / x
report y
```

`@on_error total` and `@on_error partial` override the default, as could a CLI `--on-error total|partial` and the same option in the embedding API. A host may force total. Exact spelling is a proposal, not a newly supported pragma.

Resolve the effective policy once for the invocation: a host-enforced total policy takes precedence, then an explicit host/CLI override, then a source pragma, then the inference-mode default. Report both the effective inference mode and failure policy in results/replay metadata. A nested `simulate` does not acquire a different policy merely because it enumerates internally; its atomic boundary is specified below.

The sampling default needs care in four places:

- **It changes current behavior.** Today a sampled run that faults fails. With partial as its default, it prints a partial result and exits with the partial-result status rather than 0 (see [CLI, library and playground](#cli-library-and-playground)), so scripts and CI still notice. A test still fails on any unhandled fault, whatever the mode.
- **Switching modes changes the default, not the model.** Like the run count, the failure mode belongs to the run, and the result header says which one applied.
- **A rare fault can go unseen.** No failures in n runs is not proof that none can occur, just as zero empirical variation is not proof of certainty. The breakdown can say that no run failed, not that no run can.
- **A model that fails in most runs can still consume its budget.** Defer an automatic failure-fraction cap initially; progress should expose completed/failed counts so the user can cancel. If a cap is added, it must use a documented committed batch prefix and raw attempt counts, and report an early-aborted run with the actual attempts. It is not a posterior probability threshold. Sequential stopping also needs an uncertainty contract; it must not publish ordinary fixed-run error bars for a supposedly completed requested sample.

### Completing a partial run

Sampling still attempts the requested number of runs unless a fatal limit or cancellation intervenes. It does not keep drawing replacements until it has that many successful runs: that would hide failures, change the estimator and make work unbounded. A failure in the first batch must not prevent later scheduled batches from running. Enumeration continues the remaining work under its existing budgets and approximation rules.

If no positive-weight successful execution remains after the requested work, return an explicit no-successful-outcome error with failure/rejection/unresolved diagnostics. Distinguish “all attempted runs failed” from “the model is proven to fail everywhere”; sampling cannot establish the latter. If all surviving worlds were rejected by evidence but other worlds failed before that evidence, do not misdiagnose the result as simply impossible evidence.

Earlier reports can still contain useful contributions when every world later fails. Preserve those as diagnostic partial reports attached to the error, rather than returning an ordinary completed outcome or discarding them. A fatal budget/cancellation after some batches likewise does not become a completed partial run: distinguish requested, started, completed and unrun attempts.

Partial mode is for getting a usable result from a model with a fault, not a promise that any faulty program can finish: shared resource limits remain authoritative.

### As built

The failure modes are implemented as above, with these choices:

- **Faults are classified internally.** Engine errors carry an internal fault kind (division by zero, domain, index or key, empty collection, explicit conversion, overflow), assigned at each value-dependent error site. Everything else stays fatal, including declared-type checks, which are contracts. Chances that add up to more than 100% are domain faults. No fault code is public yet.
- **Comparable weights come from a static analysis.** For each statement, the compiler records whether evidence can be applied while it runs or after it in its function, including later rounds of a loop around it. A failure's weight counts with the finished worlds' only when no evidence can follow it, there or after any call it propagates through. This is conservative: evidence on a branch the failed world couldn't take still counts.
- **Calls, solved loops and recursion carry failures** the way they carry unresolved weight: per call result (memoized with it), per chain state (scaled by its expected visits, and counted as a way out of the loop) and per round of a recursive call. `simulate` and collection callbacks are atomic, as proposed.
- **Output.** The summary line says `partial result`, then the failed share (enumeration, when comparable) or the failed run count (sampling). A `failed` section after the reports lists up to ten places, with their share, or runs and share, or the raw weight when not comparable. When the weights can't be compared, the evidence is labelled as the finished worlds' contribution.
- **Hosts.** The library returns a partial result in the error (`Error::partial`), with one diagnostic per place. `probl run` prints the result, then the diagnostics, and exits with 3, or 1 when nothing finished. The playground shows the output, marks the failures in the editor, and has an "On error" option.
- **`try` and `catch`.** The syntax is `try { … } catch F { … } catch { … }`, an expression in any position. A bare `catch` is provided, as a decision against this proposal's caution: it takes every fault, but never other errors, and nothing can follow it. `catch F as err` and `finally` are not provided. The fault names are this proposal's: `DivisionByZero`, `DomainError`, `IndexOutOfBounds`, `MissingKey`, `EmptyCollection`, `ConversionError` and `NumericOverflow`.
- **How catching works.** A fault inside a `try` of its own call sends a copy of the world, as its statement began, to the try with the fault; copies are made only inside `try` bodies. A fault for a `try` further up the calls is recorded in the call's result, as in partial mode, and the call site turns it into a caught world with the caller's state and the path's weight. Liveness keeps everything a catch reads alive throughout the body, draws don't move inside a `try` body, and the observe-after-report rule treats catches as coming after their body.
- **Not built:** failure counts in the sampling progress, per-report failure breakdowns, a failure cap, stage 0's shares in total mode, and reading a fault's message in a catch.

## Reporting and the meaning of a failed world

### Counts alone are insufficient

Enumeration merges histories, and one represented world can carry most of the probability. A “worlds failed” count would change with merging, caching or a loop solver. Preserve both useful kinds of information without conflating them:

- **Enumeration:** accumulated failure weight, source/code groups and representative diagnostics. Show model-probability mass only when its denominator is justified. Counts of failed represented states are optional execution statistics, labelled as such; they are not a canonical path count.
- **Sampling:** failed runs out of attempted runs, successful runs, evidence-rejected runs and incomplete runs as applicable. The raw fraction of failed runs is execution frequency, not automatically a posterior failure probability. Keep per-run bookkeeping for estimates and uncertainty.

Terminate a path at its first unhandled fault, so it is not counted repeatedly. Different failing paths from a shared ancestor contribute their own weights. Diagnostic storage needs bounded grouping by code/source, with a bounded number of representative messages or traces; a million failures must not produce a million log lines or prevent other runs from completing.

### Reports describe the worlds that reached them

Recommended contract: in partial mode, a report describes the worlds that reached it, retaining the existing distinction between populations and per-visit contributions. A world that fails ends there, so it doesn't reach later reports, like a world that takes a branch containing no report. Nothing is removed from a report that a world reached before it failed. Failure metadata and the reporting basis remain attached in both text and structured results, not just in an optional warning on stderr. Keeping contributions does not mean the current reach denominator can remain unchanged.

For the fully explored enumerated reciprocal example without observations:

```probl
@on_error partial

let x ~ d6 - 1
let y = 1 / x
report y
```

a possible output is:

```text
enumerated · partial result · failed mass 16.67%

y    mean 0.46 · sd 0.29 · 5% 0.20 · median 0.33 · 95% 1.00 (reached in 83.33% of worlds)

DivisionByZero at model.probl:4 — failed mass 16.67%
```

The report line matches the current output for `if x != 0 { let y = 1 / x; report y }`. The mean, `137/300` ≈ 0.456667, describes the worlds that reached the report; the reach says they are 83.33% of the original population. In this particular example, all missing reach is explained by the fault. Do not invent a mean for the whole population or describe the result as equivalent to explicitly observing `x != 0`. The report resembles a branch that skips it, but the unhandled fault still makes the invocation a partial result.

Keep inference completeness separate from program success and report coverage. Today's report completeness check conservatively includes program-level unresolved weight plus the report's own missing distribution mass; it is not restricted to unresolved paths known to reach that site. A retained report can have a complete distribution for its reached population while the invocation has faults. Its point estimates need not be withdrawn, but they must carry that population basis and the invocation's partial status. A handled fallback can define a complete model result; an unhandled failure is not repaired by continuing.

### Reach needs a denominator that includes failures

For a fully resolved population with a justified common evidence measure, let `S` be successful terminal weight, `F` failed terminal weight and `R` the weight reaching an at-most-once report. Then use `R / (S + F)` for reach, and `F / (S + F)` for failure share. With no evidence, `S + F = 1`. With all relevant evidence already applied before any failure can occur, these are conditioned on that evidence. Normal report values remain normalized by their own reached weight, not by `S + F`.

The current implementation uses successful terminal weight as `Format::program_total`. Leaving that unchanged would report 100% reach after the reciprocal failure (`(5/6)/(5/6)`) and 120% reach before it (`1/(5/6)`). Partial mode needs separate successful weight, failed-prefix accounting and a qualified denominator for reach; this is required even though no report contribution is retracted.

The formula above is deliberately restricted. When failures precede unevaluated evidence, `S + F` is not established as the original model's evidence or posterior normalizer. Do not silently call `R / (S + F)` posterior reach in that case. Mark weighted reach unavailable with its reason; sampling may additionally show the raw fraction of attempted runs reaching the site, labelled as an execution count rather than weighted posterior reach. With unresolved mass, provide only justified bounds/qualifiers and do not manufacture a complete point denominator from resolved terminals alone. Per-visit reports retain their existing non-probability interpretation.

Do not attribute the global failed share to every report's missing reach. Some failed worlds already reached a report; others might never have reached it even without the fault. The initial implementation can expose global failure groups and each report's own reach/basis without claiming a per-report causal breakdown that would require additional path provenance.

### Reports reached before a later failure

```probl
@on_error partial
let x ~ d6 - 1
report x == 0
let y = 1 / x
```

Every world reaches the report before any fails, so it shows 16.67%, as it would without the division. The header's failed mass, also 16.67%, says that those worlds failed afterwards, and the failure line names the division. Catching the division instead gives the same report.

This follows from an existing rule. No `observe` or `score` may follow a `report`, so that every report sees all the evidence its worlds will ever get. A failure isn't evidence, so it doesn't revise a report already made either.

To condition on success, say so with evidence: catch the fault and `observe false` in the handler, as in `let y = try { 1 / x } catch DivisionByZero { observe false; 0 }`. That removes the world as any observation does, and the existing rule then applies: the compiler rejects the program if a report can run before that handler.

**The alternative: successful completions only.** Reports could instead describe only the worlds that complete. In the example above, `x == 0` would show 0%. This is a possible, explicitly conditional reporting contract, but it changes the population of earlier reports based on later failures. Removing contributions would require provenance or deferred accounting, including retaining only the successful descendants' share after a split. Prefer the reached-population contract here. It avoids retraction, although failure totals, reach, sample bookkeeping and API metadata still need changes. A successful-only view can be considered later as an explicit option, alongside the [portal design](portals-and-effects.md#midway-portals-evidence-and-sampled-feedback).

`print` remains diagnostic output at execution time. Its earlier lines can include values from paths that later fail, just as today; do not promise to retract them. The run summary must still show failures even when the program contains only `print` and no reports.

### Failure is not unresolved mass or evidence

Without observations, for a fully explored normalized finite model, successful terminal mass plus failed terminal mass is 1. With legitimate unresolved paths it is necessary to account for those separately. This is the simple case in which “failed probability 1/6” is justified.

With evidence, a failure carries the weight accumulated **up to its failure point**. Later likelihoods may never be evaluated. Neither `F/S` nor `F/(S+F)` generally gives the original model's posterior failure probability when `F` uses failed-prefix weights and `S` final successful weights. Density observations can also produce weights above one; failure weights must not automatically be formatted as percentages.

For example, draw `x` uniformly from 0 and 1, evaluate `1 / x`, then apply `observe x == 1` before reporting. Failure at `x = 0` has prefix weight 1/2. That path never executes the later observation, which would reject it if evaluated. The program encountered a fault in half its prior mass; it has not measured a 50% failure probability conditional on the final evidence. Probl must not run future statements on failed worlds to try to supply the missing likelihood.

Therefore:

- Retain extended-range failed-prefix weights, source location and enough evidence-scope information to interpret them.
- Report a normalized failure probability only under a proven common measure, for example no evidence, or all relevant evidence having been applied before failure is possible. Otherwise show counts and clearly labelled prefix-weight diagnostics, with posterior failure probability unavailable.
- With unhandled failures, the accumulated successful evidence contribution is not an unqualified evidence value for the original model. Expose that contribution separately, retaining probability-versus-density units. Only expose a full evidence value from successful plus failed weights when the common evidence measure is established and no relevant mass is unresolved. Otherwise mark it unavailable/partial, exposing bounds only if justified; do not reuse today's unresolved-only completeness flag or bounds without accounting for faults.
- Never insert failures into the existing unresolved bucket. A failed calculation is known to be invalid; an unresolved path has not been resolved. Existing probability bounds cannot simply be reused, especially with later density observations.
- Sampling summaries keep the original attempted-run count. Each report's estimate uses its recorded contributions, with uncertainty estimated per run. A run that reported and later failed keeps those contributions and their squared moments; a run that failed before the report contributes no value there. Retain separate denominators/moments for report statistics, reach where justified, successful evidence contribution and failure counts. Failed/rejected runs do not vanish from invocation-wide denominators or get replaced by fresh runs. A runtime fault must not discard its whole batch's already accumulated report data.

## Scope boundaries: functions, recipes and simulate

Ordinary functions execute within the caller's model. A function that branches can return successful paths and propagate failed paths to the caller's enclosing handler or global policy. Memoization must cache and scale this weighted structure, rather than either losing failures or repeating a cached failure once per cache hit without its weight.

Distribution values require a deliberate boundary. Initially treat an operation on a recipe as one operation in its current world:

```probl
let recipe = d6 - 1
report 1 / recipe         # constructing this transformed recipe encounters zero
```

Do not silently remove the invalid outcome from that recipe and renormalize it. If recipe algebra cannot construct a valid result, the operation faults in the enclosing world. To handle failure separately for realized outcomes, draw explicitly and use `try` or partial mode. Extending `dist[T]` to carry failed outcomes would be a separate representational change.

For the same reason, the first design should keep `simulate` atomic with respect to **unhandled** local faults. A catch inside its model can define an alternative outcome. Otherwise an invalid local path makes construction of the local distribution fail, propagating to a catch around `simulate` or failing its enclosing world. Partial mode must not silently normalize only the successful local outcomes into an apparently complete distribution. This conservative boundary should be confirmed before implementation; supporting partial local inference requires explicit failure-bearing results.

An outer handler receives the enclosing world's weight and state after evaluating the inputs to `simulate`, not the failing local branch's likelihood. Observations and mutations inside the local inference scope remain local. If one outer world constructs a `simulate` result with a 1/6 invalid local branch, the construction fails for that entire outer world: 1/6 is local diagnostic information, not its outer failed mass. Do not count the same fault once inside `simulate` and again when it propagates. Recipe construction follows the same distinction between inner outcomes and outer execution; this is atomic result construction, not rollback of earlier outer statements or already emitted diagnostics.

Empty posteriors and unsupported local continuous inference remain inference/capability errors, not recoverable numeric faults. Analytic evaluation must not guess the measure of an invalid continuous region; unsupported representation remains fatal until a mathematically defined operation exists.

## CLI, library and playground

Suggested execution statuses distinguish an ordinary completion, completion with unhandled world failures, and an invocation that failed. These are separate from whether numerical estimates have unresolved mass or sampling uncertainty.

For the CLI, recommend exit status 0 for completion without unhandled failures, a distinct nonzero status for a partial result (candidate: 3), and the existing fatal-error statuses otherwise. Partial output, whether chosen or the sampling default, must not look like an ordinary success to shell automation. Caught faults do not force a nonzero status.

Changing sampled faults from `Err` to `Ok(Outcome)` in the existing library API would silently change what `program.run(&options)?` accepts. Merely adding status accessors cannot require callers to inspect them. Recommend keeping unhandled faults on the error path of the existing `run`/`run_with` API, with an optional structured partial outcome attached when execution completed in partial mode. The CLI/playground can extract and display it while still signaling non-success. Partial is still the sampling execution default: it controls how much work finishes, not whether a faulty run becomes an ordinary Rust success.

A future explicitly chosen status-bearing execution API may return distinct completed/partial variants for hosts that want to handle both. All-failed and fatal/aborted executions remain errors, potentially carrying diagnostic reports and aggregate failures. Audit compatibility against the published API when choosing the exact types/accessors.

Partial outcomes need effective policy, failure groups and coverage, plus each report's reached-population basis and qualified reach. The [library guide](../library.md#compatibility-and-remaining-work) lists reach accessors as deferred; partial mode needs them, including an unavailable reason where weights are not comparable. Audit completion, evidence, point/bound and sampling accessors together rather than adding a failure list beside otherwise misleading numbers.

The playground should show “partial result,” failure counts or justified mass, source locations and the affected reporting basis prominently. Running more samples and discovering an error should be presented as finding an invalid outcome, not as a suggestion to reduce runs until the error disappears.

Future module initialization and prologue/portal operations run outside ordinary world continuation. Their failures need scope-appropriate preparation/host-operation handling, not an instruction to drop some model worlds. This does not preclude a later explicit local recovery contract for those scopes. No new I/O, tasks or portal implementation is needed for this proposal. The [test proposal](imports-and-tests.md) must fail a test on any unhandled model fault regardless of partial execution; one test's failure need not stop other isolated tests.

## Implementation order and validation

This work is relevant now. Implement it separately from side effects, in stages that keep today's total behavior until each contract is ready:

0. **Informative total-mode diagnostics.** Preserve a bounded witness; enrich eligible pure-operation failures with a scoped share over already available inputs. Do not finish arbitrary statements, replay effects or promise a percentage for every fault. When sampling, name the run and the values ([stage 0](#stage-0-say-how-likely-the-failure-is)). No language change, and independently useful.
1. **Fault taxonomy and diagnostics.** Add stable codes and explicit recoverability to operation/runtime errors. Classify built-ins and boundary checks. Keep unknown kinds fatal.
2. **Local recovery.** Add `try`/specific catches to parsing, lowering and effect/liveness analysis. Represent successful and failed exits with their state, weight and unwind destination; preserve left-to-right evaluation and current mutation semantics.
3. **Failure propagation through inference.** Extend call summaries, memoization, recursion and chain solving. A recoverable unhandled fault is a terminal outcome for the relevant execution scope; caught faults follow their handler. Solver discovery/iterations must not double-count failure diagnostics. Disable only optimizations that cannot yet preserve the contract, with bounded fallback behavior.
4. **Partial mode and failure accounting.** A failed world stops, preserving earlier report contributions. Add failed terminal paths, qualified reach denominators, evidence status, per-run moments and bounded diagnostic aggregation; do not discard partial batches or divide earlier report weight by successful terminals alone. A new `try/catch` around the engine's current `Result` is insufficient.
5. **Hosts and documentation.** Add CLI/API options, partial-result status, playground display and normative semantics together. Preserve the existing library error channel while making partial outcomes inspectable. Make partial the sampling default only at this stage, once the breakdown is complete; enumeration keeps total as its default.

Acceptance cases should cover:

- The reciprocal model: 1/6 failed mass; the report after the failure reached by 5/6 with mean `137/300`, and printed as `if x != 0 { let y = 1 / x; report y }` prints today; zero-fallback mean `137/360`; no sampled retries to replace failures.
- Total-mode diagnostics: the pure-operation share (1/6 for the reciprocal model), including preceding evidence; a witness captured at evaluation time; no added print/draw/evidence/mutation while enriching an error; unavailable or scoped shares for incomplete inputs, callbacks, loops, repeated calls, recipes and solver discovery; bounded enrichment and deterministic batch-order diagnostics.
- Specific/multiple/nested catches, helper calls, handler failures, lazy fallback execution and errors in later operands.
- State and weight preservation through a caught error, including earlier mutations, draws, observations and printed output.
- Rejected evidence versus faults, all-failed versus impossible/rare evidence, and failure before later observations or densities.
- Reports before and after a failure, including an earlier `x == 0` report retaining probability 1/6 and reach 100%, and a later report having reach 5/6; no erroneous 120%/100% denominators; branching after an earlier report, grouped/per-visit reports and recovered worlds that complete successfully.
- Failures after common evidence versus before later observations/densities: justified reach/evidence only, with explicit unavailable bases elsewhere; global failed mass is not blindly assigned to each report's missing reach.
- Conditioning on success only through explicit evidence: `observe false` in a handler, rejected by the compiler when a report can run before it.
- Enumeration with merging/memoization/solvers on and off: matching result and failure weights, even when represented-world counts differ.
- Sampling across run counts, batch boundaries and thread counts; fixed attempted-run counts, reports and moments retained from runs that later fail, no discarded successful work in a fault-containing batch, and uncertainty calibration against independent answers.
- Recipe/`simulate` atomicity, local catches inside `simulate`, outer handlers receiving outer weights rather than local failed likelihoods, no double counting, and no silent normalization of failed outcomes.
- Fatal budgets/cancellation/unsupported/internal/type errors remaining fatal even inside `try` and in partial mode.
- Defaults: total when enumerating and partial when sampling, following the mode actually used, including CLI overrides; explicit overrides either way; a host forcing total; the partial-result exit status for a sampled run with failures.
- Existing library `run(...)?` still taking its error path for unhandled faults, inspectable partial outcomes, all-failed runs retaining earlier diagnostic reports, and cancellation/limits distinguished from completed partial execution.
- Handler effects preserved through helper calls and optimizations, including evidence-after-report rejection and print-aware caching; an unused value whose evaluation faults cannot simply be deleted inside a `try`.
- Native/WASM parity, bounded error logs, host API status and text/result agreement.

The [rational oracle](../../crates/probl-oracle/src/lib.rs) should independently model the supported discrete fault paths. Tiny exact models are especially useful for checking that recovery preserves mass and that partial mode neither hides nor double-counts it.

Implementation references: [runtime errors](../../crates/probl-engine/src/error.rs), [interpreter](../../crates/probl-engine/src/interp.rs), [world flow](../../crates/probl-engine/src/world.rs), [execution and batching](../../crates/probl-engine/src/lib.rs), [report calculations](../../crates/probl-engine/src/report/results.rs), and [public errors](../../crates/probl/src/error.rs).
