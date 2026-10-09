# Contributing to Probl

Thank you for helping! Probl is early-stage; the language and the Rust API still change, so bug reports/fixes and new examples are the easiest contributions to accept. Before you start on a language feature, open an issue to discuss it. The [reference semantics](docs/semantics.md) is the contract that the engine, the library and the playground share, and changing it needs agreement first.

## Reporting a bug

Please include:

- the smallest program that reproduces the problem, and any input it reads;
- the command you ran, and the output of `probl --version` (or the commit you built);
- what Probl printed, and what you expected instead.

A sampled run repeats exactly with the same `--seed`, and a model that uses `today` with the same `--today`. If a probability or math result is wrong, detail how you computed the correct answer (e.g. by hand or another tool's output) so it's easier to act on.

## Set up

You'll need Rust 1.85 or later. The playground also needs [Bun](https://bun.sh) and the `wasm32-unknown-unknown` target; see the README for setup and [playground development](docs/playground.md#develop-locally).

```sh
git clone https://github.com/garlab/probl
cd probl
cargo test --all
```

The [architecture](docs/architecture.md) describes what each crate does and how a program flows through them.

## Before you open a pull request

CI runs these on the latest stable Rust, and a pull request needs them to pass:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all --locked
```

Commit any change to `Cargo.lock` together with the change that caused it. If you change the playground or `probl-wasm`, also run the [playground tests](docs/playground.md#tests), because CI doesn't run them yet.

## Tests

A change in behavior needs a test that fails without it.

| What changed | Where its tests go |
|---|---|
| Parsing | `crates/probl-syntax/tests/`, with snapshots |
| Lowering, checks and analyses | `crates/probl-sema/tests/`, with snapshots |
| What a program computes | `crates/probl-engine/tests/`, in the file for its area (`dates.rs`, `statistics.rs`, …), run both enumerated and sampled where both apply |
| The `probl` API | `crates/probl/tests/library.rs` |
| The command line | `crates/probl-cli/tests/` |
| The playground | `crates/probl-wasm/tests/` and `web/test/` |

When a change to the parser or the lowering is meant to alter their output, accept the new snapshots with [cargo-insta](https://insta.rs/docs/cli/) (`cargo insta review`), and read each diff before you accept it.

`crates/probl-oracle` generates random programs and checks the engine's answers against an independent interpreter that computes with exact fractions. After a change to the engine, run it with a larger budget, for example `PROBL_ORACLE_CASES=5000 cargo test --release -p probl-oracle`. A failure prints its seed and its program; add the minimized program as a regression test. [Testing](docs/testing.md) lists the other budgets, and explains how to choose a tolerance for an assertion.

A known defect that isn't fixed yet can keep its test, marked `#[ignore = "FIXME: reason"]` and asserting the correct answer. Don't use `#[should_panic]` to turn a wrong answer into a passing test.

## Examples

Each program in `examples/` ends with a `# ── Output` block, and `cargo test -p probl-cli --test examples` checks that the program still prints it. An enumerated example must match exactly. A sampled example must agree within its sampling error. The test fixes the date at 2026-09-29. When a change is meant to alter an example's output, regenerate the output:

```sh
cargo run --release -p probl-cli -- run examples/02_craps.probl --today 2026-09-29
```

Then replace the block, prefixing each line with `# `, and say in the commit message why the numbers changed. A new example should explain its assumptions in its opening comment and get an entry in the [language overview's list](docs/language-overview.md#12-examples).

## Changing the language

A change to the language touches more than the compiler:

- **Documentation:** the [reference semantics](docs/semantics.md) and the [language overview](docs/language-overview.md). The playground's guide is built from the overview.
- **A new keyword:** `crates/probl-syntax/src/token.rs`, the `KEYWORDS` list and hover text in `crates/probl-sema/src/docs.rs`, and the playground's highlighting in `web/src/probl-lang.js`.
- **A new built-in:** its signature and summary in `crates/probl-sema/src/docs.rs`. A test fails if a public built-in has no documentation.

## Performance

Measure with `probl-bench` before and after the change, using release builds on the same machine. The [benchmarks](docs/benchmarks.md#running-them) page shows how to run it. When you compare two builds, alternate which one runs first, because the order alone can move the results by a few percent. Put the numbers in the pull request. A change that makes a large difference also gets a dated section in [benchmarks](docs/benchmarks.md).

## Commits

- All pull requests will be squash-merged to `main`, so the number of commits and their messages matter less. If the scope is large, consider splitting it into several pull requests.
- The title and description of a pull request matter much more: they should clearly indicate the nature and scope of the changes.
- We encourage following [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) for pull request titles: `type(scope): summary`, with `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `ci` or `chore` as the type. The scope (`engine`, `repl`, `web`, …) is optional.

## Releases

A maintainer releases by setting the version in `Cargo.toml`, then publishing a `vX.Y.Z` pre-release on GitHub as a dry run, and making it a release once that passes. [release.yml](.github/workflows/release.yml) then publishes the crates to crates.io and the command's container image to `ghcr.io/garlab/probl`. Every published crate shares the workspace version, and the internal dependencies are pinned to it exactly; see [packaging](docs/library.md#packaging-and-publication).

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed under the [MIT license](LICENSE-MIT) and the [Apache License, Version 2.0](LICENSE-APACHE), without any additional terms or conditions.
