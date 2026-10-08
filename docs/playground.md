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

- Run with Ctrl/⌘-Enter; stop with Escape or the Stop button. Options select mode, run count, seed, what a fault does to the other worlds (`--on-error`), exact conjugate updates and the time limit.
- The examples menu includes games, forecasts, calendars, decisions, monitoring and analytic continuous calculations. Example data files are available to `read` from memory.
- Compile diagnostics appear in the editor; runtime errors point to the failing source. Output uses the CLI's text format, with streamed `print` lines and sampled-run progress. A partial result, where some worlds failed and the others finished, shows the output with its `failed` section, says so in the status line, and marks where the worlds failed in the editor.
- Hover shows declarations or built-in documentation. Ctrl/⌘-click or F12 goes to a definition; F2 selects references for renaming. Completion and signature help use compiler symbols; after `catch`, completion offers the faults it can name, which the reference also lists.
- Share encodes the program in the URL fragment. Running a model does not upload its source or inputs to an execution server; a share link itself contains the source and should be treated accordingly.
- Local edits are saved in browser storage. Loading an example or guide snippet is undoable; editing it makes it the user's saved program.

The page supports light/dark themes, a resizable panel divider, keyboard navigation and a single-column layout on small screens.

## Architecture

| File or component | Role |
|---|---|
| [`probl-wasm`](../crates/probl-wasm/src/lib.rs) | JSON interface for checking, running, examples and reference documentation |
| [`probl.js`](../web/src/probl.js) | Loader for plain WASM exports and strings in module memory; no wasm-bindgen toolchain |
| [`worker.js`](../web/src/worker.js) | Checking/running away from the page's main thread |
| [`app.js`](../web/src/app.js) | Editor, output, options, guide, reference and worker lifecycle |
| [`build.mjs`](../web/build.mjs) / [`serve.mjs`](../web/serve.mjs) | Static build and rebuilding development server |

Two workers share a compiled module: one checks edits and one runs programs. Each instance runs synchronously on one thread. Stop and the default 30-second timeout terminate the runner and replace it. Crashes also replace the runner, so a failed model does not require reloading the page.

The adapter uses [`probl`](library.md) for loading, execution and reporting, and the compiler's internal APIs for editor symbols. Text crosses the module boundary through plain exported functions. The JavaScript interface is an application adapter, not a separately stabilized package API.

### Limits

[`Limits::browser()`](../crates/probl/src/options.rs) sets lower ceilings than native execution, including 1,000,000 worlds, 2 × 10⁹ work units, 150 nested calls, 20,000 chain states, 1 MiB output and 8 MiB input. The source definition is authoritative for the full list. The page's timeout is additional to engine budgets.

The resolver serves only supplied in-memory files. The page currently supplies bundled example data; it has no arbitrary local-file picker or stdin. The language has no general network access. Output is displayed as text.

## Tests

From the repository root, with dependencies installed:

```sh
cargo test -p probl-wasm
cargo build --release -p probl-cli
cd web
bun run build
bun run test
```

The native Rust tests cover the adapter. `test/examples.mjs` executes the actual WASM module and compares native/WASM outputs. `test/page.mjs` runs the page in headless Chrome, covering examples, Stop, limits, errors, share links, the guide and editor interactions. These check different layers; a native adapter test alone does not test WebAssembly.

The browser test defaults to macOS application paths. Set `PROBL_CHROME` to the Chrome executable on another installation. For Firefox, set `PROBL_FIREFOX` if needed and run `bun test/page.mjs firefox`. The browser must be installed separately.

The current GitHub Actions workflow runs the Rust checks only. Adding WASM parity and browser tests to CI remains a priority; see [testing](testing.md).

## Hosting and deployment

The build is static and can be served by a host that serves `.wasm` as `application/wasm`. The project's deployment script targets Cloudflare Pages using wrangler, a development dependency in `web/`. Production is served at [playground.probl.dev](https://playground.probl.dev), a custom domain of the `probl-playground` Pages project; `probl-playground.pages.dev` serves the same deployment.

Credentials come from environment variables or the ignored `web/.env`:

- `CLOUDFLARE_API_TOKEN`: an API token with Account · Cloudflare Pages · Edit permission.
- `CLOUDFLARE_ACCOUNT_ID`: the account ID.
- `CLOUDFLARE_PAGES_PROJECT`: optional project name; defaults to `probl-playground`.

From `web/`, when ready to publish:

```sh
bun run deploy
bun run deploy --preview
```

The first command deploys the working tree to production. The second deploys a preview named after the current branch. The script checks credentials, creates the Pages project if missing, builds and uploads `web/dist`. New projects use `main` as the production branch. Uncommitted changes are included and produce a warning.

To test an existing deployment:

```sh
PROBL_URL=https://playground.probl.dev bun test/page.mjs
```

Automatic deployment is not configured in the current CI workflow. Charts, user-supplied data files, structured JSON reports and parallel browser sampling remain future work. The Rust library's structured results provide a foundation, but are not yet exposed as a full browser result schema.
