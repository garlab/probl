# Error handling: local recovery and failed worlds

> Draft proposal, 7 October 2026. Intended for near-term language work, independently of [portals and external effects](portals-and-effects.md). Nothing here is implemented yet. Proposed syntax, fault codes and output are illustrative; the [current semantics](../semantics.md) still stops execution on a runtime error.

## Recommendation

First, make today's behavior, where any fault stops the run, more informative: say how likely the failure is and which values cause it ([stage 0](#stage-0-say-how-likely-the-failure-is)). That needs no language change. Then introduce two complementary mechanisms:

1. **Explicit local recovery:** an expression-valued `try` block with catches for named recoverable faults. A handler supplies an ordinary value or executes ordinary code; no nullable value, mandatory result wrapper or monadic API is introduced.
2. **A failure mode for unhandled faults:** *total*, where one world's fault fails the run as today, or *partial*, where the failed world ends and the others finish. Total is the default when enumerating, partial when sampling. A failed world stops reaching later reports, like a world that takes another branch. Each report keeps its current meaning, the worlds that reached it, and continued results must expose the failed mass and retained failure diagnostics.

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

When enumerating (`--mode enumerate` for this example), the interpreter executes a statement for all its worlds together. On a value-dependent fault, it could finish that statement for the other worlds, then stop with what it found:

```text
Error: division by zero
   ╭─[ model.probl:4:7 ]
   │
 4 │ print(1 / x)
   │       ──┬──
   │         ╰──── here
   │
   │ Note: fails in 16.67% of the worlds reaching this statement, for example when x = 0
───╯
```

- **The share is justified.** The failing weight and the weight of the worlds reaching the statement are measured at the same point, after the same evidence. Their ratio is the probability of failing there, given the evidence so far. Later evidence doesn't matter, because the run stops. Unlike a failed mass gathered over a whole continued run, it needs no caveat about evidence applied afterwards (see [failure is not unresolved mass or evidence](#failure-is-not-unresolved-mass-or-evidence)).
- **The example comes from one failing world:** the values of the variables that the failing expression reads, rather than its operands alone. `x = 0` says more than `1 / 0`.
- **Finishing the statement stays bounded.** It runs under the same limits. Another kind of fault in another world is grouped by source location and message, and only the first one is shown in full. A resource limit, a cancellation or an internal error stops at once, as today.

This is the diagnostic every enumerated run gets by default. When sampling in total mode, a single failure says nothing about its probability, so don't estimate one. Name the failing run and the values instead: `in run 37 of 1,000 (seed 0), when x = 0`. As today, the reported error is the first in batch order, so the same seed always reports the same run.

The run still fails, with the same exit status, so no program's behavior changes. The library's `Error` can expose the share and the example values through new accessors, without breaking the published API. This stage doesn't need the fault codes of stage 1: it can group by source location and message.

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

Initially omit bare `catch { ... }`, wildcard catches, an ignore-errors operator, implicit default values, exception inheritance, user-defined exception classes and `finally`. Specific catches make the fallback's scope reviewable. A value-producing `try` needs an explicit result from the handler. A statement-context handler may intentionally perform no further action, but still names the fault it handles.

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

## Failure modes: total or partial

A world always stops at its first unhandled fault. The failure mode decides what happens to the other worlds:

- **Total:** one world's fault fails the whole invocation, with its source diagnostic, as today.
- **Partial:** the failed world ends there, its fault and weight are recorded, and the other active or scheduled worlds finish. No placeholder value is inserted.

The names describe the scope of a failure. A name like "continue" would suggest that the failing world carries on, which it never does.

Local handlers always get the first opportunity to recover; the failure mode applies only to an unhandled recoverable fault. Fatal infrastructure and contract errors stop the invocation in either mode.

### The default depends on the mode

| Mode | Default | Why |
|---|---|---|
| Enumerate | Total | Enumeration is exact and deterministic, so the default favors correctness: a fault on any outcome means the answer isn't the model's. The fault is found as soon as its statement runs, and [stage 0](#stage-0-say-how-likely-the-failure-is) says how much of the model it affects. |
| Sample | Partial | A sampled run finds a fault only when it draws a failing path, possibly late in a long run. A division by zero after an hour of sampling shouldn't discard the hour: the run finishes, and its breakdown shows that, say, 2% of runs failed and where. |

The default follows the mode the run actually uses, including a CLI `--mode` or `--runs` override, not the mode the source declares. Either mode can be chosen explicitly:

```probl
@on_error partial

let x ~ d6 - 1
let y = 1 / x
report y
```

`@on_error total` and `@on_error partial` override the default, as could a CLI `--on-error total|partial` and the same option in the embedding API. A host may force total. Exact spelling is a proposal, not a newly supported pragma.

The sampling default needs care in four places:

- **It changes current behavior.** Today a sampled run that faults fails. With partial as its default, it prints a partial result and exits with the partial-result status rather than 0 (see [CLI, library and playground](#cli-library-and-playground)), so scripts and CI still notice. A test still fails on any unhandled fault, whatever the mode.
- **Switching modes changes the default, not the model.** Like the run count, the failure mode belongs to the run, and the result header says which one applied.
- **A rare fault can go unseen.** No failures in n runs is not proof that none can occur, just as zero empirical variation is not proof of certainty. The breakdown can say that no run failed, not that no run can.
- **A model that fails in most runs still uses the whole hour.** Partial mode trades a crash at the end for an hour of mostly failed runs. An open question is a cap, such as `@on_error partial(max_failed: 5%)`, that makes the run total once the failed share of attempted runs exceeds it. Checked when batches are combined in order, it wouldn't depend on the thread count. Meanwhile, the playground's progress display should show failures as they accumulate, so a user can stop early.

### Partial mode when sampling

Sampling still attempts the requested number of runs. It does not keep drawing replacements until it has that many successful runs: that would hide failures, change the estimator and make work unbounded. A failure in the first batch must not prevent later scheduled batches from running.

If no positive-weight successful execution remains after the requested work, return an explicit no-successful-outcome error with failure/rejection/unresolved diagnostics. Distinguish “all attempted runs failed” from “the model is proven to fail everywhere”; sampling cannot establish the latter. If all surviving worlds were rejected by evidence but other worlds failed before that evidence, do not misdiagnose the result as simply impossible evidence.

Partial mode is for getting a usable result from a model with a fault, not a promise that any faulty program can finish: shared resource limits remain authoritative.

## Reporting and the meaning of a failed world

### Counts alone are insufficient

Enumeration merges histories, and one represented world can carry most of the probability. A “worlds failed” count would change with merging, caching or a loop solver. Preserve both useful kinds of information without conflating them:

- **Enumeration:** accumulated failure weight, source/code groups and representative diagnostics. Show model-probability mass only when its denominator is justified. Counts of failed represented states are optional execution statistics, labelled as such; they are not a canonical path count.
- **Sampling:** failed runs out of attempted runs, successful runs, evidence-rejected runs and incomplete runs as applicable. The raw fraction of failed runs is execution frequency, not automatically a posterior failure probability. Keep per-run bookkeeping for estimates and uncertainty.

Terminate a path at its first unhandled fault, so it is not counted repeatedly. Different failing paths from a shared ancestor contribute their own weights. Diagnostic storage needs bounded grouping by code/source, with a bounded number of representative messages or traces; a million failures must not produce a million log lines or prevent other runs from completing.

### Reports describe the worlds that reached them

Recommended contract: in partial mode, a report means what it means today, the worlds that reached it, with the existing reach and per-visit distinctions. A world that fails ends there, so it doesn't reach later reports, exactly like a world that takes a branch containing no report. Nothing is removed from a report that a world reached before it failed. Failure metadata remains attached in both text and structured results, not just in an optional warning on stderr.

For the complete enumerated reciprocal example without observations:

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

The report line is exactly what Probl prints today for `if x != 0 { let y = 1 / x; report y }`: partial mode adds the failure lines, not a new way of reading reports. The mean, 137/300 ≈ 0.456667, describes the worlds that reached the report; the reach says they are 83.33% of the whole, and the failure line says why the rest are missing. Do not invent a mean for the whole population, and do not describe this result as equivalent to explicitly observing `x != 0`: it is equivalent to a branch, not to evidence.

Reports keep their existing completeness meaning, which concerns unresolved weight among the worlds that reached them, and a report that every world reaches before any failure is unaffected. What a reader needs in addition is that part of a report's missing reach failed rather than took another branch: the header and the structured results say so, and the run's status is partial. A handled fallback can be a complete model result; an unhandled failure is not repaired by continuing.

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

**The alternative: successful completions only.** Reports could instead describe only the worlds that complete. In the example above, `x == 0` would then show 0%, with the zero world's contribution removed when it later fails. That is an implicit observation of success at the end of the program: the evidence-after-report pattern the language rejects, and a contradiction of this proposal's rule that failures aren't evidence. It would also be the costliest part of partial mode. Reports write into their sinks as worlds reach them, so removing a contribution afterwards needs report provenance or deferred contributions with bounded storage, and a contribution made before a later split must keep only its successful descendants' share. Under the recommended contract, the sinks don't change. If a successful-only view proves useful, it can come later as an explicit option with that machinery. Reports that change as later evidence or failures arrive belong with the [portal design](portals-and-effects.md#midway-portals-evidence-and-sampled-feedback).

`print` remains diagnostic output at execution time. Its earlier lines can include values from paths that later fail, just as today; do not promise to retract them. The run summary must still show failures even when the program contains only `print` and no reports.

### Failure is not unresolved mass or evidence

Without observations, for a fully explored normalized finite model, successful terminal mass plus failed terminal mass is 1. With legitimate unresolved paths it is necessary to account for those separately. This is the simple case in which “failed probability 1/6” is justified.

With evidence, a failure carries the weight accumulated **up to its failure point**. Later likelihoods may never be evaluated. Summing failed-prefix weights and dividing by final successful weight does not generally produce a posterior probability of failure. Density observations can also produce weights above one; failure weights must not automatically be formatted as percentages.

Therefore:

- Retain extended-range failed-prefix weights, source location and enough evidence-scope information to interpret them.
- Report a normalized failure probability only under a proven common measure, for example no evidence, or all relevant evidence having been applied before failure is possible. Otherwise show counts and clearly labelled prefix-weight diagnostics, with posterior failure probability unavailable.
- With unhandled failures, the accumulated successful evidence contribution is not an unqualified evidence value for the original model. The evidence API/header must expose the limitation and retain probability-versus-density units.
- Never insert failures into the existing unresolved bucket. A failed calculation is known to be invalid; an unresolved path has not been resolved. Existing probability bounds cannot simply be reused, especially with later density observations.
- Sampling summaries keep the original attempted-run count. Each report's estimate uses the runs that reached it, as reach does today, with uncertainty estimated per run. Failed and rejected runs stay in the bookkeeping rather than silently reducing the sample size.

## Scope boundaries: functions, recipes and simulate

Ordinary functions execute within the caller's model. A function that branches can return successful paths and propagate failed paths to the caller's enclosing handler or global policy. Memoization must cache and scale this weighted structure, rather than either losing failures or repeating a cached failure once per cache hit without its weight.

Distribution values require a deliberate boundary. Initially treat an operation on a recipe as one operation in its current world:

```probl
let recipe = d6 - 1
report 1 / recipe         # constructing this transformed recipe encounters zero
```

Do not silently remove the invalid outcome from that recipe and renormalize it. If recipe algebra cannot construct a valid result, the operation faults in the enclosing world. To handle failure separately for realized outcomes, draw explicitly and use `try` or partial mode. Extending `dist[T]` to carry failed outcomes would be a separate representational change.

For the same reason, the first design should keep `simulate` atomic with respect to **unhandled** local faults. A catch inside its model can define an alternative outcome. Otherwise an invalid local path makes construction of the local distribution fail, propagating to a catch around `simulate` or failing its enclosing world. The global continue option must not silently normalize only the successful local outcomes into an apparently complete distribution. This conservative boundary should be confirmed before implementation; supporting partial local inference requires explicit failure-bearing results.

Empty posteriors and unsupported local continuous inference remain inference/capability errors, not recoverable numeric faults. Analytic evaluation must not guess the measure of an invalid continuous region; unsupported representation remains fatal until a mathematically defined operation exists.

## CLI, library and playground

Suggested execution statuses distinguish an ordinary completion, completion with unhandled world failures, and an invocation that failed. These are separate from whether numerical estimates have unresolved mass or sampling uncertainty.

For the CLI, recommend exit status 0 for completion without unhandled failures, a distinct nonzero status for a partial result (candidate: 3), and the existing fatal-error statuses otherwise. Partial output, whether chosen or the sampling default, must not look like an ordinary success to shell automation. Caught faults do not force a nonzero status.

The library can return a partial `Outcome` in partial mode, with mandatory status/failure accessors. A report's qualifier is its reach; the [library guide](../library.md#compatibility-and-remaining-work) lists reach accessors as deferred, and partial mode needs them. Total mode and all-failed invocations still return an error; errors should carry useful aggregate failure diagnostics where available. Audit the existing completion/evidence accessors rather than adding a failure list beside otherwise misleading numbers.

The playground should show “partial result,” failure counts or justified mass, source locations and the affected reporting basis prominently. Running more samples and discovering an error should be presented as finding an invalid outcome, not as a suggestion to reduce runs until the error disappears.

Future prologue/portal operations run once outside the world population. Their failures should use explicit host-operation handling, not the failure mode. No new I/O, tasks or portal implementation is needed for this proposal.

## Implementation order and validation

This work is relevant now. Implement it separately from side effects, in stages that keep today's total behavior until each contract is ready:

0. **Informative total-mode diagnostics.** When enumerating, finish the failing statement and report the failing share and an example; when sampling, name the run and the values ([stage 0](#stage-0-say-how-likely-the-failure-is)). No language change, and independently useful.
1. **Fault taxonomy and diagnostics.** Add stable codes and explicit recoverability to operation/runtime errors. Classify built-ins and boundary checks. Keep unknown kinds fatal.
2. **Local recovery.** Add `try`/specific catches to parsing, lowering and effect/liveness analysis. Represent successful and failed exits with their state, weight and unwind destination; preserve left-to-right evaluation and current mutation semantics.
3. **Failure propagation through inference.** Extend call summaries, memoization, recursion and chain solving. A recoverable unhandled fault is a terminal outcome for the relevant execution scope; caught faults follow their handler. Solver discovery/iterations must not double-count failure diagnostics. Disable only optimizations that cannot yet preserve the contract, with bounded fallback behavior.
4. **Partial mode and failure accounting.** A failed world leaves the population like a world that ends; the report sinks don't change. Finish the failed mass, failure/evidence metadata, per-run accumulation and bounded diagnostic aggregation before exposing partial mode to users. A new `try/catch` around the engine's current `Result` is insufficient.
5. **Hosts and documentation.** Add CLI/API options, partial-result status, playground display and normative semantics together. Make partial the sampling default only at this stage, once the breakdown is complete; enumeration keeps total as its default.

Acceptance cases should cover:

- The reciprocal model: 1/6 failed mass; the report after the failure reached by 5/6 with mean `137/300`, and printed as `if x != 0 { let y = 1 / x; report y }` prints today; zero-fallback mean `137/360`; no sampled retries to replace failures.
- Total-mode diagnostics: the failing share at the statement (1/6 for the reciprocal model), with evidence applied before it; an example world's values; grouping of different faults in one statement; immediate stops on limits and cancellation; the first failing run in batch order when sampling, for any thread count.
- Specific/multiple/nested catches, helper calls, handler failures, lazy fallback execution and errors in later operands.
- State and weight preservation through a caught error, including earlier mutations, draws, observations and printed output.
- Rejected evidence versus faults, all-failed versus impossible/rare evidence, and failure before later observations or densities.
- Reports before and after a failure, including a report before the failure keeping the failing worlds' contribution (`x == 0` at 1/6), branching after an earlier report, grouped/per-visit reports and recovered worlds that complete successfully.
- Conditioning on success only through explicit evidence: `observe false` in a handler, rejected by the compiler when a report can run before it.
- Enumeration with merging/memoization/solvers on and off: matching result and failure weights, even when represented-world counts differ.
- Sampling across run counts, batch boundaries and thread counts; fixed attempted-run counts, estimators over the runs reaching each report, and uncertainty calibration against independent answers.
- Recipe/`simulate` atomicity, local catches inside `simulate`, and no silent normalization of failed outcomes.
- Fatal budgets/cancellation/unsupported/internal/type errors remaining fatal even inside `try` and in partial mode.
- Defaults: total when enumerating and partial when sampling, following the mode actually used, including CLI overrides; explicit overrides either way; a host forcing total; the partial-result exit status for a sampled run with failures.
- Native/WASM parity, bounded error logs, host API status and text/result agreement.

The [rational oracle](../../crates/probl-oracle/src/lib.rs) should independently model the supported discrete fault paths. Tiny exact models are especially useful for checking that recovery preserves mass and that partial mode neither hides nor double-counts it.

Implementation references: [runtime errors](../../crates/probl-engine/src/error.rs), [interpreter](../../crates/probl-engine/src/interp.rs), [world flow](../../crates/probl-engine/src/world.rs), [execution and batching](../../crates/probl-engine/src/lib.rs), [report calculations](../../crates/probl-engine/src/report/results.rs), and [public errors](../../crates/probl/src/error.rs).
