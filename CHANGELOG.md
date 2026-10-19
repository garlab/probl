# Changelog

All notable changes to the published crates are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, a release that breaks compatibility increments the minor version.

## [Unreleased]

### Added

- The library's optional `json` feature exports bounded, versioned report summaries with typed group keys, quantiles, reach, uncertainty and partial-result metadata. WASM returns the same document alongside its existing text; serialization limits leave that text available. See the [schema](docs/report-json.md).
- The playground draws forecast percentile bands, deterministic lines and probability step curves from structured reports. Compatible baseline overlays and currency formatting are explicit choices, saved locally and in share links. Keyboard inspection and tables retain uncertainty, reach and partial-result qualifications; all reports remain available as text.
- Report statistics are configurable per report through `probl run --summary ID=SPEC`, Rust's `ReportSummary` / `Options::report_summary`, and the playground's “Report summaries” controls. Presets include 10/25/median/75/90 forecast columns and mean/SD; custom columns preserve their order. Explicit quantiles use the inverse CDF while `median` retains midpoint ties. Text, JSON, charts and tables agree, with unchanged defaults and seeded draws. See [summary selection](docs/report-summaries.md).
- `deck.take(n)` draws a list of items without replacement from a mutable bag, in draw order. `take()` still returns one item; zero returns an empty list, and an insufficient bag fails before removing items.
- `combinations(xs, n)` lists groups from lists, ranges and strings in input-index order, preserving duplicate positions. It supports method syntax, checked counts, and collection/work limits.
- A SET example in the CLI examples and playground uses `take(12)` and `combinations(3)` to estimate the probability that a hand contains no set.
- `any(xs, test)` checks lists, ranges and strings in order, stopping at the first match. The SET example uses it to skip unnecessary rule checks while keeping the same trials and seeded result: [about 2.7× faster in WebAssembly](docs/benchmarks.md#since-stopping-at-the-first-set-match).

## [0.3.2] - 2026-10-17

### Added

- `env` is the environment variables, a `map[str, str]` constant that the host captures once, like `today`: `env.get("REGION", "eu")` reads a variable with a default, and `"CI" in env` says whether one is set. Every world, run and thread sees the same map, and a program can't change it: `env = …`, `env["A"] = …` and `env.insert(…)` are compile errors, and `var copy = env` makes a copy that can change. A program's own `env` hides it. Values are text; `date(…)` reads a date from one, and there's no conversion to numbers yet ([semantics](docs/semantics.md#1-values-and-types)).
- `probl run` and the REPL give a program their whole environment, captured when they start; `probl run --env NAME=VALUE` sets a variable in place of the process's. In the library, `Options::env` gives a variable, and the library never reads the process's environment. The WebAssembly request takes an `env` object. The playground has none, so `env` is empty there.
- Each GitHub release has a browser bundle, `probl-X.Y.Z-browser.json` with its checksum: the WebAssembly engine, its JavaScript adapter and highlighter, the built playground, the changelog, licenses and native reference results. The release tests it in Chrome and attests it like the source archive, and the hosted playground is deployed from it ([releases](docs/releases.md#browser-distribution)).

## [0.3.1] - 2026-10-16

### Added

- `prime_factors(n)` lists the primes whose product is `n`, smallest first and repeated as often as they divide it: `prime_factors(360)` is `[2, 2, 2, 3, 3, 5]`. `is_prime(n)` and `is_squarefree(n)` say whether `n` is prime, and whether no prime divides it twice. Like the other integer functions they work on bigints and apply to each outcome of a distribution: `is_prime(d1000)` is 16.80%. `is_prime` is exact below 3.3 × 10²⁴ and uses the Baillie–PSW test above, which no known composite passes ([semantics](docs/semantics.md#1-values-and-types)).
- `divisors(n)` lists the divisors of `n`, and `primes(hi)` or `primes(lo, hi)` the primes in a range, both ends included: all 5,761,455 primes below 10⁸ take 0.3 s. `next_prime(n)` and `prev_prime(n)` find the nearest prime above and below `n`. `isqrt(n)` is the exact floor of a square root, without a float: `isqrt(10^400)` is `10^200`. `mod_pow(b, e, m)` is `b^e mod m` without forming `b^e`, so `mod_pow(3, 10^20, 7)` is 4, and `mod_inverse(a, m)` is the `x` with `a * x mod m` equal to 1. Both place results on the modulus's side of 0, as `mod` does.

### Changed

- `euler_phi` factors with Miller–Rabin and Pollard's rho method rather than trial division: `euler_phi(2^61 - 1)` takes milliseconds rather than 9.7 s, and summing it over 1 to 1,000,000 takes 1.0 s rather than 2.1 s. A large prime no longer reaches the work limit; a number with two large prime factors still can.
- A value that fails a declared type is described as `typeof` would describe it: "`b` should be a list[int], but it's a list[str]", rather than "but it's a list". A declared record type is written as `typeof` writes one, `{a: int}`.
- Comparing collections, and copying one to change it while something else holds it, count against the work limit: a unit for each pair of elements compared, or each element copied. A program that compares long lists in a loop can now reach the limit, which before let it run unbounded.

## [0.3.0] - 2026-10-15

### Changed

- A function's return type follows a `:`, like a variable's: `fn explode(): int { … }` rather than `fn explode() -> int { … }`. It's still optional. A program written the old way no longer compiles, and the error says how to write it.
- A named function used as a value reads the variables it uses when it's called, as a call by name does: after `let f = fee` and a change to the `rate` that `fee` reads, `f(100)` uses the new rate, where it used the old one. `f == fee` stays true. Lambdas still keep the values from when they were made.
- `push`, `pop`, `insert` and `remove` change a list or map in place, rather than copying all of it on every call, unless something else holds it: 20,000 pushes in a loop take 0.01 s rather than 0.97 s. On a variable with a declared type, they check only what they add, rather than the whole variable. Built-ins like `get` no longer look through every element of a list on every call.

## [0.2.2] - 2026-10-12

### Added

- More continuous calculations work when enumerating ([semantics](docs/semantics.md#analytic-outcomes-in-enumeration)). `abs`, `min`, `max`, `clamp`, and `minimum` and `maximum` of a list, are exact on a draw, including the probability at a bound: `max(x, 0) == 0` is the probability that `x` is negative. `sum` and `mean` of a list of outcomes of one draw, and the built-ins that move values without looking at them, such as `get`, `slice`, `reverse` and `pop`, keep the draw. `report x by x > 1` has a group for each side.
- Enumeration weighs finite worlds by densities: `let mu ~ one_of([0, 1]); observe 0 from normal(mu, 1)` no longer needs sampling. The evidence is then a log density, `log evidence -1.1380`, and the library's `Evidence::kind` says so. Worlds must observe as many continuous values, outside functions and without unresolved weight; other programs are rejected as unsupported.
- Enumeration updates continuous draws exactly by conjugate observations, as sampling does: `let p ~ beta(2, 3); observe 3 from binomial(5, p)` reports `p` as a beta(5, 5), with evidence 4/21. The same goes for `bernoulli` and `score` with a beta, `poisson` with a gamma or exponential rate, and `normal` with a normal mean; a uniform probability counts as a beta(1, 1). A draw restricted by a condition stays restricted. `if p`, `~bernoulli(p)` and `~binomial(n, p)` with a beta `p` branch exactly, updating `p` in each branch.
- `floor`, `ceil`, `trunc` and `round` of a draw with bounded values work when enumerating: `floor(x)` for `x ~ uniform(0, 3)` is 0, 1 or 2, a third each, and observing it restricts `x`. Assigned, it's a plain int in each world, which can index a list.
- Squares, square roots, exponentials, logarithms and reciprocals of a continuous draw work when enumerating: `x * x`, `(x - 1) ^ 2`, `sqrt(x)`, `exp(x)`, `ln(x)`, `1 / x`, `2 ^ x` and the other logarithms and exponentials. Their quantiles, CDFs and observations are exact, and their means and sds where the draw's family has formulas for them; otherwise a report shows the mean as unavailable. `x * x` for `x ~ uniform(-1, 1)` has mean 1/3, and `observe exp(z) > 2` restricts `z` to above ln 2.
- Conditions on different continuous draws combine when enumerating: `x > 0 and y > 0` for two draws has probability P(x > 0) P(y > 0), and observing or branching on one restricts each draw exactly, in parts: `observe x > 0 or y > 0` keeps the part where `x > 0`, and the part where `x ≤ 0` and `y > 0`.
- Sums and differences of normal draws work when enumerating, and so do normal draws and observations whose mean is one: `let y ~ normal(mu, 1)` has variance 2 and covariance 1 with `mu`, and `observe 1.5 from normal(y, 0.5)` updates `mu` and `y` together, exactly, with their covariance. A normal observation's mean can be affine in one draw, like `normal(2 * mu, 1)`. Conditions on a sum, and sums with a restricted draw, are unsupported.
- Continuous draws work inside `simulate`, in both modes. `simulate { let x ~ normal(0, 1); x > 0 }` is a distribution of bools, 50% each. A block whose value holds outcomes of its draws gives a distribution to draw from: each draw is fresh, and the parts of one outcome stay correlated, like the fields of a record. Its `mean`, `sd`, `quantile` and other queries come from its marginal, and the block can observe densities, so `simulate { let mu ~ normal(0, 1); observe 1.2 from normal(mu, 0.5); mu }` is a posterior to draw from.
- Reporting a record or list of continuous outcomes summarizes each field, when enumerating: `report {x: x, twice: 2 * x}` gives each field's marginal. The library's `Group::fields` gives them.
- `@integrate` (or `--integrate`, or `Options::integrate`) lets enumeration compute by numerical integration what has no formula: observations of one draw that aren't conjugate, like `observe true from bernoulli(1 - p)` or `observe 3 from poisson(rate)` for a lognormal `rate`, and means without formulas, like `exp(p)` for a beta `p`. Results computed that way are approximations to its relative tolerance (1e-9 by default): the summary line says `integrated to 1e-9`, and `Outcome::integration` gives it. Without `@integrate`, such observations stay unsupported, and their error says that `@integrate` would compute them.
- A loop that counts its rounds, or adds to a total that nothing else in the loop reads, is solved as a Markov chain over its other variables when enumerating, rather than unrolled world by world: the same results, much faster. Example 05's race runs in a sixth of the time, and a two-player snakes and ladders game 28 times faster. A loop whose worlds can never leave is found, rather than run to a limit.
- `pmf` and `cdf` of `binomial`, `poisson` and `geometric` are exact, rather than rejected for the tail their listed outcomes leave out: `cdf(poisson(3), 2)` is 42.32%.
- A count law too broad to list, like `geometric(1e-12)`, can be named and drawn from when sampling, and `mean`, `variance` and `sd` of it are exact (and `pmf`, `cdf`, quantiles and medians, for a geometric). Building one no longer fails on the outcome limit; listing it still does.
- An experimental symbolic backend, off by default and not part of the supported API: `probl run --symbolic` answers programs made of Boolean draws, Boolean expressions, `if`, `observe`, `score` and reports with binary decision diagrams instead of worlds, with the same results. Eighteen independent faults with evidence that one failed take under a millisecond rather than 0.8 s, and two hundred, which enumeration can't hold, a few milliseconds. `--symbolic=prefer` enumerates the programs it doesn't support ([plan](docs/design/symbolic-inference.md#stage-1-as-built)).
- Each GitHub release has a source archive, with the workspace's `Cargo.lock`, a manifest and checksums, all with build-provenance attestations, and the Homebrew tap updates its formula from it ([releases](docs/releases.md)).

### Changed

- Two capabilities that aren't built yet are reported as `Unsupported` errors rather than language errors: observing a value from a continuous distribution when enumerating, and arithmetic on an undrawn continuous distribution, like `normal(0, 1) * 2`.
- The container image's `probl` is built with the whole program optimized together, by the workspace's new `dist` profile: its binary is 2.8 MB rather than 3.3 MB, and most models run faster ([build profiles](docs/benchmarks.md#build-profiles)).

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

[Unreleased]: https://github.com/garlab/probl/compare/v0.3.2...HEAD
[0.3.2]: https://github.com/garlab/probl/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/garlab/probl/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/garlab/probl/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/garlab/probl/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/garlab/probl/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/garlab/probl/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/garlab/probl/tree/v0.1.0
