# Type annotations: conversions, literals and unions

> Draft proposal, revised 14 October 2026. Nothing here is implemented. Today's behavior is in the [reference semantics](../semantics.md#1-values-and-types). The examples marked as proposed don't run yet. The first draft proposed annotations that only check; [why that changed](#annotations-that-only-check).

## Recommendation

1. **A declared type is the type the value has.** After `let z: float = 1`, `typeof z` is `"float"`. Until the program converts it explicitly, a value given a declared type reports that type, or one of a union's types.
2. **Close numeric types convert automatically. The type changes, the number doesn't.** An int becomes a float when a float holds the same number, and a probability becomes a float. A number in [0, 1] becomes a probability, as it already does in `if 50%` and `bernoulli(rate)`. Nothing becomes an int automatically. Text, booleans, dates and collections never convert to another kind.
3. **Literals follow the same rule**, and are checked during compilation. `let z: float = 1` makes a float, and `let y: int = 1.0` is an error before the program runs.
4. **A collection literal takes its kind from the declared type.** `[1, 2]` is a list unless a set or a bag is declared. An existing collection never changes kind automatically: `set(xs)` converts it.
5. **Unions, `A or B`.** A value that has one of the types is kept. Otherwise it converts to `float` if it can, then to `prob`. Each step has at most one candidate, so the order of the types doesn't matter.
6. **Explicit conversions** cover what doesn't happen automatically: `int(x)`, `float(x)` and `prob(x)`, and `set(xs)`, `list(xs)` and `bag(xs)`.

Two things break. A float no longer becomes an int where an int is declared: `let n: int = 4 / 2` is an error. And an int becomes a float where a float is declared, so `typeof` says `"float"` and it prints as `1.0`. Measured, the first happens only in a playground test program written to test it. The second happens in one documentation example, whose output doesn't change ([measurement](#what-would-break)).

## Requirements

- **`typeof` runs with the program.** Types can differ between worlds, and a later version may let them depend on what a program reads. For example, if probl could read files, `let x = if read("x.txt").length mod 2 == 1 { "A" } else { -1 }`. So the compiler can't answer `typeof`.
- **`typeof` matches the annotation**, unless the program converts the value explicitly in between. After `let z: float = 1`, `typeof z` is `"float"`. Today it's `"int"`.
- **A union's order doesn't matter**, and no value fits a union in two ways.
- **No automatic conversion parses text.**

## The question

A function can already return different types in different worlds:

```probl
fn foo() {
  if 50% { return "A" } else { return 1 }
}
let a = foo()
report typeof a        # int 50.00% · str 50.00%
```

That isn't a value of a union type: in each world, `a` has exactly one type, and the worlds differ. It works without an annotation, and nothing here changes it. What's missing is a way to write the annotation: `fn foo(): str or int`.

Unions are hard to add because annotations convert:

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

Whatever the rule, it must not depend on the order of the types. Today's conversions can't guarantee that. `int or prob` has a value that could become either type: `1.0`. Under this proposal, `fifty()` returns the float: `50%` is a float, and `float` is one of the types. Nothing converts a value that already fits, and no float becomes an int, so `1.0` can only become a prob.

The same applies to bindings, record fields, parameters and the contents of typed containers, with or without functions.

## How other languages do it

Annotations that convert values are unusual, but each part of this proposal has a precedent:

- **TypeScript and Python** annotations have no effect when the program runs: TypeScript erases them, and Python ignores them. A checker reads them before the program runs. In Python, `x: float = 1` leaves `type(x)` an `int`, which is the mismatch this proposal avoids.
- **Kotlin and Rust** have no implicit numeric conversions at all: `val d: Double = 1` and `let d: f64 = 1;` are errors.
- **Swift, Go and Haskell** let the declared type choose a literal's type. In Swift, that includes collection literals: `let s: Set = [1, 2]` makes a set, but `let s: Set = a`, for an array `a`, is an error. They never convert an existing number: `let d: Double = i`, for an int `i`, is an error.
- **C, Java and C#** convert an int to a double implicitly, but never a double to an int.
- **Julia** converts on declared variables and return types with `convert`: `x::Float64 = 1` makes `x` a `Float64`. Its `convert` also turns `2.0` into an `Int`, which this proposal doesn't.
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

`float` keeps an int as it is but converts a probability, and nothing documents either. So `let z: float = 1` reports `"int"`, and `let d: dist[float] = d6` reports `"dist[int]"`.

Collections keep the kind they're built with. `let b: bag[int] = [1, 1, 2]` is an error, "`b` should be a bag[int], but it's a list", and there's no set type.

## The proposal

### The type changes, the number doesn't

| From | To | Automatic |
|---|---|---|
| int | float | Yes, if a float holds the same number. Otherwise it's an error: no float holds `2^60 + 1` |
| prob | float | Yes |
| int, float | prob | Yes, if it's in [0, 1]. Otherwise it's an error, as in `if`, `score` and `bernoulli` |
| float | int | No: use `int(x)`, `round(x)`, `floor(x)` or `x div y` |
| prob | int | No |
| bool, str, date, complex | anything else | No |
| a list, set, bag or map | another kind | No: use `set(xs)`, `list(xs)` or `bag(xs)` |

These conversions apply wherever a type is declared: bindings, assignments to declared variables, parameters, results and record fields. They apply recursively inside a container of the same kind, as conversions do today: map keys and values, bag elements, distribution outcomes and record fields. For a list of ints `xs`, `let ys: list[float] = xs` gives a list of floats. A value that can't convert stops the run, in either failure mode, as a failed conversion does today.

```probl
# Proposed.
let x: int = 1
let y: float = x       # 1.0
let r = 50%
let p: prob = r        # a prob; r stays a float
let n: int = 7 / 2     # error
```

```
Error: `n` should be an int, but it's a float
  Help: `div` divides to an int; `round(…)`, `floor(…)` and `int(…)` convert a float
```

### Why nothing becomes an int

`2.0` and `2` are the same number, so a float to int conversion would fit the rule. It's left out anyway. `/` always gives a float, and whether the result is whole can differ between worlds: `let half: int = n / 2` would work where `n` is even and stop the run where it's odd. `div`, `round` and `int(x)` say which is meant. `int(x)` converts exactly, as an `int` annotation does today, so `int(2.5)` is an error.

### Probabilities

Converting to `prob` changes behavior, not just the type. `~p` draws a trial from a probability but is the number itself for a float, and `p and q` combines two probabilities but is an error for floats. The conversion is still automatic. The declaration shows it, and `if r`, `score r` and `bernoulli(r)` already convert a float the same way.

In the other direction, a prob becomes a float wherever a float is declared. So a function declared with `x: float` always gets a float.

### Literals

A literal is a value like any other, so the same rule gives it its type: `let z: float = 1` makes a float, `let p: prob = 50%` makes a prob, and `let y: int = 1.0` is an error. Literals are checked during compilation, so these are errors before the program runs: `let y: int = 1.0`, `let p: prob = 2`, `let s: str = 1`, and `let x: float = 9007199254740993`, which no float holds.

The rule applies to values when the program runs, so a literal passed through a function value behaves like one passed by name. With `fn f(p: prob)` and `let g = f`, `f(30%)` and `g(30%)` both give `p` a prob.

### Collection literals

`[…]` with elements is a list by default, and a set or a bag where one is declared. `[k: v, …]` is a map, or a bag where one is declared, where each value gives the count of its key, as `bag(…)` already accepts.

```probl
# Proposed.
let xs = [1, 2]                         # list[int]
let s: set[int] = [1, 2, 2]             # set([1, 2])
let b: bag[int] = [1, 1, 2]             # bag([1: 2, 2: 1])
let deck: bag[str] = ["A": 4, "K": 4]
let t: set[int] = xs                    # error: `t` should be a set[int], but it's a list
let u = set(xs)                         # set([1, 2])
```

An existing collection never changes kind automatically: turning a list into a set drops its order and its duplicates, which changes the value. Within a kind, elements convert as above.

The kind is decided where the literal is built, from the declared type known there. That can be a binding, an assignment, a return, a record field, an element of another literal, or a branch of a typed `if`, `match`, `chance` or block. When a function is called by name, the compiler knows its parameter types. Through a function value, it doesn't, so the engine evaluates the function first and builds the literal arguments against its parameter types. It already builds a `with` update's values against the record's field types in the same way. `f([1, 2])` and `g([1, 2])` then behave the same.

A set literal drops duplicates rather than reporting them. Elements are often computed, and they may be equal in only some worlds: `let s: set[int] = [x, y]`.

### `set[T]`

This is a new type. Its elements compare the way map keys and bag elements do, so `1` and `1.0` are different elements, though `1 == 1.0`. A set prints as `set([1, 2])`, as a bag prints as `bag([1: 2, 2: 1])`, and `typeof` reports `"set[int]"`. Its operations are a separate design. The minimum is what a list has for size, membership and iteration.

### Unions

`A or B or …` is a type wherever a type can be written: `let x: int or str`, `fn f(): prob or str`, `list[int or str]`, `dist[int or str]`, and record fields. `or` reads naturally, and it's how the built-in reference already writes such results, as in `sqrt(x) -> float or complex`.

In each world, a value fits a union at the first of these steps that applies:

1. It has one of the types. It's kept.
2. It converts to `float` without changing its number: an int or a prob.
3. It converts to `prob`, with the range check.

Otherwise, it's an error. Step 2 can only give a `float` and step 3 only a `prob`, so no step has two candidates. The order of the types doesn't matter, and adding a type never changes a value that already fitted.

| Declared | Given | Result |
|---|---|---|
| `prob or float` | `50%` | the float |
| `prob or str` | `50%` | a prob |
| `int or float` | `1` | the int |
| `float or prob` | `1` | a float, `1.0` |
| `int or prob` | `1.0` | a prob |
| `int or str` | `1.5` | an error |

```probl
# Proposed syntax.
fn foo(): str or int {
  if 50% { return "A" } else { return 1 }    # each world's result is checked
}

fn fifty(): prob or float {
  return 50%                                 # a float: kept
}
```

A collection or a record fits a type at the latest step that any of its parts needs. `[1, 2.5]` fits `list[int] or list[float]` at step 2, as a list of floats. Two record types that differ only in which fields are floats and which are probabilities can both fit at the same step, with different results: `{a: float, b: prob} or {a: prob, b: float}`. The compiler rejects such a union.

`typeof` describes a value, not its declaration, so it never names a union: `typeof fifty()` is `"float"`.

### What `typeof` reports

For a number, a string or another single value, `typeof` reports the declared type, or one of a union's types. For a collection or a distribution, it describes the contents, so it can be narrower than the declared type. An empty `list[int]` reports `"list[unknown]"` today, and `[1]` declared as `list[int or str]` would report `"list[int]"`. The guarantee there is that `typeof` names a type the declaration allows. To name the declared type exactly, collections would have to store their element types when the program runs ([open question](#open-questions)).

### Explicit conversions

| Conversion | Accepts | Notes |
|---|---|---|
| `prob(x)` | a number in [0, 1], a bool, a prob | As today |
| `int(x)` | an int, an exactly integral float | New. It never rounds, as an `int` annotation doesn't today; `round`, `floor`, `ceil` and `trunc` round |
| `float(x)` | an int, a float, a prob | New. Unlike the automatic conversion, it rounds an int that no float holds to the nearest float |
| `set(xs)` | a list, a set, a bag | New. It drops order and duplicates |
| `list(xs)` | a list, a set, a bag | New. A set's or a bag's elements in order, with a bag's elements repeated by their count |
| `bag(xs)` | a list, a set, `[item: count, …]` | As today, and also takes a set |

These are ordinary built-ins, and values like other built-ins: `[0.3, 0.5].map(prob)`. None of them parses text: `int("42")` is an error. Reading data into a declared type still parses, as today.

### Built-in operations and data

The language's own contexts keep converting a number to a probability: `if` and `while` conditions, match guards, `score`, `chance` weights, and built-ins like `bernoulli(…)` and `quantile(d, 50%)`. That's now the same conversion an annotation makes. (`one_of` weights don't convert, since numbers there are relative weights.) Data schemas still parse text into their declared types.

Today a built-in's integer arguments accept a whole float, like `xs.get(1.0)` and `factorial(5.0)`. The same goes for positions, counts, range bounds and date parts. They should require an int too, for the reason above, and so that a built-in behaves like a function declared with `int`. This can be done separately.

## What changes for programs

| Today | Proposed |
|---|---|
| `let n: int = 4 / 2` gives an int | An error: write `4 div 2`, or `int(4 / 2)` |
| `let n: int = 2.0` | An error during compilation: write `2` |
| `fn f(n: int)`, with `f(x / 2)` | An error: write `f(x div 2)`, or `f(int(x / 2))` |
| `let z: float = 1` is an int, printed as `1` | A float, printed as `1.0` |
| `fn f(x: float)`, with `f(2)` | `x` is `2.0` |
| `let d: dist[float] = d6` gives a `dist[int]` | A `dist[float]` |
| `let b: bag[int] = [1, 1, 2]` is an error | A bag |
| `xs.get(1.0)`, `factorial(5.0)` | Errors, if built-ins follow |
| `let p: prob = 33%`, `let p: prob = rate`, `f(30%)` for `p: prob` | Unchanged |
| `let x: float = p`, for a prob `p` | Unchanged: a float |
| `bernoulli(30%)`, `if 30% { … }`, `score if sick { 95% } else { 8% }` | Unchanged |
| Data read into declared types | Unchanged |

## What would break

Each relevant conversion was logged in the engine. The logged runs covered every example, benchmark and playground test program (46 programs; `benches/snakes_three` was stopped after two minutes), and every `probl` block in the documentation:

- **A float converted to an int for a declared type:** 7 places, all in `web/test/integers.probl`, the playground test written to test it. There were none elsewhere.
- **An int kept where a float is declared:** one place, the language overview's `fee(100)` for `fn fee(amount: float)`. `amount * rate` is a float either way, so the output doesn't change.
- **A whole float given to a built-in's integer argument:** one line, `"é🦀a".get(1.0)`, in `web/test/api-contracts.probl`, which tests that contract.

Conversions to and from `prob` keep working, so they weren't counted. Unit tests weren't counted either, since many exist to test conversion.

## Alternatives considered

### Annotations that only check

This was the first draft of this proposal. An annotation would never convert: `let p: prob = 33%` would need `prob(33%)`, and `float` would accept an int unchanged. That fails the requirement that `typeof` matches the annotation: `let z: float = 1` would report `"int"`, as it does today. There are two ways to fix that. `typeof` could get its answer during compilation, which rules out types decided when the program runs. Or `float` could reject ints, which makes `let z: float = 1` an error. Either way, annotations would be stricter than `if`, which converts a float to a probability anyway.

### Literals only

As in Swift, Go and Haskell, the declared type would choose a literal's type but never convert a variable: `let p: prob = 33%` would work, but `let p: prob = r` would need `prob(r)`. But `if r` converts `r`, and a declaration shows a conversion at least as clearly. Literal types are also decided during compilation, so through a function value, `g(30%)` would differ from `f(30%)`.

### Floats that become ints

This is today's rule, and Julia's. It fits "the type changes, the number doesn't", but it makes `let half: int = n / 2` fail only in the worlds where `n` is odd.

### The first type that fits wins

`prob or float` would give a prob, and `float or prob` a float. Order shouldn't matter.

### Collections that change kind

`let s: set[int] = xs` would convert a list. That changes the value, and when `xs` has duplicates in only some worlds, they'd be dropped there without any sign.

## Open questions

- **Bare collection types.** `let s: set = [1, 2]` and `let xs: list = …` are errors today, since `list` needs its element type. Allowing a bare name to check only the kind seems reasonable. The elements would keep their own types.
- **Element types when the program runs.** If a collection stored its declared element type, `typeof` could name it exactly, even for an empty collection. That's a larger change, to every collection.
- **Containers in `typeof`.** `typeof [1, "a"]` is `"list[any]"`, and `any` isn't a type that can be written. With unions, it could be `"list[int or str]"`, and every `typeof` result would also be a valid annotation. That changes results that programs may compare against.
- **Set operations** beyond size, membership and iteration: union, intersection and difference, and whether they're operators.

## Implementation order and validation

1. **Numeric conversions.** In the engine's declared-type conversion, an int becomes a float, or it's an error if no float holds it. A float no longer becomes an int. Symbolic int outcomes become float outcomes. The compiler gets the matching literal checks. Add `int(x)` and `float(x)`. The oracle (`probl-oracle`) converts declared types too, and changes with the engine. Its generated programs then check the new rules against the engine, as before.
2. **Programs and documents.** Update `web/test/integers.probl`, the reference semantics (section 1's conversion paragraphs) and the language overview, and record the breaking change.
3. **Built-in integer arguments**, if adopted, as a separate breaking change.
4. **Collection literals as bags.** In the engine, evaluate a function value before its arguments, so that literal arguments are built against its parameter types.
5. **Sets:** `set[T]`, `set(xs)` and `list(xs)`.
6. **Unions:** the grammar (`type { "or" type }`), a union in the compiler's types, the conversion steps, and the check for ambiguous record unions.

Steps 1 to 3 break programs. The rest only add.
