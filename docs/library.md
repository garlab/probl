# Running Probl from Rust

`probl` is the embedding library. `probl-cli` depends on it and installs the executable named `probl`. The browser adapter and benchmarks use the same library for loading, execution and results. The workspace is at version `0.1.0`; the API is young and may change.

## Compile and run

From this checkout, generate the complete API reference with `cargo doc -p probl --no-deps --open`. For a local application, depend on `probl` by path to `crates/probl`; registry installation should use a published version once available.

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

Typed outcome values, typed non-real summaries, density-query methods, report reach accessors, and a public editor API remain deferred. Shared report calculations already know reach, but the public library does not expose it yet. See [library tests](../crates/probl/tests/library.rs) and [testing](testing.md) for behavioral coverage.

## Packaging and publication

The intended registry layout is `probl` for embedders and `probl-cli` for installation of the `probl` command. Publishing one does not reserve the other name or the `probl-*` prefix. Once released, the corresponding commands are `cargo add probl` and `cargo install probl-cli --locked`.

Before the first publication:

1. Complete license, repository, description and README metadata for each published package. Check that the packaged README and license files are present.
2. Pin every internal workspace dependency and the CLI's `probl` dependency to the exact matching release, alongside its local path. They are pinned exactly (`version = "=0.1.0"`); move the pins with each release. Keep the release versions aligned.
3. Inspect package contents and perform dry runs with the release toolchain. Publish dependencies in order: `probl-number`, `probl-syntax`, `probl-sema`, `probl-engine`, `probl`, then `probl-cli`. Keep benchmarks, oracle and WASM adapter unpublished.
4. Verify installation of the actual published CLI and compilation of an external application depending only on `probl`. Once a baseline exists, add API compatibility checks alongside behavioral tests.

See [Cargo's publishing guidance](https://doc.rust-lang.org/cargo/reference/publishing.html) and [dependency requirements](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html). Package availability and release readiness must be checked at publication time.
