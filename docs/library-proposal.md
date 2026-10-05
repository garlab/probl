# Proposal: Probl as a Rust library

> Revised October 5, 2026. Proposed, not built. This adds a crate named `probl`: a small public API for compiling and running Probl programs from Rust, which the command line and the [playground](playground-plan.md) would both be built on. The crate split, separate data loading, and initial result capabilities are decided below. The API sketch defines the intended contract; implementation must preserve the engine's uncertainty information before publication.

## The problem

Three things point the same way.

- **The name.** The command line's package is `probl-cli`, which installs the `probl` binary. Giving the short package name to the library is an established convention: both [Typst](https://github.com/typst/typst/blob/main/crates/typst-cli/Cargo.toml) and [Wasmtime](https://github.com/bytecodealliance/wasmtime/blob/main/Cargo.toml) use a `-cli` package depending on the short-named library and installing the short-named executable. Cargo also permits a library and binary in one package; the split fits Probl because its engine already runs in more than one host. Publishing both packages under the project's ownership protects those exact package names, not the entire `probl-*` prefix or the executable name. Availability must be checked when publishing. Homebrew's formula can remain `probl` and build `crates/probl-cli` from the release tarball.
- **Two hosts repeat the same steps.** The command line (`crates/probl-cli/src/main.rs`) and the playground (`crates/probl-wasm/src/lib.rs`) each compile a program, read its data, choose the mode and run it, with their own copies of:
  - the rule that `--runs` or `--seed` imply sampling, and the defaults when the program's `@mode` doesn't sample (`ModeChoice::resolve`, and the playground's `mode`);
  - the data's limits, derived from the engine's (`input_limits`, in both);
  - a resolver for the data (`LocalFiles`, and the playground's `Files`);
  - the choice between `probl_engine::run` and `run_on_this_thread`.
- **There's no API to promise.** A Rust program that runs Probl today needs `probl-syntax`, `probl-sema` and `probl-engine`, and their internal types: `ir::Program`, the engine's `Options` with its switches for checking the engine, `Value`, `Weight`, `Sink`. Those change with every feature, as they should.

## Decisions

1. **`probl` is the library, and `probl-cli` the tool.** `cargo add probl` adds the library; `cargo install probl-cli --locked` installs `probl`. `probl-cli` depends on `probl`, and the README explains the distinction. `cargo install probl` reports that the package has no binaries.
2. **The library defines its own types, and re-exports nothing from the other crates.** `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine` are still published, since `probl` depends on them. Their descriptions identify them as internal, and dependencies on them pin the exact matching release. Their implementation APIs can change while the supported `probl` API follows its own compatibility contract. A description alone does not prevent Cargo from selecting mismatched versions.
3. **Keep result and program types opaque.** Use private fields with constructors and accessors. Use `#[non_exhaustive]` for extensible enums and public-field structs such as `Limits`; opaque structs already allow fields to be added without changing their construction syntax. Existing methods and behavioral promises still need compatibility checks.
4. **The workspace's own tools may still use the internal crates**, for what an embedder doesn't need: editor features, the IR printer, schema suggestions (see [what stays](#what-stays-on-the-internal-crates)). Their internal dependencies are pinned and they are tested and released together.
5. **Load data separately and reuse it across runs.** `Program::load` and `Options::data` are part of the initial API. The library preserves the engine's checks that supplied data matches the program and applicable limits.
6. **Expose structured inference results, including uncertainty.** Formatted outcome labels are sufficient initially, but real numeric summaries must be accessible without parsing output. Result types must preserve unresolved bounds, sampling error status, confidence intervals where available, and the distinction between probability evidence and density evidence.
7. **Engine-debugging controls use an explicitly unstable interface.** The CLI uses a hidden `__internal` module in `probl` for its engine switches. This module is excluded from the supported API and may change every release; hiding its documentation does not restrict access. The CLI pins its `probl` dependency to the matching version.

## The API

```rust
pub const VERSION: &str;

/// Compile a program. `name` is what its diagnostics call it.
/// Warnings come with the program; errors come instead of it.
pub fn compile(name: &str, source: &str) -> Result<Program, Error>;

/// A compiled program: cheap to clone, and runnable any number of times, from any thread.
#[derive(Clone)]
pub struct Program { /* Arc<ir::Program>, Arc<SourceFile>, warnings */ }

impl Program {
    pub fn warnings(&self) -> &[Diagnostic];
    pub fn mode(&self) -> Mode;                      // its `@mode`, which Options can override
    pub fn reads_data(&self) -> bool;                // uses `read`, so needs `load` first
    /// Read its data, with the limits and the cancellation in `options`.
    pub fn load(&self, files: &mut dyn Files, options: &Options) -> Result<Data, Error>;
    /// Run it. What it `print`s is collected in the outcome.
    pub fn run(&self, options: &Options) -> Result<Outcome, Error>;
    /// Run it, giving each printed line to `print` as it's printed.
    pub fn run_with(&self, options: &Options, print: &mut (dyn FnMut(&str) + Send))
        -> Result<Outcome, Error>;
}

#[non_exhaustive]
pub enum Mode { Auto, Enumerate, Sample { runs: u64, seed: u64 } }  // Beam, Particles when built

/// How to run; every setting is optional.  Options::new().runs(50_000).seed(7)
#[derive(Clone, Default)]
pub struct Options { /* … */ }
impl Options {
    pub fn new() -> Options;
    pub fn enumerate(self) -> Self;           // = --mode enumerate
    pub fn sample(self) -> Self;              // = --mode sample
    pub fn runs(self, n: u64) -> Self;        // implies sampling, as on the command line
    pub fn seed(self, seed: u64) -> Self;     // implies sampling
    pub fn today(self, date: Date) -> Self;   // needed only if the program uses `today`
    pub fn epsilon(self, e: f64) -> Self;
    pub fn fractions(self, on: bool) -> Self;
    pub fn conjugate(self, on: bool) -> Self;
    pub fn limits(self, limits: Limits) -> Self;
    pub fn data(self, data: Data) -> Self;
    pub fn cancel(self, cancel: &Cancel) -> Self;
    pub fn progress(self, f: impl Fn(u64, u64) + Send + Sync + 'static) -> Self;
}

/// The engine's limits and the data's, in one place.
#[non_exhaustive] #[derive(Clone, Debug)]
pub struct Limits {
    pub max_worlds: usize, pub max_work: u64, pub max_call_depth: usize,
    pub max_threads: usize, pub stack_size: usize, pub max_output: usize,
    pub max_input_bytes: u64, pub max_input_values: u64, /* … every field of today's
                                                             Limits and InputLimits */
}
impl Limits {
    pub fn browser() -> Limits;   // the playground's: less memory, a smaller stack, one thread
}                                 // Default is the command line's

/// Where `read("…")` finds data.
pub trait Files {
    fn resolve(&mut self, path: &str) -> Result<String, String>;   // path → identity
    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String>;
}
pub struct MemoryFiles { /* path → bytes */ }             // the playground's
pub struct LocalFiles  { /* moved from probl-cli */ }     // not on wasm32
pub struct Snapshots<F: Files> { /* F, and the bytes it has given */ }  // the REPL's
#[derive(Clone)] pub struct Data { /* Arc<Inputs> */ }

#[derive(Clone, Default)] pub struct Cancel { /* Arc<AtomicBool> */ }
impl Cancel { pub fn cancel(&self); pub fn is_cancelled(&self) -> bool; }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Date { /* days since 1970-01-01 */ }
impl Date {
    pub fn from_ymd(y: i32, m: u32, d: u32) -> Option<Date>;
    pub fn today_utc() -> Result<Date, Error>;   // not on wasm32, where the host passes it in
}                                                // Display and FromStr: YYYY-MM-DD

pub struct Outcome { /* … */ }
impl Outcome {
    pub fn text(&self) -> &str;                      // what `probl run` prints
    pub fn render(&self, reports: Range<usize>) -> String;  // some of the reports, aligned as in `text`
    pub fn reports(&self) -> &[Report];
    pub fn report(&self, label: &str) -> Option<&Report>;
    pub fn printed(&self) -> &[String];              // with `run`; empty with `run_with`
    pub fn evidence(&self) -> Option<&Evidence>;    // probability or density, represented in log space
    pub fn unresolved(&self) -> &Unresolved;        // engine weight bound; not a posterior probability
    pub fn sampling(&self) -> Option<Sampling>;      // runs, seed, effective runs
    pub fn stats(&self) -> &Stats;                   // what `--stats` prints
    pub fn data(&self) -> &[DataSource];             // identity, bytes, sha256
    pub fn today(&self) -> Option<Date>;
}

pub struct Report { /* … */ }
impl Report {
    pub fn label(&self) -> &str;
    pub fn groups(&self) -> &[Group];                // one per `by` key; one without `by`
}
pub struct Group { /* … */ }
impl Group {
    pub fn key(&self) -> Option<&str>;               // the `by` value, as Probl prints it
    pub fn probability(&self) -> Option<&Estimate>; // a fact: its probability and uncertainty
    pub fn distribution(&self) -> Option<&[(String, Estimate)]>; // finite outcome table, if available
    pub fn numeric(&self) -> Option<&NumericSummary>; // real numeric summaries, including continuous reports
}

/// One reported quantity, with the same population and denominator as the renderer.
pub struct Estimate { /* … */ }
impl Estimate {
    pub fn point(&self) -> Option<f64>;           // no point for an incomplete population
    pub fn bounds(&self) -> Option<Interval>;     // bounds due to unresolved mass, when available
    pub fn resolved_point(&self) -> Option<f64>;  // explicitly conditional on resolved mass; only when incomplete
    pub fn sampling(&self) -> Option<&SamplingUncertainty>;
}
#[non_exhaustive]
pub enum SamplingStatus { Estimated, NotEstimable, NotComputed, IntegratedZero }
pub struct SamplingUncertainty { /* … */ }
impl SamplingUncertainty {
    pub fn status(&self) -> SamplingStatus;
    pub fn standard_error(&self) -> Option<f64>;
    pub fn confidence_interval(&self) -> Option<&ConfidenceInterval>;
    pub fn contributing_runs(&self) -> u64;
    pub fn effective_runs(&self) -> f64;
}
pub struct Interval { /* lower, upper */ }
impl Interval { pub fn lower(&self) -> f64; pub fn upper(&self) -> f64; }
pub struct ConfidenceInterval { /* interval, confidence level, method */ }
impl ConfidenceInterval {
    pub fn interval(&self) -> &Interval;
    pub fn level(&self) -> f64;
    pub fn method(&self) -> ConfidenceMethod;
}
#[non_exhaustive]
pub enum ConfidenceMethod { Wilson }

pub struct NumericSummary { /* … */ }
impl NumericSummary {
    pub fn mean(&self) -> Option<&Estimate>;
    pub fn sd(&self) -> Option<&Estimate>;
    pub fn quantile(&self, q: f64) -> Result<Estimate, Error>;
}

#[non_exhaustive]
pub enum EvidenceKind { Probability, Density }
pub struct Evidence { /* … */ }
impl Evidence {
    pub fn kind(&self) -> EvidenceKind;
    pub fn log_value(&self) -> Option<f64>;        // natural log; no complete point when unresolved
    pub fn log_bounds(&self) -> Option<Interval>; // unresolved bounds in natural-log units, when available
    pub fn relative_standard_error(&self) -> Option<f64>;
    pub fn sampling_status(&self) -> Option<SamplingStatus>;
}
pub struct Unresolved { /* … */ }
impl Unresolved {
    pub fn is_zero(&self) -> bool;
    pub fn log_weight_upper_bound(&self) -> f64;  // unnormalized engine bound; -infinity for zero
}

#[derive(Debug)]                                     // and Display, std::error::Error
pub struct Error { /* … */ }
impl Error {
    pub fn kind(&self) -> ErrorKind;
    pub fn diagnostics(&self) -> &[Diagnostic];      // compiling: all of them; running: one
    pub fn render(&self, color: bool) -> String;     // as the command line prints it
}
#[non_exhaustive]
pub enum ErrorKind { Compile, Language, Unsupported, Limit, Internal }

pub struct Diagnostic { /* with its source, so it can render itself */ }
impl Diagnostic {
    pub fn severity(&self) -> Severity;              // Error or Warning
    pub fn message(&self) -> &str;
    pub fn notes(&self) -> &[String];
    pub fn help(&self) -> Option<&str>;
    pub fn span(&self) -> Range<usize>;              // in bytes
    pub fn line_column(&self) -> (usize, usize);     // from 1
    pub fn render(&self, color: bool) -> String;
}
```

### Compiling

`compile` takes the program's name, which its diagnostics use, and its text. It gives a `Program` and its warnings, or an `Error` of kind `Compile` with every diagnostic, warnings included. A `Program` holds the compiled IR and the source behind an `Arc`: cloning it is cheap, and it can run any number of times, from any thread.

### Running

Every setting of `Options` is optional. The mode follows the command line's rules, implemented in one place by the library:

- `enumerate()` enumerates, whatever the program's `@mode` says.
- `sample()`, `runs(n)` and `seed(s)` sample. The runs and the seed are the program's, if its `@mode` samples, and otherwise 10,000 and 0. `runs` and `seed` replace them.
- Otherwise the program's `@mode` decides. `auto`, the default, enumerates for now ([language overview](language-overview.md)).

`today` is needed only when the program uses `today`. As now, the engine never reads a clock: the host gives the date. `Date::today_utc()` is for hosts that want the current date, as the command line does.

`run` collects what the program prints into `Outcome::printed`. `run_with` gives each line to a function as it's printed, as the command line and the playground do.

On `wasm32`, both run on the calling thread, since the playground can't start threads. Elsewhere they run on a thread of their own, with the stack size in `Limits`, as `probl run` does now.

### Limits

Today there are two structs, the engine's `Limits` and the data's `InputLimits`, and the command line copies three fields from one into the other. The library has one `Limits`, with both. `Limits::default()` is the command line's, and `Limits::browser()` the playground's ([playground plan](playground-plan.md#limits)). Its fields are public, so a host can change one:

```rust
let mut limits = Limits::default();
limits.max_worlds = 10_000;
```

### Data

A program that uses `read` needs its data loaded first. `program.load(&mut files, &options)` reads it with the limits and the cancellation in `options`, and `options.data(data)` gives it to a run. Loading is a step of its own so that a run never reads files, and so that the same data can be used for several runs, with different seeds for example. Running a program that reads data without giving it its data is an error. The library retains the engine's input-manifest and limit checks when a `Data` value is supplied to another program or used with different options.

`Files` is today's `Resolver` under the library's name: `resolve` turns a path, as the program writes it, into an identity, and `open` reads what an identity stands for ([reading data](data-input.md#where-data-comes-from)). The library has three:

- `MemoryFiles`: files given as bytes, by path. It's the playground's.
- `LocalFiles`: the command line's rules for local files, moved from `probl-cli`. Not on `wasm32`.
- `Snapshots<F>`: wraps another `Files`, and keeps the bytes of everything it has opened, so that reading it again gives the same data; `clear` forgets them. The REPL keeps its data this way until `:reload`, across a session whose program changes with every input. Today that's the engine's `Snapshots`, passed to `data::load` separately.

### Outcomes and reports

`text()` is exactly what `probl run` prints: the summary line and the reports. `render(range)` gives some of the reports, aligned as `text` aligns them: the REPL prints only the reports that the newest input added.

`reports()` gives the program's reports in order, and `report(label)` finds one by its label. A report without `as` is labelled with its expression, so `report win` is `"win"`. A report has a group for each key of its `by`, or a single group without one. Each group gives:

- `probability()`, for a fact: its probability result, including unresolved bounds and sampling uncertainty;
- `distribution()`, when a finite outcome table is available: each formatted outcome with its probability result, in presentation order. `None` means a finite table is unavailable, as for an analytic continuous report; it must not be represented as an empty distribution;
- `numeric()`, for supported real numeric reports: means, standard deviations, and quantiles. These include continuous reports and use the same calculations as the CLI.

An `Estimate` describes one quantity, such as a probability or a mean. Its `point()` is available for a complete population: an enumerated value up to numerical rounding, or a sampled estimate. It is not a claim that sampling has no error. For incomplete populations, `point()` is absent, `bounds()` gives unresolved bounds where the engine can establish them, and `resolved_point()` may give an explicitly conditional result over resolved mass. Bounds are not invented for statistics the engine cannot bound. This prevents a normalized resolved-only result from being presented as the answer for the whole population.

Sampling uncertainty is separate from unresolved bounds. `sampling()` is absent for enumeration. Otherwise it records a standard error where available, an optional confidence interval with its method and level, the contributing and effective run counts for that report group, and a status:

- `Estimated`: a sampling error estimate is available.
- `NotEstimable`: the observed runs cannot establish a useful error estimate, including the CLI's zero empirical error cases without integration.
- `NotComputed`: the engine does not currently compute sampling uncertainty for this statistic. It does not imply zero error.
- `IntegratedZero`: the engine identifies zero empirical Monte Carlo error from integrated outcomes. It is not a general guarantee that all symbolic or integrated computations have zero sampling error.

A confidence interval can still be available when the empirical standard error is not useful: for example, the CLI's 95% Wilson interval after zero observed successes. Preserve the current rules for when that interval applies; do not substitute it for weighted or integrated report uncertainty. Any resolved-only estimate's sampling information must be labelled as such and must not be confused with bounds for the complete population.

Formatted outcome and group labels are display strings in the first release. They are not a serialization format or a promise of exact typed identity, and distinct outcomes must not be merged merely because their labels coincide. A later typed-value API can be added without exposing the engine's `Value`.

Real numeric summaries are included in the first release. `mean()` and `sd()` return available summaries; `quantile(q)` checks a finite `q` in `[0, 1]` and uses the existing report convention. Unsupported or unrepresentable results are reported explicitly, rather than silently returning zero or an empty table. These accessors expose the renderer's numerical precision; they do not make `f64` an exact representation of arbitrary integers. Typed dates, complex summaries, and density-query methods can follow separately. No caller should need to parse `text()` to obtain the supported numeric summaries.

`stats()` has what `--stats` prints: the peak number of worlds, world-steps, calls and reused calls, solved loops and their states, solved recursive calls and their rounds, and, for each variable updated exactly, its draws delayed, observations and draws. `sampling()` has the runs, the seed and the effective number of runs. `data()` has, for each source, its identity, size and SHA-256, so that results can be traced to their data.

### Evidence and unresolved weight

`evidence()` is absent when the program did not observe or score anything. Otherwise `EvidenceKind` distinguishes probability evidence from evidence involving a continuous density. A density can exceed one and must not be presented as a percentage or clamped to `[0, 1]`.

The primary evidence representation is its natural logarithm, computed directly from the engine's extended-range weight. A tiny positive likelihood must not first underflow to zero through an `f64` conversion. A complete evidence result exposes `log_value()`; incomplete evidence exposes valid log bounds where available instead of silently returning only its resolved contribution. Impossible evidence retains the engine's existing error behavior. Logarithmic bounds can use negative infinity to represent zero.

For sampling, evidence carries the existing relative standard error and its availability status. This is a relative error on the evidence estimate, not an absolute error in log units; any displayed log-scale error approximation must be identified accordingly. It must also retain the distinction between a known integrated zero and an error that cannot be estimated.

`unresolved()` describes the engine's unnormalized unfinished-weight bound. Its logarithmic accessor preserves very small weights, and `is_zero()` distinguishes zero from underflow. It is not a posterior probability. The shared report-calculation layer accounts for report denominators and local missing mass when constructing each result's bounds; callers must not have to reconstruct them from a single global scalar.

### Errors and diagnostics

One `Error` type covers compiling, loading data and running, so that `?` works throughout. Its kinds are those of the engine, and `Compile`:

| Kind | What happened |
|---|---|
| `Compile` | The program didn't compile; `diagnostics()` has every error and warning |
| `Language` | The program did something the language doesn't allow |
| `Unsupported` | The program uses a feature that isn't built yet |
| `Limit` | The run reached a limit, or was cancelled |
| `Internal` | A bug in Probl (the command line exits with 70) |

`render(color)` gives what the command line prints now. A `Diagnostic` keeps its source, so it can render itself. Its `span()` counts bytes; the playground converts it to UTF-16 itself, as now.

## An example

```rust
use probl::{Options, compile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string("examples/02_craps.probl")?;
    let craps = compile("02_craps.probl", &source)?;

    let exact = craps.run(&Options::new())?;
    let win = exact.report("win")
        .and_then(|r| r.groups().first())
        .and_then(|g| g.probability());
    if let Some(estimate) = win {
        // For this complete enumeration: Some(244.0 / 495.0), up to rounding.
        println!("win: {:?}", estimate.point());
        // A model with unresolved outcomes would expose bounds instead.
    }

    let sampled = craps.run(&Options::new().runs(100_000).seed(1))?;
    // The same report, with sampling uncertainty and its availability status.
    print!("{}", sampled.text());
    Ok(())
}
```

## What moves

| Today | In the library |
|---|---|
| `ModeChoice` (command line), `mode` (playground) | `Options::enumerate`, `sample`, `runs`, `seed` |
| `input_limits` (both), `limits` (playground) | `Limits`, `Limits::browser` |
| `LocalFiles` (`probl-cli`'s library), `Files` (playground) | `LocalFiles`, `MemoryFiles` |
| `data::load` and `Snapshots` | `Program::load`, `Snapshots<F>` |
| `probl_engine::run` and `run_on_this_thread` | `Program::run`, `run_with` |
| `execution_date` (command line) | `Date::today_utc` |
| `RuntimeError::to_diagnostic`, `render_all` | `Error::render`, `Diagnostic::render` |
| `report::render` (REPL) | `Outcome::render` |

`probl-cli`'s library keeps only `parse_size`.

## What stays on the internal crates

- `probl ir`: the IR printer (`probl_sema::pretty`) and liveness.
- `probl schema`: `data::suggest`. It could move to the library later, if embedders need it.
- The REPL's test for a bare expression, which parses with `probl_syntax`.
- The playground's editor features: the names in a program (`compile_with_symbols`) and the reference (`probl_sema::docs`).

The engine switches `--no-merge`, `--no-memo` and `--no-solve` remain implemented by the internal engine, but the CLI configures them through `probl::__internal`. The adapter uses library-owned controls and does not re-export engine types. This allows the ordinary run path to share the library's loading, execution, and reporting logic.

The module is marked `#[doc(hidden)]` and explicitly documented in source as outside the supported API, with no compatibility promise between releases. The attribute only affects [documentation visibility](https://doc.rust-lang.org/rustdoc/write-documentation/the-doc-attribute.html#hidden); it does not make a public method private. The CLI's exact version pin and workspace integration tests make this controlled coupling explicit.

## Building it

1. **Extract shared report calculations**, including point estimates, unresolved bounds, confidence intervals, error availability, numeric summaries, and evidence. The current `Sink` and `Acc` expose some of these, but their resolved-only accessors (`chance`, `distribution`) cannot become unqualified public answers. `report::render` and the library must consume the same computed results and denominators; rendering only formats them.
2. **Add `crates/probl`**, with the API above, over the internal crates. Implement evidence conversion in log space and the explicitly unstable adapter for engine-debugging controls. Add public-only embedding examples before migrating hosts.
3. **Move `LocalFiles`** into the library, with its tests.
4. **Build the playground on it.** `probl-wasm` keeps its JSON and its exports, and its existing output tests remain unchanged.
5. **Build the command line on it**: `probl run`, `probl check` and the REPL, including the engine-debugging switches through the unstable adapter.
6. **Prepare the package graph for publication**, with exact internal version requirements, package metadata, and descriptions distinguishing supported and internal APIs.

## Tests

- **Nothing printed changes.** The examples' golden tests (`crates/probl-cli/tests/examples.rs`), the command line's other tests, `probl-wasm`'s API tests and the playground's (`web/test/examples.mjs`, which checks that the WebAssembly build prints what `probl run` prints) pass unchanged.
- **The reports agree with the text.** For every example, the structured probabilities, bounds, numeric summaries, and confidence intervals agree with the renderer at its printed precision. Also test the unrounded values against independent known answers; formatted agreement alone can hide a shared bug.
- **Uncertainty stays explicit.** Incomplete distributions and world cutoffs expose bounds without an unqualified point. Test per-group denominators and local missing mass. Sampling tests distinguish an unavailable error estimate from integrated zero and retain the existing Wilson interval eligibility rules. Numeric statistics whose errors are not computed report that status.
- **Evidence retains its scale and meaning.** Test a tiny positive likelihood whose ordinary `f64` value would underflow, density evidence above one, incomplete evidence, and sampled evidence with known or unavailable relative error. No probability clamping or normalization may change the quantity exposed.
- **Known answers.** Enumerating craps, `win` is 244/495. Means and quantiles agree with analytic numeric examples, including continuous reports without finite tables. Sampling calibration uses repeated seeds and the existing statistical test strategy, rather than requiring every random estimate to fall within a fixed number of standard errors.
- **Data remains reusable and checked.** Load once and run with different seeds; reject incompatible program inputs and preserve the engine's limit checks. Running with loaded data must not reopen files.
- **The API is the API.** The examples in `crates/probl/examples/` and the doc tests use only the supported `probl` API. Run `cargo semver-checks` against published baselines to detect covered source-compatibility changes, alongside behavioral tests. The `__internal` module is explicitly excluded from the compatibility promise, and the CLI's use of it is exercised by integration tests.

## Publishing

Publish the real library and CLI packages under the same project ownership. Package names are allocated separately from executable names; claiming `probl` does not claim `probl-cli`. Both packages should identify the same repository and clearly explain which one to install. See [Cargo's publishing guidance](https://doc.rust-lang.org/cargo/reference/publishing.html) and [target naming](https://doc.rust-lang.org/cargo/reference/cargo-targets.html).

The publishable dependency order is `probl-number`, `probl-syntax`, `probl-sema`, `probl-engine`, `probl`, then `probl-cli`. Keep one release version across them initially. `probl-bench`, `probl-oracle` and `probl-wasm` retain `publish = false`.

The workspace currently uses path-only internal dependencies. Add exact registry versions alongside those paths for every dependency on an internal crate, and for the CLI's dependency on `probl`. For example, using the current workspace version:

```toml
[workspace.dependencies]
probl-engine = { path = "crates/probl-engine", version = "=0.0.1" }
probl = { path = "crates/probl", version = "=0.0.1" }
```

Apply the same rule throughout the internal graph and update the pins with each release. The paths serve workspace development; published packages resolve their declared registry versions. This prevents an existing `probl` release from picking up a later, incompatible internal API. Embedders can use normal version requirements for the supported `probl` API. [Cargo dependency rules](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)

Add license information and appropriate repository, description, and README metadata to the published packages. Use a release toolchain supporting workspace publication; the locally checked Cargo 1.98.1 supports `cargo publish --workspace`. Its availability is separate from the library's minimum supported Rust version. Run `cargo publish --workspace --dry-run` and inspect the packaged contents before uploading, then verify that installation from the published CLI package produces `probl` and that an external project can depend only on `probl`.

## Initial scope and deferred additions

Separate data loading and the explicitly unstable engine-debugging adapter are decided. Formatted outcome labels are sufficient initially, together with structured numeric summaries and complete uncertainty metadata.

Typed outcome values, typed non-real summaries, density-query methods, and a public editor/tooling API are deferred. They can be added through new methods without exposing the engine's internal types. Before publication, finalize the shared result extraction against the tests above; a thin wrapper around today's scalar `Acc` accessors would not satisfy this proposal.
