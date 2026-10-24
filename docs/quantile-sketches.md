# Quantile sketch evaluation

Reporting a forecast every year, as the [retirement example](../examples/21_retirement.probl)
does, needs bounded empirical quantiles in addition to the implemented
moments-only policy. This experiment selects a **KLL-style lazy
compactor hierarchy** for integration. At similar retained
heap, it has much smaller rank errors than a mergeable reservoir on these
fixtures. The integrated policy defaults to capacity 512 with an explicit
approximation method; the measurements below are **not an error guarantee**.

The candidates live in the
[`quantile-sketches` benchmark example](../crates/probl-bench/examples/quantile-sketches.rs).
The selected compactor is now integrated in sampled report collection, Rust, the
CLI and WASM; see [the collection contract](report-summaries.md#approximate-quantile-collection).
The playground also exposes capacity and labels approximate percentiles; see
[the UI contract](playground.md). These measurements describe the
standalone candidates, not the integrated collector. Exact reports and
language-level `quantile` are unchanged.

## Candidates

| Candidate | Accuracy and merge behavior | Decision |
|---|---|---|
| Lazy randomized compactors | Retains observed values with power-of-two weights. Targets rank accuracy across the distribution. Fixed batches and ordered merges replay. | Implement and measure here; selected for integration. |
| Bottom-k priority reservoir | Keeps a sample of nonzero observations. Global observation IDs determine priorities, so disjoint summaries merge without resampling. The sample itself is independent of merge order. | Implement as a simple, fixed-capacity baseline; insufficient accuracy at comparable memory here. |
| t-digest | Merges centroids and interpolates queries, with particular emphasis on tails. The reference implementation includes special handling for repeated values and interpolation. | Considered from primary sources, not benchmarked here. Revisit if extreme-tail value accuracy becomes the main requirement. |
| DDSketch | Targets relative error in values, rather than additive error in ranks. Collapsing bounded stores restrict which quantiles retain that accuracy. | Considered from primary sources, not benchmarked here. A useful future option for a different accuracy contract. |

The compaction design follows the family described by
[Karnin, Lang and Liberty](https://arxiv.org/abs/1603.05346) and the practical
[streaming-quantiles implementation](https://github.com/edoliberty/streaming-quantiles).
This is an independent prototype with integer-rounded capacity decay, minimum
level capacity eight, independent parity choices and a separate zero-mass query
wrapper. It is not Apache DataSketches; its
[published accuracy calibration](https://datasketches.apache.org/docs/KLL/KLLAccuracyAndSize.html)
does not apply to this code. The other candidates' contracts come from the
[t-digest reference](https://github.com/tdunning/t-digest) and
[DDSketch reference](https://github.com/DataDog/sketches-py).

## Reproduce

```sh
cargo test -p probl-bench --example quantile-sketches --locked
cargo run --release -p probl-bench --example quantile-sketches --locked -- \
  --n=100000 --seeds=32 > sketches-100k.jsonl
target/release/examples/quantile-sketches \
  --n=1000000 --seeds=8 --sizes=256,512 \
  --fixtures=uniform,lognormal,zero_atom,retirement_100 --layouts=batches \
  > sketches-1m.jsonl
```

The default run evaluates both candidates at capacities 128, 256 and 512 in two
layouts: direct streaming, and 1,000-value batches merged in input order. The
batch layout holds at most eight pending sketches, then consumes them into the
combined sketch. This exercises overlapping storage without running the engine.
`--help` lists filters. Nine invariant/replay tests also run in ordinary workspace
tests, including actual one-, four- and seven-worker construction with ordered
combination.

There are twelve fixtures: uniform, ascending and descending uniform, normal,
lognormal, Pareto, all-zero, 90% zero plus lognormal, five repeated values,
signed extreme/subnormal floats, and retirement balances after 30 and 100 years.
The retirement populations use the example's annual recurrence and independent
normal rates in a separate Rust reference simulation. They **do not use Probl's
random stream** and do not measure interpreter performance. Every fixture is
generated once with a fixed seed, independently of sketch seeds 0 through 31.
The scale check uses seeds 0 through 7.

Each record includes a fixture fingerprint, exact zero/sign counts, per-seed
query fingerprints and maximum rank errors, selected exact/estimated quantiles,
relative value errors, retained and peak heap, and separate update/merge/query
timings. Fixture generation, exact sorting, rank scoring and JSON serialization
are outside those timings. Query time includes building the sorted view and
answering all 1,003 queries. Timing and allocation accounting use separate runs;
the latter checks every answer's bits against the former.

## Error convention and zero mass

Queries use the lower inverse empirical CDF, including at 0.5, with exact minimum
and maximum endpoints. This differs from the existing exact report's midpoint
median when the two middle values differ. No current report behavior is changed
by this experiment; production metadata must identify the approximate convention.

For an answer `x`, rank error is the distance from `q` to the interval
`[count(value < x) / n, count(value <= x) / n]`. This handles ties: returning zero
inside a large zero atom is correct, even if its upper CDF is far from `q`.
Queries cover 0 through 1 in increments of 0.001, plus 0.0001 and 0.9999. Reported
maxima are across this grid and the specified seeds, not every possible query,
input or seed. The experiment cannot establish a confidence level.

Both candidates keep exact total, negative and zero counts, plus minimum and
maximum. Signed zero is normalized to zero. Zeros never consume sketch capacity.
Queries inside the true zero interval return zero. For other queries, retained
negative/positive weights are separately normalized to their exact sign counts;
if a rare sign has no retained representative, its exact outer endpoint is the
fallback. Thus zero mass and signs survive even that case. Quantiles on either
nonzero side remain approximate. This wrapper is part of the evaluated algorithm,
not an unqualified application of another implementation's error bound.

## Results

Measured on 21 October 2026 on the Apple M2 Pro used for the other benchmarks,
with Rust 1.99.0 and the release profile. Times are indicative local medians,
not an engine speedup estimate. At 100,000 values and 32 sketch seeds, across all
twelve fixtures:

| Candidate / capacity | Streaming worst rank error | Batched worst rank error | Batched max retained heap | Batched max peak heap |
|---|---:|---:|---:|---:|
| Compactor / 128 | 1.541 pp | 1.320 pp | 6.3 KiB | 43.9 KiB |
| Compactor / 256 | 0.889 pp | 0.647 pp | 10.2 KiB | 73.4 KiB |
| Compactor / 512 | 0.386 pp | 0.365 pp | 23.2 KiB | 80.8 KiB |
| Reservoir / 128 | 13.580 pp | 13.580 pp | 3.0 KiB | 29.3 KiB |
| Reservoir / 256 | 10.483 pp | 10.483 pp | 6.0 KiB | 57.8 KiB |
| Reservoir / 512 | 7.407 pp | 7.407 pp | 12.0 KiB | 114.8 KiB |

Errors are percentage points of normalized rank. Capacities have different
meanings: the compactor's is its top-level capacity, while the reservoir's is
its total retained sample size. Compare bytes, not just the parameter. In
particular, compactor/256 uses less retained heap than reservoir/512, and its
worst observed batched rank error is over eleven times smaller.

Representative batched compactor/512 results at 100,000 values:

| Population | Worst rank error | Update | Merge | Query | Largest relative value error at p95 | At p99.99 |
|---|---:|---:|---:|---:|---:|---:|
| Uniform | 0.331 pp | 1.20 ms | 1.32 ms | 0.28 ms | 0.20% | 0.09% |
| Lognormal | 0.323 pp | 1.20 ms | 1.32 ms | 0.28 ms | 6.49% | 137.74% |
| 90% zero / lognormal | 0.037 pp | 0.47 ms | 0.16 ms | 0.04 ms | 1.37% | 556.43% |
| Retirement, year 30 | 0.185 pp | 1.14 ms | 0.59 ms | 0.16 ms | 1.67% | 48.32% |
| Retirement, year 100 | 0.038 pp | 0.48 ms | 0.20 ms | 0.05 ms | 0.69% | 25.01% |

The heavy-tail results rule out describing a rank bound as a dollar-error bound.
The zero interval and endpoints were exact in every tested run. Repeated-value
and extreme-float outputs remained finite, monotone and drawn from the input.

### Memory and tenfold scale

Heap measurements include backing-vector capacity, pending sketches, query
views and merge temporaries. Reallocation is measured with old and new buffers
overlapping, even when the system allocator could grow in place. They exclude
input/exact samples, stack storage, allocator overhead and all engine report/run
bookkeeping. They are neither RSS nor WASM linear-memory measurements.

The first implementation kept oversized vectors after levels emptied. Trimming
those allocations reduced the maximum retained heap for batched compactor/512
from 96.4 to 23.2 KiB at 100,000 values; every query fingerprint stayed identical.
Logical item count alone would have missed this overhead.

For the same batched compactor/512 and fixed eight-pending-batch layout:

| Population | Retained heap, 100k / 32 seeds | Retained heap, 1M / 8 seeds | Peak heap, 100k | Peak heap, 1M | Worst rank error, 1M |
|---|---:|---:|---:|---:|---:|
| Uniform | 17.6 KiB | 20.8 KiB | 80.8 KiB | 84.4 KiB | 0.379 pp |
| Lognormal | 17.6 KiB | 20.8 KiB | 80.8 KiB | 84.4 KiB | 0.312 pp |
| 90% zero / lognormal | 16.6 KiB | 16.4 KiB | 63.5 KiB | 72.6 KiB | 0.031 pp |
| Retirement, year 100 | 15.9 KiB | 25.2 KiB | 60.9 KiB | 59.3 KiB | 0.039 pp |

Memory does not grow tenfold. It is not perfectly flat: levels, rounding and
buffer capacities change. With `H` levels, the compactor retains fewer than
`3*k + 9*H` values; `u64` sample counts need at most 64 levels. This logical bound
is not an allocation allowance. Production must reserve actual capacity growth
and overlapping merge/query storage using the existing shared summary budget.

## Integration contract

The engine now implements the collector and API portions of this contract.
Native/WASM, worker-count, budget-failure and cancellation tests accompany it;
full-model measurements are recorded in [benchmarks](benchmarks.md#since-approximate-quantile-collection).
Playground controls preserve capacity in shared/local settings, and chart legends,
point details and tables distinguish approximate quantiles from sampling error.

- Promote the selected compactor into a fallible, cancellable collector. Reserve
  storage before allocation, including keys, per-run bookkeeping, pending
  batches, resized buffers and result/query views. Keep the existing group cap.
- Start with unweighted sampled finite scalar numbers and at most one visit per
  trial/key. Diagnose unsupported weighted, integrated and repeated-visit cases;
  retain the existing exact path as default. Preserve moments and sampling-error
  semantics, including exact depletion counts.
- Use an explicit capacity setting, initially 512, rather than an invented
  error-percentage guarantee. Report method/version/capacity separately from
  Monte Carlo error, with empirical accuracy limitations documented. The query
  convention is inverse CDF; approximate medians must be labeled accordingly.
  Exact distributions and exact quantile queries remain unavailable on a sketch.
- Domain-separate sketch random state by execution seed, report/group and batch;
  never consume model RNG. Fix batch construction and merge order. Compactor
  results need not match direct streaming or a different merge tree.
- Test actual engine model draws, counts and risk estimates across policies;
  test native/WASM and thread-count replay. Measure full retirement reports at 100,000 and
  1,000,000 trials, including summary budgets and cancellation. Keep the playground
  example's modest default.
