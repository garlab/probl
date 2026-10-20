# Browser playground

The [playground](https://playground.probl.dev) runs Probl locally in the browser through WebAssembly. It shares the Rust library and engine with the CLI. It provides an editor, output, a runnable language guide and a searchable built-in reference.

## Develop locally

Install Rust, [Bun](https://bun.sh), and Rust's WebAssembly target. From the repository root:

```sh
rustup target add wasm32-unknown-unknown
cd web
bun install --frozen-lockfile
bun run serve
```

Open `http://localhost:8000`. The development server rebuilds when page sources, the language overview, Rust crates or bundled examples change. The page reloads after a successful build and preserves editor work. A failed build leaves the previous build available.

`bun run build` generates `web/dist`. The build compiles `probl-wasm` with the workspace's `wasm` profile, bundles JavaScript with Bun and renders `docs/language-overview.md` as the guide. Complete Probl snippets get a button to load them into the editor. Built-in documentation comes from the compiler, rather than a separately maintained JavaScript list.

## Using the page

- Run with Ctrl/⌘-Enter; stop with Escape or the Stop button. Options select mode, run count, seed, what a fault does to the other worlds (`--on-error`), exact conjugate updates and the time limit. A dot on the Options button means some differ from their defaults; a click elsewhere or Escape closes the panel.
- Until the first run, the output pane lists the editor's shortcuts. The status line stays in view over long output and is marked by how the run went: running, done, partial, failed or stopped.
- The examples menu includes games, forecasts, calendars, decisions, monitoring and analytic continuous calculations. Example data files are available to `read` from memory.
- Compile diagnostics appear in the editor; runtime errors point to the failing source. Output uses the CLI's text format, with streamed `print` lines and sampled-run progress. A partial result, where some worlds failed and the others finished, shows the output with its `failed` section, says so in the status line, and marks where the worlds failed in the editor.
- Numeric reports grouped by numeric or date keys also draw a median and percentile bands; deterministic reports draw a line. Boolean reports draw a probability step curve. Hover or use the slider's arrow keys, Home and End to inspect values and uncertainty. “Values and uncertainty” opens an accessible table; the complete text output stays below the charts.
- Open “Report summaries” to choose 10–90% bands, mean/SD or custom ordered columns for each report, then “Apply and run”. Controls appear after compilation, so invalid selections can be corrected before a successful run. “Default” clears an override. [Summary specifications](report-summaries.md) include `mean,sd,median,q:0.1,q:0.9`; `median` and `q:0.5` have distinct tie conventions. The selection updates text, charts and tables together.
- “Overlay (same units)” selects a compatible deterministic report as a dashed baseline. Choose a report measured in the same units; labels alone do not prove that. Only matching typed keys are compared, and missing values break the line. Value formatting can explicitly select USD, EUR or GBP. No currency or baseline is inferred from a label.
- Hover shows declarations or built-in documentation. Ctrl/⌘-click or F12 goes to a definition; F2 selects references for renaming. Completion and signature help use compiler symbols; after `catch`, completion offers the faults it can name, which the reference also lists.
- Share encodes the program, summary selections and chart choices in the URL fragment. Older links use defaults. Running a model does not upload its source or inputs to an execution server; a share link itself contains the source and should be treated accordingly.
- Local edits are saved in browser storage. Loading an example or guide snippet is undoable; editing it makes it the user's saved program.

Summary and chart choices are saved with the exact source. Editing the source resets them so
report IDs from another program cannot silently select an overlay. Percentile
bands describe variation across outcomes at each key, not one run's trajectory
or a Monte Carlo confidence band. The point details separately show sampling
error, Wilson confidence intervals where available, incomplete-result bounds,
support and report reach. Partial failures remain qualified beside the plot.

Custom charts prefer the selected median as the main line, then the mean, then
the quantile nearest 0.5, or SD if it is the only column. Quantiles on opposite
sides of 0.5 are paired from outside inward for bands. The inspector and table
show all requested columns; mean/SD-only reports have no percentile bands.
SD-only curves cannot be used as value baselines.

The page supports light/dark themes, a resizable panel divider, keyboard navigation and a single-column layout on small screens. Text output wraps a long report at its ` · ` separators; a line with one value, such as a table row, stays whole and scrolls instead.

## Architecture

| File or component | Role |
|---|---|
| [`probl-wasm`](../crates/probl-wasm/src/lib.rs) | JSON interface for checking, running, examples and reference documentation |
| [`probl.js`](../web/src/probl.js) | Loader for plain WASM exports and strings in module memory; no wasm-bindgen toolchain |
| [`worker.js`](../web/src/worker.js) | Checking/running away from the page's main thread |
| [`app.js`](../web/src/app.js) | Editor, output, options, guide, reference and worker lifecycle |
| [`chart-data.js`](../web/src/chart-data.js) / [`charts.js`](../web/src/charts.js) | Typed chart eligibility/alignment and SVG rendering, keyboard inspection and tables |
| [`summary-controls.js`](../web/src/summary-controls.js) | Per-report column controls, using IDs from compiler metadata |
| [`build.mjs`](../web/build.mjs) / [`serve.mjs`](../web/serve.mjs) | Static build and rebuilding development server |

Two workers share a compiled module: one checks edits and one runs programs. Each instance runs synchronously on one thread. Stop and the default 30-second timeout terminate the runner and replace it. Crashes also replace the runner, so a failed model does not require reloading the page.

The adapter uses [`probl`](library.md) for loading, execution and reporting, and the compiler's internal APIs for editor symbols. Text crosses the module boundary through plain exported functions. The JavaScript interface is an application adapter, not a separately stabilized package API.

### Limits

[`Limits::browser()`](../crates/probl/src/options.rs) sets lower ceilings than native execution, including 1,000,000 worlds, 2 × 10⁹ work units, 150 nested calls, 20,000 chain states, 1 MiB output and 8 MiB input. The source definition is authoritative for the full list. The page's timeout is additional to engine budgets.

The resolver serves only supplied in-memory files. The page currently supplies bundled example data; it has no arbitrary local-file picker or stdin. Nor does it have environment variables: a WASM request may carry an `env` object, but the page sends none, so a model's `env` is empty. The language has no general network access. Charts consume the WASM adapter's [versioned report summaries](report-json.md); the text output remains available.

Charts use native SVG, with no chart library dependency. They render at most 20
reports with 1,000 groups per report; tables are built only when opened. Unsupported
shapes, categorical/mixed keys, unsafe numeric coordinates and larger series stay
in the text view, without downsampling. Numeric/date keys are ordered by their
typed values, and date axes use UTC calendar-day distances. Date-valued reports
and structured field marginals currently remain in text. An unavailable estimate
is a gap; a missing key is never assigned an invented value. Serialization limits
are separate from these display limits. None of this reduces the engine's memory
for retaining exact sample populations.

Summary controls cover the first 256 reports and allow 1–32 distinct columns
per report. Specifications are capped at 4,096 bytes. Shape, level and report-ID
errors are checked before execution; value-type compatibility is checked when
results are known. Date reports support quantiles and medians, facts keep their
probability output, and categorical/structured reports require the default.

In the October 19, 2026 Bun build at `c7ab8a9`, the chart UI and saved settings added 13,787
bytes to the minified page bundle (430,231 to 444,018 bytes), or 5,065 bytes with
gzip. This compares the same build settings against the preceding `48e6302`
version; the WASM module is unchanged.

## Tests

From the repository root, with dependencies installed:

```sh
cargo test -p probl-wasm
cargo build --release -p probl-cli
cd web
bun run build
bun run test
```

The native Rust tests cover the adapter. `test/charts.mjs` checks chart eligibility, precision, gaps, baseline alignment, custom columns and settings against actual WASM reports. `test/examples.mjs` executes the actual WASM module and compares native/WASM outputs. `test/page.mjs` runs the page in headless Chrome, covering examples, Stop, limits, errors, share links, the guide and editor interactions, plus chart values, overlays, summary controls and error recovery, keyboard access, light/dark themes, narrow layouts and fallback text. Pass `--screenshots` to save chart screenshots under `/tmp`. These check different layers; a native adapter test alone does not test WebAssembly.

The browser test defaults to macOS application paths. Set `PROBL_CHROME` to the Chrome executable on another installation. For Firefox, set `PROBL_FIREFOX` if needed and run `bun test/page.mjs firefox`. The browser must be installed separately.

CI runs both on every push and pull request, on Ubuntu with Chrome: the `playground` job in [ci.yml](../.github/workflows/ci.yml). Firefox runs only locally.

## Hosting and deployment

The build is static and can be served by a host that serves `.wasm` as `application/wasm`. Production is served at [playground.probl.dev](https://playground.probl.dev), on Cloudflare Pages; `probl-playground.pages.dev` serves the same deployment.

The hosted playground is deployed from releases, not from this repository: each release contains a browser bundle with the tested playground build ([browser distribution](releases.md#browser-distribution)), and the hosted playground serves the newest stable release's. A change to `web/` reaches it with the next release. This repository has no deployment script or credentials.

To test an existing deployment:

```sh
PROBL_URL=https://playground.probl.dev bun test/page.mjs
```

User-supplied data files, bounded-memory summaries and parallel browser sampling
remain future work. Categorical populations still use the text view.

The per-report **Moments only (sample mode)** collection choice retains mean/SD
and probability estimates without numeric populations. It is separate from the
**Mean and SD** display preset, which alone keeps exact collection. The choice is
saved in `charts.reports[id].collection` and sent as `report_collections`; share
links and local reloads preserve it. Quantile columns require exact collection.
See [collection policies](report-summaries.md#moments-only-collection).
