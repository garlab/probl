# Error handling: local recovery and failed worlds

> Draft proposal, 7 October 2026. Intended for near-term language work, independently of [portals and external effects](portals-and-effects.md). Nothing here is implemented yet. Proposed syntax, fault codes and output are illustrative; the [current semantics](../semantics.md) still stops execution on a runtime error.

## Recommendation

Introduce two complementary mechanisms:

1. **Explicit local recovery:** an expression-valued `try` block with catches for named recoverable faults. A handler supplies an ordinary value or executes ordinary code; no nullable value, mandatory result wrapper or monadic API is introduced.
2. **A global unhandled-fault policy:** stop by default, with an explicit option to terminate failed worlds and finish the remaining ones. Continued results must expose their successful-only population and retained failure diagnostics.

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
| Inspect the valid part of a faulty model | Continue other worlds, retain the failure mass, and label summaries as conditional on successful completion |

These must not be interchangeable spellings for silently discarding data.

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
| `DivisionByZero` | Real/complex division or remainder by zero | Catchable; may terminate only the affected world under continue mode |
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

## Global policy: stop or continue

Suggested spelling:

```probl
@on_error continue

let x ~ d6 - 1
let y = 1 / x
report y
```

`@on_error stop` is the default. The CLI could override with `--on-error stop|continue`, and the embedding API should expose the same policy. The host may force the stricter policy. Exact spelling is a proposal, not a newly supported pragma.

Local handlers always get the first opportunity to recover. The global policy applies only to an unhandled recoverable fault:

- **Stop:** abort the invocation with its source diagnostic, as today.
- **Continue:** terminate that world, record its fault and weight, and continue other active or scheduled worlds. No placeholder value is inserted.

Sampling still attempts the requested number of runs. It does not keep drawing replacements until it has that many successful runs: that would hide failures, change the estimator and make work unbounded. A failure in the first batch must not prevent later scheduled batches from running under continue mode. Fatal infrastructure/contract errors still stop the invocation.

If no positive-weight successful execution remains after the requested work, return an explicit no-successful-outcome error with failure/rejection/unresolved diagnostics. Distinguish “all attempted runs failed” from “the model is proven to fail everywhere”; sampling cannot establish the latter. If all surviving worlds were rejected by evidence but other worlds failed before that evidence, do not misdiagnose the result as simply impossible evidence.

Continuation is for inspecting a partial result, not a promise that any faulty program can finish: shared resource limits remain authoritative.

## Reporting and the meaning of a failed world

### Counts alone are insufficient

Enumeration merges histories, and one represented world can carry most of the probability. A “worlds failed” count would change with merging, caching or a loop solver. Preserve both useful kinds of information without conflating them:

- **Enumeration:** accumulated failure weight, source/code groups and representative diagnostics. Show model-probability mass only when its denominator is justified. Counts of failed represented states are optional execution statistics, labelled as such; they are not a canonical path count.
- **Sampling:** failed runs out of attempted runs, successful runs, evidence-rejected runs and incomplete runs as applicable. The raw fraction of failed runs is execution frequency, not automatically a posterior failure probability. Keep per-run bookkeeping for estimates and uncertainty.

Terminate a path at its first unhandled fault, so it is not counted repeatedly. Different failing paths from a shared ancestor contribute their own weights. Diagnostic storage needs bounded grouping by code/source, with a bounded number of representative messages or traces; a million failures must not produce a million log lines or prevent other runs from completing.

### Successful results must say what they describe

Recommended contract: ordinary reports under continue mode describe **successful completions that reached each report**, with normal reach/per-visit distinctions retained. All numeric summaries and probability tables must identify that successful-only population. Failure metadata remains attached in both text and structured results, not just in an optional warning on stderr.

For the complete enumerated reciprocal example without observations, a possible header is:

```text
enumerated · partial result · failed mass 16.67% · successful mass 83.33%
reports conditional on successful completion

DivisionByZero at model.probl:4 — failed mass 16.67%
```

The successful mean is about 0.456667; the missing zero case has no reciprocal. Do not invent a mean for the whole population or describe this result as equivalent to explicitly observing `x != 0`.

The user-facing distinction between “complete” and “incomplete” also needs to include failures. A report with no unresolved tail must not imply full requested-model coverage when known faults excluded paths. Keep a separate population/basis qualifier and failure coverage in estimates. A handled fallback can be a complete model result; an unhandled failure is not repaired merely by conditioning on success.

### Reports reached before a later failure

This choice needs to be explicit:

```probl
# Proposed global policy.
@on_error continue
let x ~ d6 - 1
report x == 0
let y = 1 / x
```

Under the recommended successful-completion contract, the report is 0% among successful completions, with 1/6 failure mass shown separately. Its already-recorded contribution from the zero world must be excluded when that world later fails. Catching the later division instead would keep the recovered world and its earlier report contribution.

An alternative is to retain reports as snapshots of the worlds that reached them, including paths that later fail. That can be useful, but needs per-report failure/cohort labels and is a different contract. Prefer one consistent successful-completion meaning initially; explicit snapshot reporting belongs with the [portal design](portals-and-effects.md#midway-portals-evidence-and-sampled-feedback).

This has an implementation cost: reports currently write directly into shared sinks. Simply dropping a world from the active vector cannot undo its earlier contributions, particularly after splitting and merging. Correct continuation needs report provenance or deferred contributions with bounded storage. A contribution made before a later split must retain only its successful descendants' share, not be retained or removed wholesale with an ancestor.

`print` remains diagnostic output at execution time. Its earlier lines can include values from paths that later fail, just as today. Do not promise to retract them or treat them as successful-only report data. The run summary must still show failures even when the program contains only `print` and no reports.

### Failure is not unresolved mass or evidence

Without observations, for a fully explored normalized finite model, successful terminal mass plus failed terminal mass is 1. With legitimate unresolved paths it is necessary to account for those separately. This is the simple case in which “failed probability 1/6” is justified.

With evidence, a failure carries the weight accumulated **up to its failure point**. Later likelihoods may never be evaluated. Summing failed-prefix weights and dividing by final successful weight does not generally produce a posterior probability of failure. Density observations can also produce weights above one; failure weights must not automatically be formatted as percentages.

Therefore:

- Retain extended-range failed-prefix weights, source location and enough evidence-scope information to interpret them.
- Report a normalized failure probability only under a proven common measure, for example no evidence, or all relevant evidence having been applied before failure is possible. Otherwise show counts and clearly labelled prefix-weight diagnostics, with posterior failure probability unavailable.
- With unhandled failures, the accumulated successful evidence contribution is not an unqualified evidence value for the original model. The evidence API/header must expose the limitation and retain probability-versus-density units.
- Never insert failures into the existing unresolved bucket. A failed calculation is known to be invalid; an unresolved path has not been resolved. Existing probability bounds cannot simply be reused, especially with later density observations.
- Sampling summaries use the appropriate successful contributions and denominators, keep the original attempted-run count, and estimate uncertainty per run. Failed/rejected contributions do not disappear from estimator bookkeeping through a silent reduction of the sample size.

## Scope boundaries: functions, recipes and simulate

Ordinary functions execute within the caller's model. A function that branches can return successful paths and propagate failed paths to the caller's enclosing handler or global policy. Memoization must cache and scale this weighted structure, rather than either losing failures or repeating a cached failure once per cache hit without its weight.

Distribution values require a deliberate boundary. Initially treat an operation on a recipe as one operation in its current world:

```probl
let recipe = d6 - 1
report 1 / recipe         # constructing this transformed recipe encounters zero
```

Do not silently remove the invalid outcome from that recipe and renormalize it. If recipe algebra cannot construct a valid result, the operation faults in the enclosing world. To handle failure separately for realized outcomes, draw explicitly and use `try` or continue mode. Extending `dist[T]` to carry failed outcomes would be a separate representational change.

For the same reason, the first design should keep `simulate` atomic with respect to **unhandled** local faults. A catch inside its model can define an alternative outcome. Otherwise an invalid local path makes construction of the local distribution fail, propagating to a catch around `simulate` or failing its enclosing world. The global continue option must not silently normalize only the successful local outcomes into an apparently complete distribution. This conservative boundary should be confirmed before implementation; supporting partial local inference requires explicit failure-bearing results.

Empty posteriors and unsupported local continuous inference remain inference/capability errors, not recoverable numeric faults. Analytic evaluation must not guess the measure of an invalid continuous region; unsupported representation remains fatal until a mathematically defined operation exists.

## CLI, library and playground

Suggested execution statuses distinguish an ordinary completion, completion with unhandled world failures, and an invocation that failed. These are separate from whether numerical estimates have unresolved mass or sampling uncertainty.

For the CLI, recommend exit status 0 for completion without unhandled failures, a distinct nonzero status for a partial result (candidate: 3), and the existing fatal-error statuses otherwise. Opting into continuation asks for partial output; it need not make shell automation mistake that output for an ordinary success. Caught faults do not force a nonzero status.

The library can return a partial `Outcome` under continue mode, with mandatory status/failure accessors and population qualifiers on reports. Stop mode and all-failed invocations still return an error; errors should carry useful aggregate failure diagnostics where available. Audit the existing completion/evidence accessors rather than adding a failure list beside otherwise misleading numbers.

The playground should show “partial result,” failure counts or justified mass, source locations and the affected reporting basis prominently. Running more samples and discovering an error should be presented as finding an invalid outcome, not as a suggestion to reduce runs until the error disappears.

Future prologue/portal operations run once outside the world population. Their failures should use explicit host-operation handling, not the world-continuation policy. No new I/O, tasks or portal implementation is needed for this proposal.

## Implementation order and validation

This work is relevant now. Implement it separately from side effects, in stages that retain fail-fast behavior until each contract is ready:

1. **Fault taxonomy and diagnostics.** Add stable codes and explicit recoverability to operation/runtime errors. Classify built-ins and boundary checks. Keep unknown kinds fatal.
2. **Local recovery.** Add `try`/specific catches to parsing, lowering and effect/liveness analysis. Represent successful and failed exits with their state, weight and unwind destination; preserve left-to-right evaluation and current mutation semantics.
3. **Failure propagation through inference.** Extend call summaries, memoization, recursion and chain solving. A recoverable unhandled fault is a terminal outcome for the relevant execution scope; caught faults follow their handler. Solver discovery/iterations must not double-count failure diagnostics. Disable only optimizations that cannot yet preserve the contract, with bounded fallback behavior.
4. **Global continuation and report accounting.** Finish successful-completion reporting, failure/evidence metadata, per-run accumulation and bounded diagnostic aggregation before exposing continue mode to users. A new `try/catch` around the engine's current `Result` is insufficient.
5. **Hosts and documentation.** Add CLI/API options, partial-result status, playground display and normative semantics together. Preserve the current default behavior for programs that do not opt in or add handlers.

Acceptance cases should cover:

- The reciprocal model: 1/6 failed mass; successful mean `137/300`; zero-fallback mean `137/360`; no sampled retries to replace failures.
- Specific/multiple/nested catches, helper calls, handler failures, lazy fallback execution and errors in later operands.
- State and weight preservation through a caught error, including earlier mutations, draws, observations and printed output.
- Rejected evidence versus faults, all-failed versus impossible/rare evidence, and failure before later observations or densities.
- Reports before and after a failure, branching after an earlier report, grouped/per-visit reports and recovered worlds that complete successfully.
- Enumeration with merging/memoization/solvers on and off: matching result and failure weights, even when represented-world counts differ.
- Sampling across run counts, batch boundaries and thread counts; fixed attempted-run counts, successful-subset estimators, and uncertainty calibration against independent answers.
- Recipe/`simulate` atomicity, local catches inside `simulate`, and no silent normalization of failed outcomes.
- Fatal budgets/cancellation/unsupported/internal/type errors remaining fatal even inside `try` and under continue mode.
- Native/WASM parity, bounded error logs, host API status and text/result agreement.

The [rational oracle](../../crates/probl-oracle/src/lib.rs) should independently model the supported discrete fault paths. Tiny exact models are especially useful for checking that recovery preserves mass and that continuation neither hides nor double-counts it.

Implementation references: [runtime errors](../../crates/probl-engine/src/error.rs), [interpreter](../../crates/probl-engine/src/interp.rs), [world flow](../../crates/probl-engine/src/world.rs), [execution and batching](../../crates/probl-engine/src/lib.rs), [report calculations](../../crates/probl-engine/src/report/results.rs), and [public errors](../../crates/probl/src/error.rs).
