# Changelog

All notable changes to the published crates are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a release that breaks compatibility increments the minor version.

## [Unreleased]

### Added

- More continuous calculations work when enumerating ([semantics](docs/semantics.md#analytic-outcomes-in-enumeration)). `abs`, `min`, `max`, `clamp`, and `minimum` and `maximum` of a list, are exact on a draw, including the probability at a bound: `max(x, 0) == 0` is the probability that `x` is negative. `sum` and `mean` of a list of outcomes of one draw, and the built-ins that move values without looking at them, such as `get`, `slice`, `reverse` and `pop`, keep the draw. `report x by x > 1` has a group for each side.
- Enumeration weighs finite worlds by densities: `let mu ~ one_of([0, 1]); observe 0 from normal(mu, 1)` no longer needs sampling. The evidence is then a log density, `log evidence -1.1380`, and the library's `Evidence::kind` says so. Worlds must observe as many continuous values, outside functions and without unresolved weight; other programs are rejected as unsupported.
- Enumeration updates continuous draws exactly by conjugate observations, as sampling does: `let p ~ beta(2, 3); observe 3 from binomial(5, p)` reports `p` as a beta(5, 5), with evidence 4/21. The same goes for `bernoulli` and `score` with a beta, `poisson` with a gamma or exponential rate, and `normal` with a normal mean; a uniform probability counts as a beta(1, 1). A draw restricted by a condition stays restricted. `if p`, `~bernoulli(p)` and `~binomial(n, p)` with a beta `p` branch exactly, updating `p` in each branch.
- `floor`, `ceil`, `trunc` and `round` of a draw with bounded values work when enumerating: `floor(x)` for `x ~ uniform(0, 3)` is 0, 1 or 2, a third each, and observing it restricts `x`. Assigned, it's a plain int in each world, which can index a list.
- Squares, square roots, exponentials, logarithms and reciprocals of a continuous draw work when enumerating: `x * x`, `(x - 1) ^ 2`, `sqrt(x)`, `exp(x)`, `ln(x)`, `1 / x`, `2 ^ x` and the other logarithms and exponentials. Their quantiles, CDFs and observations are exact, and their means and sds where the draw's family has formulas for them; otherwise a report shows the mean as unavailable. `x * x` for `x ~ uniform(-1, 1)` has mean 1/3, and `observe exp(z) > 2` restricts `z` to above ln 2.
- `pmf` and `cdf` of `binomial`, `poisson` and `geometric` are exact, rather than rejected for the tail their listed outcomes leave out: `cdf(poisson(3), 2)` is 42.32%.
- A count law too broad to list, like `geometric(1e-12)`, can be named and drawn from when sampling, and `mean`, `variance` and `sd` of it are exact (and `pmf`, `cdf`, quantiles and medians, for a geometric). Building one no longer fails on the outcome limit; listing it still does.

### Changed

- Two capabilities that aren't built yet are reported as `Unsupported` errors rather than language errors: observing a value from a continuous distribution when enumerating, and arithmetic on an undrawn continuous distribution, like `normal(0, 1) * 2`.

## [0.2.1] - 2026-10-09

### Added

- A container image of the `probl` command, `ghcr.io/garlab/probl`, for linux/amd64 and linux/arm64, published with each release with a provenance attestation.
- Each crate has a README of its own on crates.io: the library's and the command's show how to use them, and the internal crates' say to use `probl` instead.

## [0.2.0] - 2026-10-09

### Added

- Failure modes ([semantics](docs/semantics.md#failure-modes)). A fault that depends on a world's values, like a division by zero or an index past the end, always ends that world. In partial mode the other worlds finish, and the result says how much failed and where; in total mode the run stops, as before. Choose with `@on_error total` or `@on_error partial`, `probl run --on-error`, `Options::on_error` or the playground's "On error" option.
- A partial result: `probl run` prints what the other worlds gave, with a `failed` section, and exits with status 3. `Program::run` still returns an error, with the result in `Error::partial`.
- `try { … } catch DivisionByZero { … } catch { … }`: a world where the body faults goes on in the first catch that names its fault, or in a catch without a name, with what it did before the fault. Faults from called functions reach the caller's `try`. The faults a catch can name are `DivisionByZero`, `DomainError`, `IndexOutOfBounds`, `MissingKey`, `EmptyCollection`, `ConversionError` and `NumericOverflow`; other errors aren't caught. In the playground, completion offers the fault names after `catch`, and hovering one says what it is.
- In the `probl` crate: `FailureMode`, `Options::on_error`, `Program::failure_mode`, `Error::partial`, `Failure`, and `Outcome::failure_mode`, `failures`, `failed_share` and `finished`.

### Changed

- `try` and `catch` are keywords: a program that uses either as a name no longer compiles.
- Sampling is partial by default: a run that faults no longer stops the whole sample, and the result is partial, so `probl run` exits with status 3 instead of 1. Enumeration stays total by default. `--on-error total` or `@on_error total` keeps the old behavior.
- Explicit conversions out of range, like `prob(1.5)`, are now faults; a declared type that a value fails, like `let p: prob = 1.5`, still always stops the run.

### Fixed

- The `probl` crate's documentation linked to probl.dev, which doesn't exist; it now links to the repository.

## [0.1.0] - 2026-10-06

The first release.

- `probl`: compile Probl programs, run them by enumeration or by sampling, and read their reports as structured results with their bounds, sampling status and evidence.
- `probl-cli`: the `probl` command, with `run`, `check`, `schema` and `repl`.
- `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine`: the internal crates that `probl` depends on, with no stable API.

The [language overview](docs/language-overview.md) and the [reference semantics](docs/semantics.md) describe the language, and [status and limits](README.md#status-and-limits) lists what isn't implemented yet.

[Unreleased]: https://github.com/garlab/probl/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/garlab/probl/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/garlab/probl/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/garlab/probl/tree/v0.1.0
