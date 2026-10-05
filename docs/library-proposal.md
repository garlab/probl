# Proposal: Probl as a Rust library

> October 2026. Proposed, not built. This adds a crate named `probl`: a small, stable API for compiling and running Probl programs from Rust, which the command line and the [playground](playground-plan.md) would both be built on. The crate names and the shape of the API are settled below. The three [open questions](#open-questions) need answers before it's built.

## The problem

Three things point the same way.

- **The name.** The command line's crate is `probl-cli`, which installs the `probl` binary, and `probl` itself is free on crates.io. Projects that are languages or engines give the short name to the library and `-cli` to the tool: `typst` and `typst-cli`, `wasmtime` and `wasmtime-cli`. Projects that are only tools give it to the binary: `ripgrep`, `just`, `mdbook`. Probl is a language whose engine already runs in more than one host, so it's the first kind. Homebrew doesn't depend on crate names: its formula builds `crates/probl-cli` from the release tarball, and is called `probl` either way.
- **Two hosts repeat the same steps.** The command line (`crates/probl-cli/src/main.rs`) and the playground (`crates/probl-wasm/src/lib.rs`) each compile a program, read its data, choose the mode and run it, with their own copies of:
  - the rule that `--runs` or `--seed` imply sampling, and the defaults when the program's `@mode` doesn't sample (`ModeChoice::resolve`, and the playground's `mode`);
  - the data's limits, derived from the engine's (`input_limits`, in both);
  - a resolver for the data (`LocalFiles`, and the playground's `Files`);
  - the choice between `probl_engine::run` and `run_on_this_thread`.
- **There's no API to promise.** A Rust program that runs Probl today needs `probl-syntax`, `probl-sema` and `probl-engine`, and their internal types: `ir::Program`, the engine's `Options` with its switches for checking the engine, `Value`, `Weight`, `Sink`. Those change with every feature, as they should.

## Decisions

1. **`probl` is the library, and `probl-cli` the tool.** `cargo install probl-cli` installs `probl`, and the README says so. `cargo install probl` says the crate has no binaries.
2. **The library defines its own types, and re-exports nothing from the other crates.** `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine` are still published, since `probl` depends on them, but their descriptions say they're internal, with no promise of stability. They can change at every release without breaking a program that uses `probl`.
3. **Every public struct and enum is `#[non_exhaustive]`**, made with methods or `Default`, so that an option or a field can be added in a minor release.
4. **The workspace's own tools may still use the internal crates**, for what an embedder doesn't need: editor features, the IR printer, schema suggestions (see [what stays](#what-stays-on-the-internal-crates)). They're released together, so nothing breaks.

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
    pub fn evidence(&self) -> Option<f64>;           // the probability of what was observed
    pub fn unresolved(&self) -> f64;                 // weight cut off by limits or epsilon
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
    pub fn probability(&self) -> Option<Estimate>;   // a fact: the chance it's true
    pub fn distribution(&self) -> &[(String, Estimate)];  // a value: each one, as Probl prints it
}
#[derive(Clone, Copy, Debug)]
pub struct Estimate { pub p: f64, pub se: Option<f64> }   // the standard error when sampling

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

Every setting of `Options` is optional. The mode follows the command line's rules, which are now in one place:

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

A program that uses `read` needs its data loaded first. `program.load(&mut files, &options)` reads it with the limits and the cancellation in `options`, and `options.data(data)` gives it to a run. Loading is a step of its own so that a run never reads files, and so that the same data can be used for several runs, with different seeds for example. Running a program that reads data without giving it its data is an error.

`Files` is today's `Resolver` under the library's name: `resolve` turns a path, as the program writes it, into an identity, and `open` reads what an identity stands for ([reading data](data-input.md#where-data-comes-from)). The library has three:

- `MemoryFiles`: files given as bytes, by path. It's the playground's.
- `LocalFiles`: the command line's rules for local files, moved from `probl-cli`. Not on `wasm32`.
- `Snapshots<F>`: wraps another `Files`, and keeps the bytes of everything it has opened, so that reading it again gives the same data; `clear` forgets them. The REPL keeps its data this way until `:reload`, across a session whose program changes with every input. Today that's the engine's `Snapshots`, passed to `data::load` separately.

### Outcomes and reports

`text()` is exactly what `probl run` prints: the summary line and the reports. `render(range)` gives some of the reports, aligned as `text` aligns them: the REPL prints only the reports that the newest input added.

`reports()` gives the program's reports in order, and `report(label)` finds one by its label. A report without `as` is labelled with its expression, so `report win` is `"win"`. A report has a group for each key of its `by`, or a single group without one. Each group gives:

- `probability()`, for a fact: the chance that it's true;
- `distribution()`, for any other value: each value, as Probl prints it, with its chance, in the order `text` prints them.

An `Estimate` is a probability, with its standard error when sampling. When enumerating, `se` is `None`: the probability is exact, give or take `unresolved()`.

For now, the summaries of numbers (means, quantiles, densities of continuous reports) are only in `text()`, and values are text ([question 1](#open-questions)). Both can be added without breaking anything.

`stats()` has what `--stats` prints: the peak number of worlds, world-steps, calls and reused calls, solved loops and their states, solved recursive calls and their rounds, and, for each variable updated exactly, its draws delayed, observations and draws. `sampling()` has the runs, the seed and the effective number of runs. `data()` has, for each source, its identity, size and SHA-256, so that results can be traced to their data.

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
    let win = exact.report("win").and_then(|r| r.groups()[0].probability());
    // Some(Estimate { p: 0.4929…, se: None }): 244/495

    let sampled = craps.run(&Options::new().runs(100_000).seed(1))?;
    // the same report, now with a standard error
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
- The switches for checking the engine, `--no-merge`, `--no-memo` and `--no-solve`, unless [question 3](#open-questions) is answered the other way.

## Building it

1. **Add `crates/probl`**, with the API above, over the internal crates. The reports come from the engine's `Sink` and `Acc`, which already give chances and their standard errors (`chance`, `chance_se`, `value_se`). The library's numbers must be the ones `text` prints, with the same denominators. So that they can't drift apart, `report::render` and the library share one function that computes a report's numbers, and `render` only formats them.
2. **Move `LocalFiles`** into the library, with its tests.
3. **Build the playground on it.** `probl-wasm` keeps its JSON and its exports, and its tests don't change.
4. **Build the command line on it**: `probl run`, `probl check` and the REPL.
5. **Say the internal crates are internal**, in their descriptions.

## Tests

- **Nothing printed changes.** The examples' golden tests (`crates/probl-cli/tests/examples.rs`), the command line's other tests, `probl-wasm`'s API tests and the playground's (`web/test/examples.mjs`, which checks that the WebAssembly build prints what `probl run` prints) pass unchanged.
- **The reports agree with the text.** For every example, each report's probabilities and distributions are those `text()` prints for it, to the printed precision.
- **Known answers.** Enumerating craps, `win` is 244/495. Sampling, each estimate is within four standard errors of the exact answer, for the examples that can be enumerated.
- **The API is the API.** The examples in `crates/probl/examples/` and the doc tests use only `probl`. Once it's published, continuous integration runs `cargo semver-checks`, so that a breaking change can't ship in a minor release.

## Publishing

In dependency order: `probl-number`, `probl-syntax`, `probl-sema`, `probl-engine`, `probl`, then `probl-cli`. `cargo publish --workspace` publishes them in that order. They share one version and are released together. `probl-bench`, `probl-oracle` and `probl-wasm` aren't published.

## Open questions

1. **Report values as text, for now?** Recommended. `distribution()` gives each value as Probl prints it (`"7"`, `"[1, 2]"`), so the engine's `Value` stays private. Embedders can't compute with outcomes, only read them. Typed values can be added later, as another method, without breaking anything. Removing them would break programs.
2. **Data loaded as a step of its own?** Recommended, as above: it's explicit, and running several times with the same data costs nothing more. The alternative, giving `Files` to `run`, is one call fewer, but reads the data again for every run.
3. **The switches for checking the engine.** Recommended: a `#[doc(hidden)]` method on `Options`, so that `probl run` is built entirely on the library. The alternative is that `probl run` uses the internal crates for them, and the library has no trace of them.
