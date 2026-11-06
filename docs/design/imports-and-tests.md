# Imports and model tests

> Draft proposal, updated 6 November 2026. Imports and exports are implemented as proposed here ([as built](#as-built), [semantics, section 16](../semantics.md#16-files-and-imports)); `test` and `expect` are proposed features, not executable syntax. This document recommends contracts before implementation; the [reference semantics](../semantics.md) remains authoritative for current behavior.

## Recommendation

Add static imports of explicitly exported definitions, with an acyclic file dependency graph. Definitions (functions, types, enums) are static, as in JavaScript. A module's top level may also hold a model: it's evaluated once per run, before its importers and in dependency order, in the program's worlds, as an ECMAScript module with top-level `await` is evaluated once before the modules that import it. A draw there gives every importer the same outcome in each world, and an observation there is evidence for the whole program, so data in several files combines. Imported bindings are read-only.

Add `*.test.probl` files containing named, isolated `test` blocks and report-like `expect condition as "description"` statements. Tests construct a model, then check its final population. A failed expectation records a failure without conditioning or otherwise changing that population.

Every expectation is a cross-world query, like a report with a condition to check. `expect mean(foo) == 0` computes a population mean, while `expect foo + 1 < 6` checks a property throughout the population. The `expect` keyword establishes the query boundary; no additional `population(...)` wrapper is needed. Ordinary expressions outside that boundary retain their current per-world meaning. The query rules below specify the distinction, including correlation, lists and distribution recipes.

Start with enumerated tests. Sampled tests need a separate contract for empirical checks and statistical uncertainty; do not automatically switch an expensive test to sampling or silently judge an estimated mean as a population truth.

## Imports

### Definitions are static

Suggested first syntax:

```probl
# random_walk.probl — proposed export syntax
let unit = 1
export let step = one_of([-unit, unit])

export fn walk(turns: int): int {
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

Named imports make dependencies visible and allow aliases for collisions. Start with this one import form: no wildcard, implicit import-all, side-effect-only import, default export, dynamic path or package download. A namespace form such as `import "./random_walk.probl" as walk` can be added later; it also requires qualified names in types and record constructors and is not necessary for the first implementation. The currently parsed bare `import "file.probl"` stays rejected with a help message for now; it's reserved for importing a module for its evidence alone ([deferred](#deferred)).

Imports are top-level declarations, resolved before execution. Require them before other declarations and executable statements for readability. They are not allowed inside a function, branch, loop, `simulate` or test body. Test files import at their file level.

Importing doesn't wrap a module in a function. An exported function is a function: `walk(4)` runs in the caller's worlds each time it's called, and draws, observes or prints there, as a local function would. Returning a recipe still doesn't draw it. A recipe exported as a value (`step`) is a value: each `~step` is a fresh draw.

### A module's model runs once, in the program's worlds

A module's top level can draw, branch on chances and observe, as the entry program's can:

```probl
# prior.probl — how often a customer buys
export let rate ~ one_of([0.2, 0.5, 0.8])
```

```probl
# shop_a.probl — one shop's data, and its forecast
import { rate } from "./prior.probl"
observe 4 from binomial(5, rate)
export let next_week ~ binomial(10, rate)
```

```probl
# main.probl — shop_b.probl is shop_a.probl with its own data
import { rate } from "./prior.probl"
import { next_week as a } from "./shop_a.probl"
import { next_week as b } from "./shop_b.probl"

report rate == 0.8
report a + b
```

Each module is evaluated once per run, as a JavaScript module instance is: `prior.probl` runs once, before `shop_a.probl`, `shop_b.probl` and `main.probl`, and `rate` is one value in each world, the same in all four files. Its draw splits the program's worlds as a draw in `main.probl` would. The shops' observations are evidence for the whole program, so both datasets update the same `rate`: `rate == 0.8` is 87.28%, as when the files are written out in one, in evaluation order.

**The rule.** A program is its modules' top levels, each once, in dependency order, followed by the entry file's, each module's names in its own namespace. Splitting a program into modules must give exactly the result of the same code written out in that order.

**Why not like `simulate`.** If a module were evaluated like `simulate`, as its own normalized model, `shop_a.probl`'s observation would condition only its own copy of `rate`, and `shop_b.probl`'s only its own. The two datasets would never combine: `rate == 0.8` would be 71.58% for each copy, from one dataset each, and the program's evidence would leave out the modules' data. Caching such a module would cache a distribution, and each importer would still draw its own outcome. A module that means to export a self-contained posterior says so: `export let posterior = simulate { … }` is a distribution value, which importers draw as they choose.

### Evaluation order and effects

- **Order.** Modules are evaluated depth-first, after their dependencies, following import declarations in source order; a module reached again is reused, so a dependency shared by two modules (a diamond) runs once. This is ECMAScript's module evaluation order, without cycles.
- **In the program's worlds.** A module's statements are ordinary statements of the program: its draws split worlds, its observations and scores weight them, and liveness, draw scheduling and merging apply to them as to any statement, without changing results. Module bindings, including `var`s, have a value in each world.
- **Reports.** Only the entry file reports. Every module runs before the entry's first statement, so every observation in a module comes before every report, and the observe-after-report rule holds across files by construction.
- **`print`** in a module prints once per world, as anywhere.
- **Faults and errors.** A fault in a module follows the run's failure mode, as any statement's does: it stops the run in total mode, and ends its world in partial mode. Other errors stop the run. Diagnostics name the module's file.
- **Snapshots.** `today`, `now` and `env` are the run's, the same in every module.
- **Data.** A module's `read` follows the entry file's rules ([semantics, section 15](../semantics.md#15-data)): its data is read before the program runs, before any world exists, as a constant, the same in every world. The path is relative to the module's file. A module is evaluated once per run, so its data is read once, however many files import it.
- **Sampling.** Modules are sampled as any statement is. `simulate` inside a module always enumerates, as it does today; a sampled `simulate` is later work, not needed for imports.

### What an imported file may contain

Use the `.probl` extension for both scripts and reusable modules; validate a file's contents when it is imported. No new module-file extension or implicit entry function is needed.

| Top-level item in an imported file | First contract |
|---|---|
| Imports | Allowed, subject to the acyclic graph and host resolver |
| `fn`, `type`, `enum` | Allowed; private unless explicitly exported |
| `let` and `var` bindings, private or exported | Allowed, evaluated in the program's worlds |
| Distribution recipes such as `d6`, `one_of(...)`, recipe arithmetic | Allowed as values; construction is not a draw |
| Draws, probabilistic conditions, `chance`, observations and scores | Allowed: they run once per run, in the program's worlds, and observations are evidence for the whole program |
| `simulate` | Allowed: a local model, as anywhere |
| `print` | Allowed |
| `read` | Allowed, with the entry file's rules: read before the run, relative to the module, once per run |
| `report` | Rejected: the entry file reports |
| Invocation settings such as `@mode`, epsilon or failure policy | Rejected; the entry program/test runner controls execution |
| Tests and expectations | Rejected; test files are separate roots and cannot be imported |

These restrictions apply to private definitions as well as exports. A script that reports or sets invocation settings can't be imported as it is; move what's reusable into a module. The RPG duel example could extract `Fighter`, `attack` and `duel` into a module while keeping hero selection and reports in its entry script.

Reject reading a binding before its module has evaluated it, including through a helper; do not introduce forward value initialization or lazy cycles. Functions and type declarations keep their existing forward-reference rules.

Each module has one evaluation per run or isolated test, and nothing is shared across runs or tests: there is no mutable process-wide module state. Compiled declarations may be shared, and identical immutable values may be cached, but optimization must not change results, diagnostics or the host's resource contract.

### Mutation in modules and exported variables

Mutability in the defining scope and permission to modify an imported binding are separate decisions. Module-local mutation does not require writable imports. Recommend allowing both of these:

```probl
# Proposed module syntax.
export var x = 3
```

```probl
# A separate module.
var x = 3
export let y = x
x = 4
```

In the second example, `y` is 3: ordinary assignment captures the value at that statement, not a live alias to `x`. The final private `x` is 4. An exported named function reading `x` after the module has run sees 4; a closure created earlier retains the normal capture-by-value behavior. Exporting a definition must not change either rule.

Likewise, `export var x = 3; x += 1` exposes the final value 4 once the module has run. `var` permits rebinding in its defining module's top level; `export` controls visibility. A module variable has a value in each world, and importers see that world's final value. Neither grants write access in an importer:

```probl
import { x } from "./settings.probl"
x = 5             # error: imported bindings are read-only
```

An importer that wants its own mutable state can write `var local = x`. That binding then follows the ordinary per-world mutation and collection-copy rules, without changing the module or another importer. The same applies to a mutable bag copied from an exported initial value.

This agrees with the existing function rule: functions may read outer variables but cannot assign to them. An exported function cannot become a setter for module variables merely because they were declared with `var`. While the module runs, helper calls read the values available at the call; afterwards, named functions read the module's final values in the calling world. No new global-mutation permission is introduced.

Writable imports would be a separate feature, requiring decisions about aliases across importers and copies at world splits. Such state could be defined per world; it is not inherently impossible. It is unnecessary for either example above, and this proposal does not add it.

### Visibility, captures and identity

`export` may prefix a function, type, enum or `let`/`var` binding. Everything else is private to its defining module. An exported function may call private helpers and read private module bindings under the capture rules above. Its names resolve in its defining module, never against coincidentally named bindings in the importer.

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

Use resolver-provided canonical identities and a visiting/finished graph traversal. Reaching a visiting identity reports the full cycle with import locations. Reaching a finished identity reuses it. A simple visited set would wrongly classify a diamond as a cycle. Reject cycles before evaluating any module, including dependencies whose exports happen not to be used.

Paths are literal UTF-8 strings with an explicit `.probl` suffix, resolved relative to the importing source, not the process's current directory. Start with relative file paths; defer package names, extension guessing, directories-as-modules, URLs and network resolution. `..` is subject to host authority. A native resolver must canonicalize aliases/symlinks consistently and enforce any host-configured root on the resolved target; merely removing `..` is not a sandbox. The existing CLI data resolver's local filesystem policy need not become a claim of confinement.

The source resolver receives both the importing identity and requested path. The data resolver needs the same for a module's `read`, whose path is relative to the module; today it receives only a path relative to the entry file. A host may supply only an in-memory source map, including in WASM. Imports never grant the engine ambient filesystem access; permission to read data is not permission to load it as source, nor the reverse. `probl check --data` checks every module's data against its declared type.

Freeze source bytes and identities for one compilation/test invocation. Repeated loads of an identity must not observe different revisions. Bound individual and total source bytes, module count and dependency depth; modules' evaluation counts against the run's ordinary budgets. Reject oversized graphs cleanly. Canonical identity, not source text equality, determines module identity. Record source snapshots or content digests for replay and cache invalidation, including transitive changes.

### As built

- **Compiler.** `probl_sema::compile_program` loads the entry and its imports through a host `Loader`, depth first, each file once, with a visiting/finished traversal that reports a cycle with its chain. Files share one offset space (a `SourceMap`), so every span says which file it's in. Lowering puts each file's top level in the main function, in evaluation order, with a namespace of its own; imported functions, types, enums and values refer to the exporting file's definitions and slots, and named functions capture their own file's variables by slot, so they read them whichever file calls them.
- **Limits.** At most 1,000 files, 64 imports deep, 8 MiB per file and 32 MiB together.
- **Hosts.** The library's `compile_with` takes a `Sources`: `LocalSources` (canonical paths on disk, `.probl` suffix, no root confinement, as `LocalFiles`) or `MemorySources`. `probl run`, `probl check` and the REPL use `LocalSources`. A `read` in an imported file is rewritten at compile time through `data_path`, relative to that file, so the host's `Files` are unchanged.
- **Not yet.** The namespace form, re-exports, the playground's and WASM API's multi-file sources, editor symbols across files, and tests.

### Deferred

- **A module imported for its evidence alone.** A module that observes data but exports nothing would need a side-effect import. No program needs one yet: a module that observes can export a name its importer uses. When one does, the form is the bare `import "./data.probl"` the parser already recognizes, evaluating the module once, like any import, not including its text.
- **Sampled `simulate`.** A module that exports a `simulate` result enumerates it, as everywhere today. Sampling `simulate` blocks is separate work that modules don't depend on.

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

Outside tests, a test file may contain imports, helper functions/types/enums and fixtures. Its top level follows the [module rules](#what-an-imported-file-may-contain), and, with the modules it imports, it's evaluated afresh for each test: the imported modules first, then the file's top level, then the test body, all in that test's worlds. So a module's model is part of every test that imports it, and nothing is shared between tests. A fixture such as `let fixture = roll + 1`, where `roll` is drawn in an imported module, has a value in each of the test's worlds, correlated with that test's `roll`; whether it's shared in an expectation follows from what it depends on ([below](#what-varies-across-worlds)), not from its having no draw of its own. Fixtures may use `var`, but completed fixtures are read-only to test bodies; a test can copy one into its own local `var`. A test has a fresh environment, weight 1, evidence scope, resource accounting and diagnostic/report sinks. Mutating a local collection or bag in one test cannot affect another. Test return values do not become a distribution in a parent model; there is no parent model population.

This is like `simulate` in isolation, but a runner executes each test as its own entry model. It must not be implemented as a normal call returning a mixture of all tests or as a nested `simulate` that loses assertion sites. Each test has its own pass/fail status. A model error fails that test and normally leaves the runner free to execute other independent tests; cancellation or an internal engine failure may abort the suite.

Data fixtures are loaded explicitly for the relevant test through existing host input mechanisms, before that test executes, with paths based on the test source. Shared immutable input bytes may be snapshotted once, but decoded inputs must fit the compiled test. Capture one `now` per suite, in UTC at the host's precision, with `today` its date, as `probl run` does, and record both in replay metadata. `--now` and `--today` pin them with `probl run`'s rules: `--today` alone means midnight on that date, and the two must agree on the date. CI for time-sensitive tests should pin `--now`; `--today` is enough when only the date matters. If sampling is added, derive independent test seeds from a documented suite seed plus stable relative test identity, so adding or reordering unrelated tests does not shift another test's random stream. Do not use process-random hash functions or absolute checkout paths for that identity.

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

During setup, `foo` is still an `int` in each world. Inside `expect`, a reference to a binding from that setup is a read-only expression indexed by the test's joint worlds. There is no first-class population value to pass back into the model, and a binding does not become a scalar merely because enumeration merged all its outcomes or it happens to be deterministic.

#### What varies across worlds

Whether an expression in an expectation is world-indexed or shared is decided before running, by what it reads, never by the values a run happens to produce.

- **Every binding is a population.** A binding of the test's setup, of the test file or of an imported module is world-indexed, whatever it's computed from: all of them run in the test's worlds. `let xs = [1, 2, 3]` is a population of lists, the same list in every world; a binding of data from `read` is a population too. A binding never becomes a shared scalar because it happens to be the same everywhere.
- **Expressions follow what they read.** An expression is world-indexed when it reads a binding, directly or through a function it calls: through its arguments, a lambda's captures, or the bindings a named function reads, transitively. A call through a function value reads the binding or captures that hold it.
- **Shared** are literals, built-in constants, the run's snapshots (`today`, `now`, `env`), recipe expressions written in the expectation (`d6 > 3`), and calls of built-ins or of functions that read no binding, with shared arguments.

So every expression is classified, and binding a value during setup always gives it population meaning.

For example, with a module that has

```probl
export let roll ~ d6
export fn current() { roll }
```

`current()` takes no argument, but it reads `roll`, so it's world-indexed: `expect mean(current()) == 3.5` averages it across the test's worlds, and `expect current() == roll` compares it with `roll` world by world, the two always agreeing.

**Reducers.** A statistical query applied to a world-indexed argument reduces across the test's worlds, integrating recipe values, as defined below. Applied to a shared argument, it's the ordinary built-in: `mean([1, 2, 3])` is the list's mean, 2; `mean(d6)` is the recipe's, 3.5; `mean(2)` is a type error, as outside a test. Parameters that aren't the reduced argument, such as a quantile's level, don't change the query's meaning: a world-indexed one is accepted when it has the same value in every world, checked when it's evaluated, so `let level = 90%` in setup works.

#### Query operations

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

Start with the existing population query families (`mean`, `variance`, `sd`, medians, quantiles, `P`, CDF/PMF and support) where their types and completeness contracts can be preserved. Parameter arguments such as a percentile or comparison threshold must have one value across the test's worlds ([above](#what-varies-across-worlds)); reject a world-varying threshold rather than silently choosing one. Methods and statically known aliases of these built-ins must resolve to the same query operation; identify the resolved definition, not its spelling. World-varying callable targets that could select a reducer are unsupported initially.

Ordinary helper functions retain ordinary bodies. A call of a pure helper is world-indexed when an argument is, or when the helper reads a world-indexed binding through its captures or free variables ([above](#what-varies-across-worlds)); it's then evaluated pointwise. It does not make a hidden `mean` inside that function reach into the caller's population. A user function shadowing `mean` is that user function, not a reducer selected by name. This keeps imported helpers independent of whether their caller is a test. A future API for user-defined population reducers is separate work.

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
| `mean(unit_list)` inside `expect`, where `unit_list` is a module's constant list | Error, as for a setup list: every binding is a population |
| `mean(current())` inside `expect`, where `current` reads a module binding | Mean across test worlds |
| `mean([1, 2, 3])` inside `expect` | Ordinary query of a shared literal collection, 2 |
| `mean(2)` inside `expect` | Type error, as outside a test |
| `mean(d6)` | Ordinary recipe mean, 3.5 |
| `P(foo > 0)` inside `expect` | Probability of the fact across test worlds |

The cost of the concise syntax is a small query language inside `expect`, with diagnostics and editor hints that explain its world-indexed operands. That is preferable here to a `population(...)` wrapper on every aggregate, because the user has explicitly chosen a cross-world assertion. It does not make scalar statistical queries valid in ordinary code, change the existing `report mean(foo)` behavior, or require implementing general portals now. The list and helper rules above are proposed details to validate before implementation.

### Empty, incomplete and faulty models

A green test needs actual coverage. Require at least one expectation and a nonempty, positive-weight final population. Reject a test with no expectations during checking. Impossible evidence is an error, not vacuous success; report its location and available evidence diagnostics. A runtime failure before the assertion section fails the test, even if some worlds could still complete.

The proposed [error-handling policy](error-handling.md) must not let `@on_error partial` turn a partially faulty model into a passing test. If partial execution is later supported by the test runner, it may collect more diagnostics, but any unhandled model fault still makes the test unsuccessful. Explicitly caught, modeled alternatives can be tested normally.

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
- Link modules into one entry program: module top levels become a prologue in evaluation order, each with its own namespace and source identity, so liveness, draw scheduling, merging and memoization apply across files unchanged.
- Current [effect analysis](../../crates/probl-sema/src/effects.rs) primarily propagates evidence and print effects. Expectation expressions need a stronger check for draws, mutation, input access, nested inference and indirect calls. “No print and no observe” does not mean deterministic. Module top levels and a test file's top level don't need it: they run as model code. The expectation rules also need a dependence analysis, to tell world-indexed expressions from shared ones through captures and module bindings.
- Tests need independent entry functions/settings and a final expectation query plan. Track shared versus world-indexed expressions, pointwise operations and reduction dependencies. Capture only referenced values/projections with correct weights; preserve their liveness through setup and maintain joint expressions until after projection. Assertion sites must survive optimizations, including assertions of deterministic false expressions.
- Reuse structured distribution/statistic calculations, but preserve completeness information and do not base test results on rendered reports. A test runner written on top of today's report text would inherit rounding and conditional-population ambiguities.

Recommended order:

1. Multi-source resolver, identities, source maps and graph/limit diagnostics.
2. Export/import name resolution, module evaluation as a prologue in dependency order, and read-only import bindings.
3. Isolated enumerated tests with terminal boolean expectations and structured results.
4. Population query reductions, including joint-value, shared/world-indexed operand and completeness checks; these can ship with stage 3 if ready.
5. Native/WASM/editor integration and regression coverage; only then design sampled checks and richer assertion scopes.

Modules and basic tests do not require the deferred portal implementation or local `try`/`catch`. They should reuse future population/error metadata where helpful, without pretending those features already exist. Tests expecting particular runtime faults can later build on stable fault codes; do not begin by matching English error strings or treating any error as a successful negative test.

## Acceptance cases

| Area | Required regression cases |
|---|---|
| Graph loading | Direct/transitive imports, diamond reuse, self/cyclic imports with traces, alias paths, symlinks, missing sources, malformed UTF-8, depth/count/total-byte limits |
| Boundaries | Private export rejection, aliases/collisions, qualified enum variants, record constructors, private helpers/captures, public signatures, same declaration through two paths |
| Module evaluation | Each module once per run, after its dependencies and before its importers, a diamond's shared dependency once; a module's draw the same for every importer in each world; observations in several modules combining (the 87.28% example); private/exported `var` with a final value per world, value snapshots versus later assignments, existing closure captures, read-only imports and independent mutable copies; `simulate` and `print` as in the entry file; `report` and invocation settings rejected; a module's `read` once per run, before it, relative to the module, the same in every world; faults under total and partial modes; forward value dependencies rejected; nothing shared across runs or tests |
| Semantics across files | Splitting a program into modules gives exactly the result of the same code written out in evaluation order, under merging, memoization, recursion, sampling and evidence; correct source locations and cache invalidation after transitive edits |
| Test grammar/discovery | Test-only source mode, no imported tests, duplicate names, optional labels, empty suite/filter, no expectation, illegal nested/late model statements |
| Isolation | Mutable collections/bags, observations, captures, input snapshots, `now` and `today`, budgets and output isolated across tests; imported modules and file-level fixtures evaluated afresh per test, a fixture derived from an imported draw correlated with it; order does not affect results |
| Expectations | Non-boolean rejection, all-world/recipe predicates, tiny positive counterexample weight, failed expectations do not filter, all expectations share the final population |
| Statistics | Symmetric mean, weighted posterior, scalar/list/recipe distinctions, correlated projection, every binding a population whatever it's computed from (`xs`, `actual`, module constants), world dependence through captures, free variables and module bindings (`current()` above), reducers on shared arguments as ordinary built-ins, a constant world-indexed parameter accepted and a varying one rejected, aliases and user-function shadowing, reducer dependencies, incomplete summaries from setup, no implicit recipe draws |
| Non-success | Impossible evidence, runtime errors, later test execution, unresolved mass even below display precision, assertion evaluation errors, cancellation and internal failures |
| Hosts | CLI exit status, structured library results, in-memory imports, actual WASM parity, source navigation and bounded diagnostics |

Use small models with independent hand/rational answers, not only snapshots of the engine's own report output. Extend the [independent oracle](../../crates/probl-oracle/src/lib.rs) to check weighted predicates and projections in its supported subset. These user-facing tests complement the [Rust/oracle/host suites](../testing.md); they cannot replace independent checks of Probl itself.

## Relevant precedents

- **Zig** uses named `test` declarations and an `expect` helper, showing that this vocabulary works without BDD nesting. Its tests can live alongside normal code and are omitted outside test builds; Probl's separate-file restriction would be a deliberate difference. [Zig 0.15.2 language reference](https://ziglang.org/documentation/0.15.2/#Zig-Test).
- **Rust** treats integration tests as separate consumers of the library's public API, while in-module tests can access private items. The separate-file proposal follows the former visibility model initially; private-testing privileges would need a separate decision. [Rust test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html).
- **ECMAScript** provides explicit named imports/exports and aliases. Its modules are evaluated once, after their dependencies and before their importers, and a module with top-level `await` makes its importers wait for its result. Probl adopts that evaluation model, with a draw in the role of `await`: importers see its outcome, the same in each world. It doesn't adopt live mutable exports or cyclic initialization. [Import syntax](https://tc39.es/ecma262/multipage/ecmascript-language-scripts-and-modules.html#sec-imports), [cyclic module records](https://tc39.es/ecma262/multipage/ecmascript-language-scripts-and-modules.html#sec-cyclic-module-records).

The key Probl-specific additions are weighted assertion semantics, preservation of joint relationships, and an honest distinction between a checked population, unresolved inference and sampled evidence.
