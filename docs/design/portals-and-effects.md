# Portals, the prologue and external effects

> Deferred design proposal, 7 October 2026. Portals, general filesystem actions and HTTP functions are not implemented. This records the intended direction and the questions that must be settled before implementation. Syntax and type names below are sketches, not additions to the [reference semantics](../semantics.md). Improve reporting and structured results first.

## Direction agreed so far

External side effects would be allowed in two places:

- **The prologue:** the implicit part of execution before any world split.
- **A portal:** an explicit boundary where code can inspect a population of worlds and interact with the host.

Portals should have read-only access to the worlds they inspect. They may return a value, allowing the model to continue. Uses include custom reporting, sending results to another system, and querying additional data midway through a model. They are not restricted to the end of a program.

`report` and `print` remain permitted language intrinsics outside explicit portals. Their implementations already bridge model execution and host output; users should not need to wrap existing diagnostics or reports in a new construct.

Prefer synchronous calls in the initial design. Tasks, `await` and user-facing algebraic effects are possible later directions, not prerequisites. The host may use asynchronous machinery internally without exposing it in the language.

The rest of this document distinguishes recommended contracts from open choices. In particular, the exact definition of the prologue, population query syntax, returned-value types and evidence after a portal are not yet settled.

## Motivation and the basic example

```probl
# Proposed syntax: not executable today.
let n: int = if 50% { 1 } else { 0 }

portal {
  # Here n exposes its population: 1 with weight 50%, 0 with weight 50%.
  # Inspect it, produce a custom report, or make an external request.
}
```

Outside the portal, `n` is an integer in each world. Inside, it denotes a view over the captured population. The portal body runs once for its population boundary, with ordinary control flow and portal-local variables. It does not run once per world or sampling run.

This gives a model two useful perspectives: describe what happens within a possible world, then inspect the weighted alternatives together. It also makes cross-world statistics explicit: a portal could query the mean of `n` as 0.5 in complete enumeration, while ordinary `mean(n)` remains invalid for a scalar outside it.

Useful applications include:

| Application | External interaction |
|---|---|
| Custom reporting | Export a table, chart data or an uncertainty-aware summary when `report` is insufficient |
| Forecasting from current data | Fetch a baseline once in the prologue and share it across worlds |
| Adaptive data acquisition | Inspect current uncertainty, select another measurement, fetch its result and continue conditioning |
| Decision support | Compare outcomes, choose one action and submit it once from portal code |

Submitting a chosen action differs from performing every action represented by the worlds. A model can simulate orders without placing them; a portal can deliberately place one selected order.

## The prologue is an execution phase

“Before any world split” should describe a semantic phase, not a test of the engine's current world count. Sampling often keeps one realized path per run; merging or evidence can reduce many worlds to one. None of those events should reopen permission for ordinary side effects.

Recommended properties:

- The prologue executes once for the program invocation, before sampled runs or enumeration branches start. It is not repeated per sampling batch or worker.
- Creating a distribution recipe, such as `let die = d6`, does not by itself split worlds.
- A draw or probabilistic branch closes the prologue according to a source-level effect rule, even if sampling takes one branch or an optimization delays the draw. Do not grant permission based on a particular seed or on a distribution happening to have one retained outcome.
- Deterministic functions called during the prologue may perform authorized host operations. Calling the same effectful function after the prologue must require a portal. Functions, aliases and callbacks cannot hide their effects from this check.
- A portal does not reopen the prologue in the following model code. Further external actions need another portal.

One conservative initial rule is to end the prologue at the first operation that may initiate probabilistic execution, rather than proving that a particular execution splits. The precise treatment of local inference remains open: `simulate` can split internally while leaving the outer world intact. Regardless of that decision, its model body must not inherit prologue I/O permission. Similar questions arise for calls that draw internally, certain probability conditions and draws from singleton distributions.

Checks must follow the program's semantic ordering. Draw scheduling, memoization and changes to inference mode must not move an external operation into or out of the prologue. An effect analysis is needed even if users never see effect annotations.

## A portal receives a joint snapshot

Separate marginal distributions are insufficient:

```probl
# Proposed portal syntax.
let n = if 50% { 1 } else { 0 }
let m = n

portal {
  # A query of n == m must give 100%, not 50%.
  # A pointwise view of n - m must contain only 0.
}
```

Under Probl's existing recipe semantics, consuming two ordinary distributions makes independent draws. A portal therefore cannot implement capture by turning each variable into an independent `dist[T]`. It needs one joint population, with projections that retain their common origin and alignment.

The snapshot is logical, not necessarily a list of all worlds. Capture the bindings that portal code actually uses; retaining every local variable would prevent useful merging and inflate inference costs. Preserve joint relationships among those bindings, their weights and the metadata needed to interpret them. Identical captured values may be combined without changing enumeration answers, but sampling uncertainty still needs the appropriate per-run contribution information.

Read-only means portal code cannot assign into captured world variables, rewrite their weights, consume their bags or apply `observe`/`score` to the running population. It can create local values and derived query views. A filtered query view, if supported, must carry its conditioning/reach information and leave the model unchanged.

Types must not vary with the number of surviving outcomes. A captured binding should not switch from population view to scalar because evidence or a particular sample made all its observed values equal. How statically shared constants are exposed remains a type-design question.

### Tools for manipulating populations

The names `Population[T]`, `PopulationView[T]` and `Estimate[T]` describe possible roles, not committed public types. Two surface designs are plausible:

1. **Implicit population expressions:** captured `n` and `m` support pointwise expressions such as `n - m`, and an explicit aggregate queries the result. Operators must preserve alignment rather than reuse independent distribution lifting.
2. **Explicit joint queries:** capture a joint record and evaluate a pure callback against each logical row. For example, a future API could express `joint({ n: n, m: m }).probability(row -> row.n == row.m)`.

The second is more explicit about correlation; the first is closer to the initial portal example. Neither syntax is selected yet. Required capabilities are clearer than their spelling:

- Joint capture, projection and pure pointwise transformations.
- Probabilities, expectations, quantiles, grouping and conditional queries with defined populations and denominators.
- Typed rows or summaries for serialization, without making users parse formatted output.
- Access to completion, bounds, sampling uncertainty, reach and stage provenance.

Initially, views from different portal snapshots should not combine implicitly: a cross-stage pairing needs an explicit identity and conditioning contract. Converting a view into an ordinary distribution recipe, if offered, must be explicit about independence and about any loss of inference metadata. Population views should not escape into ordinary model code by accident.

Aggregate callbacks should remain pure. External actions belong in ordinary portal statements or explicit loops over materialized results, where their repetition is visible. A portal running once does not prevent its author from deliberately sending several requests.

Analytic continuous outcomes also need a representation and supported-query contract. A portal must not silently discretize them or imply that every population has enumerable rows.

## Returning a value and continuing the model

The recommended initial interpretation is a single shared return value. A portal reads its snapshot, evaluates its body once, and returns an immutable ordinary value that becomes available unchanged in every continuing world. The worlds retain their prior state, relationships and unnormalized weights. Normalization used for a portal query does not itself rewrite those weights.

The following sketches the intended flow; HTTP calls, response decoding and portal syntax are all hypothetical:

```probl
let n: int = if 50% { 1 } else { 0 }

let observed: int = portal {
  let response = http_get("https://example.org/measurement")
  decode_int(response)
}

observe n == observed
report n
```

There is one acquired response for this boundary, not one request per value of `n`. If the response is 1, the subsequent observation keeps the `n == 1` worlds. The response itself is ordinary data; it does not automatically condition the model. For noisy measurements, the program supplies the appropriate likelihood explicitly.

This resembles `simulate` in being an expression block that returns something, but their roles differ. `simulate` constructs a local distribution from model execution. A portal inspects an existing population and can return host data or a decision to its continuations. Portal results should not automatically be wrapped as a distribution or silently drawn. Initially restrict escape to well-defined ordinary data; whether explicit distribution values can be returned, and with which metadata, remains open.

Read-only access still permits feedback through the return value. That is intentional, and is why portals are more than final output callbacks.

## Midway portals, evidence and sampled feedback

A midway snapshot is conditioned on the evidence available at that boundary. Later observations can change subsequent reports, but cannot retract an HTTP request or revise a file already exported. Portal outputs need a stage identity and must not be described as the final posterior when more evidence can follow.

The current rule that evidence cannot follow a `report` remains in force until explicitly redesigned. A midway portal would introduce an explicit snapshot capability, not silently weaken that rule. Before implementation, decide how portal reporting is distinguished from final `report`, and what `report` means inside a portal. A data-fetching portal need not claim to publish a final inference result.

Adaptive acquisition must record which query or measurement was selected as well as its response. When selection is relevant to the likelihood, the model must account for it; fetching data does not automatically supply a correct observation model.

### Sampling introduces a further boundary

If a portal computes a mean, chooses a policy or selects a query from sampled results, and then feeds that value into those same runs, subsequent contributions can share a random estimate derived from the whole population. The current independent-run standard-error formulas cannot simply be assumed valid for that procedure.

For example, comparing every run's `n` with an estimated population mean introduces a shared estimated threshold. Selecting the best-looking policy from noisy estimates and evaluating it on the same scenarios also needs an explicit statistical contract. Distinguish evaluating a fixed decision conditional on a recorded response from assessing the entire adaptive procedure across repeated experiments.

Possible restrictions or later solutions include read-only terminal summaries first, separate selection/evaluation samples, or explicit staged inference with fresh runs and defined conditioning. Merely continuing the old runs with a new random-number seed does not by itself remove the dependence. Where uncertainty is not justified, expose it as unavailable rather than reusing an inapplicable error bar.

Portals may be evaluated once per reached stage in both modes, but their request arguments and decisions can differ because sampling approximates the population. Mode-independent invocation boundaries are not a promise of identical external decisions.

## Synchronization and scope

A useful initial scope would allow multiple top-level portals at common control-flow joins, each reached at most once per continuing world in its stage. Midway queries remain possible. Portals inside probabilistic branches, repeated loops, recursion or `simulate` should wait until their synchronization contract is specified.

The engine must gather the stage's population before running the portal body. It cannot invoke the body separately for each sampling batch or memoized call. Reaching a portal is a barrier: world-varying decisions and effects on either side cannot be reordered through it without a proof that the snapshot and continuation are unchanged.

Before broader placement is allowed, define:

- Which worlds belong to an invocation when only one branch reaches the portal, and how its reach and return value are represented.
- What happens when some paths terminate or never reach the barrier.
- Whether a portal in a loop aggregates one iteration, all visits, or something else; do not infer this from execution scheduling.
- Empty/impossible populations and unresolved mass at a boundary. An error or cancellation before the boundary must not run its body. An incomplete but usable result needs an explicit, visible policy rather than being treated as complete.
- How delayed conjugate variables, analytic values and future symbolic representations are inspected without introducing undocumented draws.

The compiler must keep captured bindings live up to the barrier. The sampler currently runs complete paths in batches, so a midway portal would require staged execution or equivalent continuation machinery. This is a substantive engine change, not just a new built-in.

## Existing read, report and print

| Existing feature | Relationship to the proposal |
|---|---|
| `read` | A declared input loaded by the host before model execution. Its existing schema, authority and snapshot contract stays intact; it is not automatically converted into a dynamic file-opening function. |
| `report` | Aggregates weighted contributions across worlds. It provides useful implementation foundations for population queries, but currently publishes under the final-evidence rules of its scope. |
| `print` | Emits diagnostic output for each executing world; merged worlds count once. It remains a privileged intrinsic, without being redefined as a once-per-population portal. |

It is reasonable to call `report` and `print` implementation-level portals in the broad sense that they cross into host output. Their existing cardinality and evidence contracts nevertheless differ. Keeping these intrinsics available does not grant arbitrary user-defined I/O the same privileges, and runtime-print callbacks should not be promoted as a reliable business-action mechanism.

## Synchronous effects first

Start with calls that return their response before the next portal/prologue statement executes:

```probl
# Proposed host operations, usable only in an authorized effect scope.
let response = http_post(endpoint, headers, body)
let data = decode_json(response)
```

Source-level ordering is then straightforward, and no task value needs to be cloned, merged, serialized or carried through a split. The host can still implement cancellable asynchronous I/O internally, and the browser can keep its UI responsive. Synchronous syntax must not mean an uninterruptible network operation.

If explicit concurrency later proves useful, an alternative is `let pending: task = http_post(...); let response = await pending`. That would additionally need contracts for task ownership, lifetime, cancellation, unawaited failures and whether tasks can cross a population boundary. Algebraic effects might help implement handlers or express capabilities, but are not needed as user-visible machinery for the first version.

The language still needs internal effect information: a helper that performs I/O must not be memoized, replayed by a solver or invoked from an unauthorized model callback. Permission checks must work transitively and independently of how many outcomes a particular call realizes.

## Host authority, replay and failures

The host grants filesystem/network capabilities. A model cannot enlarge them by entering a portal. Useful controls include permitted paths/endpoints, redirect policy, input/output sizes, deadlines and cancellation. Credentials belong to host configuration. The effect interface should be mockable and usable with recorded inputs; this also avoids baking browser-specific networking constraints into model semantics.

Record effect results and sufficient request/stage provenance to replay a run without contacting the original service. Byte snapshots matter: the same URL and seed alone do not reproduce a changing response. Do not put credentials in a replay log. HTTP method alone does not distinguish acquisition from mutation; some queries use POST, and requests can have observable server behavior even when their purpose is reading.

“Once per portal invocation” is a local execution rule, not exactly-once delivery. A timeout can leave it unknown whether a remote action succeeded. Retries and replay must not silently repeat mutating actions; integrations may need idempotency identifiers or explicit recovery.

Prologue and midway effects can already have happened when later model execution fails or evidence proves impossible. They are not rolled back with discarded worlds. Read-only population access does not make the whole program an external transaction. Request failures are host-operation failures, not zero-likelihood evidence; the model may only treat an observed service failure as data through an explicit, defined mechanism.

## Reporting comes first

Do not implement portals or new general I/O now. First improve the existing reporting surface:

1. Make typed results and useful tables available without parsing text.
2. Preserve joint relationships in a query API, with explicit conditioning and pure transformations.
3. Expose reach, unresolved bounds, uncertainty status and provenance consistently across the library, CLI and playground.
4. Add useful structured export and validate its schemas against the existing report calculations.

These are independently useful and exercise the difficult population/result contracts without introducing midway external actions. Current applications can already acquire data through a host, provide `MemoryFiles`, run the [Rust library](../library.md), and handle its structured result outside inference.

Before a portal implementation, write semantic cases covering: correlated captures; prologue closure under merging and sampling; indirect effectful calls; once-per-stage execution across batches and solver settings; unchanged continuation weights; broadcast return values; later evidence versus earlier snapshots; incomplete/empty populations; cancellation and ambiguous request failure; and sampled feedback with a justified uncertainty contract. Use a mock host with an effect log rather than live filesystem/network actions for these checks.

Related contracts: [functions and effects](../semantics.md#6-functions-and-effects), [reports](../semantics.md#9-reports), [input authority and snapshots](../data-input.md#where-data-comes-from), [structured results](../library.md#read-reports-without-parsing-text), and [use-case gaps](../use-cases.md).
