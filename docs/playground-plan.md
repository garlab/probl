# Plan: a playground in the browser

> September 2026. The [implementation plan](implementation-plan.md) had "a WebAssembly build and a browser playground" in phase 7; this plans it as a project of its own. **Phases 1 and 2 are built:** the engine runs in WebAssembly and prints exactly what the command line prints, and the page works in Chrome and Firefox (sections [Phase 1, as built](#phase-1-as-built) and [Phase 2, as built](#phase-2-as-built)). The open questions at the end were answered with the proposals. Hosting waits for a remote.

## How hard is it?

**Moderate, and mostly web work.** The engine is close to running in a browser already:

- **Dependencies:** its crates, and their dependencies (`ariadne`, `rustc-hash`, `libm`, `csv`, `serde_json`, `sha2`), are pure Rust.
- **Randomness:** its random numbers come from the seed, not the operating system.
- **Clock and I/O:** it reads no clock, no files and no network. The one input, data from `read`, goes through a resolver that the host supplies.
- **Limits:** host limits and cancellation were built for hosted use (audit I3). The architecture already says that "a hosted playground should also run models in a separate, cancellable worker" (implementation plan, §3.11).

What stands in the way is small and in one place, `probl-engine/src/lib.rs`, plus a few conventions:

| What | Why it matters in a browser | Fix |
|---|---|---|
| `run` starts a thread with a 64 MiB stack, and sampling starts worker threads | WebAssembly without shared memory can't start threads | a way to run on the calling thread; batches run in line when there's one thread, with the same output |
| About 20 calls to the platform's math library (`ln`, `exp`, `powf`, `log10`, in operators, built-ins, distributions and reports), and `powi`, whose rounding Rust doesn't specify | the browser's math can differ from macOS's in the last bit, and §14 promises the same output on every machine | use `libm` everywhere, as the samplers already do |
| Panics are caught | in WebAssembly a panic aborts the whole instance | a panic hook records the message, and the page replaces the worker |
| Deep recursion (about 5 KB of stack per Probl call; 500 calls allowed) | browsers give WebAssembly a small native stack | measure it per browser, and lower the call-depth limit for the playground |

There's nothing to disable in the language. `read` doesn't touch the disk itself: whoever runs the program says what a path means. The playground's resolver can serve the examples' data files from memory, so example 08 works, and refuse everything else. `print` output comes back with the result.

**Effort:**

| Version | What | Effort |
|---|---|---|
| A bare prototype | a text box, a Run button, the text output | 1–2 days |
| A first release | an editor with highlighting and errors in place, runs in a worker with Stop, the examples, share links, the same output as the command line | 7–12 days |
| Later, each optional | charts, your own data files, parallel runs, runnable snippets in the docs | 1–5 days each |

## What it looks like

- **Layout:** an editor on the left and the output on the right, stacked on a phone. A bar above has an examples menu, Run (Ctrl-Enter or Cmd-Enter), Stop and Share.
- **Output:** the same text as `probl run`: the summary line, the reports, and what `print` printed.
- **Errors:**
  - **Compile errors** are underlined as you type, and listed as the command line renders them.
  - **Runtime errors** point at the statement that failed.
- **Options:** runs, seed, and exact updates on or off, in a small panel. By default, the program's own `@mode` applies.
- **Share:** a link with the program compressed in the URL's fragment, the part after `#`. Browsers don't send it to the server, so programs never leave the browser: nothing needs a server, an account or a database.
- **A note:** the program runs in your browser, with lower limits than the command line.

## Architecture

```text
crates/probl-wasm/     new: the engine's API for JavaScript (a cdylib, built with wasm-bindgen)
web/                   new: the page, as static files
  index.html
  app.js               editor, controls, output, share links
  worker.js            loads the WebAssembly module and runs programs
  probl-mode.js        syntax highlighting
```

### The WebAssembly API

Programs, options and results cross as JSON strings, so the interface stays small and independent of the binding tool:

- **`check(source)`** gives the diagnostics: severity, message, span, line and column, notes, help, and the text as the command line renders it. It also gives the program's names, for the editor (see [the guide and the editor, as built](#the-guide-and-the-editor-as-built)).
- **`run(source, options, on_print, on_progress)`** gives the output, or the error with its span.
  - **Options:** mode, runs, seed, exact updates.
  - **Callbacks:** `on_print` receives what `print` prints, and `on_progress` how many runs are done, after each batch.
  - **Limits:** the playground sets them, not the page.
- **`examples()`** gives the bundled examples with their data files.
- **`docs()`** gives the reference: every built-in and keyword, with its signature and a summary.

The API is plain Rust around `probl_sema::compile` and `probl_engine::run_on_this_thread`, so its tests run natively with `cargo test`. The module exports it as plain functions, and a small loader passes the text through the module's memory (see [Phase 1, as built](#phase-1-as-built)).

### The worker

The engine runs synchronously, so it runs in a Web Worker, never on the page's thread:

- **Run:** the page sends the program; the worker calls `run` and sends back the result, the printed lines and progress as they come.
- **Stop and time out:** the page terminates the worker and starts a new one. This needs nothing from the engine, and works even inside a loop that never checks for cancellation. Compiling the module once and instantiating it per worker takes milliseconds.
- **Crashes:** a panic, a stack overflow or running out of memory ends the instance with a trap. The worker reports it as an internal error with the panic's message, and the page replaces it.
- **Memory:** WebAssembly memory grows but never shrinks. After a run that used a lot of it, the page replaces the worker.

### Limits

The playground passes its own `Limits`, lower than the command line's, because a browser tab has less memory and a smaller stack:

| Limit | Command line | Playground (to be tuned) |
|---|--:|--:|
| worlds per statement | 10,000,000 | 1,000,000 |
| work | 2 × 10¹⁰ | 2 × 10⁹ |
| nested calls | 500 | what every browser's stack allows, perhaps 150 |
| output | 64 MiB | 1 MiB |
| time | none | 30 seconds, then Stop, with a button to allow longer |

A model that hits one says so, with the same error as the command line and a note that the playground's limits are lower.

### Data

`read` keeps its meaning, and the playground's resolver decides what a path means:

- **First release:** the examples' data files, served from memory, so example 08 runs. Any other path is refused with "the playground can only read the examples' files". `read("-")` is refused.
- **Later:** a "data" tab where you paste or open CSV and JSON files. They stay in the page and are served to `read` by name. The resolver doesn't change, only the page.

## Speed

Everything runs on one thread at first. WebAssembly usually runs this kind of code 1.5–2× slower than one native core, to be measured in phase 1. On one native core:

- the enumerated examples take 1–270 ms;
- the sampled ones take 0.5–1.9 s, and `ab_test` 1.8 s.

So the slowest examples would take 2–4 seconds in the browser, with a progress bar.

**Parallel runs later.** Batches of runs are independent, and combining them in order makes the output the same on any number of threads (§14). So several workers can run a share of the batches each, with the page combining them in batch order. This needs no shared memory or special server headers, and works on any static host. It needs the engine to run single batches and send their results as data: 3–5 days, if the one thread is too slow.

## Building and hosting

- **Tools:** the `wasm32-unknown-unknown` target, and nothing else for the module (see [Phase 1, as built](#phase-1-as-built)). The page uses CodeMirror 6 for the editor and esbuild to bundle it: a handful of npm packages, with a lockfile.
- **Size:** the module includes the parser, the engine, the diagnostics renderer, and the CSV and JSON readers: 1.1 MB, and 400 KB compressed, without `wasm-opt`.
- **The stack:** the call-depth limit keeps recursion within what the browser allows (see [Phase 1, as built](#phase-1-as-built)).
- **Hosting:** static files, since everything runs in the browser. Any static host works, such as GitHub Pages, provided it serves `.wasm` as `application/wasm`. The repository has no remote yet.
- **Continuous integration:** check that the engine builds for `wasm32` on every change, run the tests below, and publish the page from the main branch.

## Tests

- **The same output as the command line.** Every example runs through the WebAssembly build in Node, and must print exactly what `probl run` prints: enumerated and sampled alike, byte for byte. This is what the move to `libm` is for, and it guards the promise that a seed gives the same output everywhere.
- **The API:** diagnostics, runtime errors, printed lines, progress, refused paths and each limit, tested natively.
- **The worker:** stopping, timing out, and a crash, which must leave the page working.
- **Browsers:** a smoke test in Chrome, Firefox and Safari. It loads the page, runs each example and checks the summary line. It also finds each browser's deepest recursion, which sets the call-depth limit.

## Phases

1. **A portable engine, and a WebAssembly build** (2–4 days):
   - running on the calling thread, and batches in line;
   - `libm` everywhere, with the examples' outputs unchanged natively;
   - the panic hook;
   - `probl-wasm` and its native tests;
   - the build, and the Node test against the command line;
   - measuring size, speed and stack.
2. **The page** (4–6 days):
   - the worker protocol;
   - the editor and its highlighting, with diagnostics in place;
   - output, examples, options, share links;
   - the layout on phones.
3. **Hosting** (1–2 days): the static build, continuous integration and publishing. This needs a remote.
4. **Later, each on its own:**
   - charts of distributions and fan charts of `by` reports, which need the structured output (`--format json`) planned for phase 5 (3–5 days);
   - the data tab (1–2 days);
   - parallel workers (3–5 days);
   - runnable code blocks in the language overview: done, in the guide (see [below](#the-guide-and-the-editor-as-built)).

## Risks

| Risk | Mitigation |
|---|---|
| Browsers differ: less memory on phones, and different stack sizes | Conservative limits; a smoke test per browser; limit errors that say the playground's limits are lower |
| The browser prints different numbers from the command line | `libm` everywhere, and the byte-for-byte test on every example |
| Slow models freeze the page | The worker; Stop; a time limit; a progress bar |
| A shared link runs someone else's program | It runs in the viewer's browser, with no access to files or the network, within the limits. Output is shown as text, never as HTML |
| A JavaScript toolchain to maintain in a Rust repository | A few pinned packages; the page is static files; its build is one script |
| `wasm-bindgen` versions drift | The command-line tool is pinned to the crate's version. The JSON interface would also work over plain exported functions, if it had to |

## Phase 1, as built

- **The engine:**
  - `probl_engine::run_on_this_thread` runs a program without starting a thread. With one thread allowed, sampling runs its batches one after another, with the same output as on several threads.
  - Every floating-point function goes through `libm`. No example's output changed natively.
- **`crates/probl-wasm`:**
  - **The API:** `check`, `run` and `examples`, as above. The playground's limits are 1,000,000 worlds, 2 × 10⁹ units of work, 150 nested calls, 20,000 chain states and 1 MiB of output. Data may be 8 MiB. `read` reads only the files given with the request, and there's no standard input.
  - **Plain exported functions, not `wasm-bindgen`.** The interface is only text, so `probl_alloc`, `probl_run` and the others take and give it through memory. `web/src/probl.js`, 70 lines, does the rest. That needs no generated glue and no tool pinned to a version: `cargo build -p probl-wasm --target wasm32-unknown-unknown --profile wasm` is the whole build.
  - **Panics:** a panic hook sends the message to the loader before the module stops, and the loader reports it as a crash.
- **Measured, in Node:**
  - **Output:** every example prints exactly what `probl run` prints, sampled ones included (`node web/test/examples.mjs`).
  - **Speed:** WebAssembly is 1.7–2.1× slower than one native core. The slowest example, 07, takes 3.2 s; the enumerated ones take up to 0.46 s. Memory reached 31 MiB.
  - **Stack:** V8's own stack is what limits recursion, not WebAssembly's. A recursive function worked 600 calls deep and failed at 800 with "Maximum call stack size exceeded", which the loader reports as a crash. The limit of 150 nested calls leaves a margin, to check in each browser's workers in phase 2.

## Phase 2, as built

`web/`: `npm run build` builds the module and bundles the page into `web/dist`, and `npm run serve` serves it on port 8000.

- **The page:**
  - CodeMirror, with Probl's highlighting, and compile errors underlined as you type.
  - Run (Ctrl-Enter or ⌘-Enter), Stop (Escape), Share, the examples, and options: the mode, runs and seed, exact updates, and the 30-second limit.
  - Output as `probl run` prints it, what `print` prints as it comes, and progress for sampled runs.
    - Before a run, the output pane says how to run the program, and where the guide and the reference are.
    - A report too long for the pane wraps at its ` · ` separators, under its values. Each wrapped line starts with its separator. The text is unchanged.
  - A runtime error is underlined where it happened.
  - Light and dark, and one column on a phone.
- **Two workers share the compiled module.** One checks the program as it's edited; the other runs it. Stop, the time limit and a crash all end the runner, and a new one takes its place.
- **Data:** every program may read the examples' data, from memory.
- **Share links** carry the program compressed in the fragment.
- **The program you're working on is kept:**
  - The last program you edited is kept in the browser. Programs opened from a link, the menu or the guide aren't: they aren't yours.
  - Once you edit an example or a linked program, the menu and the address stop naming it, so reloading keeps your edits.
  - When an example or the guide replaces a program of your own, the status line says that undo (⌘Z or Ctrl+Z) brings it back. Each program loaded is one step in the undo history.
- **Sizes:** the page's script is 406 KB, 132 KB compressed, mostly CodeMirror; the module is 1.1 MB, 400 KB compressed.
- **Tested in headless Chrome and Firefox** (`node web/test/page.mjs`, with `firefox` for Firefox):
  - every example prints exactly what `probl run` prints;
  - a long run shows its progress, stops, and the next run works;
  - 149 nested calls work, and 200 are the limit's error;
  - a compile error is underlined;
  - a share link opens the same program.
- **Measured in the browsers:**
  - **Speed:** similar to Node. Example 07 takes 2.6 s in Chrome and 3.0 s in Firefox.
  - **Recursion:** Chrome's worker handled 350 nested calls and crashed at 400, when V8's stack ran out. Firefox handled 600 and failed at 800, when WebAssembly's own 1 MiB stack ran out. The limit of 150 leaves a margin of 2.3× in Chrome. A crash is reported as one, and the worker replaced.
  - **Clicks in tests:** headless Firefox doesn't always deliver WebDriver clicks to a page that just loaded, so the tests press buttons through the DOM.

Left: hosting and continuous integration (phase 3, which needs a remote), and the later items in phase 4.

## The guide and the editor, as built

Beside the output are two more tabs: the language overview as a **guide**, and a **reference** for the built-in functions and keywords. The editor knows the program as the compiler does.

- **The program's names.** The compiler records where each name is declared and used as it lowers the program (`probl_sema::symbols`). So the editor resolves names exactly as the compiler does: shadowing, a function's own variables, and the top-level variables functions can use. `check` gives them with the diagnostics. While the program doesn't parse, the last names stay, following the edits.
- **Fields** lead to their record when that's certain, since a field is found by name when the program runs. In a record written with its type, like `Day { visitors: 10 }`, it is. Elsewhere, like `day.visitors`, it is when only one record type has a field of that name and no record without a type does.
- **In the editor:**
  - **Hover** a name to see its declaration, what it is, its line, and the comments above it or at the end of its line. Hover a built-in or a keyword for its documentation. Words in comments and strings aren't names.
  - **Go to a definition** with Cmd-click or Ctrl-click, or F12. While the key is down, the name under the mouse is underlined as a link. On a built-in or a keyword, the same keys show its entry in the reference.
  - **The name under the cursor** is highlighted, with each of its uses.
  - **Completion** comes as you type, or with Ctrl-Space. It offers the names in scope first, nearest first, then the built-ins and keywords, with documentation beside the list. After an enum's name and a dot it offers the enum's variants; after any other dot, fields, and functions as methods.
    - It asks the checker about the program as it is, so a name declared a moment ago is offered.
    - It stays quiet while you name something new, after `let`, `var`, `fn`, `type`, `enum` or `for`.
    - Enter takes a completion only when that changes the text. With the whole name typed, Enter starts a new line.
  - **Signature help:** while a call's arguments are typed, its signature shows above it, with the parameter being typed in bold, and the function's documentation. It works for built-ins and the program's own functions, including ones declared since the last check.
    - It shows when `(` or `,` is typed, or with Cmd-Shift-Space or Ctrl-Shift-Space.
    - It goes with Escape, or when the cursor leaves the call.
  - **Indentation:** a line inside brackets is one level in from the line that opened them, and a closing bracket goes back out to that line's level.
- **The reference** documents every public built-in and keyword, from `probl_sema::docs`, and can be searched. A test fails if a built-in has no documentation.
- **The guide** is `docs/language-overview.md`, rendered when the page is built:
  - A list of contents leads to its sections.
  - Its Probl blocks are highlighted as the editor highlights them, by the same tokenizer. So is the code in descriptions of names.
  - 12 of its 20 Probl blocks are complete programs, as the module itself checks them. Each has a button that runs it in the editor.
  - Its links to the examples open them. Links to the other documents are left as text.
- **Sizes:** the guide is 49 KB, or 16 KB compressed, and loads when first shown. The page's script grew by 8 KB, to 414 KB.
- **Tested:**
  - the names natively, in `probl-sema/tests/symbols.rs` and the API's tests;
  - in headless Chrome and Firefox: hover, going to definitions, highlights, completion, signature help and indentation as a person types, the tabs, the reference's search, the guide's programs and links, the cursor against light and dark backgrounds, and the page's width on a phone.

## Open questions

1. **When:** before Markov-chain solving, the next step in the plan, or after? And before phase 5's structured output, which charts would need?
2. **Where:** GitHub Pages needs a remote. Otherwise, another static host or your own domain?
3. **Data in the first release:** the examples' files only (proposed), a data tab straight away, or no `read` at all?
4. **Output in the first release:** text, as `probl run` prints it (proposed), or charts first, which need structured output?
5. **The editor:** CodeMirror 6, with npm and esbuild (proposed), or a plain text box with no JavaScript dependencies?
