# Model tests

The checkout supports isolated, enumerated model tests in `.test.probl` files:

```probl
test "a die stays in bounds" {
  let roll ~ d6
  expect roll >= 1 and roll <= 6 as "bounds"
  expect roll == roll as "one realized draw"
  expect d6 > 0 as "every outcome of a recipe"
}

test "ordinary list statistics" {
  let actual = mean([1, 2, 3])
  expect actual == 2
}
```

Run `probl test model.test.probl` or `probl test tests/`. `--filter text` selects
test names containing that text; `--verbose` shows captured output on success.
An empty selection is an error. Discovery sorts canonical paths, deduplicates
overlapping roots, skips symlinks, hidden directories, `target` and `node_modules`,
and accepts only `.test.probl` files. Tests cannot be imported as modules.
Paths print relative to the current directory. A failed expectation says how
much of the final weight it's false for, as a report prints a probability
(`false for 16.67% of the weight`), or its power of ten when that's too small
to print. An error in the test file outside its tests, or in a file it imports,
is reported once for the file, and none of its tests run.

`probl check model.test.probl` compiles every test without running it, and
`--data` reads the data its fixtures read. `probl run` refuses a test file and
names the command that runs it; a `test` block in an ordinary program is an
error saying where tests go.

Every test starts afresh: imports execute in dependency order, then file-level
fixtures, then the test's setup. Imported draws and fixtures retain their
correlation within that test. Fixture variables are read-only in test bodies;
copy a fixture into a local `var` to modify it. Nothing mutable carries over to
another test.

`expect` statements form the final section of a test. Setup cannot follow an
expectation, and expectations cannot be nested in loops or branches. Early test
returns are rejected. Optional diagnostic `report` statements may follow the
first expectation; grouped reports are deferred.

An expectation checks a boolean property in every positive-weight final world.
A boolean recipe is integrated, never sampled to decide pass or fail. Numbers
and probabilities alone are errors. A failed expectation does not condition the
model, so later expectations see the same population. Counterexample weights are
checked before conversion to display probabilities: even a share that underflows
to zero fails. Results retain the log of its unnormalized weight. Supported
analytic predicates are checked almost surely; measure-zero exceptions do not
fail an assertion. If a comparison needs a nonzero tail beyond the analytic
CDF's precision, it produces a capability error instead of certifying a pass.

Assertion and diagnostic-report expressions are read-only. Runtime guards follow
function values and callbacks and reject draws, chance branches, observations,
scores, printing and nested `simulate`. Pure helpers may use their own local
variables and deterministic loops. Compute nested inference during setup.

The runner distinguishes **pass**, **failure**, **error**, and **inconclusive**.
Unresolved setup weight or missing recipe mass cannot produce a pass. Executing
an incomplete `simulate` or a statistic of an incomplete distribution also marks
the test inconclusive, even if the resulting scalar no longer carries that
metadata. A `simulate` outcome whose positive weight cannot fit in the recipe's
binary64 probabilities also prevents a pass. This is conservative, including
when that result is later unused.
A positive counterexample in retained setup worlds still establishes failure.
When setup inference was incomplete, the first runner conservatively leaves all
assertions inconclusive: it cannot yet distinguish a definite counterexample
from a predicate of a scalar calculated from incomplete outcomes. Impossible evidence and
uncaught runtime faults are errors; independent tests continue after them.
Only an all-pass selected suite exits successfully.

The CLI captures `now` once for the suite and derives `today`. Pin either with
`--now DATETIME` or `--today YYYY-MM-DD`; today alone means midnight, and explicit
snapshots must agree. The suite's datetime is printed for replay. Data reads use
the declaring file's directory, with immutable bytes snapshotted within each
test file's suite and decoded for each test.

`--max-work` and `--max-worlds` bound each test. `--timeout` stops the suite and
marks later tests unrun. Discovery allows at most 1,000 files and 64 directory
levels. Compilation/execution is limited to 1,000 tests and 64 MiB of source
processed across tests, including repeated compilation of their imports.
Normal source, input, output and engine limits also apply.

## Library API

`probl::compile_tests(name, text, &mut sources)` explicitly enables test syntax;
changing the name passed to ordinary `compile` does not. The returned `TestSuite`
owns its source snapshot. Iterate `suite.tests()` and call
`test.run(&options, &mut files)` with the same pinned clock and environment
options for every test. Hosts control data access and cancellation as for a
normal program. A `Snapshots<Files>` wrapper can freeze input bytes across tests.

`suite.warnings()` are the warnings about the test file and its imports outside
any test, given once rather than with each test; `test.program()` is the test's
compiled program, or why it doesn't compile, for checking it without running it.
`TestResult` contains its status, diagnostics, captured output, clock metadata
and assertion results with labels, file spans, counterexample probabilities and
log weights, and the log of a counterexample share too small for its
probability (`log_false_share`). `error_kind` distinguishes runtime, compilation, capability, limit
and internal errors. Hosts should stop on cancellation or internal failures.
Results use typed engine data, not report text. Requests for sampling or numerical
integration are rejected; the first runner cannot certify approximate checks.

## Next stages

Population reductions such as `expect mean(x) == 3.5` and `expect P(x > 3) == 50%`
need the query planner described in [the design](design/imports-and-tests.md).
For now, statistical query built-ins inside an expectation, including through a
helper or alias, produce an explicit unsupported error. This includes shared
queries such as `mean(d6)`; compute ordinary per-world statistics in setup.
Assertions can still inspect collections pointwise with helpers such as `len`.

Static effect diagnostics, richer counterexample values, sampled tests, and
WASM/playground test execution remain later work. The playground highlights the
syntax and documents it but does not yet execute test sources.
