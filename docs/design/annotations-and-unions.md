# Type annotations: checks, conversions and unions

> Draft proposal, 13 October 2026. Nothing here is implemented. Today's behavior is in the [reference semantics](../semantics.md#1-values-and-types). The examples marked as proposed don't run yet.

## Recommendation

1. **Annotations check; they don't convert.** A value given a declared type must already have that type, or the run stops with an error that says how to convert it. Today an annotation also converts: `let p: prob = 33%` turns a float into a probability.
2. **Conversion is explicit.** `prob(x)` already exists. Add `int(x)`, which converts exactly as today's implicit conversion does, and `float(x)`.
3. **`float` accepts any real number.** An int or a probability is accepted where a float is declared, unchanged, as Python's type checkers accept an int where a float is declared.
4. **Built-in operations and data are unchanged.** `bernoulli(30%)`, `if 30%`, `score 95%`, `chance` weights and `quantile(d, 50%)` keep accepting numbers where they expect a probability: that's what those operations accept, not an annotation. Reading data into a declared type keeps parsing and converting, since data is text.
5. **Then add unions, `A or B`.** A value must have one of the types. With no conversion, there is nothing to choose between, so the order of the types doesn't matter and no union is ambiguous.

The measured cost is small: annotations convert in one example and two benchmarks, outside the tests written to check conversion ([measurement](#what-relies-on-conversion-today)). It's a breaking change, best made before 1.0.

## The question

A function can already return different types in different worlds:

```probl
fn foo() {
  if 50% { return "A" } else { return 1 }
}
let a = foo()
report typeof a        # int 50.00% · str 50.00%
```

That isn't a value of a union type: in each world `a` has exactly one type, and the worlds differ. It works without an annotation, and nothing here changes it. What's missing is a way to write the annotation: `fn foo(): str or int`.

Unions are hard to add because annotations convert today:

```probl
let a: prob = 50%
let b = 50%
report typeof a        # prob
report typeof b        # float
```

So what should this return?

```probl
# Proposed syntax.
fn fifty(): prob or float {
  return 50%
}
```

If annotations convert, the union must choose: keep the float, or convert it to a probability. Whatever the rule, `int or prob` has a value that could become either type: `1.0`. If annotations only check, there's no choice to make. `50%` is a float, `float` is one of the types, and `fifty()` returns the float.

The same applies to bindings, record fields, parameters and the contents of typed containers, with or without functions.

## How other languages do it

Annotations that convert values are unusual:

- **TypeScript and Python** annotations have no effect when the program runs: TypeScript erases them, and Python ignores them. A checker reads them before the program runs. Probl's annotations already do more, since they're checked when the program runs.
- **Kotlin and Rust** have no implicit numeric conversions at all: `val d: Double = 1` and `let d: f64 = 1;` are errors.
- **Swift, Go and Haskell** let an annotation choose the type of a literal, so `let d: Double = 1` works in Swift. They never convert an existing value: `let d: Double = i`, for an int `i`, is an error.
- **C, Java and C#** widen implicitly, from int to double for instance, but never narrow with a check, as probl's float to int does.
- **Julia** is the close precedent. A declared variable or return type converts the value with `convert`, and `2.5` into an `Int` throws an `InexactError`, as in probl. Function arguments aren't converted, though: they only select a method.
- **Validation libraries** like Pydantic convert data into declared types, which is what probl's data schemas do.

## What probl does today

Wherever a type is declared (bindings, parameters, return types, record fields, and recursively in lists, maps, bags and distributions), the value is converted:

| Declared | Given | Result |
|---|---|---|
| `prob` | an int or a float in [0, 1] | a prob |
| `int` | an exactly integral float | an int; `2.5` is an error |
| `float` | a prob | a float |
| `float` | an int | the int, unchanged |
| `int` | a prob | an error |

Numeric literals are converted to probabilities during compilation. Other values are converted when the program runs. An expected type also flows into the branches of `if`, `chance`, `match` and blocks: `let x: prob = if c { 30% } else { 1 }` is a prob either way.

Two things stand out. `float` keeps an int as it is but converts a probability, and nothing documents either. And conversion to `prob` changes behavior, not just the type: `~p` draws a trial from a probability but is the number itself for a float, and `p and q` combines two probabilities but is an error for floats. Today an annotation can make that change without anything at the use site showing it.

## What relies on conversion today

Each conversion was logged in the engine and in the compiler, and every example, benchmark and playground test program was run (46 programs; one benchmark, `snakes_three`, was stopped after two minutes):

- **Converting values when the program runs:** only in the two playground test programs written to test it, `web/test/integers.probl` and `web/test/types.probl` (13 places). None in any example or benchmark.
- **Converting literals during compilation:** 213 places, almost all in built-in contexts that the proposal keeps: `if 50%`, `bernoulli(…)`, `quantile(…, 50%)`, `chance` weights. Annotations converted 15 literals in 9 lines, outside the test programs:
  - [`examples/10_quantum_key.probl`](../../examples/10_quantum_key.probl): `photon(0%)`, `kept(0%)` and `kept(100%)`, for parameters declared `prob`, and `let shares: list[prob] = [0%, 10%, 50%, 100%]`.
  - `benches/tennis.probl`: `let a_serve: prob = 64%`, twice.
  - `benches/reliability_staged.probl`: `either(96%, 96%)`, for parameters declared `prob`.

Unit tests weren't counted, since many exist to test conversion.

## The proposal

### Annotations check

A declared type is a contract, checked where it is today: bindings, assignments to declared variables, parameters, results, record fields, and recursively through containers. A value that doesn't have the type stops the run, as a failed conversion does now. It isn't a fault of the world's values, so it does so in both failure modes. Literals that can't have the type are still reported during compilation.

The error says how to convert:

```
Error: `p` should be a prob, but it's a float
  Help: write `prob(…)` to convert it
```

`typeof` of a value with a declared type then always names that type, or, for `float`, an int or a prob.

### Conversion is explicit

| Conversion | Accepts | Notes |
|---|---|---|
| `prob(x)` | a number in [0, 1], a bool, a prob | As today |
| `int(x)` | an int, an exactly integral float | New. What `let n: int = x` does today: it never rounds. Use `floor`, `ceil`, `trunc` or `round` to round |
| `float(x)` | an int, a float, a prob | New. A large int rounds to the nearest float |

All three are ordinary built-ins: they apply to each outcome of a distribution as other numeric built-ins do, and are values like other built-ins: `[0.3, 0.5].map(prob)`.

### `float` accepts any real number

`float` accepts an int, a float or a prob, unchanged. That keeps today's behavior for ints, and stops converting probabilities. Converting an int would round a large one, against the rule that conversions never round, and converting a prob would drop the trial behavior where the program declared no conversion.

### Built-in operations and data

A built-in decides what it accepts, as a function does. `bernoulli(p)` accepts a number in [0, 1] and `factorial(n)` an exactly integral number, as now. So do the language's own contexts: an `if` or `while` condition, a match guard, `score` and `chance` weights. Most probabilities in programs are written there, and none of those change. (`one_of` weights don't convert at all: numbers are relative weights.)

Data schemas still convert. Reading `"0.3"` from a CSV file into a `prob` field is parsing, and there's no value to convert explicitly before the data is read.

### Unions

`A or B or …` is a type wherever a type can be written: `let x: int or str`, `fn f(): prob or str`, `list[int or str]`, `dist[int or str]`, and record fields. `or` reads naturally, and it's how the built-in reference already writes such results, as in `sqrt(x) -> float or complex`.

A value has a union type if it has any of its types. That's checked in each world, as other annotations are:

```probl
# Proposed syntax.
fn foo(): str or int {
  if 50% { return "A" } else { return 1 }    # each world's result is checked
}

fn fifty(): prob or float {
  return 50%                                 # a float: accepted, unchanged
}
```

Since nothing converts, the order of the types doesn't matter, adding a type never changes a value that already fitted, and no value fits in more than one way. A union that one of its types already covers, like `prob or float` (a `float` accepts probabilities), is allowed, and the compiler warns that it's the same as `float`.

`typeof` describes a value, not its declaration, so it never names a union for one value: `typeof fifty()` is `"float"`.

## What changes for programs

| Today | Proposed |
|---|---|
| `let p: prob = 33%` | `let p = prob(33%)` |
| `fn f(p: prob) { … }` with `f(30%)` | `f(prob(30%))`, or declare `p: float` if the body only passes it to built-ins |
| `let n: int = 2.0` | `let n = int(2.0)` |
| `let xs: list[prob] = [0.3, 0.5]` | `let xs = [0.3, 0.5].map(prob)` |
| `let x: float = p`, for a prob `p` | Unchanged, but `x` stays a prob; `float(p)` converts |
| `bernoulli(30%)`, `if 30% { … }`, `score if sick { 95% } else { 8% }` | Unchanged |
| `one_of([Boom: 20%, Slump: 80%])`, `quantile(d, 50%)` | Unchanged |
| Data read into declared types | Unchanged |

## Alternatives considered

**Keep converting, and give unions a rule.** A value that already has one of the types is kept. Otherwise it's converted to the one type it can become, and a union where some type could become two of its types, like `int or prob` for a float, is rejected during compilation. That works: `fifty()` returns the float, `let x: prob or str = 30%` gives a prob. But it keeps conversions invisible where they change behavior, it needs the ambiguity rule, and every new conversion would have to be checked against every union.

**Literals take their type from the annotation**, as in Swift, Go and Haskell. `let p: prob = 33%` and `photon(0%)` keep working, while a variable needs `prob(rate)`. That covers all nine annotation-driven conversions measured above. But probl checks types when the program runs: a call through a function value can't see the parameter types during compilation, so `f(30%)` would work while `let g = f; g(30%)` wouldn't. That's the same kind of difference as the one between calling a function by name and through a reference. This can be added later without breaking any program, so it's better left until `prob(…)` proves too noisy.

**The first type that accepts the value wins.** `prob or float` would give a prob and `float or prob` a float. Order shouldn't matter.

## Open questions

- **Containers in `typeof`.** `typeof [1, "a"]` is `"list[any]"`, and `any` isn't a type that can be written. With unions it could be `"list[int or str]"`, and every `typeof` result would also be a valid annotation. That changes results programs may compare against.
- **`float` of a large int.** It rounds, which an explicit conversion can be allowed to do. Or it's an error, like the other conversions that never round.
- **Built-in integer arguments.** `factorial(5.0)` and `[10, 20].get(1.0, -1)` accept exactly integral floats. That's consistent with "a built-in decides what it accepts", but it means `factorial(5.0)` works while `let n: int = 5.0` doesn't.

## Implementation order and validation

1. Add `int(x)` and `float(x)`.
2. Make annotations check only: the engine's declared-type conversion becomes a check, with the error above, and the compiler stops turning literals into probabilities for annotations, keeping it for built-in contexts.
3. `float` accepts probabilities unchanged.
4. Update the examples, benchmarks, playground tests, the [language overview](../language-overview.md) and the reference semantics (section 1's conversion paragraphs), and record the breaking change.
5. Unions: the grammar (`type { "or" type }`), a union in the compiler's types, and the check.

The oracle (`probl-oracle`) converts declared types too, and changes with the engine in step 2. Its generated programs then check the new behavior against the engine as before.
