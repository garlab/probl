# Type annotations: conversions, literals and unions

> Draft proposal, revised 28 October 2026. Steps 1 to 5 and 8 of the [plan](#implementation-order-and-validation) are implemented. Bare `bag` annotations and the new constructors belong to step 6. Today's behavior is in the [reference semantics](../semantics.md#1-values-and-types). The examples marked as proposed don't run yet. The first draft proposed annotations that only check; [why that changed](#annotations-that-only-check).

## Recommendation

1. **A declared type is the type the value has.** After `let z: float = 1`, `typeof z` is `"float"`. Until the program converts it explicitly, a value given a declared type reports that type, or one of a union's types. For a collection, it reports a type the declaration allows (item 6).
2. **Close numeric types convert automatically. The type changes, the number doesn't.** An int becomes a float when a float holds the same number, and a probability becomes a float. A number in [0, 1] becomes a probability, as it already does in `if 50%` and `bernoulli(rate)`. Nothing becomes an int automatically. Text, booleans, dates and collections never convert to another kind.
3. **Numbers compare by value, at any depth.** `1 == 1.0` already, and now `[1] == [1.0]` too. `1` and `1.0` are one map key, one set element and one bag element, and `pmf` finds either. So a conversion never changes what a comparison or a lookup finds.
4. **Literals follow the same rule**, and are checked during compilation where the compiler knows the declared type. `let z: float = 1` makes a float, and `let y: int = 1.0` is an error before the program runs.
5. **A collection literal takes its kind from the declared type**, where the compiler knows it: `let b: bag = [1, 1, 2]` makes a bag, as does `let b: bag = [1: 2, 2: 1]`. Bare `bag` checks only the kind; `bag[int]` also checks the elements. In a union, the default kind wins: a list for `[1, 2]`, a map for `[1: 2]`. Otherwise the union must have exactly one kind the literal could be. For `set[int] or bag[int]`, write `set(1, 2)` or `bag(1, 2)`. An existing collection never changes kind automatically: `set(...xs)` converts it.
6. **A collection's `typeof` describes its elements. A declaration limits the variable, not the value.** This is today's rule, kept. After `var c = [1]`, `c.push("bar")` works, and `typeof c` becomes `"list[int or str]"`. For `var b: list[int]`, the same push is an error. An empty list reports `"list[unknown]"`, which fits any declared list.
7. **Unions, `A or B`.** A value that has one of the types is kept. Otherwise it converts to `float` if it can, then to `prob`. Each step has at most one candidate, so the order of the types doesn't matter. At any depth, a union has at most one type of each collection or distribution kind, and no two record types one record could fit, so a collection's parts convert in only one way. `typeof` uses unions to describe mixed contents, in place of `any`.
8. **Explicit conversions** cover what doesn't happen automatically. A number converts with its type's name: `int(x)`, `float(x)` and `prob(x)`. A collection's type name builds one from its elements, as in `set(1, 2, 3)`, so a collection converts by spreading into another: `set(...xs)`, `list(...s)` or `bag(...xs)`.

For single numbers, two things break. A float no longer becomes an int where an int is declared: `let n: int = 4 / 2` is an error. And an int becomes a float where a float is declared, so `typeof` says `"float"` and it prints as `1.0`. Measured, the first happens only in a playground test program written to test it. The second happens in one documentation example, whose output doesn't change ([measurement](#what-would-break)). A distribution converts only if its outcomes do, apart from the far tail a count law already cuts off: `poisson(3)` can be a `dist[float]`, but `geometric(1e-16)`, which passes 2^53 four times in ten, can't. No program measured declares a `dist[float]`. Inside collections, numbers compare by value: `[1] == [1.0]` becomes true, and so does `[1: "a"].contains(1.0)`. `typeof` also describes mixed contents differently: `"list[int or str]"` rather than `"list[any]"`.

## Requirements

- **`typeof` runs with the program.** Types can differ between worlds, and a later version may let them depend on what a program reads. For example, if probl could read files, `let x = if read("x.txt").length mod 2 == 1 { "A" } else { -1 }`. So the compiler can't answer `typeof`.
- **`typeof` matches the annotation**, unless the program converts the value explicitly in between. After `let z: float = 1`, `typeof z` is `"float"`. Today it's `"int"`. A collection's `typeof` describes its elements, so it names a type the declaration allows: an empty `list[int]` reports `"list[unknown]"`.
- **A conversion doesn't change what a value equals**, or what a lookup with it finds.
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

`float` keeps an int as it is but converts a probability, and nothing documents either. So `let z: float = 1` reports `"int"`, and `let d: dist[float] = d6` reports `"dist[int]"`. A count law too broad to list, like `geometric(1e-16)`, fails a `dist[float]` declaration: "`d` should be a dist[float], but it's a dist[int]".

Collections keep the kind they're built with. `let b: bag[int] = [1, 1, 2]` is an error, "`b` should be a bag[int], but it's a list[int]", and there's no set type.

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
| a list, set, bag or map | another kind | No: spread it into another, as in `set(...xs)` |

These conversions apply wherever a type is declared: bindings, assignments to declared variables, parameters, results and record fields. They apply recursively inside a container of the same kind, as conversions do today: map keys and values, bag elements, distribution outcomes and record fields. For a list of ints `xs`, `let ys: list[float] = xs` gives a list of floats. A value that can't convert stops the run, in either failure mode, as a failed conversion does today.

A named record fits an anonymous record type with its field names, as it does today, but only if no field needs converting. With `type Point = {x: int}`, `let p: {x: int} = Point {x: 1}` keeps the `Point`, and `let q: {x: float} = Point {x: 1}` is an error. Converting `x` would leave a record that isn't a `Point`, so it would lose its name without any sign. Today, `q` is a `Point` whose `x` is still an int.

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

### Numbers compare by value

Done (step 1). `1 == 1.0` was true, and so was `1 == complex(1, 0)`. But inside a collection, numbers compared with their types: `[1] == [1.0]` was false, `1` and `1.0` were different map keys, and `pmf(one_of([1.0, 2.0]), 1)` was 0%. Conversions would have made that matter. After `let m: map[float, str] = [1: "a"]`, the key becomes `1.0`, so `m.contains(1)` would turn false, and after `let d: dist[float] = d6`, `pmf(d, 3)` would be 0%.

So numbers compare by value everywhere, at any depth:

- `[1] == [1.0]`, `{x: 1} == {x: 1.0}` and `[50%] == [prob(0.5)]` are true.
- `1` and `1.0` are one map key, one set element and one bag element. `[1.0: "a"].contains(1)` is true, and `m[1]` finds the key `1.0`. Adding a key equal to one already there keeps that key and replaces its value, as a repeated key in a literal does today: `[1: "a", 1.0: "b"]` is `[1: "b"]`, and `bag(1, 1.0)` holds two copies of the int `1`.
- `pmf(d, x)` adds up the outcomes equal to `x`, so `pmf(one_of([1, 1.0]), 1)` is 100%, as `P(one_of([1, 1.0]) == 1)` already is. `support` lists each number once, so `pmf` over it adds up to 1. Where outcomes of different types hold one number, it lists the first of them in the order int, prob, float, complex.

Equality stays exact: an int equals a float only if the float holds that int, so `2^60 + 1 == 2.0^60` stays false. Text, booleans and dates never equal a number.

Worlds keep their types. Merging worlds and memoizing calls still tell `1` from `1.0`, and so does a distribution's list of outcomes, so `typeof` can still report `int 50.00% · float 50.00%`. Only comparisons and lookups go by value.

Python, Julia and Lua do the same. In Python, `[1] == [1.0]` is true, `{1: "a"}[1.0]` is `"a"`, and `{1: "a", 1.0: "b"}` is `{1: "b"}`. Julia's dictionaries and Lua's tables treat `1` and `1.0` as one key too.

### Distributions

A declared `dist[T]` converts a distribution's outcomes, as `list[T]` converts a list's elements: `let d: dist[float] = d6` has the outcomes 1.0 to 6.0. Its outcomes must all convert, and that's decided at the declaration, from the distribution itself, before anything is listed or drawn.

A count law with no largest outcome, like `poisson(3)` or `geometric(0.5)`, already has a line for what counts as an outcome. When it's listed, the listing stops once less than 1e-18 of its weight is left, far out in its tail, and that weight counts as unresolved, like weight a `simulate` couldn't resolve. Converting to `float` uses the same line: a count law converts if its outcomes that no float holds have a chance below 1e-18 together. A float holds every int from −2^53 to 2^53, and skips more and more of them beyond that, so the engine bounds that chance by the chance of an outcome beyond 2^53.

The engine decides from what it knows about the distribution:

- **Every outcome is listed**, as for dice and `one_of` over a list. Nothing is cut off, so each outcome must convert, however small its weight. `one_of([2^60])` converts, since a float holds 2^60, and `one_of([2^60 + 1])` is an error.
- **A count law** is decided from its parameters, whether it can be listed or not. The chance of an outcome beyond 2^53 is exact for `geometric`, and an upper bound for `poisson` and `binomial`, so where the bound is off, it's only ever too strict. A law with one outcome converts if a float holds it. Converting to `prob` allows no tail: every outcome must be in [0, 1], so only a law like `binomial(1, p)` or `geometric(1)` converts. With a tail, a `dist[prob]` could be asked about an outcome that's no probability, as in `pmf(d, 2)`.
- **Anything else** is an error, which says the conversion can't be checked. That includes a distribution that's only partly listed, with no law to say what's left out, such as one where a `simulate` left weight unresolved.

For the count laws:

| Law | As a `dist[float]` |
|---|---|
| `geometric(p)` | Converts if p is above about 4.6 × 10⁻¹⁵. Its chance of passing 2^53 is (1 − p)^(2^53) |
| `poisson(rate)` | Converts: a rate can't be above 10¹⁵, and the bound rejects only rates within about 10⁹ of 2^53 |
| `binomial(n, p)` | Converts if n ≤ 2^53, or if its chance of passing 2^53 is below 1e-18 |
| A law with one outcome, like `geometric(1)` or `binomial(n, 1)` | Converts if a float holds that outcome |

So `poisson(3)`, `geometric(0.5)` and `geometric(1e-12)` convert, whether they're listed or not: `geometric(1e-12)` passes 2^53 with a chance of about e^−9007. `geometric(1e-16)` is an error at the declaration. It passes 2^53 four times in ten, and most ints out there have no float.

```
Error: `d` should be a dist[float], but geometric(1e-16) has outcomes that no float holds
  Help: `dist[int]` keeps them as ints
```

The decision depends only on the law, not on the outcome limit, the seed or the mode. `dist[int]` and `dist[int or float]` need no conversion, and always work.

A law that converts is kept as a law, and each outcome converts as it's listed or drawn, including by the shortcuts that draw from a law directly. A draw that no float holds, which has a chance below 1e-18, is drawn again, as sampling never draws a listed law's tail. The numbers don't change, so queries mostly answer as they do for the law today. A listed law answers `pmf` and `cdf` from its listed outcomes, so its cut tail gives 0. A law too broad to list answers `mean` and `sd` from formulas, and `pmf` and `cdf` too for `geometric`; for `poisson` and `binomial`, they aren't supported yet. Two answers change:

- `pmf(d, x)` is 0 where no float holds `x`, since no outcome can equal it, and the law's answer otherwise. `cdf` keeps the law's answer. The only weight it counts that isn't an outcome lies beyond 2^53, where `cdf` is already within 1e-18 of 1, and no float can show a difference that small.
- `quantile`, `median`, the smallest outcome and `support` give floats. Each converts exactly: the law has less than 1e-18 beyond 2^53, less than the gap of about 1.1 × 10⁻¹⁶ between 1 and the largest float below it, so every quantile short of 100% is at most 2^53.

The outcome limit can still stop a law that converts from being listed.

An int computed from a continuous draw needs no special rule. `floor`, `round`, `ceil` and `trunc` of a float always give an int that a float holds, since a float beyond 2^53 is already whole. So wherever `floor(x)` can be computed, its result converts: for any draw when sampling, and for a bounded one, like a uniform draw, when enumerating. Enumerating can't round an unbounded draw, like a normal one, yet, with or without a declaration. Once a world has drawn an int, a declared `float` checks only that int. A declared `dist[float]` promises more: it checks every outcome the distribution can have.

### Literals

A literal is a value like any other, so the same rule gives it its type: `let z: float = 1` makes a float, `let p: prob = 50%` makes a prob, and `let y: int = 1.0` is an error. Where the compiler knows the declared type, it checks literals, so these are errors before the program runs: `let y: int = 1.0`, `let p: prob = 2`, `let s: str = 1`, and `let x: float = 9007199254740993`, which no float holds.

The rule applies to values when the program runs, so a number passed through a function value converts as one passed by name. With `fn f(p: prob)` and `let g = f`, `f(30%)` and `g(30%)` both give `p` a prob. If `f` took `n: int`, `g(1.0)` would be the same error as `f(1.0)`, found when the program runs.

### Collection literals

`[…]` with elements is a list by default, and a set or a bag where one is declared. `[k: v, …]` is a map, or a bag where one is declared, where each value gives the count of its key: `let b: bag = [1: 2, 2: 1]` holds two copies of 1 and one of 2.

```probl
# Proposed.
let xs = [1, 2]                         # list[int]
let s: set[int] = [1, 2, 2]             # set(1, 2)
let b: bag[int] = [1, 1, 2]             # a bag of three items
let deck: bag[str] = ["A": 4, "K": 4]
let t: set[int] = xs                    # error: `t` should be a set[int], but it's a list[int]
let u = set(...xs)                      # set(1, 2)
```

Bare `bag` is a declared type wherever `bag[T]` is allowed. It chooses the literal's kind and requires the variable to remain a bag, but leaves its elements' types unchanged. `let b: bag = [1, "A"]` is allowed, and `typeof b` describes those contents as usual; it doesn't report just `"bag"`. `bag[T]` adds the element checks and conversions. Both `let b: bag = []` and `let b: bag = [:]` make an empty bag. In a union, bare `bag` counts as the bag kind, so `bag or bag[int]` is rejected by the same rule as two parameterized bag types.

An existing collection never changes kind automatically: turning a list into a set drops its order and its duplicates, which changes the value. Within a kind, elements convert as above.

The kind comes from the brackets and the declared type, before any element is looked at. `[…]` can be a list, a set or a bag, and is a list by default. `[k: v, …]` can be a map or a bag, and is a map by default. Only a union's own types count, not the types inside them:

1. If the declared type includes the default kind, the literal is that kind.
2. Otherwise, if it includes exactly one kind the literal can be, the literal is that kind.
3. If it includes several, as `set[int] or bag[int]` does, it's an error, which asks for `set(…)` or `bag(…)`.
4. If it includes none, the literal is the default kind, and the declaration rejects it.

Without a declared type, it's the default kind. Once the kind is chosen, its type gives the elements theirs, if an element type is declared, and an element that doesn't convert is an error: the compiler doesn't try another kind. In a bag written `[item: count]`, each item gets the bag's element type if present, and each count must be an int that isn't negative. Counts are checked even for bare `bag`.

```probl
# Proposed.
let a: list[float] or set[int] = [1, 1, 2]    # a list, [1.0, 1.0, 2.0]
let b: list[str] or bag[int] = [1, 2]         # error: it's a list, and 1 isn't a str
let c: set[int] or bag[int] = [1, 1, 2]       # error: write set(…) or bag(…)
let d: set[int] or bag[int] = []              # an error too: being empty doesn't choose
let e: set[int] or bag[int] = set(1, 1, 2)    # a set of two elements
let f: set[int] or bag[int] = bag(1, 1, 2)    # a bag of three
let g: map[int, int] or bag[int] = [1: 2]     # a map, the default
```

Reversing a union's types changes none of these. `set(…)` and `bag(…)` choose the kind themselves. The declared type doesn't reach their arguments, which are built as usual, and then convert as elements.

The kind is decided where the literal is built, from the declared type known there. That can be a binding, an assignment, a return, a record field, an element of another literal, or a branch of a typed `if`, `match`, `chance` or block: the type flows through each, as an expected type does today for numbers. A literal in a `list[bag[int]]` is a list, and each literal inside it is a bag.

The compiler chooses the kind, so it has to know the type. Through a function value, or in a `with` on a record whose type the compiler doesn't know, it doesn't, and a literal has its default kind. With `fn f(b: bag[int])` and `let g = f`, `f([1, 1, 2])` passes a bag, but `g([1, 1, 2])` passes a list, which is an error at the call:

```
Error: `b` should be a bag[int], but it's a list[int]
  Help: `bag(1, 1, 2)` builds a bag
```

A number passed through a function value converts at the call, so `g(30%)` gives the same prob as `f(30%)`. A collection can't, since it never changes kind automatically. So passing a literal through a function value can make an error, but never a different value. Choosing the kind when the program runs is [possible, but costly](#collection-kind-through-function-values).

A set literal drops duplicates rather than reporting them. Elements are often computed, and they may be equal in only some worlds: `let s: set[int] = [x, y]`.

### `set[T]`

This is a new type. Its elements compare the way map keys and bag elements do, by value, so `set(1, 1.0)` is `set(1)`. A set prints as it's [built](#building-collections), `set(1, 2)`, and `typeof` reports `"set[int]"`. Its operations are a separate design. The minimum is what a list has for size, membership and iteration.

### Unions

`A or B or …` is a type wherever a type can be written: `let x: int or str`, `fn f(): prob or str`, `list[int or str]`, `dist[int or str]`, and record fields. `or` reads naturally, and it's how the built-in reference already writes such results, as in `sqrt(x) -> float or complex`.

In each world, a number or another single value fits a union at the first of these steps that applies:

1. It has one of the types. It's kept.
2. It converts to `float` without changing its number: an int or a prob.
3. It converts to `prob`, with the range check.

Otherwise, it's an error. Step 2 can only give a `float` and step 3 only a `prob`, so no step has two candidates, and the order of the types doesn't matter. Adding a type never changes a value that fitted without converting. A collection literal chooses its kind first, [as above](#collection-literals).

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

A collection, a distribution or a record fits the union's one type of its kind, or the one record type it can be. Each of its parts then fits its own declared type by the same steps: `[1, 2.5]` fits `list[float] or str` as `[1.0, 2.5]`, and `list[int or float]` as it is. A distribution's outcomes convert [as above](#distributions). The steps apply to each part, not to the collection as a whole.

Two types of one kind could both take a value by converting different parts of it. `[1: 1]` would fit `map[int, float] or map[float, int]` by converting its value in one and its key in the other. Both take the same step, and the results, though equal, have different types, which `typeof` tells apart.

So the compiler checks every union, wherever it's written, before the program runs:

1. Nested unions are flattened, and repeated types removed. `list[int] or list[int]` is just `list[int]`, and `list[int or str] or list[str or int]` is just `list[int or str]`.
2. A union has at most one type of each kind: `list`, `set`, `bag`, `map` and `dist`. Types of different kinds are fine. Two record types with the same field names are allowed only if both are named.
3. The same goes inside: in element types, both map keys and values, distribution outcomes, and record fields, including the fields of named record types. Each position is checked on its own, so two map types in different fields of a record are fine.

All of these are errors, in either order:

```text
map[int, float] or map[float, int]
list[float] or list[prob or float]
set[int] or set[float]
bag[int] or bag[str]
dist[int] or dist[float]
list[map[int, float] or map[float, int]]
{a: float, b: prob} or {a: prob, b: float}
```

The error names the two types and where they are, as in "list element: two map types". Its help can suggest the union inside, but must say that it accepts more: `map[int or float, int or float]` also takes a map from ints to ints, or from floats to floats. Where the pairing matters, named records keep it. A record of a named type has that type and no other, so `Point or Vector`, with the same fields, is allowed. `Point or {x: float, y: float}` isn't, when those are `Point`'s fields, since a `Point` fits both.

It also rules out some unions that would be unambiguous, like `list[int] or list[str]`, a list of all ints or all strings. `list[int or str]` takes both, and mixed lists. And since no union has two list types, an empty list never has to choose between them. A later version could allow two types of one kind when no value could fit both ([alternatives](#types-of-one-kind-in-a-union)).

`typeof` describes a value, not its declaration, so it never names a union: `typeof fifty()` is `"float"`.

### Collections and their element types

A collection has no element type of its own. `typeof` describes the elements it holds, and a declaration limits the variable. Every change to a declared variable is checked, including a `push`, and a change inside one of its fields or elements. That's today's rule, and it stays:

```probl
# Proposed: today, `a` and `c` end as "list[any]".
var a = []                # list[unknown]
a.push(1)                 # list[int]
a.push("foo")             # list[int or str]

var b: list[int] = []     # list[unknown]
b.push("bar")             # error

var c = [1]               # list[int]
c.push("bar")             # list[int or str]
```

```
Error: `b` should be a list[int], but it's a list[str]
```

A failed declaration describes the value as `typeof` does (step 8, done). It used to say only "but it's a list".

This works because probl's lists are values. After `var d = b`, `d.push(2)` leaves `b` unchanged, and a parameter can't be pushed to. So a list changes only through a variable, and if that variable is declared, every change is checked. Languages that keep an element type in the list itself, like Swift, C# and Julia, need it because two names can share one mutable list.

A declaration also stops a variable from changing kind. Pushing to a list already needs `var`, as in Swift, and a `var` can be given any value: after `var lst = [1]`, `lst = 5` is allowed. `var lst: list[int] = [1]` rules out both `lst = 5` and `lst.push("bar")`.

With unions, `typeof` describes mixed contents exactly, in place of `any`:

| Value | `typeof` today | Proposed |
|---|---|---|
| `[1, "a"]` | `"list[any]"` | `"list[int or str]"` |
| `[1, 2.0]` | `"list[any]"` | `"list[int or float]"` |
| `[[], [1]]` | `"list[any]"` | `"list[list[int]]"` |
| `[[1], ["a"]]` | `"list[any]"` | `"list[list[int or str]]"` |
| `[{x: 1}, {x: "a"}]` | `"list[any]"` | `"list[{x: int or str}]"` |
| `[Point {x: 1}, {x: 2}]` | `"list[any]"` | `"list[{x: int}]"` |
| `one_of([1, "x"])` | `"dist[any]"` | `"dist[int or str]"` |
| `[]` | `"list[unknown]"` | Unchanged |

`unknown` merges into the type beside it: an empty list is also a `list[int]`, so `[[], [1]]` is a `list[list[int]]`. Types of one kind merge too, with a union inside, so `typeof` always gives a type that could be declared, and that the value fits: `[[1], ["a"]]` is a `list[list[int or str]]`. A union's types are listed in a fixed order, so the same contents always give the same string. Five lines of the engine's type tests compare against `any`, and the reference semantics and the language overview describe it.

Records merge as far as a union allows them to. Anonymous records with the same field names merge into one, with a union inside each field: `[{x: 1}, {x: "a"}]` is a `list[{x: int or str}]`. A named record with those field names merges into it too, since it fits it without converting: `[Point {x: 1}, {x: 2}]` is a `list[{x: int}]`. Named records stay apart from each other, even with the same fields, so a value declared `list[Point or Vector]` is still described by name.

For a single value, `typeof` reports the declared type, or one of a union's types. For a collection or a distribution, it names a type the declaration allows, which can be narrower than the declaration. An empty `list[int]` reports `"list[unknown]"`, and `[1]` declared as `list[int or str]` reports `"list[int]"`.

Other languages split the same way:

| Language | A list's element type when the program runs | `[1]`, then push `"bar"` |
|---|---|---|
| JavaScript, Python | None: `typeof []` is `"object"`, and `type([1])` is `list` | Works. Python's checkers reject it before the program runs |
| TypeScript | None, since types are erased. `let a = []` widens as elements are pushed | An error before the program runs |
| Java, Kotlin | None, since types are erased | An error before the program runs |
| Swift, C# | Fixed: `Array<Int>`, `List<int>` | An error |
| Julia | Fixed: `Vector{Int64}`, and `[]` is a `Vector{Any}` | An error. Pushing `2.0` converts it to `2` |
| R, NumPy | One type for the whole vector | Converts every element: `c(1, "bar")` is `"1" "bar"` |

Probl is in the first group, with a `typeof` that reads the elements.

### Explicit conversions

| Conversion | Accepts | Notes |
|---|---|---|
| `prob(x)` | a number in [0, 1], a bool, a prob | As today |
| `int(x)` | an int, an exactly integral float | New. It never rounds, as an `int` annotation doesn't today; `round`, `floor`, `ceil` and `trunc` round |
| `float(x)` | an int, a float, a prob | New. Unlike the automatic conversion, it rounds an int that no float holds to the nearest float. Like other built-ins, it lifts over a distribution's outcomes, which for a law too broad to list fails on the outcome limit |

A number is built from a number, so a number's type name can convert one: `int(x)` can't mean anything else. These are ordinary built-ins, and values like other built-ins: `[0.3, 0.5].map(prob)`. None of them parses text: `int("42")` is an error. Reading data into a declared type still parses, as today.

A collection converts by spreading into another ([building collections](#building-collections)).

### Building collections

A type's name builds a value from its parts, as `date(2027, 1, 31)` and `complex(1, 2)` do today. `date("2027-01-31")` parses text instead, so it gives way to a date literal, `'2027-01-31'`, and to `parse_date(s)` for text read when the program runs ([literals](literals.md)). A collection's parts are its elements:

```probl
# Proposed.
set(1, 2, 2)          # set(1, 2)
bag("A", "A", "K")    # a bag holding two As and one K
list(1, 2)            # [1, 2]
set()                 # an empty set, a set[unknown]
set(xs)               # a set with one element: the list xs
```

A bag has three construction forms. These build equal values:

```probl
# Proposed.
let a = bag(1, 1, 2, 3)
let b: bag = [1, 1, 2, 3]
let c: bag = [1: 2, 2: 1, 3: 1]
```

The constructor enumerates items. The declared type makes either bracket form a bag, with the mapping form giving counts. Items and counts can be computed expressions, so `let deck: bag = [card: copies]` doesn't need to enumerate `copies` items. Write `bag[T]` when the elements should also be checked or converted. The constructor has no named count argument.

A constructor builds from its arguments as they are, as a literal does, and never lifts over a distribution. `list(d6, 0)` is `[d6, 0]`, a list that holds a recipe. Today `bag` lifts: `bag(one_of([[1], [2]]))` is a `dist[bag[int]]`. It would be a bag holding one recipe.

So a collection's type name never converts. Spreading does: `...xs` passes what `for x in xs` gives, as separate elements or arguments.

```probl
# Proposed.
set(...xs)            # a list's elements, without duplicates
list(...s)            # a set's elements, in order
list(...b)            # a bag's items, repeated by their counts
bag(...xs)            # each element is one item
[...xs, ...ys]        # both lists, one after the other
set(...a, ...b)       # the elements of both sets
```

`f(...xs)` means exactly `f(x1, …, xn)`, so `max(...[d6, 3])` lifts, as `max(d6, 3)` does. (For a list's largest element, `maximum(xs)` already exists, and takes a `default:` for an empty list.) Spreading a distribution is an error, as a `for` over one is today: a value has to be drawn first. So `list(...xs)` is `xs`, for any list.

A map spreads as its `[key, value]` pairs, as `for` gives them, so `bag(...m)` is a bag of pairs. To build from counts, write the mapping literal where a bag is declared: `var deck: bag = [2: 4, 3: 4, 10: 16]`. This proposal doesn't add a direct conversion from an existing map of counts: `let b: bag = m` is still an error. Moving an existing `bag(m)` call therefore needs a decision about whether its entries are items or counts; spreading preserves the pairs as items, while counts should be constructed in a bag-typed literal.

Spreading works in collection literals, and in calls to the built-ins that take any number of arguments: `set`, `list`, `bag`, `min` and `max`. A literal with a spread takes its kind like any other, so `let s: set[int] = [...xs, 4]` is a set; the spread shows the conversion, as `set(...xs)` does. A user function has a fixed number of parameters, and a spread's length can differ between worlds, so the compiler couldn't check the count. Spreading into one can come later.

Today, `bag(xs)` converts a list or a map of counts, and in Python, `set(xs)` converts a list. Such a call would now build a collection of one element, without any sign. So for one release, `set`, `list` or `bag` with a single collection written as its only argument is an error. Its help suggests `...` for elements, and a bag-typed mapping literal for counts. An argument that comes from a spread is never rejected, so `list(...xs)` is `xs` even when `xs` is `[[1]]`. After that release, a single collection builds a collection of one element. Meanwhile, a set of one list can be written `set(...[xs])`.

A set prints as `set(1, 2)`. When its elements print as source code, that builds an equal value. During the release that rejects a single collection, a set holding exactly one collection prints with a spread, `set(...[[1]])`, which that release accepts.

A bag's display must keep counts compact and identify it as a bag, including inside another collection. A count of 10^12 must never expand into 10^12 printed items. Its exact display notation remains an [open question](#open-questions); it needn't be an executable expression. To write that value as source, use `let b: bag = [1: 10^12]`. Today's `bag([1: 2, 2: 1])` display must change when the constructor changes, since it would then describe a bag holding one map.

A value that holds a distribution already can't always print as code. A listed distribution prints as `dist(1: 16.67%, …)`, which isn't code, and a map can't have a distribution as a key. A bag of recipes still needs a compact display of items and their counts, without requiring that display to parse. (A prob prints as `50%`, which builds an equal float.)

JavaScript (`[...xs]`, `Math.max(...xs)`), Python (`[*xs]`, `{*xs}`, `f(*args)`) and Kotlin (`setOf(*array)`) spread the same way.

### Built-in operations and data

The language's own contexts keep converting a number to a probability: `if` and `while` conditions, match guards, `score`, `chance` weights, and built-ins like `bernoulli(…)` and `quantile(d, 50%)`. That's now the same conversion an annotation makes. (`one_of` weights don't convert, since numbers there are relative weights.) Data schemas still parse text into their declared types.

Done (step 4). A built-in's integer arguments accepted a whole float, like `xs.get(1.0)` and `factorial(5.0)`, and so did positions, counts, range bounds and date parts. They require an int now, for the reason above, and so that a built-in behaves like a function declared with `int`.

## What changes for programs

| Today | Proposed |
|---|---|
| `let n: int = 4 / 2` gave an int | An error: write `4 div 2`, or `int(4 / 2)` (step 2, done) |
| `let n: int = 2.0` | An error during compilation: write `2` (step 2, done) |
| `fn f(n: int)`, with `f(x / 2)` | An error: write `f(x div 2)`, or `f(int(x / 2))` (step 2, done) |
| `let z: float = 1` was an int, printed as `1` | A float, printed as `1.0` (step 2, done) |
| `fn f(x: float)`, with `f(2)` | `x` is `2.0` (step 2, done) |
| `let d: dist[float] = d6`, or `poisson(3)`, gave a `dist[int]` | A `dist[float]` (step 2, done) |
| `let d: dist[int] = poisson(3)` lost its law, so `pmf(d, 2)` was an error | The law survives, and `pmf(d, 2)` is `pmf(poisson(3), 2)` (step 2, done) |
| `let q: {x: float} = Point {x: 1}` was a `Point` with an int `x` | An error (step 2, done) |
| `let b: bag[int] = [1, 1, 2]` was an error | A bag (step 5, done) |
| `let b: bag = [1, 1, 2]` rejects the bare type | A bag, with no restriction on its element types |
| `bag([1, 1, 2])`, `bag(cards)` and `bag(["A": 4])` convert to a bag | An error for one release, then a bag of one item: write `bag(1, 1, 2)`, `bag(...cards)` or `let b: bag = ["A": 4]` |
| `bag(one_of([[1], [2]]))` is a `dist[bag[int]]` | A bag holding one recipe |
| A bag prints as `bag([1: 2, 2: 1])` | A compact display that identifies it as a bag; exact notation to be decided |
| `[1] == [1.0]` and `{x: 1} == {x: 1.0}` were false | True (step 1, done) |
| `[1: "a"].contains(1.0)` was false, and `[1: "a", 1.0: "b"]` had two keys | True, and it's `[1: "b"]` (step 1, done) |
| `pmf(one_of([1, 1.0]), 1)` was 50% | 100% (step 1, done) |
| `typeof [1, "a"]` is `"list[any]"` | `"list[int or str]"` |
| `xs.get(1.0)`, `factorial(5.0)` worked | Errors: write `xs.get(1)`, or `int(…)` (step 4, done) |
| `let p: prob = 33%`, `let p: prob = rate`, `f(30%)` for `p: prob` | Unchanged |
| `let x: float = p`, for a prob `p` | Unchanged: a float |
| `bernoulli(30%)`, `if 30% { … }`, `score if sick { 95% } else { 8% }` | Unchanged |
| Data read into declared types | Unchanged |

## What would break

Each relevant conversion was logged in the engine. The logged runs covered every example, benchmark and playground test program (46 programs; `benches/snakes_three` was stopped after two minutes), and every `probl` block in the documentation:

- **A float converted to an int for a declared type:** 7 places, all in `web/test/integers.probl`, the playground test written to test it. There were none elsewhere.
- **An int kept where a float is declared:** one place, the language overview's `fee(100)` for `fn fee(amount: float)`. `amount * rate` is a float either way, so the output doesn't change.
- **A whole float given to a built-in's integer argument:** one line, `"é🦀a".get(1.0)`, in `web/test/api-contracts.probl`, which tests that contract.
- **`bag(…)` given one collection,** found by searching rather than logging: 16 lines. The blackjack example and benchmark build their decks with `bag([2: 4, 3: 4, …])`, the SET example and benchmark with `bag(cards)`, and the language overview's table of common models shows `bag([card: count, …])`. The other 11 are in the playground tests, most of them testing bags.

Conversions of a number to and from `prob` keep working, so they weren't counted. Unit tests weren't counted either, since many exist to test conversion.

That measurement came before the rule for a distribution's outcomes. But no example, benchmark, playground test or documentation block declares a `dist[float]` or a `dist[prob]`, so the rule can't break any of them.

Comparing numbers by value reversed a documented contract: map keys, bag elements and `pmf` used typed identity. Step 1 updated the reference semantics, the architecture and testing notes and the [collection values](collection-values.md) design, and rewrote the engine tests that checked it, ten of them, to check the opposite. No example, benchmark or playground test program changed its output.

## Alternatives considered

### Annotations that only check

This was the first draft of this proposal. An annotation would never convert: `let p: prob = 33%` would need `prob(33%)`, and `float` would accept an int unchanged. That fails the requirement that `typeof` matches the annotation: `let z: float = 1` would report `"int"`, as it does today. There are two ways to fix that. `typeof` could get its answer during compilation, which rules out types decided when the program runs. Or `float` could reject ints, which makes `let z: float = 1` an error. Either way, annotations would be stricter than `if`, which converts a float to a probability anyway.

### Literals only

As in Swift, Go and Haskell, the declared type would choose a literal's type but never convert a variable: `let p: prob = 33%` would work, but `let p: prob = r` would need `prob(r)`. But `if r` converts `r`, and a declaration shows a conversion at least as clearly. Literal types are also decided during compilation, so through a function value, `g(30%)` would differ from `f(30%)`.

### Floats that become ints

This is today's rule, and Julia's. It fits "the type changes, the number doesn't", but it makes `let half: int = n / 2` fail only in the worlds where `n` is odd.

### The first type that fits wins

`prob or float` would give a prob, and `float or prob` a float. Order shouldn't matter.

### Types of one kind in a union

The previous draft let a union have several types of one kind, and a collection fitted at the latest step any of its parts needed: `[1, 2.5]` fitted `list[int] or list[float]` at step 2, as a list of floats. The compiler would reject a union only where two types could fit one value at the same step with different results. That allows `list[int] or list[str]`, but the line between the unions it allows and those it rejects is hard to see: `list[int] or list[float]` passes, and `list[float] or list[prob or float]` fails, since a list of a prob and an int fits both at step 2, keeping the prob in one and converting it in the other. A change could also convert what's already there: on a variable declared `list[int] or list[float]` holding `[1, 2]`, `push(2.5)` would give `[1.0, 2.0, 2.5]`, so a push would check the whole list, rather than only what it adds.

### Element types stored in collections

As in Swift, C# and Julia, a list would keep its element type, and `typeof b` would be `"list[int]"` even when empty. But `var c = [1]; c.push("bar")` would fail unless `c` is declared `list[int or str]`. `var a = []` would stay a `list[any]` after `a.push(1)`, which tells less than today. The type would follow copies: after `var d = b`, `d.push("x")` would fail, though `d` has no declaration. And every built-in that returns a collection, such as `map` or `filter`, would have to choose an element type. Probl's lists are values, so a variable's declaration already gives the same protection.

### Element types that widen

This is TypeScript's empty array, kept when the program runs: a list without a declaration would widen its type as elements are added. For such a list, that's today's rule with a memory: after the string is popped, it would still say `int or str`. The only gain is that a declared empty list reports its declared type, and that type would then follow copies, as above.

### No element types in `typeof`

As in JavaScript and Python, `typeof [1]` would be `"list"`, which always matches the declaration's kind. But probl uses the detail: the language overview shows `typeof d6` as `"dist[int]"`, and `typeof` across worlds shows it, as in `list[int] 50.00% · list[str] 50.00%`. 19 lines in tests and documentation compare element types.

### Collections that change kind

`let s: set[int] = xs` would convert a list. That changes the value, and when `xs` has duplicates in only some worlds, they'd be dropped there without any sign.

### Every outcome, however unlikely

The previous revision rejected every `geometric(p)` with p < 1 and every `poisson(rate)` with a positive rate as a `dist[float]`, since their outcomes have no bound, and some have no float. That's simple, and it breaks no program measured. But it rejects `poisson(3)`, whose chance of passing 2^53 is far below anything the engine keeps, for a reason about floats that has nothing to do with the program. So a function declared with `d: dist[float]` couldn't take a Poisson count. The engine already treats the last 1e-18 of a listed count law's weight as unresolved, so conversion draws its line in the same place.

An earlier revision went the other way: a law too broad to list kept its law, and each draw converted as it was made, stopping the run at one no float held. Then whether a program runs would depend on the seed and the mode.

### A type's name that converts

The previous revision converted with the type's name, as Python and Swift do: `set(xs)` was a set of `xs`'s elements, and `bag(xs)` as today. But a type's name also builds a value from its parts: `date(2027, 1, 31)`, `complex(1, 2)`, and now `set(1, 2, 3)`. Then `set(xs)` would mean either a set of one list or a set of its elements, depending on how many arguments there are. Numbers don't have that problem, so they keep `int(x)`, `float(x)` and `prob(x)`.

### Conversion methods

`xs.to_set()`, `s.to_list()` and `xs.to_bag()`, as in Kotlin (`setOf(1, 2)` builds, `xs.toSet()` converts) and Ruby. They read well, but spreading already converts collections by their elements, concatenates, takes a union, and passes elements to a built-in. A method that interpreted a map's values as counts would do something different; that conversion isn't proposed here.

### A named argument for bag counts

A named count argument would add a fourth construction form. The bag-typed mapping literal already expresses the items and their counts directly, so the constructor only enumerates items. Compact printing is a display concern and doesn't require another constructor form.

### `xs as set`

`as` would use the type's own syntax, and convert the elements with the kind: `xs as set[float]`. But `as` already names things: a report's label, `report x as "label"`, an expectation's description in the tests design, and an import's alias in the imports design. `report total as float` and `report total as "float"` would differ only by their quotes. A union after `as` would need parentheses, since `x as int or y` could also be a boolean `or`. And in TypeScript, Kotlin and C#, `as` is a cast that never changes the value, while `xs as set` drops duplicates.

### Collection kind through function values

A literal passed through a function value could take its kind from the function's parameter, as one passed by name does. The type is known only when the program runs, after the function value is evaluated. The compiler would mark each literal an argument reaches, through branches and nested literals, with where to read its type: in the function, at this parameter, along this path. Converting at the call is too late, since by then the branches have built lists, and a list never changes kind automatically.

The compiler already keeps the function in a temporary while arguments with statements run. But the engine would need every function's parameter types when the program runs, and liveness and world merging would need to keep the function while a literal can still read it. A `with` on a record of unknown type would need the same, since the engine's field types reach only literals written in place, not those in branches.

That's a lot of machinery for one case: a set or a bag written as a literal and passed through a function value. Without it, that case is an error at the call, with help that suggests `set(…)` or `bag(…)`. A later version can add it without changing what any working program does.

## Later: families of types

A function can accept any list of ints, `fn total(xs: list[int])`, or anything at all, `fn total(xs)`. It can't accept "any collection of ints". The same goes for "any number" and "any record with an `x` field". This isn't needed now, but unions and conversions should leave room for it.

- **Unions.** `list[int] or set[int] or bag[int]` works as soon as unions do. It's long, and a shorthand like `(list or set or bag)[int]` would apply the element type to each kind.
- **Aliases.** `type Ints = list[int] or set[int] or bag[int]` would name a union without a new concept. A family over any element type needs a type parameter: `type collection[T] = list[T] or set[T] or bag[T]`. `type` already declares records and enums, so an alias would be its third use.
- **Built-in families,** such as `collection[T]` and `number`, for `int or float`. With the conversions above, `x: float` already accepts any number by converting it. `number` would be for a function that keeps an int an int.
- **Record shapes.** Today an anonymous record type accepts only a record with exactly its fields: `let p: {x: int} = {x: 1, y: 2}` is an error. A shape type, written something like `{x: int, ..}`, would accept any record with at least those fields, named or not, and keep the others.

In a statically checked language, a trait or an interface also says which operations a function may use on the value. Probl checks types when the program runs: using an operation the value doesn't have is an error where it runs, as for a parameter with no declared type. So a family needs only a membership test, and a named union can express what a trait would. Conversions follow the union steps: `collection[float]`, given a list of ints, converts the elements and keeps the list.

## Open questions

- **Other bare collection types.** Bare `bag` is part of this proposal. Whether `list`, `set` and `map` should also check only their kind without type arguments remains open.
- **Bag printing.** Choose a compact display with an explicit bag label, including for nested bags and bags holding recipes. It must stay proportional to the distinct items, and mustn't look like a constructor call that now builds a different value. Source construction uses the three forms above; the display needn't be source code.
- **Set operations** beyond size, membership and iteration. Spreading gives a union, `set(...a, ...b)`. Intersection and difference need their own, and could be operators.

## Implementation order and validation

1. **Numbers compare by value.** Done. `==`, map keys, bag elements, `pmf` and `support` compare numbers by exact value, at any depth ([numbers compare by value](#numbers-compare-by-value)). `Value`'s order sorts by that comparison first and by kind second, so values `==` finds the same sort next to each other, and a lookup checks the neighbours of where a key would go. Its hash agrees with both: a float that holds an int hashes as that int, and a complex number with no imaginary part as its real part. World merging, memoization and a distribution's stored outcomes keep comparing with types, so the [architecture notes](../architecture.md)' two comparisons stay apart. Sets, when step 6 adds them, use the same lookups.
2. **Numeric conversions.** Done. In the engine's declared-type conversion, an int becomes a float only if a float holds it, and a float no longer becomes an int. The compiler gets the matching literal checks. Add `int(x)` and `float(x)`. A distribution's conversion is decided from its outcomes or its law, before anything is listed or drawn ([distributions](#distributions)). A count law's chance of an outcome no float holds comes from its parameters: exactly for `geometric`, and from a Chernoff bound for `poisson` and `binomial`. Converting to `prob` allows no tail. A draw from a law that converted, which no float holds, is drawn again. Converting a listed law keeps its weights exactly, rather than normalizing them again. Queries on a converted law answer as its source does today, except that `pmf` at a value no float holds is 0, and `quantile`, `median`, the smallest outcome and `support` give floats. A named record fits an anonymous record type only if no field converts. A listed count law keeps its law in `Dist.law`, and `Dist.missing` records the tail it cut off, so the check must read the law, not only `Dist.outcomes`. A law kept as a law carries its conversion, which enumerating, sampling, direct draws and `support` all apply. `Dist`'s equality and hash ignore `law`, so world merging and memoized calls need checking: they mustn't lose a law a conversion depends on. The oracle (`probl-oracle`) rejects annotations today, so it can't check these rules until it supports them. Until then, engine tests cover them, in both modes.
3. **Programs and documents.** Done. Update `web/test/integers.probl`, the reference semantics (section 1's conversion paragraphs) and the language overview, and record the breaking change.
4. **Built-in integer arguments.** Done, as a separate breaking change: `ops::integer` takes only ints, and `int(x)` keeps its own check for a whole float.
5. **Collection literals as bags.** Done. The compiler flows a declared type into collection literals, through branches and nested literals, including map keys, and chooses the kind before converting any element ([collection literals](#collection-literals)). A union's kinds come with step 7. The engine's field types for a `with` on a record of unknown type still convert values, but don't choose a kind, so a literal written in place and one in a branch agree. Test literals in branches and in other literals, by name and through a function value.
6. **Sets, constructors and spreading:** `set[T]`, the constructors `set(…)`, `list(…)` and `bag(…)`, and `...` in collection literals and in calls to `set`, `list`, `bag`, `min` and `max` ([building collections](#building-collections)). Accept bare `bag` wherever a type is allowed: choose the bag literal kind, check counts, leave element types unchanged, and require assignments to remain bags. `bag[T]` keeps its existing element checks. The lexer reads `...` before `..` and `..<`. `bag` accepts only positional items and stops lifting over a distribution; constructors build from their arguments as they are, and spreading a distribution is an error. A single collection written as the only argument to `set`, `list` or `bag` is an error for one release, but not one that comes from a spread. Sets print as they're built; settle the compact bag display before changing its constructor. Move the 16 `bag(…)` calls to enumeration, spreading or bag-typed mapping literals, and update the language overview's table.
7. **Unions:** the grammar (`type { "or" type }`), a union in the compiler's types, flattened and without repeats, the conversion steps, and the check that a union has at most one type of each kind, at any depth ([unions](#unions)). The check follows named record types into their fields, stopping at a record type it has already seen. A collection literal chooses its kind from a union's types. `typeof` merges types of one kind to describe mixed contents, and records as far as a union allows ([collections and their element types](#collections-and-their-element-types)). The compiler never merges a declared union's types: `list[int] or list[str]` stays an error.
8. **Errors that describe the value.** Done: a failed declaration describes the value as `typeof` does, as in "but it's a list[any]" until unions exist. A declared record type is written as `typeof` writes one, `{a: int}`.

Steps 1 to 4 and 6 break programs, and step 7 changes how `typeof` describes mixed contents. The rest only add.

### Acceptance cases for these rules

The rules above need at least these tests. The syntax doesn't run yet.

| Area | Tests |
|---|---|
| Comparing numbers | `[1] == [1.0]`, `{x: 1} == {x: 1.0}` and `[1] == [complex(1, 0)]` are true, and `[2^60 + 1] == [2.0^60]` is false. A map built from `1` then `1.0` has one key, the first, with the last value. `contains`, `get`, indexing and removal find a key of either type, in maps, sets and bags. `pmf` adds up equal outcomes, and `support` lists each number once. `typeof` across worlds still tells `1` from `1.0`. |
| Building collections | `set(1, 1, 2)`, `set()`, `bag("A", "A")`, `bag()`, and a single collection as an error, with its help. The three bag forms above build equal values, and a named count argument is rejected. `list(d6, 0)` holds a recipe, and `bag` no longer lifts. `max(...[d6, 3])` lifts like `max(d6, 3)`, and spreading a distribution is an error. `...` of a list, a set, a bag, a map, a range and a string, in literals and in each built-in that takes it, with lengths that differ between worlds. `let s: set[int] = [...xs, 4]`. Printed sets build an equal value, including a set holding one collection during the transition, unless an element's display isn't source code. Bag display stays compact for a count of 10^12, identifies nested bags, and handles recipes. `list(...[[1]])` is `[[1]]` during the transition too. `1..6` and `0..<n` still read as ranges. |
| Bare bags | Both bracket forms, both empty forms, mixed element types without conversion, and counts that are zero, negative or not ints. Bare `bag` in bindings, assignments, parameters, returns and nested types uses the same contextual rules as `bag[T]`. Reassigning a bare bag variable to a list or map fails. `typeof` describes its contents. `bag[T]` still checks and converts elements. Annotating an existing map as a bag fails; `bag(...m)` builds a bag of pairs, without interpreting the values as counts. |
| Unions | Every error example above, including inside map keys and values and record fields, with the same error in either order. Repeated types are removed; types of different kinds, and named records with the same fields, are allowed. A named record fits an anonymous record type only if no field converts. |
| `typeof` | Mixed anonymous records, a named record with an anonymous one, and a `list[Point or Vector]`. Every description of mixed contents passes the union check, and the value fits it. |
| Literal kinds | Every literal example above, both empty forms, and nested literals. The default kind is kept even when an element doesn't convert, and a union with a set and a bag asks for `set(…)` or `bag(…)`. |
| Where the type reaches | Literals as the result of `if`, `match`, `chance` and blocks, in `list[bag[int]]`, and as map keys and values. Through a function value, or a `with` on a record of unknown type, a literal is a list, written in place or in a branch. A variable holding a list fails a bag parameter. |
| Distributions | Each count law on both sides of its line: `geometric(1e-12)` and `geometric(1e-16)`, `poisson(3)` and a rate near 2^53, `binomial` with n just above 2^53, and laws with one outcome. The bound for `poisson` and `binomial` never accepts a law whose exact chance is 1e-18 or more. Listed outcomes at ±2^53, 2^53 + 1, 2^60 and 2^60 + 1, whatever their weight. `floor` of a bounded draw converts in both modes, and of an unbounded one when sampling; a partly listed distribution with no law doesn't convert. `binomial(1, p)` is a `dist[prob]`, and `poisson(1e-10)` isn't. Each decision is the same when enumerating and sampling, with a different outcome limit, and whether or not the distribution is used. Converting draws nothing. Converted outcomes are floats with the same numbers, when listed, drawn or given by `support`, and queries give the source's answers, including 0 for a listed law's cut tail. `float(d)` of a law too broad to list fails on the outcome limit. A converted `geometric(5e-15)` gives 0 for `pmf` at 2^53 + 1, and floats for its quantiles. |
