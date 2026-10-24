# Multivariate-normal numerical prototype

October 24, 2026. This is a standalone experiment for the retirement roadmap's
M5 proposal. **`multivariate_normal` is not yet a Probl builtin.** The experiment
does not change the language, engine sampling paths, or existing seeded results.
The numerical rules now also have a [bounded engine implementation](multivariate-normal-kernel.md),
with shared live-memory accounting, work/cancellation checks and a factor cache.

The [prototype](../crates/probl-bench/examples/multivariate-normal.rs) evaluates
a deterministic pivoted Cholesky factorization for up to 64 dimensions. It uses
Probl's existing `Rng::normal`, including its `libm` arithmetic. There is no BLAS
dependency, platform eigensolver, or additional random stream.

## Numerical contract under evaluation

Inputs are a nonempty square finite covariance matrix. Negative diagonal entries
fail, even when tiny. A zero variance requires an exactly zero row and column;
its sampled coordinate is its mean. Empty, ragged, nonfinite and oversized inputs
have explicit errors.

Let `d` be the dimension, `s[i] = sqrt(cov[i][i])`, and
`tau = 64 * d * f64::EPSILON`. Normalize positive-variance entries as
`cov[i][j] / max(s[i], s[j]) / min(s[i], s[j])`. This avoids forming the product
of two standard deviations. Positive-variance diagonal entries are one; zero
rows remain zero. A normalized off-diagonal magnitude above `1 + tau` fails.
Both triangles are checked: normalized asymmetry above `tau` fails, otherwise
their midpoint defines the symmetric matrix `R` used below.

Factor `R` with complete diagonal pivoting, choosing the lowest original index
on a tie. Stop when the largest remaining pivot is at most `tau`. **Then check
every entry of `R - L Lᵀ`**, including off-diagonal residuals. Reject a nonfinite
residual or a maximum absolute residual above `tau`. Stopping on a small
diagonal alone is unsafe: `[[1,1,1],[1,1,-1],[1,-1,1]]` would stop after one pivot
despite being indefinite. This fixture is a regression test.

The supported numerical domain is matrices reconstructible within that normalized
entrywise tolerance. Tiny negative modes can be accepted and small positive
modes can be dropped; this is not an exact eigenvalue test or a nearest-PSD
projection. Symmetrization adds at most `tau/2` per normalized input entry before
the reconstruction check, apart from floating-point rounding. Near-singular
behavior and the tolerance must be documented if this becomes a language API.
The tolerance is a matrix arithmetic rule, not a Monte Carlo error estimate.

Draw `rank` independent standard normals once, then compute
`x[i] = mean[i] + s[i] * sum(L[i,k] * z[k])` in fixed index order. Zero rank
consumes no random numbers. Repeated calls make independent vector draws;
coordinates within a draw share the same `z`. Means must be finite and have the
same dimension. A nonfinite output is an explicit error.

Correlation scaling matters: the fixture `[[1e300, 0.5], [0.5, 1e-300]]` keeps
both variance modes. A single absolute covariance threshold could discard the
smaller one. The smallest positive subnormal variance also has a fixture.

Pivoted Cholesky supports rank-deficient matrices, but its termination status
alone is not a PSD validator; LAPACK documents that limitation explicitly. The
full residual test above is part of this prototype's acceptance rule.
[LAPACK DPSTRF](https://netlib.org/lapack/explore-html/de/de0/group__pstrf_ga27d24f9a4af47099060aafcb15fbd296.html).

An eigendecomposition or SVD is another option. NumPy exposes both alongside
Cholesky and cautions that linear algebra choices can change samples across
builds. Probl's replay contract motivates evaluating a small fixed-order Rust
kernel rather than delegating factorization to a platform library.
[NumPy Generator.multivariate_normal](https://numpy.org/doc/stable/reference/random/generated/numpy.random.Generator.multivariate_normal.html).

## Reproduce

From the repository root:

```sh
cargo test --locked -p probl-bench --example multivariate-normal
cargo build --locked --release -p probl-bench --example multivariate-normal
target/release/examples/multivariate-normal --samples=100000
cargo build --locked -p probl-bench --example multivariate-normal --target wasm32-unknown-unknown --profile wasm
bun crates/probl-bench/examples/multivariate_normal/replay.mjs
```

Eight tests cover independent Gram-matrix reconstruction (dimensions 1–64),
invalid and asymmetric matrices, tolerance boundaries, singular dependence,
zero variance, unchanged RNG on invalid means, and batch replay with 1/2/4/7
workers. Empirical means, covariances and cross-covariances between independent
draws use 100,000 pairs at three fixed seeds, with seven-standard-error
tolerances. Those statistical checks supplement the algebraic tests.

The actual native and WASM binaries agree on fingerprints covering all thirteen
fixtures, factors and draws, across three 1,000-draw batches per fixture:

| Seed | Fingerprint |
|---|---|
| 0 | `bf315d640a36324a` |
| 42 | `2c4b2bf4d83c518a` |
| 4294967295 | `1959d7020b07885a` |

This verifies the standalone kernel on the tested native/WASM builds. Engine
integration will still need tests through the language, interpreter, effects,
limits, report outputs, and actual WASM request adapter.

## Initial cost measurements

Two consecutive local macOS release runs, 100 constructions and 100,000 draws
per fixture. These are small kernel timings, not retirement-model speedups or
browser timings. Construction includes validation, factorization, reconstruction
checking and temporary allocation; drawing includes mean validation and the
normal scratch/output vectors.

| Fixture | Dimension / rank | Construction, ns | Draw, ns | Retained numeric bytes |
|---|---:|---:|---:|---:|
| Correlated | 2 / 2 | 117–141 | 76–79 | 48 |
| Perfect positive correlation | 2 / 1 | 104–122 | 57–58 | 48 |
| All zero | 3 / 0 | 146–173 | 24–29 | 96 |
| Gram matrix | 8 / 3 | 397–455 | 121–124 | 576 |
| Gram matrix | 32 / 5 | 4,982–5,035 | 261–269 | 8,448 |
| Gram matrix | 64 / 8 | 21,195–21,645 | 515–516 | 33,280 |
| Full-rank correlation matrix | 64 / 64 | 91,830–93,022 | 2,882–2,908 | 33,280 |

Retained bytes count vector capacities for the dense factor and scales only.
They exclude headers, input covariance, means, scratch, caches, allocator
overhead and engine state; they are not measured peak heap or RSS. The prototype
keeps a `d × d` factor even when rank is smaller. The production design must
budget all growing allocations, not only these coefficients.

Maximum reconstruction residual was `4.44e-16` for the regular Gram fixtures.
The deliberately near-singular two-dimensional fixture drops one mode, with
residual `2.00e-14` against tolerance `2.84e-14`. All-zero and perfectly
correlated fixtures reconstructed exactly in these measurements.

For two variables with covariance `[[4,3],[3,9]]`, a hand-written shared-factor
kernel uses `x = 2*z0` and `y = 1.5*z0 + sqrt(6.75)*z1`. Alternating
general/shared/shared/general, twice, measured 77–81 ns for the general factor
sampler and 53–55 ns for the shared-factor kernel. Both include the returned
vector allocation. The general sampler additionally validates means, traverses
the factor and allocates normal scratch. This comparison motivates constructor
reuse and retains shared scalar draws as a useful simple alternative.

## Remaining integration work

The original experiment still uses ordinary infallible `Vec` allocations,
constrained only by the dimension cap. The separate bounded engine kernel adds
resource controls and a cache; its tests now compare engine answers with this
reference. Language/value/effect integration remains pending. Sampling-only
diagnostics must cover enumeration, density,
queries, lifting, mixtures and nested `simulate`; no operation may substitute
independent marginals or silently discretize the vector.
