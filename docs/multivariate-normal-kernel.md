# Bounded multivariate-normal kernel

October 24, 2026. The engine now contains the numerical kernel and bounded
constructor cache for M5. **The language builtin is not exposed yet.** The
[prototype](multivariate-normal-prototype.md) remains a separate reference;
its covariance acceptance, pivot order and random-number consumption are unchanged.

## Ownership and limits

`probl_engine::multivariate` is an internal engine API, not a stabilized library
interface. `Normal::new` validates finite means and covariance, constructs a
shared immutable factor and owns a copy of the means. `Normal::sample` returns
a numeric vector. Factors retain canonical covariance entries for cache keys
and future distribution identity; signed zeros in keys and means become zero.

The public `Limits` mirrors two new engine controls:

- `max_multivariate_dimension`: defaults to 64 and can lower the kernel's hard
  ceiling of 64. Dimension is checked before traversing rows or copying data.
  The existing collection ceiling also applies to the vector/row length.
- `max_multivariate_bytes`: 64 MiB native, 16 MiB for `Limits::browser()`.
  All clones of an execution's work budget share one live byte allowance,
  including workers. These fields are reserved for the M5 kernel until the
  builtin is connected to execution.

The allowance charges requested vector capacity for covariance keys, means,
factor coefficients, scales, construction/draw scratch and cache entries, plus
the fixed recipe/factor structures and their shared ownership headers. Buffers
reserve before allocation and use `try_reserve_exact`. Cache growth charges
old and new allocations together; factor replacement also keeps old recipes
charged until their final owner drops. Reservations are released after the
payload, including on errors. Shared clones retain the original charge instead
of duplicating or detaching it.

This is an implementation-dependent accounting ceiling. It excludes caller-owned
inputs/model values, returned sample lists, fixed stack/counter/cache-object
overhead, allocator rounding/metadata and other engine storage. Returned vectors
use fallible allocation and existing collection/work controls. Fixed-size `Arc`
headers use the standard allocator after reserving their accounted size; growing
buffers are fallible. The allowance is not a process-memory or WASM-linear-memory
limit. Its high-water counter measures reservations, including failed allocation
attempts within the allowance, not physical peak heap.

The kernel receives already-converted `f64` inputs. Future language integration
must also budget numeric conversion buffers; it cannot treat those temporaries
as free caller-owned input. Recipes and caches reject use with a different
execution's resource counter, preventing retained factors from bypassing a new
execution's allowance. Work-budget clones within the same execution are allowed.

## Work, cancellation and errors

Dimension/shape/finite checks, buffer initialization, normalization, pivot scans,
dot products, residual validation and cache lookup/movement consume existing
work units. Checks run before each bounded construction row or pivot scan.
Draws preflight work, scratch and result allocation before consuming RNG, then
poll cancellation while generating normals and evaluating rows. A failed
preflight leaves the model random stream unchanged; cancellation during a draw
aborts that operation without returning a partial vector.

Malformed matrices or means use `DomainError` and identify an argument or
coordinate. Resource failures/cancellation use `Limit`; a nonfinite computed
draw uses `NumericOverflow`. There is no fallback covariance or silently dropped
coordinate. Invalid means are checked before factor construction, even on hits.

## Constructor cache

`Cache` keeps one most recent covariance per `(function, source offset)` site.
It owns a sorted, fallibly allocated entry buffer; the caller supplies an entry
cap, intended to be `max_cached_calls`. Zero disables retention. At the entry
cap, a new site clears the cache, following the existing call-cache policy.
`clear()` releases its entries and capacity, while recipes still owning factors
keep their charges. Interpreter wiring must call it at every batch boundary.

Keys compare finite numeric covariance entries exactly, with both signed zeros
equal. Means do not enter the factor key: a hit still validates and copies the
new means, and incurs work and memory charges. Reusing a factor never reuses a
draw. No process-global cache or sampled user-function memoization is added.
Changed covariance allocates a new factor before replacing the previous one.

## Validation and reproduction

```sh
cargo test --locked -p probl-engine multivariate
cargo test --locked -p probl-bench --example multivariate-normal
cargo test --locked -p probl multivariate
cargo build --locked --release -p probl-bench --example multivariate-normal
target/release/examples/multivariate-normal --engine --samples=100000
cargo build --locked -p probl-bench --example multivariate-normal --target wasm32-unknown-unknown --profile wasm
bun crates/probl-bench/examples/multivariate_normal/replay.mjs
```

Engine tests sweep every insufficient byte allowance and work allowance for a
construction, and every insufficient allowance for growing a populated cache.
They check reservation release, retained recipes after eviction, shared clones,
cache hits under exhausted budgets, cancellation, capacity overflow, explicit
allocation failure, dimension/domain diagnostics, and concurrent workers sharing
one ceiling. Library tests check the native/browser defaults and limit mapping.

The benchmark tests compare engine and reference coefficients, residuals,
samples and following RNG draws across all thirteen fixtures, both directly and
through the cache. The actual native and WASM engine kernels reproduce the
prototype's three recorded fingerprints. Existing numerical/statistical tests
remain in place. Cached batches also replay across 1/2/4/7 workers with explicit
batch clearing. Workspace tests, Clippy, formatting and the 5,000-case release
oracle passed for this engine change.

## Initial measurements

Local native release build, ordered prototype/engine/engine/prototype. Each run
uses 100 cold constructions and 100,000 draws per fixture, plus 1,000 constructors
against a warmed factor cache. The engine timings include work/byte checks and
owned covariance/means; the prototype has neither those controls nor a cache.

| Fixture | Dimension / rank | Engine construction, ns | Cache hit constructor, ns | Engine draw, ns | Retained accounted bytes |
|---|---:|---:|---:|---:|---:|
| Correlated | 2 / 2 | 271–272 | 67–70 | 89–90 | 352 |
| Perfect correlation | 2 / 1 | 265–290 | 67–75 | 70–71 | 352 |
| Gram matrix | 64 / 8 | 27,734–28,367 | 3,869–3,990 | 564–565 | 66,816 |
| Full rank | 64 / 64 | 109,780–114,632 | 3,988–4,024 | 3,804–3,849 | 66,816 |

The full-rank prototype measured 92,798–93,109 ns per construction and
2,856–2,863 ns per draw. Resource control and owned keys have a cost; repeated
construction benefits substantially from factor reuse. At two dimensions the
prototype measurements varied more (136–256 ns construction, 75–108 ns draw),
so those short runs do not support a precise percentage comparison.

A cache hit still charges validation/key comparison and new means: 14 work
units at dimension 2, 8,322 at dimension 64. It neither redraws nor refactorizes.
Retained accounted bytes include one recipe and factor with covariance keys;
they are not directly comparable to the prototype's coefficient-only byte count.

Two constructor sites per worker, dimension 64, a 16 MiB shared allowance,
and the latest recipe at each site held during replacement:

| Workers | Covariance | Peak bytes, 100 calls/worker | Peak bytes, 1,000 calls/worker |
|---:|---|---:|---:|
| 1 | Fixed | 165,896 | 165,896 |
| 1 | Changing each site's covariance | 232,736 | 232,736 |
| 4 | Fixed | 663,584 | 663,584 |
| 4 | Changing each site's covariance | 930,944 | 930,944 |

Both measured runs had these same accounting peaks, and every workload returned
to zero charged bytes after dropping caches/recipes. Tenfold calls did not grow
retained storage with a fixed set of sites/owners; workers and live old/new
factors do increase it. These are kernel workloads, not full-model heap or browser
measurements. The 64/16 MiB allowances remain initial engineering defaults.

A separate exact-retirement smoke comparison used 10,000 trials, one thread,
seed 42 and execution date 2026-10-24 in before/after/after/before order. Text
output was byte-identical. Times were 1.406–1.410 s before versus 1.420 s after,
about 0.8% higher in this small comparison; no default-path speedup is claimed.

## Remaining integration

No distribution value, builtin registration, interpreter cache or language
syntax changed in this slice. Next wire constructor sites and batch clearing,
typed numeric conversion, `Value` identity/formatting/type inspection, fallible
sampling/reporting, finite mixtures and explicit unsupported query/effect paths.
Then test through native/WASM requests and the playground, and measure actual
model workloads. The standalone kernel/cache measurements do not establish
interpreter speed or full-model memory use.
