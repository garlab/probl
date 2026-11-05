# Static checking and explanations

> Draft proposal, 5 November 2026, revised after review the same day. Priority 2 of the [development priorities](../architecture.md#priorities). Nothing here is implemented. It proposes what the compiler should reject or warn about before a program runs, in which order to build it, and how to measure it. The survey below was taken at commit `befb69f`.

## Recommendation

1. **Report only definite mistakes that certainly run as errors.** An error before running means the construct fails every time it's evaluated, whatever the values, and that it's evaluated whenever the program runs. A definite mistake in code that might not run is a warning; anything less certain is nothing. A program that runs today never gains an error.
2. **Start with kinds, not full types.** The first slice tells facts (settled values), recipes (distributions) and continuous recipes apart, with the scalar types needed to say why. That covers the mistakes the language makes easiest to write: `P` of a fact, `match` on a distribution, arithmetic on an undrawn `normal`, spreading a distribution.
3. **Describe built-ins once, as data.** A typed signature table in `probl-sema` drives the checker, and the reference's signature strings are generated from it or tested against it, so documentation and checking can't drift apart.
4. **Then types, then effects.** Scalar and collection types catch the largest group of runtime errors. Effects come third, mostly as warnings: a callback that draws only fails if it's called, which depends on the operation and the collection's size, and draws inside `simulate` don't count.
5. **Explain on the error path first.** The world limit and lost conjugate updates get explanations where they happen: when a run hits the limit, and in `--stats`. This follows stage 0 of [error handling](error-handling.md): nothing is added to a run that succeeds.
6. **Measure with the test corpus.** Keep the survey below as a test. Each stage states which runtime errors it must now catch before running; the programs that run today must still compile without a new error, and their new warnings are listed and reviewed.

## The problem

Mistakes about kinds and types surface only when the program runs. The README says so ("some type errors are detected only at runtime"), and the compiler has no pass that infers the type or kind of an expression: `probl-sema`'s `types.rs` only canonicalizes declared unions. These pass `probl check` today and fail when run:

```probl
let x ~ d6
report P(x > 3)                 # `P` expects a boolean distribution (dist[bool]), found a bool

report normal(0, 1) * 2         # `*` on an undrawn normal distribution isn't supported yet

let d = d6
match d { 1 => report 1, _ => report 2 }   # `match` needs a settled value, not a distribution over ints

report [...d6]                  # can't spread a distribution
```

Each is visible in the source: `x` is drawn, so `x > 3` is a fact in each world; `normal(0, 1)` is a continuous recipe; `d6` is a recipe whatever value it would take. When sampling, such an error can surface only after the runs before it, and a model's author learns about it last.

### Survey: which runtime errors are decidable before running

Method: every string literal in the `probl-engine`, `probl` and `probl-cli` tests that looks like a program (2,142 of them) was compiled and run in enumeration, with a two-second limit. 1,129 ran, 532 didn't compile, and 481 stopped with a runtime error:

| Runtime outcome | Programs |
|---|--:|
| A language error that isn't a fault | 272 |
| A fault, which depends on values (division by zero, index, domain) | 119 |
| An unsupported feature, mostly analytic enumeration's limits | 70 |
| A resource limit | 20 |

The 272 non-fault language errors, by what deciding them before running would need:

| Category | Programs | Decidable? | Examples |
|---|--:|---|---|
| Types of operands, arguments and declarations | 127 | yes, with types | `` `binomial`'s number of trials needs an int, found the float 1.4``; `` `m` should be a map, but it's a list[int]``; `can't add an int and a str` |
| Callback effects | 27 | as warnings (below) | `the function given to map can't branch on chances, draw values or observe` |
| Kinds: distribution or value | 20 | yes, with kinds | the four above; `support` of a continuous distribution |
| Arity and call shape through a function value | 19 | yes, with function values | `let f = round; f(1, 2, 3)`; `sort(abs)` |
| Use before a value | 1 | yes | `` `x` has no value here`` |
| Values or data | 36 | no | a weight that isn't finite, chances above 100%, an empty `reduce`, a date out of range |
| The model's behavior | 26 | no | impossible evidence, a loop some worlds never leave |
| The host | 16 | no | a program reading data that wasn't loaded |

So 194 of the 272 (71%) are decidable from the source with the analyses in this proposal, 27 of them as warnings, and 78 aren't. The corpus is biased: tests are written to reach each runtime check, so these are counts of checks, not of mistakes in real programs. It's still the right regression corpus, since every runtime check that could move earlier is in it. Arity of direct calls is already checked during compilation (`round(1, 2, 3)`, `f(1, 2)`); the 19 above call through a value. Faults and unsupported features are out of scope, except that a fault whose operands are literals (`1 / 0`) can be a warning ([below](#the-contract)).

## What exists

- **Compile-time checks** cover names, syntax structure, patterns, pragmas, collection literal kinds, declared literals (`let n: int = 2.0`, `no float holds 9007199254740993`), arity of direct calls, the observe-after-report rule and `catch` names. Warnings exist (`this value isn't used`, `this catch never runs`) and reach the CLI and the playground's editor.
- **Built-in metadata** (`probl-sema/src/builtins.rs`): name, arity, whether a built-in lifts over distributions or receives them whole (`Lifting::Lift` or `Raw`), whether it calls functions, which parameters are probabilities. Signatures with types exist only as text, for hover and the reference (`docs.rs`: `binomial(n: int, p: prob) -> dist[int]`).
- **Effects** (`effects.rs`): whether each function may observe or print, conservatively, with any closure counted as possibly any lambda. It doesn't track draws or branching on chances.
- **Liveness and conjugacy** (`liveness.rs`, `conjugate.rs`): which variables are live where, and which conjugate priors can be delayed and until which statement.
- **Statistics**: `--stats` prints peak worlds, world-steps and calls, and for each delayed conjugate variable its line, draws delayed, observations and how often it was drawn. Neither says why a variable was drawn early or which statement multiplied the worlds.

## The contract

**Definite mistakes.** A construct is a definite mistake when it fails every time it's evaluated, for every value its operands can have. `P(x > 3)` with `x` drawn is one: `x > 3` is a fact in every world, whatever `x` is. `P(c)` where `c` is a parameter without a declared type isn't: nothing is known about `c`.

**Errors only where it certainly runs.** A definite mistake is an error only when it's certainly evaluated whenever the program runs: in a statement of the main program that isn't inside a conditional branch, a `match` arm, a loop's body, a `try`, or the right of `and` or `or`; or in the body of a function that such a statement certainly calls, under the same conditions there. Anywhere else it's a warning. `if false { report 1 + "a" }` runs today, and keeps running, with a warning. So a program that runs today never gains an error: if it ran, it never evaluated its definite mistakes. (If no world gets as far as the mistake, an earlier error, fault or impossible evidence already stops the run.) The compile-time checks of declared literals (`let n: int = 2.0`) stay as they are: they're already part of the language.

**Faults are recoverable.** A fault whose operands are literals, like `1 / 0`, is a warning at most, never an error: `try` catches faults, also from a called function (`fn f() { 1 / 0 }` is caught by `try { f() } catch DivisionByZero { 0 }` in its caller), and partial mode lets the other worlds finish. There's no warning when a `try` in the same function catches that fault: `report try { 1 / 0 } catch DivisionByZero { 42 }` is valid and intended. Kind and type mistakes aren't faults, and `try` doesn't catch them, so this only concerns faults.

**Unknown stays silent.** Types and kinds are inferred from literals, declarations, data schemas, built-in signatures, draws (`~` gives a fact), and the bodies of functions. What can't be inferred, such as an unannotated parameter used in several ways, a value read through `env`, or a closure's result, is `unknown`, and nothing about `unknown` is an error. Unions are sets of possibilities: an operation is a definite error only if it fails for each member.

**Runtime checks stay.** Static checks add earlier diagnostics; they don't remove a runtime check. A program the checker accepts must still behave as today, and the two diagnostics should say the same thing.

**Same words.** The static message reuses the runtime one, with the reason the compiler knows: "`P` expects a boolean distribution (dist[bool]); `x > 3` is a fact in each world, because `x` is drawn (line 1)". Help says what to do: `report x > 3` to aggregate across worlds, or `P(d6 > 3)` for a recipe.

**Warnings are few.** A warning is for a construct that's valid but almost always unintended. Each needs a way to say "I meant it" without changing the program's meaning, and must have no false positives in the examples, benchmarks and playground programs.

## First slice: kinds

**The lattice.** For each expression: `fact[T]` (a settled value of type `T`, including lists, ranges and other collections), `dist[T]` (a recipe whose outcomes can be listed, or a count law), `continuous` (an undrawn continuous distribution), `joint[T]` (a `simulate` result whose outcomes include continuous draws' outcomes, like an analytic datetime forecast), `analytic[T]` (a continuous draw's outcome when enumerating, which is a fact for these rules), a union of these, or `unknown`. `T` is one of the scalar types, a collection type or `unknown`; the first slice only needs enough of `T` to tell `bool` from the rest.

**Rules**, from the [semantics](../semantics.md#2-events-and-identity):

- Draws (`let x ~ D`, `~D`) give facts. Distribution expressions (`d6`, `one_of`, `normal(…)`, `simulate`) give recipes, and `let die = d6` binds a recipe.
- A built-in that lifts maps a recipe argument to a recipe result. A raw built-in states, input by input, what it accepts; whether a recipe can be listed doesn't decide it. For example:

  | Query | Accepts | A definite mistake |
  |---|---|---|
  | `P(d)` | `dist[bool]` | a scalar, including a `bool` or `prob` fact |
  | `pmf(d, v)` | listed recipes and count laws; continuous distributions, where it's 0; `joint` forecasts, including analytic datetimes; nonempty lists and ranges | a scalar |
  | `support(d)` | listed recipes and count laws; lists and ranges | a continuous distribution; a scalar |

  Each row is taken from the built-in's runtime dispatch, and tested against it: every accepted form must run, and every form called a mistake must fail at runtime with the matching error.
- Operators lift over listable recipes. Arithmetic on a `continuous` operand is unsupported in both modes: an error, until it's supported (then the rule goes).
- `match`, spread and map keys need facts or collections; `if`, `while` and guards accept `bool`, `prob` and `dist[bool]` (numbers are checked as probabilities at runtime, so they're not an error here).
- Function results are inferred from their bodies, as the join of their `return`s; recursion starts from `unknown`.

**Errors in the first slice**: the kind mismatches the survey lists, decided as above. **Not yet**: anything that needs types beyond `bool` versus non-`bool`, such as "`binomial` needs an int, found a float".

**One possible warning**: a recipe bound with `=` and used more than once in an expression, as in `let die = d6; report die + die`. That's two independent dice, as specified, but it's often meant as one roll. The fix is `let die ~ d6`; the way to say it's meant is to write the recipe out twice. Measure it on the corpus before deciding.

## Built-in signatures as data

Replace the text signatures with a table: parameters with their kinds and types (with type variables, such as `list[T] -> T`), optional and named parameters, whether each lifts, and the result. Generate the reference's signature strings from it, or test that they agree. The checker, hover, completion and the playground's reference then share one description, and the 177 public built-ins become reviewable in one place. The table can grow with each slice: kinds first, then full types.

## Second slice: types

Scalar and collection types, with unions and unknowns, cover the largest group (127 of the 272): int/float and probability contexts (`/` always gives a float, so `let n: int = x / 2` is definite), integer arguments of built-ins, collection kinds, record fields, dates, durations and their dimensions, and complex values in ordered contexts. Function values carry their arity when known, which covers the 19 arity errors through values. This slice needs the lifting rules for distributions and the conversion rules of [annotations and unions](annotations-and-unions.md) stated as checker rules, so it's larger than the first and should follow it.

## Third slice: effects, as warnings

Extend `effects.rs` with draws and branching on chances, as definite ("on every path through the function") as well as possible.

**`simulate` contains its effects.** Draws, branching on chances and observations inside a `simulate` block belong to that local model: they don't make the function around it draw or observe. `[1].map(x -> simulate { let y ~ d6; x + y })` is valid, and stays silent. Printing isn't restricted in callbacks, so it isn't an effect here either.

**A callback is a mistake only if it's called.** The runtime reports a callback that draws, branches on chances or observes when it runs it, and whether it runs depends on the operation. For the main ones:

| Operation | Calls its function |
|---|---|
| `map`, `filter`, `count`, `any` | once per element, so at least once on a nonempty collection |
| `sort`, `sort_desc`, `minimum`, `maximum` with a comparator | only with two or more elements |
| `reduce` | with two or more elements, or with one and an initial value |

`[1].sort((a, b) -> { let x ~ d6; x })` and `[1].reduce((a, b) -> { let x ~ d6; x })` run today, since neither calls its function, and so does `[].map(x -> pick())`. A collection's size is known only for literals and ranges with literal bounds.

**Severity.** An error needs all of: the call certainly runs (as [above](#the-contract)); its function is certainly called, by the table and a known size; and the function draws, branches on chances or observes on every path, outside `simulate`. When the function does so on every path but it might not be called, it's a warning. When it might do so, nothing. This covers the 27 callback errors, mostly as warnings, without rejecting a program that runs.

## Explanations

**State growth.** When enumeration hits `max_worlds`, the error says only how many worlds were too many. Add, on that error path only: the statement where the count passed the limit, the variables live there that distinguish the worlds (liveness knows which are live; the worlds' values show which differ), and the draws they came from. For example: "worlds differ by `hand` (line 5) and `deck` (line 3); `deck` is used again on line 12". This costs nothing on a run that stays within the limit. A per-statement profile of world counts for `--stats` can follow, behind the flag, if the error alone isn't enough.

**Lost conjugate updates.** The compiler's conjugacy analysis already knows which delayed variables are drawn early, at which statement, and which observations follow. `--stats` can say why each one lost exactness: "`p` is used on line 4, before 3 observations (lines 6–8): they weight the runs instead of updating `p`"; or "the observation on line 9 isn't conjugate to `p`'s beta prior". `probl check --explain` can say the same before running. This is information, not a warning: the model is valid, and only the inference is less efficient.

## Tooling

A formatter and a standalone language server come after the first slice. The language server needs positions, hover and completion from `docs.rs`, which the playground already uses, and the checker's diagnostics. The formatter needs the parser to keep comments and blank lines, which is a separate piece of work. Neither changes the language.

## Implementation order and validation

1. **Survey as a test.** Turn the survey into a test over the same corpus: for each program, whether it runs, fails to compile, or fails at runtime, with the category. It fails when a program that ran gains an error, which the rule of [errors only where it certainly runs](#the-contract) excludes by construction, so a failure is a checker bug. It lists the warnings those programs gain, for review, and records which runtime errors moved before running.
2. **Signatures as data, for kinds.** Table and docs agreement test.
3. **Kinds.** Definite kind mistakes, as errors or warnings by where they are; the 20 kind cases caught; zero new errors on the 1,129 programs that run, the examples, the benchmarks and the playground's programs, and zero new warnings on the last three. Native and WASM give the same diagnostics.
4. **Explanations on the error path.** World limit and `--stats` conjugacy reasons, with tests on small models whose answer is known.
5. **Types.** The 127 type cases and 19 arity cases, under the same no-new-errors rule.
6. **Effects warnings**, and the recipe-reuse warning if the corpus supports it.
7. **Formatter and language server.**

Each stage updates the semantics where a check moves before running, the changelog, and the README's "status and limits".

## Open questions

- **Warnings in CI.** Should `probl check` have a `--deny-warnings` option, so a project can treat a definite mistake in a branch as an error?
- **Saying "I meant it".** For warnings: a pragma, a comment, or rewriting the program? Rewriting is preferred where it doesn't change the meaning.
- **Data.** Programs that `read` data have declared schemas, so their inputs' types are known. Should the checker also use the data file when `probl check --data` is given?
- **Cost.** The checker should be linear in the program's size, so the playground can check as you type. The first slice's fixpoint over function results needs a bound for recursion.
