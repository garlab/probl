# Test coverage and quality

Current test workflow and coverage gaps, October 2026.

Probl has a substantial test suite, including an independent oracle. It does not yet provide enough assurance about how the expanding APIs compose. The recent bugs are evidence of that gap: individual functions can be exercised while their shared contracts remain untested. A larger test count or a high line-coverage percentage alone would not establish that the answers are correct.

The repository does not publish an instrumented coverage baseline. The current CI workflow has no coverage, browser or mutation-testing job. The tooling additions below are proposals, not existing quality gates.

## Run the checks

From the repository root:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

For the actual WASM module and browser, install Bun, the Rust WASM target and a browser, then follow the [playground test commands](playground.md#tests). These checks are separate from `cargo test`.

Generated tests can run larger budgets through `PROBL_ORACLE_CASES`, `PROBL_SAMPLING_CASES`, `PROBL_FUZZ_CASES`, `PROBL_DATA_CASES`, `PROBL_CHAIN_CASES`, `PROBL_RECURSION_CASES` and `PROBL_CONJUGATE_CASES`. Record seeds and preserve minimized failing programs as regressions.

## API regression contracts

[api_consistency.rs](../crates/probl-engine/tests/api_consistency.rs) registers the original manual checks separately for enumeration and sampling. All nine numbered findings are fixed; none of these regressions is ignored.

```sh
# Run every original audit regression in both modes.
cargo test -p probl-engine --test api_consistency

# Run the broader agreed-contract matrix.
cargo test -p probl-engine --test api_contracts

# Focus on one former defect in both modes.
cargo test -p probl-engine --test api_consistency incomplete_cdf
```

Future known defects can use `#[ignore = "FIXME(API-XX): reason"]` while awaiting a fix, retaining compilation and an assertion of the intended contract. Remove the attribute with the fix. Do not use `#[should_panic]` to turn a known incorrect answer into a passing expectation. [Rust test harness documentation](https://doc.rust-lang.org/rustc/tests/).

| Audit finding / manual probes | Regression checks |
| --- | --- |
| API-01: nested numeric ordering | Separate low/high median and quantile checks; active sort/CDF control; list and distribution inputs |
| API-02: incomplete finite versus continuous mixtures | Separate CDF/PMF consistency checks; active checks of the report's actual unresolved bounds |
| API-03: map key-conversion collisions | Rejection checks for int/float-to-prob, prob-to-float, and annotated literals; active collision-free map and count-preserving bag controls |
| API-04: key identity and probability outcomes | Membership/removal, membership/insertion, bag membership/count/removal, and total PMF over support; active exact-key and numeric-equality controls |
| API-05: invalid computed probabilities | Range checks on returned `prob` values, complements, and reconversion over multiple population sizes; explicit out-of-range user input remains rejected |
| API-06: continuous overflow and instability | Separate overflow, large SD, uniform midpoint, symmetric beta, and translated triangular-variance checks; independent ordinary-moment controls |
| API-07: relative-weight overflow | Support and mass under large common scaling; active ordinary/tiny weights and all-zero rejection |
| API-08: quantile validation and tiny tails | Separate heterogeneous and record-distribution validation checks; q=1 tail check; active positive-tail magnitude and ordinary endpoint controls |
| API-09: inconsistent `get` indices | Index/get agreement and invalid-key rejection; active present/absent integer indices |
| Other manual controls and design questions | Unicode text, date arithmetic/statistics, integer/complex functions, enum/bool ordering, multi-argument min/max lifting and scalar rejection, sequence capabilities, typed container equality, singleton ordering validation, rounding types, bag defaults, and PMF argument types |

The agreed contracts are now explicit: incomplete scalar probability queries reject unresolved input, PMF and keys use typed identity, and indexing/get share checked integral-float conversion. The min/max, sequence and singleton tests reflect the selected design rather than characterizing the former overloads.

Both modes use deterministic queries, a pinned date, and a fixed sampling seed. The sampling cases test API behavior, not statistical convergence. Assertions inspect returned values and raw bounds. Tiny-tail tests use relative error and require positive mass; probability range checks have no tolerance. Ordinary approximate arithmetic uses a scale-aware tolerance, so it does not require accidental bit-for-bit agreement after distribution normalization.

[integer_conversion.rs](../crates/probl-engine/tests/integer_conversion.rs) adds checks in both modes for exact contextual conversion, declared boundaries, recursive containers, indexing and defaults, integer arguments, date/range/count operations, fractional rejection, probabilistic errors, large values and resource limits. Native/WASM fixtures also exercise this contract.

[ordering.rs](../crates/probl-engine/tests/ordering.rs) covers custom comparators, stable ties in both directions, callback errors and effects (including print visibility to caching and draw scheduling), work limits, bounded behavior for inconsistent comparators, nested numeric order statistics, singleton validation, tiny positive tails and adjacent representable probabilities around mass boundaries. The same API has native/WASM fixtures.

[api_contracts.rs](../crates/probl-engine/tests/api_contracts.rs) covers unresolved finite/continuous mixtures, sub-epsilon tails through lifting, the typed key/outcome matrix, probability bounds, relative-weight scaling, representable large moments, overflow and singularity rejection, direct range statistics, bigint ranks, Unicode lookup and resource limits. A unit test separately distinguishes permissible computed-probability roundoff from invalid results. The native/WASM fixture exercises the same contracts.

[min_max.rs](../crates/probl-engine/tests/min_max.rs) tests the separate candidate, collection and distribution extrema contracts. They cover comparator selection and stable ties, independent recipes versus correlated draws, exact support bounds, unresolved/unbounded rejection, required top-n counts, callback validation/effects, explicit seeded reduction and work limits. [function_values.rs](../crates/probl-engine/tests/function_values.rs) tests named/builtin references, captures, declared boundaries, arities, probabilistic calls, callback restrictions and print-aware caching/draw scheduling. Most cases run in both inference modes. These enforce the separate candidate, collection and distribution contracts.

[reduction_defaults.rs](../crates/probl-engine/tests/reduction_defaults.rs) tests `reduce(xs, f, initial?)` and extrema defaults. It covers empty/filtered/singleton collections, left-fold order, callback validation and effects, distribution lifting and independence, named defaults through aliases, malformed keywords, eager evaluation order, and print-aware optimization. The former `reduce(xs, initial, f)` call sites are migrated; there is no automatic argument-order detection. Defaults apply only to empty extrema collections and do not compete with existing elements or hide invalid distributions.

The existing shared `close` helper uses absolute error below `1e-9`. That is suitable for some moderate-size values, but would accept zero for a `1e-13` tail and would miss a small violation of `[0,1]`. Choose assertions according to the property rather than applying one tolerance everywhere.

## What is already covered

| Layer | Existing protection | Important limit |
| --- | --- | --- |
| Syntax and lowering | Unit tests, parser/lowering snapshots including examples | Snapshots preserve structure; they are not independent semantic oracles |
| Engine and numeric library | Feature and regression suites for math, bigints, complex values, dates, text, statistics, inference, limits, and reporting | Many interactions across type, representation, scale, and execution mode were missing |
| Independent semantics | [Rational-arithmetic oracle](../crates/probl-oracle/src/lib.rs); 400 generated cases by default, checked with merging/memoization on and off | Covers a bounded discrete subset, with i64 reference integers; does not cover the newer statistical, date, complex, and collection contracts broadly |
| Sampling | [Generated sampling comparisons](../crates/probl-oracle/tests/sampling.rs); 150 programs and 2,000 runs per program by default | Calibration checks are valuable, but cannot establish correctness for every distribution or rare event |
| Robustness | [Program mutation/noise tests](../crates/probl-oracle/tests/fuzz.rs), 300 cases by default; [data round-trip and malformed-input tests](../crates/probl-engine/tests/data_fuzz.rs), a 2,000-case default budget | Seeded generators run in CI, but these are not coverage-guided fuzzers; no automatic shrinking framework is configured |
| CLI and host API | Golden examples, input/CLI tests, native tests of the WASM-facing API | Native API tests do not execute a `.wasm` binary |
| Library API | [The `probl` crate's tests](../crates/probl/tests/library.rs), through its public API only: known answers, unresolved bounds, sampling statuses, evidence scale, data reuse and misuse, and agreement of every example's numbers with its printed text; the golden examples also run through the library | Agreement with the text is checked at printed precision; no published baseline yet for `cargo semver-checks` |
| Actual WASM and browser | [Native/WASM parity](../web/test/examples.mjs) and [headless browser tests](../web/test/page.mjs) | Present in the repo but absent from the current GitHub Actions workflow |

The [current CI workflow](../.github/workflows/ci.yml) runs formatting, Clippy, and `cargo test --all` on Ubuntu. Those commands include the native oracle and fuzz-style tests. They do not run the JavaScript parity suite, a real browser, a coverage report, or mutation testing. Native/WASM agreement is useful portability evidence, but both builds can share the same algorithmic bug—as every audit reproduction did.

## Recommended additions, in order

### 1. Put existing WASM and browser tests in CI

This closes a concrete automation gap before adding another test framework. Build the release CLI and actual WASM module, run `web/test/examples.mjs`, then build the playground and run `web/test/page.mjs` with an explicitly installed Linux browser and `PROBL_CHROME` pointing to it. Keep failures visible rather than allowing this job to fail silently. When fixing an API defect, move a small representative case into the WASM fixtures as well as enabling its Rust regression.

### 2. Measure coverage with cargo-llvm-cov

Use [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) to publish a browsable line/region coverage artifact. Stable line/region reporting is a useful starting point; consult the tool's documentation for optional branch-coverage requirements. Start with visibility, then set per-area expectations after measuring a baseline. Avoid an arbitrary global percentage that can be increased by low-value assertions.

Proposed local setup and baseline command:

```sh
rustup component add llvm-tools-preview
cargo install --locked cargo-llvm-cov
cargo llvm-cov --workspace --html
```

Inspect uncovered dispatch arms and error paths in `builtins.rs`, `ops.rs`, `continuous.rs`, and coercion/lowering first. Track ignored regressions separately; they are outstanding defects, not passing protection. Include CLI subprocess profiles when configuring reporting, and identify any excluded targets. Do not interpret the native report as JavaScript/browser coverage.

### 3. Add generated API invariants with shrinking

Keep the independent oracle and extend its supported subset deliberately. For the wider API matrix, [Proptest](https://proptest-rs.github.io/proptest/proptest/tutorial/shrinking-basics.html) can shrink generated failures to smaller inputs and [persist regression seeds](https://proptest-rs.github.io/proptest/proptest/failure-persistence.html). That would complement the existing fixed-seed generators.

Generate small valid populations and vary representation, types, boundaries, and scale. Useful properties include:

- Lists and equivalent finite distributions answer population queries consistently.
- Medians agree with the chosen ordering; CDFs and quantiles describe the same cumulative population.
- Positive common weight scaling preserves a distribution.
- Translating a distribution preserves variance when the inputs and result remain representable.
- Membership, lookup, insertion, and removal share the chosen key identity.
- Typed map conversion either preserves entries or reports a collision.
- Computed probabilities stay finite and within `[0,1]`.

Use independent small reference calculations where possible. Agreement between two functions that share implementation is weaker than agreement with an independent expected result. Keep generated tests bounded, reproducible, and able to report a useful minimal source program.

### 4. Check assertion strength with targeted mutation testing

[cargo-mutants](https://mutants.rs/getting-started.html) deliberately changes code and reruns tests. A surviving mutation can expose a weak assertion even on fully executed lines; some mutations are equivalent and require review. Begin with a narrow scheduled/manual run on comparison logic, statistics, normalization, and coercion, once their known failures have been fixed and enabled. [Interpreting mutation results](https://mutants.rs/using-results.html).

Do not start with an expensive whole-workspace mutation gate. Fix meaningful survivors first, preserve independent expected values, and expand the scope based on runtime and usefulness.

### 5. Expand robustness runs independently of semantic assertions

Rotate the existing oracle/fuzz seeds in scheduled jobs, keeping fixed regression seeds on pull requests. Later, add [cargo-fuzz/libFuzzer](https://rust-fuzz.github.io/book/cargo-fuzz.html) targets for parsing, compilation/diagnostics, and data loading. Preserve work, memory, and cancellation limits. A run that does not crash is a robustness success, not evidence that its probability calculations are correct.

The first priorities are executing existing platform tests in CI, measuring a baseline, and strengthening semantic properties. Coverage reports and mutation testing answer different questions; both are useful, and neither replaces an explicit API contract.
