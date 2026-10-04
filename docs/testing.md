# Test coverage and quality

Assessment: 4 October 2026, following the [API consistency review](api-consistency-review.md).

Probl has a substantial test suite, including an independent oracle. It does not yet provide enough assurance about how the expanding APIs compose. The recent bugs are evidence of that gap: individual functions can be exercised while their shared contracts remain untested. A larger test count or a high line-coverage percentage alone would not establish that the answers are correct.

No instrumented coverage percentage has been measured for this assessment. The repository has no coverage-reporting CI job, and `cargo-llvm-cov` and LLVM coverage tools were not installed in this environment. The recommendations below are proposed tooling additions; no new testing tool has been installed.

## Executable checks from the audit

[api_consistency.rs](../crates/probl-engine/tests/api_consistency.rs) contains 51 checks, each registered separately for enumeration and sampling: **102 tests**, of which **58 pass and 44 are ignored known failures**. Findings API-03 and API-09 have been fixed and their checks enabled. The 22 remaining failing checks cover the other seven findings. They assert the desired invariant, not the observed incorrect answer.

```sh
# Normal, green suite: ignored bodies are still compiled.
cargo test -p probl-engine --test api_consistency

# Reproduce the known failures. A failing exit status is expected today.
cargo test -p probl-engine --test api_consistency -- --ignored

# Work on one defect, in both modes.
cargo test -p probl-engine --test api_consistency lower_median -- --ignored

# List the outstanding compiled tests (the normal run prints FIXME reasons).
cargo test -p probl-engine --test api_consistency -- --ignored --list
```

Each ignored test has `#[ignore = "FIXME(API-XX): reason"]`; the ID refers to the numbered audit finding. Remove the attribute when the fix makes the test pass. Do not use `#[should_panic]` for a known incorrect answer: that would make the defect a passing expectation. Rust's ignored tests remain compiled and can be selected explicitly, unlike commented-out test bodies. [Rust test harness documentation](https://doc.rust-lang.org/rustc/tests/).

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
| Other manual controls and design questions | Unicode text, date arithmetic/statistics, integer/complex functions, enum/bool ordering, scalar min/max lifting, sequence capabilities, typed container equality, singleton ordering passthrough, rounding types, bag defaults, and PMF argument types |

Where the final design is open, tests express agreement rather than selecting a new rule. For example, incomplete CDF/PMF paths may agree on a result or both explicitly reject unresolved input. Indexing and `get` now both admit integral floats, as required by the checked integer-conversion policy. Characterization tests for documented or undecided overloads are named/commented accordingly; revise those deliberately if the design changes.

Both modes use deterministic queries, a pinned date, and a fixed sampling seed. The sampling cases test API behavior, not statistical convergence. Assertions inspect returned values and raw bounds. Tiny-tail tests use relative error and require positive mass; probability range checks have no tolerance. Ordinary approximate arithmetic uses a scale-aware tolerance, so it does not require accidental bit-for-bit agreement after distribution normalization.

Validation when the audit tests were first added: `cargo test --all` passed with 470 passing tests and 50 ignored; `cargo clippy --all-targets -- -D warnings` passed. Explicitly running the ignored audit cases produced 50 expected failures and no compile-error failures. These figures count registered test cases, including separate execution modes, not distinct bugs or generated inputs.

[integer_conversion.rs](../crates/probl-engine/tests/integer_conversion.rs) adds checks in both modes for exact contextual conversion, declared boundaries, recursive containers, indexing and defaults, integer arguments, date/range/count operations, fractional rejection, probabilistic errors, large values and resource limits. Native/WASM fixtures also exercise this contract.

Validation after fixing API-03 and API-09: `cargo test --all` passed with **499 passing tests and 44 ignored known failures**. The integer-conversion suite contributes 23 tests, and six audit tests were enabled. Clippy, formatting, native/WASM builds, and the native/WASM parity suite all passed.

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
| Actual WASM and browser | [Native/WASM parity](../web/test/examples.mjs) and [headless browser tests](../web/test/page.mjs) | Present in the repo but absent from the current GitHub Actions workflow |

The [current CI workflow](../.github/workflows/ci.yml) runs formatting, Clippy, and `cargo test --all` on Ubuntu. Those commands include the native oracle and fuzz-style tests. They do not run the JavaScript parity suite, a real browser, a coverage report, or mutation testing. Native/WASM agreement is useful portability evidence, but both builds can share the same algorithmic bug—as every audit reproduction did.

## Recommended additions, in order

### 1. Put existing WASM and browser tests in CI

This closes a concrete automation gap before adding another test framework. Build the release CLI and actual WASM module, run `web/test/examples.mjs`, then build the playground and run `web/test/page.mjs` with an explicitly installed Linux browser and `PROBL_CHROME` pointing to it. Keep failures visible rather than allowing this job to fail silently. When fixing an API defect, move a small representative case into the WASM fixtures as well as enabling its Rust regression.

### 2. Measure coverage with cargo-llvm-cov

Use [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) to publish a browsable line/region coverage artifact. Its current branch coverage is optional and requires nightly; stable line/region reporting is a useful starting point. Start with visibility, then set per-area expectations after measuring a baseline. Avoid an arbitrary global percentage that can be increased by low-value assertions.

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
