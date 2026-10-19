# Structured report JSON, version 1

Probl can export report summaries without parsing its printed output. Enable the
`json` feature of the `probl` crate. For a project beside a local Probl checkout:

```toml
probl = { path = "../probl/crates/probl", features = ["json"] }
```

The feature is in the unreleased source following 0.3.2. A published dependency
can enable the same feature once a release containing it is available.

```rust
let program = probl::compile("forecast.probl", source)?;
let outcome = program.run(&probl::Options::new())?;
let document = outcome.reports_json(&probl::ReportJsonLimits::default())?;
```

The playground's WASM adapter uses the same serializer. Successful and partial
responses contain the document as an object in `reports`, alongside the existing
`output`, stats and diagnostics. If serialization exceeds a budget, the response
instead has `reports_error: {kind: "limit", message: "…"}`. This does not turn a
successful model into a failed one, discard a partial result, or truncate its
text. Consumers should retain the text view when structured data is unavailable.

## Envelope and identity

The document contains `schema_version: 1`, `run`, `failures`, and `reports`.
Consumers must check the version and tolerate additional fields in version 1.
Removing fields or changing their meaning requires a new schema version.

Reports are in source order, including unreached report sites:

| Field | Meaning |
|---|---|
| `id` | Zero-based report index within this compiled program |
| `label` | The explicit `as` label or expression text |
| `key_label` | Source text of the `by` expression, or `null` |
| `visits` | `once`, `per_key`, or `per_visit` |
| `reach` | `{share, standard_error}` where known, otherwise `null` |
| `groups` | Groups in the engine's key order; empty if unreached |

Labels may repeat. IDs distinguish those reports, but are not identities that
persist through arbitrary program edits. `per_key` proves at most one contribution
per world and key. `per_visit` counts every visit, not distinct worlds. Reach is
report-level information; its absence must not be treated as 100% reach.

Each group has a zero-based `id` within its report, `key`, `kind`, `probability`,
`summary`, `support`, `complete`, `unresolved_share`, and `fields`. An unkeyed
group's `key` is `null`. A key is a typed object:

```json
{"kind":"int","value":"9007199254740993","label":"9,007,199,254,740,993"}
```

The label is for display, not parsing or identity. Supported key kinds are `int`
(decimal string), `float` (JSON number), `prob` (JSON number in [0,1]), `date`
(ISO calendar string), `bool`, `str` and `unit` (`null`). Integers are always
strings, even when small, so JavaScript cannot round them silently. Other kinds
use `opaque`, with a `null` value and the engine's display label; group IDs still
distinguish them. Do not sort keys lexically or merge keys by their labels.

## Summaries and availability

Group `kind` is `probability`, `number`, `date`, `categorical`, or `structured`.
Categorical groups have no exported population: keep the text/table view or use
the Rust distribution API. Structured groups contain `{path, group}` entries in
`fields`. These are marginal summaries of related fields, not independent joint
samples. Other groups have an empty `fields` array.

Numeric and date summaries contain:

- `mean` and `sd`: estimates, or `null` when unavailable (including dates).
- `quantiles`: five entries with `level` 0.05, 0.25, 0.5, 0.75 and 0.95, an
  `estimate` where available, and a typed `value` where the quantile is a retained
  discrete value. These typed values use the key encoding, with a null label.
- `percent`: whether numeric values are probability parameters.
- `integrated_moments`: whether computing moments required numerical integration.
- `quantile_method: "exact"`: no quantile sketch or sample compression was used.
- `deterministic`: the resolved population consists of one concrete number/date.

A true `deterministic` flag does not override incompleteness or partial-failure
metadata. Quantiles follow Probl's report convention: the 50% entry uses the
midpoint median; other levels select outcomes. These conventions can differ from
interpolated sample percentiles in other libraries.

Dates and integers that cannot be represented exactly in the numeric summary
retain typed quantile values even when their `estimate` or mean/SD is unavailable.
A typed quantile value is authoritative for exact integer/date display. Numeric
estimates are floating-point summaries; they do not make arbitrary-precision
integer arithmetic available in JavaScript. Continuous quantiles have an estimate
but no retained typed value. Null means unavailable, never zero.

Full-precision floating estimates can differ in their last rounding bits across
native and WASM targets, especially weighted effective sample sizes and reach
errors near zero. Transport tests use narrow numeric tolerances for these fields;
typed values, identities, counts and displayed output are checked exactly.

`complete` says whether unresolved probability could change a group's quantities.
It can be null for a parent whose summaries are in its fields; inspect the child
metadata. `unresolved_share` is also retained, but a share that rounds to zero
does not establish completeness. `support`, when sampled, has independent
`contributing_runs` (decimal string) and `effective_runs` (number).

## Uncertainty and run provenance

An estimate has `point`, `nonfinite`, `complete`, `bounds` and `sampling`:

- `point` is a finite number or null. A nonfinite result is explicit in
  `nonfinite` as `"+infinity"`, `"-infinity"` or `"unavailable"`.
- `bounds` encloses unresolved-model uncertainty where supported; it is not a
  confidence interval. Endpoints can use the same nonfinite strings.
- `sampling` is null for enumeration, or contains `status`, `standard_error`,
  `confidence_interval`, `contributing_runs`, and `effective_runs`.
- Sampling status is `estimated`, `not_estimable`, `not_computed`, or
  `integrated_zero`, with the meanings in the [reporting semantics](semantics.md).
  A confidence interval, when present, has `level`, `method: "wilson"`, `lower`
  and `upper`. No interval is fabricated when the engine cannot estimate one.

Percentile bands describe variation across outcomes. They are not Monte Carlo
confidence intervals or simultaneous bands around an entire trajectory. Exact
sample quantiles still have sampling uncertainty; version 1 does not compute
that uncertainty. Numerical integration is a separate approximation, identified
by `run.integration_tolerance` and the relevant summary metadata.

`run` includes mode, sampling information, execution date, whether any world
finished, whether the result is partial, failed share, evidence, unresolved weight
and integration tolerance. Sampled run count and seed are decimal strings. Zero
unresolved weight has `zero: true` and `log_weight_upper_bound: "-infinity"`;
this is an unnormalized bound (per run when sampling), not a failure probability.
Evidence retains its kind, logarithmic value/bounds, completeness, relative
standard error and sampling status. Log values can use explicit infinity strings.

`failures` contains messages, UTF-8 byte spans `{start,end}`, failed shares,
standard errors, run counts and first-run indices where known. Reports in a
partial result describe the populations allowed by the existing failure rules;
never hide that qualification just because the summary itself is complete.

## Limits and cost

`ReportJsonLimits` defaults to 4 MiB encoded UTF-8, 10,000 report/group/field/failure
nodes, and 20 million units reserved for population scans and inspected text.
These budgets apply after execution. They bound document size and summary work,
not the memory the model or its exact result populations already consumed. They
are independent of model execution limits and do not consume model randomness.

Serialization streams reports/groups into a bounded buffer. It does not copy raw
sample populations into JSON. Structured fields are assembled within their group
and obey the same node/work budgets. A budget error returns no partial JSON;
callers may still use `Outcome::text()` and the existing query API. Hosts that
need cancellation during export should run it in a cancellable worker, as the
playground does. Work units are not a wall-clock deadline.

For fixed groups, document size follows the number of summaries, not the number
of trials. Computing exact quantiles still scans the retained populations; this
feature does not solve report-collection memory growth. The [playground charts](playground.md)
consume these summaries; configurable summaries and streaming accumulators are
subsequent work.

An October 19, 2026 spot check of the retirement example (5,000 trials, seed 42,
101 years including year zero) produced 303 groups in 481,419 JSON bytes. Using
Bun and the actual WASM loader on macOS arm64, two runs before this export took
1.745–1.764 seconds; two runs with export took 1.777–1.781 seconds. WASM linear
memory grew from 156.875 to 157.438 MiB. Each measurement used a fresh instance
after a 200-trial warmup, excluding module compilation, in before/after/after/before
order. Printed output matched exactly. These few runs measure total adapter cost,
including JSON parsing; they are a spot check, not a performance guarantee.
