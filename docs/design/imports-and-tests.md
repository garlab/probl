# Imports and model tests

> Draft proposal, 7 October 2026. Imports, exports, `test` and `expect` below are proposed features, not executable syntax. The parser currently recognizes a bare import, but lowering rejects it. This document recommends contracts before implementation; the [reference semantics](../semantics.md) remains authoritative for current behavior.

## Recommendation

Add static imports of explicitly exported definitions, with an acyclic file dependency graph. Imported files declare reusable definitions; they do not run a model on import. Consequently, importing does not depend on how many worlds exist.

Add `*.test.probl` files containing named, isolated `test` blocks and report-like `expect condition as "description"` statements. Tests construct a model, then check its final population. A failed expectation records a failure without conditioning or otherwise changing that population.

Every expectation is a cross-world query, like a report with a condition to check. `expect mean(foo) == 0` computes a population mean, while `expect foo + 1 < 6` checks a property throughout the population. The `expect` keyword establishes the query boundary; no additional `population(...)` wrapper is needed. Ordinary expressions outside that boundary retain their current per-world meaning. The query rules below specify the distinction, including correlation, lists and distribution recipes.

Start with enumerated tests. Sampled tests need a separate contract for empirical checks and statistical uncertainty; do not automatically switch an expensive test to sampling or silently judge an estimated mean as a population truth.

## Imports

### Import declarations, not an execution

Suggested first syntax:

```probl
# random_walk.probl — proposed export syntax
let unit = 1
export let step = one_of([-unit, unit])

export fn walk(turns: int) -> int {
  var position = 0
  repeat turns {
    position += ~step
  }
  position
}
```

```probl
# main.probl — proposed import syntax
import { walk, step as move } from "./random_walk.probl"

let position = walk(4)
report position
report move
```

Named imports make dependencies visible and allow aliases for collisions. Start with this one import form: no wildcard, implicit import-all, side-effect-only import, default export, dynamic path or package download. A namespace form such as `import "./random_walk.probl" as walk` can be added later; it also requires qualified names in types and record constructors and is not necessary for the first implementation. The currently parsed bare `import "file.probl"` should remain rejected with a migration/help message, not become textual inclusion.

Imports are top-level declarations, resolved before execution. Require them before other declarations and executable statements for readability. They are not allowed inside a function, branch, loop, `simulate` or test body. Test files import at their file level.

This answers the single-world question: **importing is not a runtime operation at all**. Calling an imported function after a split is fine. If it draws, observes or prints, it does so under the same rules as an equivalent local function, in the caller's execution and evidence scope. Returning a recipe still does not draw it. Importing that function does none of these things.

Requiring “one world” would be a poor boundary: sampling commonly represents one realization at a time, and merging can reduce many histories to one state. It also would not prevent printing, input loading or local inference during initialization.

### What an imported file may contain

Use the `.probl` extension for both scripts and reusable modules; validate a file's contents when it is imported. No new module-file extension or implicit entry function is needed.

| Top-level item in an imported file | First contract |
|---|---|
| Imports | Allowed, subject to the acyclic graph and host resolver |
| `fn`, `type`, `enum` | Allowed; private unless explicitly exported |
| Immutable `let name = expression` | Allowed with a deterministic, effect-free initializer |
| Distribution recipes such as `d6`, `one_of(...)`, recipe arithmetic | Allowed as immutable values; construction is not a draw |
| `var`, assignments or standalone executable statements | Rejected |
| Draws, probabilistic conditions, `chance`, observations and scores in initializers | Rejected, including indirectly through helper calls |
| `simulate` in initializers | Rejected initially, even though it returns one distribution value |
| `read`, `print`, `report` or future host effects during initialization | Rejected |
| Invocation settings such as `@mode`, epsilon or failure policy | Rejected; the entry program/test runner controls execution |
| Tests and expectations | Rejected; test files are separate roots and cannot be imported |

These restrictions apply to private definitions as well as exports. Do not quietly ignore a script's reports or statements and extract only its functions: reject it as an import and suggest moving reusable definitions into a module. The RPG duel example could extract `Fighter`, `attack` and `duel` into a module while keeping hero selection and reports in its entry script.

The initializer rule is about effects, not the number of resulting values. `let x = ~one_of([1])` and `let x = if 50% { 1 } else { 1 }` remain forbidden. Pure arithmetic, collections, recipe constructors and calls to helpers proven to satisfy the initializer restrictions are allowed, subject to budgets. Unproven indirect calls are conservatively rejected; this is not permission to execute arbitrary helpers to discover whether they split.

Initially keep initializers independent of host data and the `today` snapshot too. Put computations needing those inputs in functions and pass the inputs explicitly. Function bodies retain the normal language's capabilities and restrictions, including the existing restriction on where `read` may appear; imports do not create a new I/O mechanism.

Initialize dependencies before dependents, and immutable bindings within a module in source order, before any entry-model execution. Reject reading a not-yet-initialized binding, including through a helper; do not introduce forward value initialization or lazy cycles. Functions and type declarations can keep their existing forward-reference rules. A module initializer error fails preparation, not one random world, and cannot be rescued by the proposed world-continuation policy.

Each module has one logical immutable initialization per program execution or isolated test. Compiled declarations may be shared, and identical immutable values may be safely cached, but there is no mutable process-wide module state. Optimization must not change initialization diagnostics or the host's resource contract.

### Visibility, captures and identity

`export` may prefix a function, type, enum or eligible immutable binding. Everything else is private to its defining module. An exported function may call private helpers and capture private immutable bindings. Its names resolve in its defining module, never against coincidentally named bindings in the importer.

Imported bindings are read-only. Same-scope name collisions are errors and require an alias; normal inner-scope shadowing remains possible. Exporting an enum exports its qualified variants: after importing `Direction`, use `Direction.Left`. It does not inject every variant into the importer's bare-name scope. Exported record declarations bring their constructor and type under the imported name. Declared public signatures and fields must not expose a private named type; initially export that type explicitly rather than introducing opaque public types.

Preserve declaration identity. Importing an enum or named record through two dependency paths refers to the same declaration; importing two different modules that both declare `State` does not make them the same declaration. Runtime equality and shape compatibility still follow the existing type contracts. Import aliases must not duplicate enum IDs, function IDs, memoization identities or report/source locations.

Start without re-export syntax or module namespace values. Imported definitions remain usable privately in the module. An explicit wrapper function can provide a public facade; broader re-export and package-public visibility can be designed later. Test files initially have the same access to exports as ordinary importers; a matching filename is not permission to access private definitions.

### Transitive imports, cycles and source identity

Allow finite transitive chains and shared dependencies. Reject self-import and every cycle, including a cycle involving only functions or types:

```text
main.probl -> pricing.probl -> calendar.probl       allowed
a.probl -> common.probl <- b.probl                 shared once
a.probl -> b.probl -> a.probl                      error
```

“Recursive imports” here means revisiting an active module during loading. It does not remove Probl's existing function recursion; ordinary recursive functions inside an acyclic module graph retain their current inference rules.

Use resolver-provided canonical identities and a visiting/finished graph traversal. Reaching a visiting identity reports the full cycle with import locations. Reaching a finished identity reuses it. A simple visited set would wrongly classify a diamond as a cycle. Reject cycles before evaluating any initializer, including dependencies whose exports happen not to be used.

Paths are literal UTF-8 strings with an explicit `.probl` suffix, resolved relative to the importing source, not the process's current directory. Start with relative file paths; defer package names, extension guessing, directories-as-modules, URLs and network resolution. `..` is subject to host authority. A native resolver must canonicalize aliases/symlinks consistently and enforce any host-configured root on the resolved target; merely removing `..` is not a sandbox. The existing CLI data resolver's local filesystem policy need not become a claim of confinement.

The source resolver receives both the importing identity and requested path. This differs from the current data resolver. A host may supply only an in-memory source map, including in WASM. Imports never grant the engine ambient filesystem access, and permission to read data is not automatically permission to load it as source.

Freeze source bytes and identities for one compilation/test invocation. Repeated loads of an identity must not observe different revisions. Bound individual and total source bytes, module count, dependency depth and initialization work; reject oversized graphs cleanly. Canonical identity, not source text equality, determines module identity. Record source snapshots or content digests for replay and cache invalidation, including transitive changes.

## Test files and isolation

Suggested file and command:

```probl
# random_walk.test.probl — all test/import syntax is proposed
import { walk } from "./random_walk.probl"

test "one step is centered and bounded" {
  let foo = walk(1)
  expect mean(foo) == 0 as "a symmetric step has zero mean"
  expect foo + 1 < 6
}

test "zero steps stays at the origin" {
  let position = walk(0)
  expect position == 0
}
```

```sh
probl test random_walk.test.probl
probl test tests/
```

Recognize `test "name" { ... }` only at the top level of a test source. Names are nonempty string literals and unique within a file. `expect expression` has an optional `as "description"` label, using the same label convention as `report`. Test names are stable identifiers for filtering and diagnostics; expectation labels need not be unique.

Use `.test.probl` as the CLI/editor convention and an explicit test-source kind in compiler/library APIs. Renaming a diagnostic display name must not accidentally grant test syntax. A host-provided virtual test source uses the same test parse mode. Ordinary sources reject test declarations/expectation statements; contextual recognition can preserve ordinary identifiers named `test` or `expect`. Test sources cannot be imported, including from other test sources. Shared helpers belong in normal modules.

Outside tests, a test file may contain imports, helper functions/types/enums and eligible immutable fixture declarations under the module restrictions. No shared mutable fixture or model execution at file scope. A test has a fresh environment, weight 1, evidence scope, resource accounting and diagnostic/report sinks. Mutating a local collection or bag in one test cannot affect another. Test return values do not become a distribution in a parent model; there is no parent model population.

This is like `simulate` in isolation, but a runner executes each test as its own entry model. It must not be implemented as a normal call returning a mixture of all tests or as a nested `simulate` that loses assertion sites. Each test has its own pass/fail status. A model error fails that test and normally leaves the runner free to execute other independent tests; cancellation or an internal engine failure may abort the suite.

Data fixtures are loaded explicitly for the relevant test through existing host input mechanisms, before that test executes, with paths based on the test source. Shared immutable input bytes may be snapshotted once, but decoded inputs must fit the compiled test. Capture `today` once per suite and print it in replay metadata; date-sensitive CI should supply `--today`. If sampling is added, derive independent test seeds from a documented suite seed plus stable relative test identity, so adding or reordering unrelated tests does not shift another test's random stream. Do not use process-random hash functions or absolute checkout paths for that identity.

## Expectations inspect a final population

### Separate model setup from assertions

For the first version, permit `expect` only as a direct statement in the test body, in a final assertion section. The first expectation closes the model setup. After it, allow more expectations and optional diagnostic reports, but no draws, observations, mutation or other model statements. Loops, branches and helper calls remain available during setup; nested expectations, early test returns and nested tests are rejected initially.

This gives all expectations one coherent population and avoids an assertion becoming a midway synchronization mechanism. No later observation can change an earlier expectation. A future need for per-visit assertions or assertions at several stages should get an explicit scope/reach contract rather than inheriting accidental loop semantics.

An expectation expression is read-only: it cannot draw, observe, score, mutate test state, print, load inputs or run nested inference, directly or through a helper. Pure deterministic helpers and queries of existing recipes are fine. A helper may use its own local variables and deterministic loops; those do not modify the test population. Compute any needed `simulate` result during setup. Diagnostic report expressions in the assertion section have the same restrictions. This boundary is stricter than the current collection-callback restrictions and needs its own transitive analysis and runtime validation where callees are indirect.

### Boolean properties

`expect foo + 1 < 6` checks the predicate in every positive-weight world of the completed test model. It passes only if the predicate is true throughout that population, subject to the completeness rule below. One positive-weight counterexample fails it, however small its weight. Show a representative counterexample and its weight, not just a rounded success percentage.

The final query must describe a boolean property: one aggregate boolean, a boolean per world, or a boolean recipe in each world. Like `report`, the last case integrates the recipe's outcomes; it never draws a pass/fail result. For example, `expect d6 > 0` passes because the recipe has no false outcome. Numbers and `prob` alone are errors; `expect 99%` must not mean “draw a test result.” A probability threshold instead needs a comparison, such as `expect P(foo > 0) == 50%`.

For an all-world/recipe property, retain positive false weight directly. Do not decide it by rounding a success probability to 1 or reading a displayed `100%`. Explicit statistical equality still uses ordinary floating-point comparison of the calculated quantity.

For supported continuous analytic predicates, this is an **almost-sure** property, not a proof about every real point in the support. State that distinction in results: a measure-zero exception is not a positive-mass counterexample. Unsupported analytic predicates fail with the normal capability diagnostic, not a silently sampled replacement.

A failed expectation does not reject its counterexample worlds. Later expectations inspect the same population, including those worlds. Assertion failure is runner metadata, not a model exception that a future `catch` could suppress. Record all evaluable expectation failures rather than stopping at the first false predicate. An error while evaluating an expectation is an error result, not a false value or an observation.

### The expectation is the population boundary

The original spelling works because the whole expectation is a population query:

```probl
test "my test" {
  let foo = if 50% { 1 } else { -1 }
  expect mean(foo) == 0 as "the mean is zero"
  expect foo + 1 < 6
}
```

During setup, `foo` is still an `int` in each world. Inside `expect`, a reference to a binding from that setup is a read-only expression indexed by the test's joint worlds. Literals and fixed module definitions are shared values. There is no first-class population value to pass back into the model, and a binding does not become a scalar merely because enumeration merged all its outcomes or it happens to be deterministic.

Define three query operations:

1. **Pointwise evaluation.** Arithmetic, comparisons, field/index access, collection construction and ordinary pure helper calls operate on corresponding values from the same world. Shared scalar arguments broadcast to each world. Combining two world-indexed expressions aligns them by world; it does not form an independent Cartesian product.
2. **Population reduction.** Statistical queries consume a world-indexed expression and return one shared result. For complete finite worlds, `mean(foo)` is `sum(w_i * foo_i) / sum(w_i)`. `P(foo > 0)` similarly sums the true weight and divides by total weight. These are posterior weights after test observations, not one vote per stored world.
3. **Checking.** A shared boolean is checked once. A remaining world-indexed boolean property must hold throughout the population. A mixture such as `expect foo <= mean(foo)` broadcasts the aggregate mean back into the read-only query, then checks the comparison in each world. It does not feed the mean into model execution.

For example, preserve relationships before reducing them:

```probl
test "a captured relationship preserves correlation" {
  let x ~ d6
  let y = x
  expect x == y
  expect mean(x - y) == 0
  expect P(x == y) == 1
}
```

Do not implement this by independently turning each captured name into an ordinary `dist`: that would make `x == y` compare independent rolls. Keep a shared joint source and build pointwise query expressions before reducing. `mean(abs(x - y))` is therefore well-defined, as is a centered statistic such as `mean((x - mean(x))^2)`. The query expression has dependencies between reductions, but it never creates a new model branch or revises evidence.

Distribution recipes retain their ordinary meaning within a world. If setup binds `let d = d6`, `d == d` still compares independent recipe outcomes; if it binds `let x ~ d6`, `x == x` compares the same realized outcome. Reducing a recipe-valued expression integrates its outcomes with each world's weight, consistently with numeric reports; it does not sample the recipe. Thus `mean(d)` and `mean(d6)` both give 3.5 for that deterministic recipe. Incomplete recipes retain missing-mass metadata. Supported analytic expressions use existing inference capabilities; unsupported joint/nonlinear operations remain errors.

Start with the existing population query families (`mean`, `variance`, `sd`, medians, quantiles, `P`, CDF/PMF and support) where their types and completeness contracts can be preserved. Parameter arguments such as a percentile or comparison threshold must be shared scalar query results initially; reject a world-varying threshold rather than silently choosing one. Methods and statically known aliases of these built-ins must resolve to the same query operation; identify the resolved definition, not its spelling. World-varying callable targets that could select a reducer are unsupported initially.

Ordinary helper functions retain ordinary bodies. Calling a pure helper with world-indexed arguments lifts the call pointwise; it does not make a hidden `mean` inside that function reach into the caller's population. A user function shadowing `mean` is that user function, not a reducer selected by name. This keeps imported helpers independent of whether their caller is a test. A future API for user-defined population reducers is separate work.

Collections require an explicit rule, not automatic flattening. A captured `xs` is a population of lists; `mean(xs)` in this query context cannot average lists and should give a targeted diagnostic. Compute each world's list mean during setup when that is the intended assertion. For example:

```probl
test "list mean" {
  let xs = [1, 2, 3]
  let actual = mean(xs)          # ordinary list statistics, in model setup
  expect actual == 2
  expect mean(actual) == 2       # mean across the test population
}
```

This makes the boundary visible even when it changes the argument's interpretation:

| Expression and context | Meaning |
|---|---|
| `mean(foo)` outside `expect`, where `foo` is a scalar | Type error, as today |
| `mean(foo)` inside `expect`, where `foo` comes from setup | Mean across test worlds |
| `mean(xs)` inside `expect`, where `xs` is a setup list | Error: the population elements are lists; compute per-world list means during setup |
| `mean([1, 2, 3])` inside `expect` | Ordinary query of a shared literal collection, 2 |
| `mean(d6)` | Ordinary recipe mean, 3.5 |
| `P(foo > 0)` inside `expect` | Probability of the fact across test worlds |

The cost of the concise syntax is a small query language inside `expect`, with diagnostics and editor hints that explain its world-indexed operands. That is preferable here to a `population(...)` wrapper on every aggregate, because the user has explicitly chosen a cross-world assertion. It does not make scalar statistical queries valid in ordinary code, change the existing `report mean(foo)` behavior, or require implementing general portals now. The list and helper rules above are proposed details to validate before implementation.

### Empty, incomplete and faulty models

A green test needs actual coverage. Require at least one expectation and a nonempty, positive-weight final population. Reject a test with no expectations during checking. Impossible evidence is an error, not vacuous success; report its location and available evidence diagnostics. A runtime failure before the assertion section fails the test, even if some worlds could still complete.

The proposed [error-handling policy](error-handling.md) must not let `@on_error continue` turn a partially faulty model into a passing test. If continuation is later supported by the test runner, it may collect more diagnostics, but any unhandled model fault still makes the test unsuccessful. Explicitly caught, modeled alternatives can be tested normally.

Start conservatively with incomplete inference: any positive unresolved mass prevents a pass based only on resolved outcomes, even if it prints as `< 1e-12`. A known counterexample can establish failure; otherwise the result is **inconclusive**, which is non-success in CI. In particular, `expect mean(x) == target` must not silently use only the resolved part and pass. Enforce this at the test boundary and for incomplete distributions consumed by assertions; some current descriptive query functions intentionally summarize only resolved outcomes.

This metadata must survive setup too: `let m = mean(incomplete_recipe); expect m == target` cannot launder an incomplete summary into a passing assertion. Track whether an executed summary depended on unresolved outcomes, conservatively marking that test inconclusive initially. Likewise, an incomplete result from nested `simulate` must not disappear merely because setup stored a scalar derived from it. A known failing predicate with no such dependence can still establish failure.

Later, allow an expectation to pass when a sound enclosure proves it for all unresolved completions. A probability threshold can sometimes be decided from bounds; an unrestricted mean generally cannot. This is a separate capability, not permission to reuse a display-rounding threshold as proof. Tests of unbounded models such as craps will need this work, a genuinely bounded test case, or explicitly sampled checks.

### Numerical comparison and sampling

`==` keeps ordinary equality. Do not introduce an invisible assertion epsilon or compare rendered report strings. Symmetric two-outcome enumeration can yield exactly zero, but general floating-point calculations need an explicit tolerance:

```probl
expect abs(mean(foo) - target) <= 1e-10 as "mean within tolerance"
```

Tests can define a reusable pure comparison helper when relative and absolute tolerances are both needed. A tolerance for floating-point arithmetic is not a confidence interval for sampling and is not an allowance for unresolved model mass.

The first runner should use enumeration, including currently supported analytic operations, with explicit budgets and no automatic fallback. Reject sample-mode requests clearly until sampled test semantics are implemented. No need to delay module imports or deterministic/enumerated tests for this extension.

For a later sampled runner, distinguish two questions:

- **Empirical property checks:** a predicate held in all attempted completed runs. A pass must say “sampled check,” including runs and seed; it is not a proof of probability one. Failed/rejected runs cannot be replaced until enough successes appear.
- **Population claims:** a mean or probability satisfies a bound. Require an explicit estimation/confidence contract, including tolerances, low effective sample size, multiple expectations and inconclusive outcomes. A fixed seed aids replay but does not establish the claim or remove false positives/negatives.

Do not silently interpret `expect mean(foo) == 0` as “the estimated mean is sufficiently close.” The sample-query representation must retain uncertainty; wrapping observed samples in an apparently exact `dist` and discarding estimator metadata would be misleading. A simpler empirical-only test mode is possible, but must say explicitly that expectations address the sampled population.

## Runner, hosts and diagnostics

Discover only `*.test.probl`, in deterministic relative-path order, within explicitly selected files/directories. A no-argument `probl test` can search the current project directory with documented exclusions for `.git`, build outputs and dependencies. Do not follow directory symlinks during discovery; canonicalize/deduplicate explicitly named test roots. Importing a module does not discover or run adjacent tests. A filter matching no tests is non-success by default, with a deliberate allow-empty option if later needed.

`probl run file.test.probl` should direct the user to `probl test`. `probl check` may validate a test file's syntax, imports and declarations without executing its expectations, and must say that tests were not run. Tests in ordinary `.probl` files are errors rather than silently omitted blocks.

Test results should distinguish **pass**, **assertion failure**, **execution/compile error**, and **inconclusive**. Only an all-pass selected suite exits successfully. Continue independent tests after ordinary assertion/model errors; report unrun tests when compilation dependencies, cancellation or host failures prevent execution. Do not classify every unrun test as a passing or failing expectation.

An assertion result should retain test identity, source span, optional label, original expression, actual values/query results, evidence/reach basis, completeness and bounded representative counterexamples. Compare typed values, not text output. Capture `print` and report output separately per test, showing it on failure or with a verbose flag. Avoid materializing every world or dumping all failures; limits apply to capture support, diagnostics and total suite work as well as individual tests.

The library should expose compilation of a test suite and structured test results, using the same source resolver and semantics as the CLI. A resolver-backed compile entry point can be added alongside the current single-source `compile(name, source)`; the latter should remain filesystem-free and report that an unresolved import needs source resolution. A compiled program/suite owns its complete source snapshot, and repeated execution does not reread source files.

The playground can begin with a host-supplied virtual source map. A later multi-file editor should use the same resolver, source identities, file-kind validation and diagnostics; it must not gain arbitrary URL loading. Code navigation and highlighting need to understand imported definitions and test-only syntax rather than presenting either feature as a CLI-only convenience.

## Implementation implications and order

The current [AST](../../crates/probl-syntax/src/ast.rs) and [parser](../../crates/probl-syntax/src/parser.rs) already contain an import-path placeholder; [lowering](../../crates/probl-sema/src/lower.rs) explicitly rejects it. Cycle detection itself is a small graph task. The important work is making a multi-file program preserve existing semantics:

- [Spans](../../crates/probl-syntax/src/span.rs) currently contain only byte offsets, and the public [Program](../../crates/probl/src/program.rs) retains one source file. Add source identities and a source map, or an equivalent explicit mapping, throughout compile/runtime diagnostics, report labels, editor symbols and snippets. Concatenating files without a source mapping is insufficient.
- Name resolution currently registers functions/types in one compilation context. Give each module a lexical namespace, resolve exports/import aliases to declaration identities, and remap IDs once when linking. Preserve captures, effect analysis, liveness, draw scheduling, recursive calls and memoization.
- Current [effect analysis](../../crates/probl-sema/src/effects.rs) primarily propagates evidence and print effects. Module initializers and expectation expressions need a stronger check for draws, mutation, input access, nested inference and indirect calls. “No print and no observe” does not mean deterministic.
- Tests need independent entry functions/settings and a final expectation query plan. Track shared versus world-indexed expressions, pointwise operations and reduction dependencies. Capture only referenced values/projections with correct weights; preserve their liveness through setup and maintain joint expressions until after projection. Assertion sites must survive optimizations, including assertions of deterministic false expressions.
- Reuse structured distribution/statistic calculations, but preserve completeness information and do not base test results on rendered reports. A test runner written on top of today's report text would inherit rounding and conditional-population ambiguities.

Recommended order:

1. Multi-source resolver, identities, source maps and graph/limit diagnostics.
2. Export/import name resolution, declarative module validation and initialization contracts.
3. Isolated enumerated tests with terminal boolean expectations and structured results.
4. Population query reductions, including joint-value, shared/world-indexed operand and completeness checks; these can ship with stage 3 if ready.
5. Native/WASM/editor integration and regression coverage; only then design sampled checks and richer assertion scopes.

Modules and basic tests do not require the deferred portal implementation or local `try`/`catch`. They should reuse future population/error metadata where helpful, without pretending those features already exist. Tests expecting particular runtime faults can later build on stable fault codes; do not begin by matching English error strings or treating any error as a successful negative test.

## Acceptance cases

| Area | Required regression cases |
|---|---|
| Graph loading | Direct/transitive imports, diamond reuse, self/cyclic imports with traces, alias paths, symlinks, missing sources, malformed UTF-8, depth/count/total-byte limits |
| Boundaries | Private export rejection, aliases/collisions, qualified enum variants, record constructors, private helpers/captures, public signatures, same declaration through two paths |
| Initialization | Recipes accepted; hidden draws/evidence/print/read/`simulate` rejected; forward value dependencies rejected; unused invalid dependencies diagnosed; no shared mutable state |
| Semantics across files | Matching inline/imported models under merging, memoization, recursion, sampling and evidence; correct source locations and cache invalidation after transitive edits |
| Test grammar/discovery | Test-only source mode, no imported tests, duplicate names, optional labels, empty suite/filter, no expectation, illegal nested/late model statements |
| Isolation | Mutable collections/bags, observations, captures, input snapshots, date, budgets and output isolated across tests; order does not affect results |
| Expectations | Non-boolean rejection, all-world/recipe predicates, tiny positive counterexample weight, failed expectations do not filter, all expectations share the final population |
| Statistics | Symmetric mean, weighted posterior, scalar/list/recipe distinctions, correlated projection, aliases and user-function shadowing, reducer dependencies, incomplete summaries from setup, no implicit recipe draws |
| Non-success | Impossible evidence, runtime errors, later test execution, unresolved mass even below display precision, assertion evaluation errors, cancellation and internal failures |
| Hosts | CLI exit status, structured library results, in-memory imports, actual WASM parity, source navigation and bounded diagnostics |

Use small models with independent hand/rational answers, not only snapshots of the engine's own report output. Extend the [independent oracle](../../crates/probl-oracle/src/lib.rs) to check weighted predicates and projections in its supported subset. These user-facing tests complement the [Rust/oracle/host suites](../testing.md); they cannot replace independent checks of Probl itself.

## Relevant precedents

- **Zig** uses named `test` declarations and an `expect` helper, showing that this vocabulary works without BDD nesting. Its tests can live alongside normal code and are omitted outside test builds; Probl's separate-file restriction would be a deliberate difference. [Zig 0.15.2 language reference](https://ziglang.org/documentation/0.15.2/#Zig-Test).
- **Rust** treats integration tests as separate consumers of the library's public API, while in-module tests can access private items. The separate-file proposal follows the former visibility model initially; private-testing privileges would need a separate decision. [Rust test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html).
- **ECMAScript** provides explicit named imports/exports and aliases, and specifies a substantially richer cyclic-module lifecycle. Borrowing the explicit dependency spelling does not require Probl to adopt live mutable exports, module execution or cyclic initialization. [Import syntax](https://tc39.es/ecma262/multipage/ecmascript-language-scripts-and-modules.html#sec-imports), [cyclic module records](https://tc39.es/ecma262/multipage/ecmascript-language-scripts-and-modules.html#sec-cyclic-module-records).

The key Probl-specific additions are weighted assertion semantics, preservation of joint relationships, and an honest distinction between a checked population, unresolved inference and sampled evidence.
