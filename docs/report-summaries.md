# Choosing report summaries

Report summaries can select ordered columns for individual numeric or date
reports. The same selection controls CLI text, the Rust JSON export and playground
charts and tables. Reports without an override keep their existing output.
These options are available in the unreleased source following 0.3.2.

## Command line and playground

For the retirement example, choose the 10/25/50/75/90 presentation for its first
report, leaving depletion probabilities and the baseline at their defaults:

```sh
probl run examples/21_retirement.probl --summary 0=forecast
```

`--summary ID=SPEC` is repeatable for different reports. IDs start at zero and
follow the compiled program's report order, including reports that are never
reached. Labels need not be unique. IDs belong to that program and can change
when its source changes. Repeating an ID on the command line is an error.

In the playground, open **Report summaries**, choose a preset or **Custom
columns** for a report, and press **Apply and run**. The controls are available
after compilation, before running the model. **Default** clears that report's
override. Choices are saved with the exact source and included in share links;
editing the source resets them along with chart settings.

| Specification | Ordered columns |
|---|---|
| `forecast` | `q:0.1,q:0.25,median,q:0.75,q:0.9` |
| `standard` | `q:0.05,q:0.25,median,q:0.75,q:0.95` |
| `moments` | `mean,sd` |
| Custom, e.g. `mean,median,q:0.9` | The listed columns in that order |

`standard` explicitly requests five columns. It is different from omitting an
override: default ungrouped text also shows moments where appropriate, and default
JSON includes moments and five quantiles. To enter `standard` in the playground,
select **Custom columns**.

Charts use the requested midpoint median as their main line, otherwise the mean,
otherwise the requested quantile nearest 0.5, or SD for an SD-only selection.
Requested quantiles below and above 0.5 are paired from the outside inward for
bands. Tables preserve the requested column order. Mean/SD selections have no
percentile bands. These are outcome summaries; bands are not Monte Carlo
confidence intervals. Probability reports keep their probability and uncertainty.

## Median and quantile conventions

`median` uses Probl's existing midpoint rule at ties. `q:LEVEL` uses the weighted
inverse CDF, including at `q:0.5`. With half the mass at 0 and half at 10, `median`
is 5 and `q:0.5` is 0. Both columns may be requested together. Levels 0 and 1 are
allowed; a continuous distribution's endpoint may be infinite.

Dates support median and quantiles, including midpoint dates. Exact integer and
date values are preserved in text and typed JSON values. Means and SDs are
unavailable when a numeric summary cannot represent the population; unavailable
values are never replaced with zero. Categorical, structured and mixed
Boolean/other reports keep their default display and reject explicit summary
overrides. Boolean reports accept an override but retain probability output.

## Rust

```rust
use probl::{Options, ReportStatistic, ReportSummary, compile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = compile("model.probl", "report d6 as \"roll\"")?;
    for (id, label) in program.report_labels().enumerate() {
        println!("{id}: {label}");
    }
    let summary = ReportSummary::new(vec![
        ReportStatistic::Mean,
        ReportStatistic::Median,
        ReportStatistic::Quantile(0.9),
    ])?;
    let outcome = program.run(&Options::new().report_summary(0, summary))?;
    print!("{}", outcome.text());
    Ok(())
}
```

`ReportSummary::forecast()`, `standard()` and `moments()` construct presets.
`"mean,median,q:0.9".parse::<ReportSummary>()` uses the same parser as the CLI and
WASM. Calling `Options::report_summary` again for an ID replaces its override.
`Outcome::render` preserves selections when rendering a slice of reports.

Selections do not restrict the existing Rust result query API. In particular,
`NumericSummary::quantile(0.5)` retains its existing midpoint convention, while
`ReportStatistic::Quantile(0.5)` explicitly requests the inverse CDF.

## WASM and JSON

The WASM `check` response includes `reports: [{id, label}]`, or `null` if the
program did not compile. The run request accepts:

```json
{
  "source": "report d6; report d6 > 3",
  "report_summaries": {"0": "mean,median,q:0.9"}
}
```

Keys must be canonical zero-based integer strings and values must be summary
specification strings. Omit an ID to keep its default. The resulting
[report JSON](report-json.md) records ordered tokens in `summary_selection` and
each quantile's `convention`. Unrequested mean/SD fields are null; the quantile
array contains only selected median/quantile columns. Facts retain their existing
probability representation.

## Validation and cost

A selection must contain 1–32 distinct columns. Quantile levels must be finite
and within [0,1]; equivalent levels such as `q:0.1,q:0.10` are duplicates.
Specifications are limited to 4,096 bytes and a run accepts at most 256 report
overrides. The playground exposes controls for its first 256 reports.

Invalid specifications and unknown report IDs fail before model execution or
input-file loading. Value-type compatibility is checked after execution, once
dynamic report types are known; an incompatible selection returns a diagnostic
at that report. A never-reached report has no values to check.

Selections do not change the model, its random draws, retained populations or
sampling uncertainty. They avoid exporting and rendering unrequested quantiles;
moments remain available to Rust queries. JSON work budgets account for the
number of requested quantiles. This feature does not provide bounded-memory
sampling; use the explicit collection policy below to discard numeric populations.

## Moments-only collection

Collection is a separate, opt-in choice, configured before execution:

```sh
probl run examples/21_retirement.probl --collect 0=moments --runs 100000
```

The equivalent Rust option is
`Options::new().report_collection(0, probl::ReportCollection::Moments)`;
WASM accepts `"report_collections": {"0": "moments"}`. The playground has an
**Exact collection / Moments only (sample mode)** selector for each report.
Selecting moments-only collection also selects mean/SD display columns. It does
not switch the execution mode; the program or run options must request sampling.
Collection choices persist with the source and share links like summary choices.
`--collect ID=exact` / `ReportCollection::Exact` restores default retention.

Moments-only collection supports sampled scalar numbers and Boolean facts.
Weighted observations and repeated visits use the same per-run denominators,
effective sample sizes and sampling-error calculations as exact collection.
Means and population SDs use weighted central-moment updates merged in batch
order. Their last rounding bits can differ from exact population scans, but the
model's random draws, probability counts and thread-count replay do not change.
Mean and SD are the default columns; explicit `mean`, `sd` or both are allowed.
Probability reports retain their probability and uncertainty output.

Numeric populations are discarded. `Group::distribution()` returns `None` and
`NumericSummary::quantile()` returns `SummaryError::Unavailable`. The report's
`collection()` accessor and JSON `collection` field identify the policy. Numeric
JSON summaries have `quantile_method: "unavailable"` and an empty quantile array.
A constant numeric report is not advertised as a deterministic baseline from a
missing population. Other reports and language-level distribution queries are
unaffected.

Dates, strings, records, lists, complex numbers, unevaluated distributions and
integers not exactly representable as `f64` are rejected; use exact collection
for these. Draw a distribution (`let x ~ d6; report x`) instead of reporting its
law (`report d6`). Mixed numeric/Boolean reports are rejected. Mode, ID and
quantile-column conflicts fail before execution and input loading. Unsupported
values fail when encountered, including when incompatible batches are merged;
these are configuration/unsupported errors, not silently dropped trials.

With fixed groups, retention no longer grows with trial count. Active batches
still keep per-run bookkeeping, and parallel workers/pending batches multiply
that cost. `Limits::max_report_groups` caps groups per moments-only report across
batches (default 10,000), with a `Limit` error before a new group exceeds it.

`Limits::max_report_summary_bytes` additionally limits accounted live collector
storage across **all moments-only reports and workers in one execution**. It
defaults to 1 GiB natively and 128 MiB in the browser. The CLI accepts sizes such
as `--max-report-summary 128M`; Rust hosts can set the field on `Limits`.
The playground uses its fixed browser allowance.

Charges cover group accumulators, nested key payloads, per-run hash tables and
their growth, temporary reach bookkeeping, and completed batches awaiting merge.
They are reserved before collector allocation and released as tables, duplicate
groups and batches are discarded. A group transferred to the combined result
keeps its charge. Repeated visits to an existing run/key need no new storage.
Shared key payloads are conservatively charged for each retained occurrence;
container charges include spare capacity and bookkeeping. Large integer keys
use an allowance based on the hard integer-size ceiling, since a small result
can retain a larger limb buffer. Plain scalar, enum,
range and collection keys are supported, nesting at most 128 levels. Keys that
contain functions or distributions require exact collection.

This is an implementation-dependent accounting budget, not a process-memory or
WASM-linear-memory ceiling. It excludes exact reports, model execution and
caches, source/data, result construction/formatting, allocator overhead and
copies made by callers. Mixing exact and moments reports leaves the exact
populations outside this allowance. An exceeded limit stops the execution with
a `Limit` error; it never silently drops a report or trial. More workers and a
different batch schedule can cross a tight live limit; completed runs retain
the same seeded results. Reducing threads can help a native run fit.

Bounded quantile sketches remain future work. The playground default remains
5,000 retirement trials.
