# Running Probl from Rust

`probl` is the embedding library. `probl-cli` depends on it and installs the executable named `probl`. The browser adapter and benchmarks use the same library for loading, execution and results. The workspace is at version `0.2.1`; the API is young and may change.

## Compile and run

Add the library to a project with `cargo add probl`. The complete API reference for each release is on [docs.rs](https://docs.rs/probl); from this checkout, `cargo doc -p probl --no-deps --open` generates it for the current source. To use unreleased changes, depend on `probl` by path to `crates/probl`.

```rust
use probl::{Options, compile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = "let total ~ 2d6\nreport total >= 10 as \"ten or more\"";
    let program = compile("dice.probl", source)?;
    let outcome = program.run(&Options::new())?;
    print!("{}", outcome.text());

    if let Some(chance) = outcome.report("ten or more")
        .and_then(|r| r.groups().first())
        .and_then(|g| g.probability())
    {
        println!("point: {:?}, complete: {}", chance.point(), chance.is_complete());
    }
    Ok(())
}
```

`compile(name, source)` returns a reusable, cheaply cloneable `Program`, with warnings accessible through `warnings()`, or an error containing the diagnostics. The name is used in diagnostics. Compilation does not run the model or open its data files, and some type errors remain runtime checks.

`run` collects `print` output in `Outcome::printed()` and report text in `Outcome::text()`. `run_with` streams printed lines to a callback instead. An outcome also exposes structured reports, evidence, unresolved weight, sampling metadata, execution statistics, input provenance and the date used.

The longer [craps example](../crates/probl/examples/craps.rs) demonstrates unresolved bounds and sampling:

```sh
cargo run -p probl --example craps
```

## Options and modes

Settings are optional and use builder methods:

```rust
let options = probl::Options::new().runs(50_000).seed(7);
```

- `enumerate()` forces enumeration.
- `sample()`, `runs(n)` or `seed(s)` select sampling. Unspecified run count and seed come from the program's sampled `@mode`, or default to 10,000 and 0.
- Without an override, the program's mode applies. `auto` currently enumerates; it does not fall back to sampling.
- `Mode::Beam` and `Mode::Particles` exist for parsed programs, but execution returns `Unsupported`.
- `today(date)` supplies the immutable date snapshot when a model uses `today`. The native host can obtain it with `Date::today_utc()`; WASM hosts supply it themselves. Pin the date for reproducibility.
- `epsilon`, `fractions` and `conjugate` control approximation/display settings and exact conjugate updates.
- `on_error(FailureMode::Total)` or `on_error(FailureMode::Partial)` chooses what a fault in one world does to the others, whatever the program's `@on_error` says. Without either, a run is total when it enumerates and partial when it samples ([partial results](#partial-results)).
- `limits`, `cancel` and `progress` give the host control over resources and execution.

Native execution uses a thread with the configured stack size. WASM runs on the calling thread; the playground supplies its own worker isolation.

## Load data once, run it repeatedly

Models using `read` need their data loaded before execution:

```rust
use probl::{MemoryFiles, Options, compile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = compile("data.probl", r#"
        let values: list[int] = read("values.json")
        report values.sum()
    "#)?;
    let mut files = MemoryFiles::new();
    files.insert("values.json", b"[1, 2, 3]".to_vec());
    let options = Options::new();
    let data = program.load(&mut files, &options)?;
    let outcome = program.run(&options.data(data))?;
    print!("{}", outcome.text());
    Ok(())
}
```

`Data` is cloneable and reusable across runs of its program. Execution does not reopen files. Missing data, or data loaded for another program, produces a `Usage` error. Loaded values must still satisfy the run's limits, even if loading used looser limits.

`Files` maps model paths to identities and byte streams. The implementations are:

- `MemoryFiles`: explicitly supplied bytes, suitable for uploaded or bundled inputs.
- `LocalFiles`: native filesystem rules, relative to a base directory or a program's location. This is not a sandbox; absolute paths and parent traversal are allowed. Standard input requires `.stdin(true)`; `.cancel(&cancel)` allows cancellation while waiting for it.
- `Snapshots<F>`: caches bytes from another provider until `clear()`. The REPL uses this across inputs, with `:reload` to refresh.

The host grants read authority. Use a restricted provider for untrusted models, rather than relying on literal paths to provide confinement. The [data guide](data-input.md) specifies formats, schemas, input limits and provenance.

## Limits and diagnostics

`Limits::default()` uses native defaults; `Limits::browser()` uses lower memory/work limits and one thread. The public fields can be adjusted after construction. `Cancel` is cloneable and can be triggered from another thread. Program pragmas cannot raise the host's ceilings.

One `Error` type covers compilation, loading and execution. `kind()` distinguishes `Compile`, `Usage`, `Language`, `Unsupported`, `Limit` and `Internal`. `render(color)` formats source diagnostics. Individual diagnostics expose messages, notes, help, byte spans and one-based line/column positions. Frontends using UTF-16 positions must convert byte spans.

Resource budgets and caught native panics improve containment; they do not replace process isolation for a service executing untrusted models. See [host boundaries](architecture.md#host-boundaries).

### Partial results

In partial mode, a fault such as a division by zero ends only the world it happens in, and the others finish ([failure modes](semantics.md#failure-modes)). When some worlds failed, `run` and `run_with` still return an error, so `program.run(&options)?` doesn't accept a partial answer as a complete one. The error's `partial()` has what the other worlds gave:

```rust
match program.run(&options) {
    Ok(outcome) => print!("{}", outcome.text()),
    Err(e) => match e.partial() {
        Some(outcome) => {
            for failure in outcome.failures() {
                eprintln!("{}: {:?} failed", failure.diagnostic(), failure.share());
            }
            print!("{}", outcome.text());
        }
        None => return Err(e.into()),
    },
}
```

- The error is a `Language` error, with a diagnostic for each place worlds failed (the first ten), noting how much failed there.
- `failures()` groups them by place and kind of fault, in the order they first failed. Each `Failure` has its `diagnostic()`, the `weight()` the worlds had when they failed and, when sampling, the `runs()` that failed and the `first_run()`.
- `share()` and `failed_share()` are the share of the weight, or of the runs, that failed, with a `standard_error()` when sampling. They're `None` when the failed worlds could still have met evidence: their weight then stops short of the finished worlds', and the two can't be compared. Reach is unavailable then too, and `evidence()` is only the finished worlds' contribution, as the summary line says.
- The reports describe the worlds that reached them, failed worlds included when they reported before failing. `finished()` says whether any world finished at all.
- `failure_mode()` says which mode the run used, and `Program::failure_mode()` gives the program's `@on_error`, if any. An outcome without failures is an ordinary result, whatever its mode.

## Read reports without parsing text

`Outcome::reports()` returns report sites in order; `report(label)` looks one up. Without `as`, the label is the report expression. A report contains groups for its `by` keys, or one group without a key.

| Accessor on `Group` | Meaning |
|---|---|
| `key()` | Optional formatted group label |
| `probability()` | A boolean report's probability estimate |
| `distribution()` | An available finite table of formatted outcomes and probability estimates |
| `numeric()` | Supported real numeric mean, standard deviation and quantile queries |

Facts have `probability()` and no redundant true/false table. A continuous analytic report has no finite table: `None` means unavailable, not an empty population. Outcome labels are display strings, not a typed serialization format; distinct outcomes can have the same display label.

Numeric summaries expose the renderer's precision. They do not make `f64` an exact representation of arbitrary integers. `quantile(q)` returns `InvalidQuantile` for nonfinite/out-of-range probabilities and `Unavailable` when the summary cannot be supplied.

### Point estimates and unresolved mass

`Estimate::point()` returns a point over resolved mass whenever it can be computed. `None` means no point is available, not simply that a tail remains unresolved. In the craps example, the useful point remains available despite a tiny positive unresolved tail.

Always interpret it with `is_complete()` and `bounds()`. There is no epsilon threshold that marks positive unresolved mass as complete. When incomplete, a report point describes the resolved population; it is not an unqualified full-population answer.

Enumeration supplies unresolved bounds for boolean probabilities and finite-table probabilities where available. Means, standard deviations and quantiles have no such bounds. A tiny missing share cannot by itself bound the error of a mean over unbounded values. Sampled runs cut short are also incomplete, but do not expose unresolved bounds: plug-in bounds would themselves be estimates.

For complete enumeration, results still have numerical rounding. For sampling, `is_complete()` does not eliminate Monte Carlo error or mean that every possible outcome was visited.

### Sampling uncertainty

`sampling()` is absent for enumeration. Sampled quantities carry contributing/effective run counts, an optional standard error and confidence interval, and an explicit status:

| Status | Meaning |
|---|---|
| `Estimated` | An error estimate is available |
| `NotEstimable` | The observed runs cannot establish a useful error estimate |
| `NotComputed` | This statistic's sampling error is not computed |
| `IntegratedZero` | The engine identifies zero empirical Monte Carlo error from integrated outcomes |

A sampled fact's zero empirical standard error is `IntegratedZero` only for the supported integrated case; otherwise it is `NotEstimable`. Zero errors for table values and means are `NotEstimable`; standard deviation and quantile errors are `NotComputed`. These statuses must not all become “± 0”.

Eligible unweighted, single-contribution Bernoulli reports receive a 95% Wilson interval when every contributing run agrees or fewer than 30 runs contribute. This interval is not a replacement for weighted or integrated uncertainty. Sampling information on an incomplete result describes resolved mass, separately from unresolved uncertainty.

### Evidence and execution metadata

`evidence()` is absent when there was no observation or score. Otherwise its `kind()` distinguishes probability from density evidence; a density may exceed one.

`log_value()` preserves the natural logarithm of the resolved evidence contribution without first converting a tiny likelihood to an underflowing `f64`. Unlike report points, evidence is not renormalized over resolved worlds. `is_complete()` and `log_bounds()` describe incompleteness. Zero contributions or lower bounds use negative infinity.

For sampled evidence, `relative_standard_error()` is relative error on the evidence estimate, not absolute error in log units. Its status is `Estimated` whenever finite, including zero when all runs have the same weight, and `NotEstimable` with one run.

`unresolved()` is an unnormalized engine-weight bound, not a posterior probability; when sampling it is scaled per run. Do not reconstruct report-specific bounds from that scalar. `stats()`, `sampling()`, `data()` and `today()` expose execution counters, run settings, input identities/byte counts/SHA-256 hashes and the date snapshot.

## Compatibility and remaining work

The library owns its public types. `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine` remain internal APIs. The hidden `probl::__internal` adapter lets workspace tools configure merge/memoization/solver checks; it is explicitly outside the supported API and can change each release.

Typed outcome values, typed non-real summaries, density-query methods, report reach accessors, public fault codes, local recovery from faults and a public editor API remain deferred. Shared report calculations already know reach, but the public library does not expose it yet. See [library tests](../crates/probl/tests/library.rs) and [testing](testing.md) for behavioral coverage.

## Packaging and publication

Two crates are meant to be used directly: `probl`, the library (`cargo add probl`), and `probl-cli`, which installs the `probl` command (`cargo install probl-cli --locked`). They depend on `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine`, which are published too, but whose descriptions say they have no stable API. The benchmarks, the oracle and the WASM adapter are not published (`publish = false`).

Every published crate has the workspace's version. In the root `Cargo.toml`, each internal dependency is pinned exactly to that version (`version = "=0.2.1"`) next to its path, so a release of `probl` always uses the internal crates from the same commit. The packages leave out `tests/` and `examples/`, because those read the workspace's `examples/` and `benches/`.

Each published crate has its own `README.md`, selected by its package manifest and displayed on crates.io. The library and CLI READMEs explain how to use those packages; internal crate READMEs describe their role and direct users to `probl`. The repository's root README remains the language overview. Use absolute links in crate READMEs so they work on both GitHub and crates.io.

To release:

1. In the root `Cargo.toml`, set the new version in `[workspace.package]` and in each pin in `[workspace.dependencies]`. Then build once so that `Cargo.lock` follows, and add the release's section to the [changelog](../CHANGELOG.md). Commit these together.
2. Check the packages with `cargo publish --workspace --dry-run --locked`. `cargo package --list -p <crate>` shows what a package contains.
3. On GitHub's releases page, publish a **pre-release** with the tag `vX.Y.Z` on that commit. This is a dry run: the [release workflow](../.github/workflows/release.yml) checks that the tag matches the version, runs the tests and the packages' dry run, and builds and tests the container image on linux/amd64 and linux/arm64 runners. Nothing is published.
4. When it passes, edit the pre-release and make it a release. The same checks run again, then the crates are published through crates.io's Trusted Publishing, once a maintainer approves the `release` environment, and the image as `ghcr.io/garlab/probl:X.Y.Z`, `:X.Y` and, for the latest release, `:latest`, with a provenance attestation. Neither needs a stored token. Crates already on crates.io at that version are skipped, so a failed publish can be re-run from the workflow's page.
5. Check that docs.rs built each crate's documentation, and that `cargo install probl-cli --locked` installs the new version.

A failed dry run leaves its tag behind: delete the pre-release and its tag before trying again, or use the next version if the tag is protected.

Trusted Publishing can only be configured for a crate that already exists on crates.io. So a newly published crate's first release is done by hand, after which its crates.io settings name this repository, `release.yml` and the `release` environment as its trusted publisher.

Before 1.0, Cargo treats `0.1.x` releases as compatible with each other, so a release that breaks the `probl` API must be `0.2.0`. [cargo-semver-checks](https://github.com/obi1kenobi/cargo-semver-checks) can compare `probl` against its last published version; CI doesn't run it yet. See [Cargo's publishing guidance](https://doc.rust-lang.org/cargo/reference/publishing.html) and [SemVer compatibility](https://doc.rust-lang.org/cargo/reference/semver.html).
