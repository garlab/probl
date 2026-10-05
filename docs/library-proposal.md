# Proposal: Probl as a Rust library

> Revised October 5, 2026, and built: `crates/probl` has the API below, and the command line, the [playground](playground-plan.md) and the benchmarks are built on it. [As built](#as-built) records what the implementation settled where this proposal left room. The exact version pins under [Publishing](#publishing) wait for the first release.

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
5. **Load data separately and reuse it across runs.** `Program::load` and `Options::data` are part of the initial API. The library preserves the engine's checks that supplied data matches the program and applicable limits. A mismatch is the caller's mistake, so it's a `Usage` error, not the internal error the engine reports today.
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
pub enum Mode {
    Auto,
    Enumerate,
    #[non_exhaustive] Sample { runs: u64, seed: u64 },
    #[non_exhaustive] Beam { worlds: u64 },             // designed, not built: Unsupported
    #[non_exhaustive] Particles { runs: u64, seed: u64 }, // designed, not built: Unsupported
}

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
pub struct MemoryFiles { /* path → bytes */ }             // insert, contains, FromIterator
pub struct LocalFiles  { /* moved from probl-cli */ }     // new, next_to, stdin, cancel; not on wasm32
pub struct Snapshots<F: Files> { /* F, and the bytes it has given */ }  // the REPL's; clear
#[derive(Clone)] pub struct Data { /* Arc<Inputs> */ }   // sources(): what was read

#[derive(Clone, Default)] pub struct Cancel { /* Arc<AtomicBool> */ }
impl Cancel { pub fn cancel(&self); pub fn is_cancelled(&self) -> bool; }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Date { /* days since 1970-01-01 */ }
impl Date {
    pub fn from_ymd(y: i32, m: u32, d: u32) -> Option<Date>;
    pub fn today_utc() -> Option<Date>;          // not on wasm32, where the host passes it in
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
    pub fn distribution(&self) -> Option<&[(String, Estimate)]>; // finite outcome table, in value order
    pub fn numeric(&self) -> Option<&NumericSummary>; // real numeric summaries, including continuous reports
}

/// One reported quantity, with the same population and denominator as the renderer.
pub struct Estimate { /* … */ }
impl Estimate {
    pub fn point(&self) -> Option<f64>;           // estimate over resolved mass, when defined
    pub fn is_complete(&self) -> bool;           // no unresolved mass affects this quantity
    pub fn bounds(&self) -> Option<Interval>;     // bounds due to unresolved mass, when available
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
    pub fn quantile(&self, q: f64) -> Result<Estimate, SummaryError>;
}
#[non_exhaustive]
pub enum SummaryError { InvalidQuantile, Unavailable }  // q not finite in [0, 1]; no such summary

#[non_exhaustive]
pub enum EvidenceKind { Probability, Density }
pub struct Evidence { /* … */ }
impl Evidence {
    pub fn kind(&self) -> EvidenceKind;
    pub fn log_value(&self) -> Option<f64>;        // natural log of resolved evidence contribution, when defined
    pub fn is_complete(&self) -> bool;            // no unresolved contribution to the evidence
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
pub enum ErrorKind { Compile, Usage, Language, Unsupported, Limit, Internal }

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

A program that uses `read` needs its data loaded first. `program.load(&mut files, &options)` reads it with the limits and the cancellation in `options`, and `options.data(data)` gives it to a run. Loading is a step of its own so that a run never reads files, and so that the same data can be used for several runs, with different seeds for example. Running a program that reads data without its data, or with data loaded for another program, is a `Usage` error. The engine already checks both, and checks that the loaded strings and integers fit the run's limits, which can differ from those the data was loaded with. Today it reports data loaded for another program as an internal error, a bug in Probl; the library reports it as the caller's mistake.

`Files` is today's `Resolver` under the library's name: `resolve` turns a path, as the program writes it, into an identity, and `open` reads what an identity stands for ([reading data](data-input.md#where-data-comes-from)). The library has three:

- `MemoryFiles`: files given as bytes, by path. It's the playground's.
- `LocalFiles`: the command line's rules for local files, moved from `probl-cli`. Not on `wasm32`.
- `Snapshots<F>`: wraps another `Files`, and keeps the bytes of everything it has opened, so that reading it again gives the same data; `clear` forgets them. The REPL keeps its data this way until `:reload`, across a session whose program changes with every input. Today that's the engine's `Snapshots`, passed to `data::load` separately.

### Outcomes and reports

`text()` is exactly what `probl run` prints: the summary line and the reports. `render(range)` gives some of the reports, aligned as `text` aligns them: the REPL prints only the reports that the newest input added.

`reports()` gives the program's reports in order, and `report(label)` finds one by its label. A report without `as` is labelled with its expression, so `report win` is `"win"`. A report has a group for each key of its `by`, or a single group without one. Each group gives:

- `probability()`, for a fact: its probability result, including unresolved bounds and sampling uncertainty;
- `distribution()`, when a finite outcome table is available: each formatted outcome with its probability result, in the order of the values, so that numbers and dates come in numeric order (labels alone couldn't be sorted). `None` means a finite table is unavailable, as for an analytic continuous report; it must not be represented as an empty distribution;
- `numeric()`, for supported real numeric reports: means, standard deviations, and quantiles. These include continuous reports and use the same calculations as the CLI.

An `Estimate` describes one quantity, such as a probability or a mean. Its `point()` gives the estimate over the resolved population whenever that quantity is defined, including when some mass remains unresolved. For example, the craps program's tiny unresolved tail must not make its useful win-probability estimate disappear. `None` means no point can be computed, such as when no mass has resolved; it does not mean merely that the result is incomplete.

`is_complete()` states whether unresolved mass affects this quantity. When it is false, the point is explicitly conditional on resolved mass, and `bounds()` gives bounds for the full-population quantity where the engine can establish them. There is no epsilon threshold that silently changes the meaning of `point()` or marks a small positive tail as complete.

For complete enumeration, the point is the model's result up to numerical rounding. For sampling, it is a sampled estimate even when `is_complete()` is true: completion refers to accounted-for execution mass, not to visiting every possible outcome or eliminating Monte Carlo error. Callers receive a result object with its completion and uncertainty metadata, not a promise that an available point is exact.

Bounds are not invented for statistics the engine cannot bound. A tiny unresolved share relative to a report's resolved mass gives tight probability bounds, but it does not by itself bound the error of a mean over unbounded values. In that case the mean's point can be available with `is_complete() == false` and no finite bounds. Applications choose their own accuracy requirements from the quantity's bounds and sampling information, rather than from the global unresolved weight alone.

Sampling uncertainty is separate from unresolved bounds. `sampling()` is absent for enumeration. Otherwise it records a standard error where available, an optional confidence interval with its method and level, the contributing and effective run counts for that report group, and a status:

- `Estimated`: a sampling error estimate is available.
- `NotEstimable`: the observed runs cannot establish a useful error estimate, including the CLI's zero empirical error cases without integration.
- `NotComputed`: the engine does not currently compute sampling uncertainty for this statistic. It does not imply zero error.
- `IntegratedZero`: the engine identifies zero empirical Monte Carlo error from integrated outcomes. It is not a general guarantee that all symbolic or integrated computations have zero sampling error.

A confidence interval can still be available when the empirical standard error is not useful. Today the renderer gives a 95% Wilson interval when every contributing run agreed (the estimate is 0% or 100%) or fewer than 30 runs contributed, provided the report isn't weighted and each run contributed one ordinary observation. Preserve these rules; do not substitute a Wilson interval for weighted or integrated report uncertainty. When `is_complete()` is false, the sampling information describes the estimate over resolved mass, and is separate from the bounds for the full population.

Formatted outcome and group labels are display strings in the first release. They are not a serialization format or a promise of exact typed identity, and distinct outcomes must not be merged merely because their labels coincide. A later typed-value API can be added without exposing the engine's `Value`.

Real numeric summaries are included in the first release. `mean()` and `sd()` return available summaries. `quantile(q)` uses the existing report convention; a `q` that isn't a finite number in `[0, 1]` gives `SummaryError::InvalidQuantile`, and a report without quantiles gives `SummaryError::Unavailable`. These are the caller's to handle, so they don't use the program's `Error`, whose diagnostics point into the source. Unsupported or unrepresentable results are reported explicitly, rather than silently returning zero or an empty table. These accessors expose the renderer's numerical precision; they do not make `f64` an exact representation of arbitrary integers. Typed dates, complex summaries, and density-query methods can follow separately. No caller should need to parse `text()` to obtain the supported numeric summaries.

`stats()` has what `--stats` prints: the peak number of worlds, world-steps, calls and reused calls, solved loops and their states, solved recursive calls and their rounds, and, for each variable updated exactly, its draws delayed, observations and draws. `sampling()` has the runs, the seed and the effective number of runs. `data()` has, for each source, its identity, size and SHA-256, so that results can be traced to their data.

### Evidence and unresolved weight

`evidence()` is absent when the program did not observe or score anything. Otherwise `EvidenceKind` distinguishes probability evidence from evidence involving a continuous density. A density can exceed one and must not be presented as a percentage or clamped to `[0, 1]`.

The primary evidence representation is its natural logarithm, computed directly from the engine's extended-range weight. A tiny positive likelihood must not first underflow to zero through an `f64` conversion. `log_value()` remains available when a resolved contribution can be computed, and `is_complete()` distinguishes complete evidence from a partial contribution. Evidence is not renormalized over resolved worlds: this is the log of the accumulated evidence contribution, unlike a report point conditioned on resolved mass. Incomplete evidence also exposes valid log bounds where available. Impossible evidence retains the engine's existing error behavior. A zero resolved contribution or logarithmic lower bound is represented by negative infinity.

For sampling, evidence carries the existing relative standard error and its availability status. This is a relative error on the evidence estimate, not an absolute error in log units; any displayed log-scale error approximation must be identified accordingly. It must also retain the distinction between a known integrated zero and an error that cannot be estimated.

`unresolved()` describes the engine's unnormalized unfinished-weight bound. Its logarithmic accessor preserves very small weights, and `is_zero()` distinguishes zero from underflow. It is not a posterior probability. The shared report-calculation layer accounts for report denominators and local missing mass when constructing each result's bounds; callers must not have to reconstruct them from a single global scalar.

### Errors and diagnostics

One `Error` type covers compiling, loading data and running, so that `?` works throughout. Its kinds are those of the engine, with `Compile` and `Usage`:

| Kind | What happened |
|---|---|
| `Compile` | The program didn't compile; `diagnostics()` has every error and warning |
| `Usage` | The host misused the library: it ran a program that reads data without its data, or with data loaded for another program |
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

    let enumerated = craps.run(&Options::new())?;
    let win = enumerated.report("win")
        .and_then(|r| r.groups().first())
        .and_then(|g| g.probability());
    if let Some(estimate) = win {
        // Close to 244.0 / 495.0; this example retains a tiny unresolved tail.
        println!("win: {:?}", estimate.point());
        println!("complete: {}", estimate.is_complete()); // false
        if let Some(bounds) = estimate.bounds() {
            println!("win bounds: {}..{}", bounds.lower(), bounds.upper());
        }
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
- **Uncertainty stays explicit.** Incomplete distributions and world cutoffs retain defined point estimates, mark them incomplete, and expose full-population bounds where available. Test tiny positive tails as well as large missing mass, per-group denominators, and local missing mass. With no resolved mass, the point is absent; with a complete population it agrees with the ordinary result. Tiny tails never become complete through a display threshold, and an unbounded mean does not gain an unsupported error bound. Sampling tests distinguish an unavailable error estimate from integrated zero and retain the existing Wilson interval eligibility rules. Numeric statistics whose errors are not computed report that status. `quantile` rejects a `q` outside `[0, 1]`, or not finite, with `InvalidQuantile`.
- **Evidence retains its scale and meaning.** Test a tiny positive likelihood whose ordinary `f64` value would underflow, density evidence above one, incomplete evidence, and sampled evidence with known or unavailable relative error. No probability clamping or normalization may change the quantity exposed.
- **Known answers.** The analytic craps win probability is 244/495. The shipped example also tracks an unbounded roll count and stops with a tiny unresolved tail: its point is available, `is_complete()` is false, and its bounds contain the analytic answer within numerical tolerance. Means and quantiles agree with analytic numeric examples, including continuous reports without finite tables. Sampling calibration uses repeated seeds and the existing statistical test strategy, rather than requiring every random estimate to fall within a fixed number of standard errors.
- **Data remains reusable and checked.** Load once and run with different seeds. Running without the data, or with data loaded for another program, is a `Usage` error, never `Internal`; the engine's limit checks on loaded strings and integers still apply. Running with loaded data must not reopen files.
- **The API is the API.** The examples in `crates/probl/examples/` and the doc tests use only the supported `probl` API. Run `cargo semver-checks` against published baselines to detect covered source-compatibility changes, alongside behavioral tests. The `__internal` module is explicitly excluded from the compatibility promise, and the CLI's use of it is exercised by integration tests.

## Publishing

Publish the real library and CLI packages under the same project ownership. Package names are allocated separately from executable names; claiming `probl` does not claim `probl-cli`. Both packages should identify the same repository and clearly explain which one to install. See [Cargo's publishing guidance](https://doc.rust-lang.org/cargo/reference/publishing.html) and [target naming](https://doc.rust-lang.org/cargo/reference/cargo-targets.html).

The publishable dependency order is `probl-number`, `probl-syntax`, `probl-sema`, `probl-engine`, `probl`, then `probl-cli`. Keep one release version across them initially. `probl-bench`, `probl-oracle` and `probl-wasm` retain `publish = false`.

Every dependency on an internal crate, and the CLI's dependency on `probl`, needs an exact registry version alongside its path. For example, at the current workspace version:

```toml
[workspace.dependencies]
probl-engine = { path = "crates/probl-engine", version = "=0.0.1" }
probl = { path = "crates/probl", version = "=0.0.1" }
```

Apply the same rule throughout the internal graph and update the pins with each release. The paths serve workspace development; published packages resolve their declared registry versions. This prevents an existing `probl` release from picking up a later, incompatible internal API. Embedders can use normal version requirements for the supported `probl` API. [Cargo dependency rules](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)

Each published package needs license, repository, description and README metadata. `cargo publish --workspace` needs Cargo 1.90 or later: that's a requirement on the release toolchain, separate from the crates' minimum supported Rust version (`rust-version`, 1.85). Run `cargo publish --workspace --dry-run` and inspect the packaged contents before uploading, then verify that installation from the published CLI package produces `probl` and that an external project can depend only on `probl`.

## Initial scope and deferred additions

Separate data loading and the explicitly unstable engine-debugging adapter are decided. Formatted outcome labels are sufficient initially, together with structured numeric summaries and complete uncertainty metadata.

Typed outcome values, typed non-real summaries, density-query methods, and a public editor/tooling API are deferred. They can be added through new methods without exposing the engine's internal types. Before publication, finalize the shared result extraction against the tests above; a thin wrapper around today's scalar `Acc` accessors would not satisfy this proposal.

## As built

Built October 5, 2026. Where the proposal left room, the implementation settled it this way:

- **One calculation, two uses.** `probl_engine::report::results` computes every report's numbers, and the renderer only formats them (`report::render_results`). The engine's `Outcome` keeps the results, and the library wraps them. Every example still prints exactly what it printed before, and a test checks that each example's structured probabilities and means are the ones its text shows.
- **Bounds.** Unresolved bounds are given when enumerating, for facts and for each value of a table: as if the unresolved weight, and the group's missing mass, had all gone to that value or none of it. When sampling, an estimate over runs cut short isn't complete, but has no bounds: the plug-in bounds would themselves be estimates. Means, standard deviations and quantiles have no bounds.
- **Statuses.** A fact's sampled standard error of zero is `IntegratedZero` when its runs integrated outcomes (the renderer's "zero empirical MC error"), and `NotEstimable` otherwise. A value's or a mean's zero is `NotEstimable`, as the renderer prints it; standard deviations and quantiles are `NotComputed`. The sampled evidence's relative standard error is `Estimated` whenever it's finite, zero included (every run had the same weight), and `NotEstimable` with a single run.
- **Unresolved weight**, when sampling, is per run, so that it compares with enumeration's.
- **Facts** have a `probability()` and no `distribution()`: a table of `true` and `false` would only say it again.
- **`Date::today_utc()`** gives `None` when the clock is outside the supported dates, rather than an error with no place in a program.
- **`Mode`** has `Beam` and `Particles`, which programs can already ask for, and which runs refuse as `Unsupported`.
- **`LocalFiles`** reads standard input only when `.stdin(true)` allows it, and stops waiting for it when `.cancel(&cancel)` is cancelled. The command line allows it for `run`, `check` and `schema`, and the REPL doesn't.
- **The playground** keeps its own wording for files it doesn't have, with a thin `Files` around `MemoryFiles`, and its `run` takes the progress function as `probl_wasm::Progress`.
- **`__internal`** has `EngineChecks` (`merge`, `memoize`, `solve`) and `engine_checks(options, checks)`, used by `probl run`'s hidden switches and by the benchmarks.
- **Not yet in the API:** a report's reach ("reached in 40% of worlds"), which the renderer prints and the shared results already compute, and typed values. Both can be added as methods.
