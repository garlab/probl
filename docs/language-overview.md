# Probl: language overview

> Current language guide, October 2026. Enumeration and sampling are implemented; particles, beam search and functions marked as future work are not. Some type checks still happen at runtime. The precise rules are in the [reference semantics](semantics.md), which wins where the two disagree.
> See also: [architecture and priorities](architecture.md) · [examples](../examples)

Probl is a small programming language for **explicit random choices and weighted worlds**. A draw or a `chance` block explores alternatives, each in its own *world*, weighted by how likely it is. An `if` follows a boolean fact, or makes a fresh trial from a probability or boolean distribution. A program doesn't produce one answer: it produces the distribution over every world it could end up in.

```probl
let weather = chance { 30% => "rain", else => "sun" }
report weather
```
```
weather    sun 70.00% · rain 30.00%
```

It is built for two kinds of work:

- **Game simulation**: dice, cards, boards and fights, with odds computed over every possibility. *What's the chance this attack takes the territory? Does going first matter? How many hit points make this fight fair?*
- **Forecasting**: estimates, scenarios and evidence. *When will we ship? Will revenue reach $100k a month? Given the pilot's numbers, how many sign-ups should we expect next month?*

**Contents:** [1 The model](#1-the-model-weighted-worlds) · [2 Five rules](#2-five-rules) · [3 Syntax tour](#3-syntax-tour) · [4 Distributions](#4-distributions) · [5 Evidence](#5-evidence) · [6 Reports](#6-reports) · [7 simulate](#7-simulate-distributions-from-code) · [8 Execution modes](#8-execution-modes) · [9 Semantics](#9-semantics-in-one-table) · [10 Pitfalls](#10-what-the-language-protects-you-from) · [11 Where Probl fits](#11-where-probl-fits) · [12 Examples](#12-examples) · [Appendices](#appendix-a-grammar)

---

## 1. The model: weighted worlds

A running Probl program is a set of **worlds**. Each world is an ordinary program state, in which every variable has one plain value, plus a **weight**: initially its probability, then its unnormalized contribution after evidence. A program starts as a single world with weight 1. The walkthrough below describes enumeration; sampling follows one path per run and estimates the same model.

- **Explicit choices split worlds.** `chance { p => A, else => B }` sends a copy of each world into `A` with its weight multiplied by p, and another into `B` with its weight multiplied by 1 − p. `if` and `while` follow boolean facts, and split worlds when given probabilities or boolean distributions.
- **Drawing splits worlds too.** `let r ~ 2d6` turns each world into eleven, one per total, each weighted by the chance of that total.
- **Identical worlds merge.** Where branches rejoin, worlds that have reached the same state become one, and their weights add up. This is what makes the approach practical:

  ```
  var pos = 0
  repeat 3 { pos += chance { 50% => 1, else => -1 } }

  after step 1:          -1 (½)                +1 (½)
  after step 2:    -2 (¼)         0 (¼ + ¼)          +2 (¼)      ← two paths reach 0: one world
  after step 3:  -3 (⅛)    -1 (⅜)          +1 (⅜)          +3 (⅛)
  ```

  A thousand steps have 2¹⁰⁰⁰ paths but only 1,001 possible positions, and positions are all the engine keeps. Many games behave the same way: the tree of possible games is astronomically large, but the set of distinct board states is small. Merging only helps when histories stop mattering, though: a model that keeps a whole hand of cards or a trajectory keeps its worlds apart.
- **Evidence re-weights worlds.** `observe` multiplies each world's weight by the probability of what was observed. Worlds that contradict it disappear.
- **Results are aggregated.** `report x` collects `x` from every world that reaches it and prints its weighted distribution.

## 2. Five rules

Everything else follows from these five rules.

1. **Choices follow every possibility.** Draw with `~` or use `chance { 30% => A, else => B }` for explicit weighted branching. `if 30% { A } else { B }` makes a fresh trial; a boolean condition follows an existing fact. Bare `observe` requires a boolean fact.
2. **A program is a set of weighted worlds.** Inside one world, every variable holds a single ordinary value. The uncertainty is in how many worlds there are and how much each one weighs.
3. **`~` draws an outcome; `=` binds an expression's result.** `let r ~ 2d6` gives `r` one number per world. `let d = 2d6` binds the distribution recipe; consuming it twice makes independent draws. An ordinary function call can itself draw, so `=` does not imply that its result is a distribution.
4. **Identical worlds merge.** When branches rejoin, worlds with the same state combine, ignoring variables that will never be read again. Merging changes nothing but rounding. Draws of distributions written out, like `let pump ~ bernoulli(95%)`, are taken just before their first use, so eligible draws can avoid creating combinations before they are needed. This optimization does not make arbitrary draws or large state spaces free.
5. **Output looks across worlds.** `report` prints distributions over all worlds, and `observe` conditions them on evidence. Code running inside a world only ever sees that world.

## 3. Syntax tour

Probl reads like a small modern scripting language: braces, no semicolons, `#` comments and optional type annotations.

### Literals

```probl
42   1_000_000   3.14   2.5e-3       # int and float
0b111   0xfab101   0xFA_B101          # binary and hex integers
30%   12.5%   -5%                     # percentages: 30% is 0.3
"hello, {name}"                       # strings, with interpolation
true   false                          # facts
[1, 2, 3]                             # list
["sun": 70%, "rain": 30%]             # map;  [:] is the empty map
{ hp: 12, ac: 16 }                    # record
1..6   0..<n                          # ranges: both ends included / end excluded
d6   2d6   d20   3d8                  # dice: these are distributions
5 to 10                               # an estimate: 90% sure it's between 5 and 10
```

### Text and sequences

Strings are immutable Unicode text, stored as UTF-8. Length, indexing, iteration and slicing count Unicode scalar values, with each element represented by a one-scalar string. For example, `len("🙂")` is 1 and `"a🙂b"[1]` is `"🙂"`. A displayed character can contain several scalars: `len("é")` is 2 (an `e` followed by a combining accent), and `len("🇫🇷")` is 2. Slices and reversal can separate these components. Text equality is exact, without automatic Unicode normalization; precomposed `"é"` and decomposed `"é"` are different strings.

Use `slice(xs, start, end)` for a string, list or range. Start is inclusive, end exclusive, and omitting end selects the rest. Both bounds use checked integer conversion (`1.0` is accepted, `1.4` is rejected), with `0 <= start <= end <= len(xs)`; invalid bounds are errors. Equal bounds give an empty result. Slicing preserves the sequence type, including compact ranges.

```probl
report "a🙂bc".slice(1, 3)           # "🙂b"
report [10, 20, 30].slice(1)        # [20, 30]
report slice(10..20, 2, 5)          # 12..14
report " \t hello\n".trim()         # "hello"
report "__foo__".trim("_")         # "foo"
report "abbacacb".trim("ab")        # "cac"
report "__foo__".trim_start("_")   # "foo__"
report "__foo__".trim_end("_")     # "__foo"
report "hello".starts_with("he")   # true
report "hello".ends_with("lo")     # true
report chars("a🙂")                # ["a", "🙂"]
```

Trimming removes Unicode whitespace by default, including tabs and newlines. An optional second argument supplies the set of scalars to remove instead. It is not a substring: order and repetition in the set do not matter. An empty set removes nothing. These functions return new values and leave their inputs unchanged. `upper` and `lower` use Unicode mappings independent of locale and may change length, such as `"ß".upper()` giving `"SS"`.

`map`, `filter`, `reduce`, `count`, sorting, `enumerate` and `zip` also accept strings, using scalar elements. `map`, `filter` and sorting return lists; use `join` to turn a list back into text. `reverse` of a string returns a string. Functions lift over finite distributions, and integer ranges stay compact until an operation needs to materialize their elements.

```probl
report "aß".map(c -> upper(c))          # ["A", "SS"]
report "abca".filter(c -> c != "a")     # ["b", "c"]
report "abca".filter(c -> c != "a").join("") # "bc"
report enumerate("a🙂")                # [[0, "a"], [1, "🙂"]]
report split("a,,b,", ",")             # ["a", "", "b", ""]
report split("é", "")                 # ["e", "́"], like chars("é")
```

Separators, prefixes and suffixes are literal text; there are no regular expressions. Splitting on a nonempty separator preserves empty fields. Splitting on `""` gives scalar elements, and returns `[]` for empty text. Text operations respect string byte, collection size and work limits; see [text semantics and limits](semantics.md).

### Dates

Dates are immutable Gregorian calendar dates, without a time of day or timezone. Construct one with `date("2027-01-31")` or `date(2027, 1, 31)`. Years run from 1 through 9999; strings require exactly `YYYY-MM-DD`. Invalid dates are errors.

`today` is a value captured once at the start of execution, shared by every world, function and nested `simulate`. It never advances while a model runs. The command line and playground use the UTC date by default. Pin it with `probl run model.probl --today 2026-09-29` to reproduce a forecast; `--stats` records the date used. A REPL session keeps its initial date throughout the session. Like the math constants, `today` can be hidden by a program's own name.

```probl
let holidays = [date("2026-10-02")]
let delivery = today.add_workdays(3, holidays)
report delivery
report delivery <= today + weeks(1)
report delivery.is_workday(holidays)
```

Adding or subtracting an integer shifts by calendar days; subtracting two dates gives an integer day count. `days(n)` rounds to a whole day and `weeks(n)` rounds `7*n`. `add_workdays(d, n, holidays?)` counts Monday–Friday, excluding any dates in the optional holiday list. It does not count the starting date; a negative count goes backwards and zero leaves the date unchanged. No national holidays are assumed. `weekday(d)` returns an English name such as `"Monday"`.

```probl
let anchor = date(2027, 1, 31)
report anchor.add_months(1)      # 2027-02-28
report anchor.add_months(2)      # 2027-03-31
report anchor.start_of_month()  # 2027-01-01
report anchor.end_of_month()    # 2027-01-31
report date(2024, 2, 29).add_years(1)  # 2025-02-28
report anchor.year()           # 2027; also month() and day()
report anchor                  # still 2027-01-31
```

`add_months` and `add_years` clamp to the last valid day of the target month. Consequently, adding one month twice may differ from adding two months once. For recurring schedules, compute each date from the original anchor, as in the [invoice example](../examples/12_invoice_calendar.probl). These functions, component extraction and workday helpers also lift over finite distributions. Month/year/workday counts and constructor components use checked integer conversion: `1.0` is accepted, `1.4` is rejected. All transformations return new dates and reject results outside the supported range.

### Sorting with a comparator

`sort` and `sort_desc` return stable copies: tied elements keep their original order. Supply a comparator to order values such as complex numbers or records:

```probl
let xs = [complex(2,3), complex(1,4), complex(0,5)]
report xs.sort((a, b) -> real(a) - real(b))
# [complex(0,5), complex(1,4), complex(2,3)]
report xs.sort_desc((a, b) -> abs(a) - abs(b))
```

The comparator returns a negative number, zero, or a positive number to put `a` before, tied with, or after `b`. It must return an int or finite float and define a consistent order. As with other collection callbacks, it cannot draw, observe or branch on uncertainty outside a local `simulate`. Default sorting rejects unordered types, including singleton complex lists.

### Math

Integers grow automatically: `factorial(30)` and `choose(100, 50)` return exact integers, and `10^100 + 1 - 10^100` is 1. There is no separate bigint syntax or type. Integer arithmetic and comparisons preserve all digits, including when comparing an integer with a float. `/`, negative powers, real math functions and mixed float/complex arithmetic produce approximations. `(10^400) / (10^400)` is 1.0, but passing `10^400` directly to `sin` is an error because it cannot be converted to a finite float. Resource limits bound integer size and computation; see [integer semantics](semantics.md#1-values-and-types).

Integer contexts accept exactly integral finite floats without rounding. This applies equally to `let n: int = 1.0`, `factorial(5.0)`, and `[10, 20].get(1.0, -1)`. Fractional inputs are errors; use `floor`, `ceil`, `trunc`, or `round` when rounding is intended. An unannotated `let x = 1.0` remains a float. Conversion preserves the stored float's value, including any rounding that happened before conversion.

Math functions use radians: `sin(pi / 2)` is 1 and `atan2(1, -1)` is `3 * pi / 4`. The constants `pi`, `e` and `euler_gamma` are floats; they can be hidden by your own names. `euler_gamma` is the Euler–Mascheroni constant, distinct from Euler's number `e` and the `gamma(shape, scale)` distribution. Results are floating-point approximations, so test identities with a tolerance rather than exact equality.

```probl
@mode sample(runs: 1000, seed: 1)
let angle ~ uniform(-pi, pi)
report sin(angle)               # a settled angle, transformed in each run
report cos(d6)                 # a finite distribution, transformed outcome by outcome
```

Use `log1p(x)` for `ln(1 + x)` and `expm1(x)` for `exp(x) - 1` when `x` may be tiny, and `hypot(x, y)` for a distance without squaring large or tiny numbers. Arguments outside a function's domain, such as `asin(2)`, and overflowing results are errors. Continuous distributions must be drawn before applying these functions.

For real inputs, the inverse hyperbolic functions are `asinh(x)` (any finite number), `acosh(x)` (`x >= 1`) and `atanh(x)` (`-1 < x < 1`). Like the forward functions, they return floats. Their complex extensions are described below.

For real inputs, `cbrt(x)` gives the real cube root, including negative inputs: `cbrt(-8)` is −2. `exp2(x)` gives 2 to the power `x`, complementing `log2`. Both return floats; overflow is an error, and very small results may underflow to zero.

`round(x)` rounds to an int, with halves away from zero. Give an integer `digits` to select decimal places; negative values round tens, hundreds, and so on. With `digits`, ints stay exact ints, while floats and probabilities return floats. Binary floating-point scaling can affect results near halfway cases. Rounding changes the value, not its display format.

```probl
report round(1.234, 2)     # 1.23
report round(1.125, 2)     # 1.13: halves away from zero
report round(1234, -2)     # 1200
report round(1.5)          # 2, an int
report round(1.5, 0)       # 2, a float
report trunc(-1.9)         # -1, an int: drop the fractional part toward zero
```

Integer mathematics stays exact: `choose`, `factorial`, `gcd`, `lcm` and `euler_phi` take ints and return ints, with an error if the result is too large. `factorial` supports 0 through 20; `ln_gamma(n + 1)` gives the logarithm of a larger factorial without forming it. `ln_gamma(x)` requires positive `x`. `erf(x)` is the error function used in normal probabilities.

Use `erfc(x)` when you need `1 - erf(x)`: it computes the complement directly, preserving small tails that subtraction would lose. For example, `erfc(8)` is about 1.12e-29, while `1 - erf(8)` rounds to zero. The standard normal probability above `x` is `erfc(x / sqrt(2)) / 2`.

```probl
report choose(52, 5)       # 2,598,960 five-card hands
report factorial(6)        # 720 orders of six items
report gcd(54, 24)         # 6
report lcm(6, 8)           # 24
report euler_phi(12)       # 4: the coprime integers are 1, 5, 7, 11
report ln_gamma(101)       # ln(100!), approximately 363.74
report erf(1)              # approximately 0.8427
```

`bit_length(n)` counts binary digits in an integer's magnitude: `bit_length(0)` is 0 and `bit_length(-7)` is 3. `ilog2(n)` gives the exact floor of the base-2 logarithm of a positive integer. Both work directly on bigints in constant time; `ilog2(2^100 - 1)` is exactly 99.

Binary (`0b`/`0B`) and hexadecimal (`0x`/`0X`) literals produce ordinary integers, regardless of size. Underscores may separate digits, as in `0b1111_0000`; hexadecimal digits accept either case. Use `-0xff` for a negative value. Prefixes and leading zeros do not fix a width: `bit_length(0x000f)` is 4. Reports still show integers in decimal.

Use functions for bit operations; `^` remains exponentiation. They require `int` arguments, work exactly on bigints, and lift over finite distributions.

```probl
report bit_and(0b1010, 0b1100)    # 8
report bit_or(0b1010, 0b1100)     # 14
report bit_xor(0b1010, 0b1100)    # 6
report bit_not(0)                 # -1
report bit_count(-0b1011)          # 3: count ones in the magnitude
report bit_and(bit_not(10), 0xff) # 245: an eight-bit complement
```

AND, OR, XOR and NOT use infinite two's-complement sign extension, so `bit_not(n)` is `-n - 1`. Use a mask such as `0xff` when you want a fixed width. `bit_count` ignores the sign, like `bit_length`. Integer size, work and memory limits apply.

### Complex numbers

Use `complex(re, im)` for complex numeric data. The imaginary component defaults to zero. Arithmetic supports real and complex operands; powers take integer exponents. `abs` gives the magnitude, `abs2` its square, `conj` the conjugate, and `arg` the phase in radians. `cis(theta)` constructs a unit-magnitude phase, up to floating-point rounding.

```probl
let i = complex(0, 1)
let z: complex = complex(3, 4)
report z * conj(z)         # complex(25, 0)
report abs(z)              # 5
report abs2(z)             # 25
report real(z)             # 3
report imag(z)             # 4
report i ^ 2               # complex(-1, 0)
report cis(pi / 2)         # approximately i
report mean(one_of([z, conj(z)]))   # complex(3, 0)
report sqrt(complex(-1))           # complex(0, 1)
report ln(complex(-1))             # complex(0, pi)
report exp(i * pi)                 # approximately complex(-1, 0)
report cos(i)                     # approximately complex(1.54308, 0)
```

Complex components are finite floats; overflow and division by zero are errors. There is no ordering or implicit conversion to a probability. Type annotations require an actual complex value: write `let z: complex = complex(1)`. Compare approximate results using `abs(a - b) < tolerance`.

Roots, logarithms, exponentials, trigonometric and hyperbolic functions (including their inverses) accept complex values. Real inputs keep their real domains: `sqrt(-1)` is still an error. Complex functions return one principal value. Other logarithm branches are explicit: `ln(z) + complex(0, 2*pi*k)` for integer `k`; they do not create probabilistic alternatives. `ln(0)` is undefined. See [complex semantics](semantics.md) for branch cuts and the distinction between real `cbrt(-8)` and principal complex `cbrt(complex(-8))`.

A distribution over complex values is ordinary uncertainty about a number. Opposite outcomes do not cancel. Complex values are a foundation for amplitude calculations; quantum states and gates are future work described in the [design note](design/quantum-and-complex.md).

### Variables

```probl
let price = 49              # immutable
var customers = 0           # mutable
customers += 120

let roll ~ 2d6              # a draw: one world per possible total
var market ~ one_of([Boom: 20%, Steady: 60%, Slump: 20%])
```

Prefix `~` draws within an expression: `let roll = ~2d6` is equivalent to `let roll ~ 2d6`. It binds tightly: `~d6 + ~d6` draws two faces, and `~(d6 > 3)` draws a boolean. A `prob` draws directly to a boolean. Other plain values remain point values: `~30%` is the float `0.3`; use `~prob(30%)` for a trial.

All values have **value semantics**, including lists, maps and records: assigning or passing one gives an independent copy. Copies are cheap because the collections are persistent data structures. Nothing is shared between worlds, which is what makes splitting and merging safe.

### Branching

```probl
let rainy ~ bernoulli(30%)
if rainy {                             # true in 30% of worlds
  umbrella = true
}

let hit ~ d20 + 5 >= 15
let damage = if hit { 8 } else { 0 }    # `if` is also an expression

chance {                               # several weighted branches
  50% => pos += 1
  30% => pos -= 1
  else => {}                           # the remaining 20%: nothing happens
}

let weather = chance { 60% => "sun", 30% => "rain", else => "snow" }

match weather {
  "sun" => mood += 1
  "rain" | "snow" => mood -= 1
}
```

Conditions and match guards accept `bool`, `prob`, or `dist[bool]`. A boolean follows its existing outcome; a probability or boolean recipe makes a fresh trial on each evaluation. Numbers convert contextually after checking `[0, 1]`, so `if 30% { "rain" } else { "sun" }` and `let p = 30%; if p { ... }` work. Variables and calculations use the same check; `if 2` is an error. `let hit ~ d20 + 5 >= 15` draws the boolean distribution into two outcomes, without keeping the die's twenty individual faces.

The weights in `chance` are probabilities, such as `P(2d6 == point)`, or numbers checked in `[0, 1]`, such as `hit_chance - 5%`. Literals, variables and calculations all convert contextually. `else` takes the remainder. Weights above a total of 100% are an error, as is a value-producing `chance` with a positive remainder and no `else`.

### Loops

```probl
for month in 1..18 { … }
repeat 10 { … }
while hp > 0 and foe_hp > 0 { … }
loop { …; if done { break } }
```

A loop with an uncertain condition runs until every world has left it. Some loops never end with certainty: `while ~d6 != 6` could in principle roll forever.

- **Loops that come back are solved.** If the worlds come back to states they were in before, as a tennis game returns to deuce, Probl solves the loop exactly, as a Markov chain.
- **Other loops stop at ε.** A loop that counts its rounds never comes back to a state. Probl stops such a `while` or `loop` once the worlds still inside weigh less than ε times the weight that entered it (ε is 10⁻¹² by default; change it with `@epsilon 1e-9`). That remainder is reported as *unresolved* weight rather than silently dropped.

`for` and `repeat` always run to the end.

Recursion works the same way. A function can call itself, even with the same arguments, as in "roll again on a 6":

```probl
fn explode() -> int {
  let r ~ d6
  if r == 6 { return 6 + explode() }
  return r
}
report explode()          # mean 4.20 · … · a 7 is a 6 then a 1: 2.78%
```

Probl works out such a call by rounds, each using the previous round's result, until the weight still waiting is below ε. A call that can never return is an error.

```probl
var rolls = 1
while ~d6 != 6 {        # a fresh roll each time: 5/6 chance of going round again
  rolls += 1
}
report rolls           # mean 6.00 · sd 5.48 · 5% 1 · median 4 · 95% 17
```

### Functions

```probl
fn attack(a: Fighter, target: Fighter) -> int {
  let roll ~ d20
  if roll > 1 and roll + a.to_hit >= target.ac {
    let dmg ~ a.dice + a.bonus
    dmg                                   # the last expression is the result
  } else {
    0
  }
}

let doubled = [3, 5, 8].map(x -> x * 2)   # lambdas; x.f(y) is the same as f(x, y)
let magnitudes = [-2, 3].map(abs)        # named functions and builtins are values
let largest = [-8, -3, -12].reduce(max) # starts with the first element
let total = [].reduce((a,b)->a+b, 0)    # optional initial value handles empty lists
```

Collection callbacks (`map`, `filter`, `count`, `reduce`, and ordering comparators) cannot execute draws, observations, scores or probabilistic branches in the caller's worlds. This restriction is the same in enumeration and sampling, including helper calls. Use a loop for random traversal. A callback can compute a local distribution with `simulate`, or transform an existing distribution without drawing it.

Functions can branch, draw and observe, so calling one can split the caller's world. A call doesn't normalize anything: the splits and observations inside a function become part of the caller's weights. There is one restriction: **a function can read anything in scope, but it can only assign its own local variables.** It returns whatever it changes. That keeps every function a *probabilistic function of its inputs*, so the engine can compute the distribution of `attack(hero, goblin)` once and reuse it in every world and every round. (A function that calls `print`, directly or not, runs every time instead: its output is debug output, one line per world that runs it.)

Function references capture their free variables when the reference is created. Builtins retain their normal argument counts, including optional and variadic arguments. Calls through a variable retain the same runtime validation and callback effect rules.

For extrema, `min(a,b,…)`/`max(a,b,…)` compare at least two candidates; distribution candidates produce a distribution. Use `minimum(xs,compare?,default?)`/`maximum(xs,compare?,default?)` to select a collection element, or `minimum(d)`/`maximum(d)` for distribution support bounds:

```probl
report max(d6, 3)                         # a distribution
report maximum(3d8)                       # 24
report maximum([-8, -3, -12])              # -3
report [].maximum(default: 0)              # 0
report [-8, -3].maximum(default: 0)         # -3; default is not a candidate
report [complex(1),complex(2)].maximum((a,b)->abs(a)-abs(b))
report [3,1,2].highest(2)                  # [3,2]; count is required
```

Reduction folds left to right. Without an initial value, empty collections error and singleton collections return their element. With an initial value, every element participates; migrate the former `reduce(initial, f)` order to `reduce(f, initial)`.

Collection selectors retain the first element on ties and never combine recipes implicitly. Optional extrema defaults apply only to empty collections: `xs.maximum(compare, default: value)` also works, as does a third positional default after the comparator. All argument expressions are evaluated eagerly. Distribution bounds require fully resolved support and a finite endpoint; continuous bounds refer to closed support. See [extrema semantics](semantics.md) for the complete contract.

### Types

```probl
type Fighter = { hp: int, ac: int, to_hit: int, dice: dist[int], bonus: int }
enum Market { Boom, Steady, Slump }

let hero = Fighter { hp: 12, ac: 16, to_hit: 5, dice: 1d8, bonus: 3 }
let tougher = hero with { hp: 20 }       # a copy with some fields changed
```

The built-in types are `bool`, `int`, `float`, `prob`, `str`, `date`, `list[T]`, `map[K, V]`, `bag[T]` and `dist[T]`, plus records and enums. Note `dice: dist[int]`: distributions are ordinary values you can store, pass and return.

Use the prefix operator `typeof` to inspect a runtime value. It returns a string and inspects a distribution without drawing from it:

```probl
report typeof 33%              # "float"
report typeof prob(33%)        # "prob"
report typeof 0.33             # "float"
report typeof (33% + 1%)       # "float"
report typeof 150%             # "float"
report typeof d6               # "dist[int]"
report typeof (d6 > 3)         # "dist[bool]"
report typeof normal(0, 1)     # "dist[float]", even when enumerating
report typeof [d6, d8]         # "list[dist[int]]"
let x ~ d6
report typeof x                # "int"
report typeof x == "int"       # true
```

`33%` is shorthand for `0.33`, a float. `prob(x)` checks that a number is finite and within `[0, 1]` and returns a probability; it also explicitly converts booleans to 0 or 1. It never clamps or draws. Numbers convert implicitly, with the same range check, when a declared type, parameter, condition or `score` expects `prob`. This expectation flows into the result branches of `if`, `chance`, `match` and blocks: `let p: prob = 33%` creates an actual probability, as does the argument in `bernoulli(33%)`. Variables and arithmetic also work: `let rate = 33%; binomial(100, rate)` and `binomial(100, 1 - rate)`. This leaves `rate` a float; `let p: prob = rate` creates a probability. Typed containers convert their contents recursively without changing the source collection. Ordinary probability arithmetic returns numbers, and probabilities widen to floats in numeric contexts. Boolean, numeric, probability and distribution values remain distinct.

Parenthesize compound operands: `typeof (x + 1)`. `typeof` evaluates its operand once, so calls and errors still occur. It describes current contents, not an inferred static type: empty lists give `"list[unknown]"`, mixed element types give `"list[any]"`, and named records/enums give their type name. `unknown` and `any` here are descriptive markers, not annotation types. Directly inspecting a delayed sampled parameter with `typeof p` preserves exact Bayesian updates; calculating an expression involving `p` still needs its value. See [the type inspection contract](semantics.md#1-values-and-types).

Type annotations are optional for program values and required for file input. Impossible literal annotations are rejected during compilation; general type checks currently happen at runtime when the expression is reached. Full static type inference is still planned. Annotations also supply context for checked numeric conversions, so `typeof` reflects the converted value.

### Data

```probl
type Day = { day: date, visitors: int, signups: int }
let pilot: list[Day] = read("data/pilot.csv")      # next to the program
let counts: list[int] = read("-")                  # standard input, one value per line
```

`read` loads CSV (as a list of records), JSON (as any type data can have) and plain lines. The declared type decides how the data is read. Fields match columns and keys ignoring case and punctuation, and unused columns are ignored. Data that doesn't fit the type is an error before the program runs, with the line it's on. The data is read once, and is the same in every world and every run. `probl schema FILE` suggests a type. The details are in [reading data](data-input.md).

Three of these types describe uncertainty, and keeping them apart is what lets `and` and `or` mean what they say:

- `bool` is a **fact**: `true` or `false` in each world. Comparisons of settled values give facts, and so do `and`, `or`, `not` and `in` on facts.
- `prob` is a **probability**: a checked finite value from 0 to 1, such as a success rate. Use it directly in conditions, draws (`~p`), `score p`, or a `chance` weight. `bernoulli(p)` explicitly constructs the equivalent `dist[bool]`. Arithmetic (`p * 2`, `1 - p`) gives a `float`. A probability-consuming context checks and converts it; `prob(x)` also constructs a probability explicitly.
- `dist[T]` is a **distribution**: `d6`, `bernoulli(30%)`, or `d6 > 4`, a `dist[bool]` that is an uncertain fact.

## 4. Distributions

### Making them

| Kind | Examples |
|---|---|
| Dice | `d6`, `2d6`, `d20 + 5`; `roll(4, d6)` gives the individual dice, sorted high to low |
| Choices | `one_of(["rock", "paper", "scissors"])`, `one_of(1..10)`, `one_of([Boom: 20%, Steady: 60%, Slump: 20%])` |
| Yes/no | `bernoulli(30%)`: `true` with probability 30%, so `let rain ~ bernoulli(30%)` is a fact |
| Counts | `binomial(n, p)`, `poisson(rate)`, `geometric(p)` |
| Cards | `bag([card: count, …])`, then `let c = deck.take()` draws without replacement |
| Continuous | `normal(mean, sd)`, `lognormal(mu, sigma)`, `uniform(lo, hi)`, `beta(a, b)`, `gamma(shape, scale)`, `exponential(rate)`, `triangular(lo, mode, hi)`, `pert(lo, mode, hi)` |
| Estimates | `5 to 10`: 90% confident it's between 5 and 10, as a lognormal, so both ends must be positive (as in Squiggle). For a quantity that can be negative, say which shape you mean: `normal_range(-8%, 0%)` is a normal with that 90% interval |
| From code | `simulate { … }`: the distribution of a block's result (section 7) |

`deck.take()` selects an item, removes one copy from `deck` in that world, and returns the item unchanged. The deck must be declared with `var`; taking from an empty bag is an error. Each call selects anew from the remaining items. `one_of(deck)` instead constructs a distribution without changing the bag, and `~one_of(deck)` draws without removing anything. If an item is itself a probability or distribution, `deck.take()` returns that recipe; `~deck.take()` additionally draws from the returned recipe.

Continuous distributions can't list their outcomes, so what you can do with them depends on the mode. Drawing one (`let x ~ normal(0, 1)`) produces a float: a concrete value in sampling, or an analytic outcome in enumeration. Enumeration supports affine arithmetic on a shared draw, threshold conditions, and numeric reports; nonlinear and multivariate calculations require sampling. Comparing one with a number works in both modes, from its CDF: `normal(0, 1) > 1.96` is 2.50%. `mean`, `sd`, `median`, `quantile`, `cdf` and `pdf` use the formulas. A choice among options that include one, such as `chance { 35% => 1 to 3, else => 0 }`, is a mixture, and drawing from it picks an option first. Arithmetic on a continuous distribution (`normal(0, 1) * 2`) isn't supported yet: draw a value, then compute with it.

### Computing with them

Operators and built-in functions accept distributions and return new ones:

```probl
d20 + 5                             # 6 … 25
2d6 * 10                            # 20, 30, …, 120
roll(4, d6).highest(3).sum()        # "4d6, drop the lowest": mean 12.24
roll(5, d10).count(x -> x >= 8)     # successes in a dice pool: mean 1.5
```

**Every occurrence of a distribution is an independent draw:**

```probl
let die = d6
report die + die      # two independent rolls: 2 … 12, peaking at 7 (the same as 2d6)
report 2 * die        # one roll, doubled: 2, 4, …, 12
let r ~ d6
report r + r          # r is settled, so this is also 2, 4, …, 12
```

### Comparing them gives uncertain facts

```probl
2d6 >= 10             # 16.67%
d20 + 5 >= 15         # 55.00%
normal(0, 1) > 1.96   # 2.50%
2d6 > 2d6             # 44.37%: two independent rolls
```

Each of these is a `dist[bool]`. `report` prints its probability of true; a condition makes a fresh trial. Draw and bind an event before observing and reporting the same outcome.

### Events need identities

`and`, `or` and `not` work on facts. Draw an event with `~` and it becomes a fact, true in some worlds and false in the others, and ordinary logic applies:

```probl
let rain ~ bernoulli(30%)     # a fact: true in 30% of worlds
let late ~ bernoulli(20%)
report rain and late          # 6%: two independent events
report rain and rain          # 30%: one event, checked twice
```

Recipes compose independently, just like dice arithmetic: `prob(30%) and prob(30%)` gives `prob(9%)`, and `(d6 > 3) and (d6 > 3)` gives a `dist[bool]` with a 25% chance of true. `not p` complements a probability; `p or q` gives `p + (1-p)*q`. Combining a boolean distribution with another recipe preserves a distribution, including any unresolved mass. Only an actual `false` short-circuits `and`, and only an actual `true` short-circuits `or`. Draw first when both uses must refer to the same event.

A probability that is itself uncertain stays a distribution. Here a success rate is 10% or 90%, equally likely, and two trials share it:

```probl
let rate ~ simulate { chance { 50% => prob(10%), else => prob(90%) } }   # one rate per world
let a ~ bernoulli(rate)
let b ~ bernoulli(rate)
report a and b                                        # 41%: ½ × 0.1² + ½ × 0.9²
```

### Asking about them

Statistical queries take an explicit population: a distribution or a nonempty list. `mean(1d8 + 3)` is 7.5; `mean([1, 1, 4])` is 2. Lists weight every element equally, including repetitions. `variance` and `sd` use the population definition (divide by the list length, not length minus one). These queries never look across worlds.

```probl
report median_low(["aa", "cc", "bb"])    # "bb"
report median([1, 2, 3, 4])                # 2.5
report median(one_of([1, 4]))              # 2.5, just like median([1, 4])
report median_low([1, 4])                  # 1
report median_high([1, 4])                 # 4
report quantile([1, 2, 3, 4], 50%)         # 2
report mean([complex(1, 2), complex(3, 4)]) # complex(2, 3)
```

`median` accepts real numbers or dates, and averages the lower and upper medians when they differ. Lists and distributions follow the same rule: weights count, so a distribution's bounds differ only when exactly half its resolved probability lies on either side of a gap. An integral midpoint of two ints stays an exact int; fractional and floating-point midpoints return floats and must fit the finite float range.

`median_low` and `median_high` select the lower or upper middle element of an even-length list, or the endpoints of a distribution's median interval. They accept strings, dates, bools (`false` before `true`) and enums as well as numbers. `median` rejects strings, bools and enums even for singleton or odd-length populations: use the explicit low/high functions. Complex numbers have no ordering, so all three medians reject them. `quantile` keeps its existing lower-quantile rule and never interpolates finite outcomes; `quantile([1, 4], 50%)` is 1.

`mean` accepts numeric lists (real or complex) and date lists, or distributions of those values. Date means average calendar-day positions; date midpoint medians average their two endpoints. Both round to the nearest calendar day, with exact half-day ties choosing the earlier day, independently of the 1970 epoch. For example, October 1 and October 9 give October 5, while October 1 and October 2 give October 1. `median_low` and `median_high` always preserve the selected date. Strings cannot be averaged.

`cdf`, `pmf` and `support` also accept lists; `pdf` requires a continuous distribution. Empty lists are errors, and list elements that are distributions must be drawn first or explicitly combined with `one_of`.

Scalars are errors: `mean(pi)`, `median(pi)` and `mean(x)` after a draw do not silently become singleton distributions. Use a report to summarize values across worlds, or put the model inside `simulate` to obtain a distribution:

```probl
let x = if 50% { e } else { pi }
report x                                  # summarizes the two outcomes
let d = simulate { if 50% { e } else { pi } }
report mean(d)                            # (e + pi) / 2
```

`simulate { x }` captures the current world's already-bound `x`; it does not rerun the choice that produced it. If a singleton population is intended, make it explicit with `[x]` or `one_of([x])`.

`P(d)` specifically requires `dist[bool]`: `P(d6 > 4)` is 33.33%. For a bound boolean, use `report event` to measure its probability across worlds or `prob(event)` to convert it to 0 or 1. `P(true)` and `P(prob(30%))` are errors.

## 5. Evidence

```probl
let sick ~ bernoulli(1%)                      # 1 in 100 people has the disease
score if sick { 95% } else { 8% }             # likelihood of their positive test
report sick                                   # 10.71%
```

- `observe c` requires a boolean and removes worlds where it is false. An observed event then reports true with probability 100%.
- `score p` multiplies the world's weight by a probability likelihood. It accepts numeric literals, variables and calculations after checking `[0, 1]`.
- `observe ~p` observes an anonymous boolean draw. It has the same likelihood as `score p` for a probability, without naming an outcome. Reporting `p` still reports the original recipe; bind `let event = ~p` if the outcome must be reused.
- `observe v from D` multiplies by the probability of seeing the value `v` under `D`. For example: `observe 11 from binomial(250, rate)`. When sampling, a continuous `D` contributes its density at `v`.
- The **evidence** is the weight that survives: the probability of all the observations. Reports are normalized over it, and the run summary shows it (8.87% above). Observations inside a function count; those inside `simulate` don't (section 7).
- If the observations rule out every world, the evidence is impossible, and the run stops with an error rather than printing reports that mean nothing.

**Use `~` to bind the outcome you want to condition.** `=` stores a recipe. `let x = 3d8; observe x > 10; report x` now fails because its comparison is a distribution. To condition the total, write:

```probl
let x ~ 3d8
observe x > 10
report x             # 11–24; mean 15.11, sd 2.94; evidence 76.56%
```

Here `x` is the same total throughout each world, and `observe` removes worlds where it is 10 or less. To store the conditional distribution as a reusable recipe, write `let above_ten = simulate { let x ~ 3d8; observe x > 10; x }`.

**No `observe` or `score` may follow a `report`.** The compiler rejects a program in which one could, including through a loop or a function that observes, so every report sees all the evidence. A small finite-state filter can instead carry a distribution and condition it inside a fresh `simulate` scope for each reading, as in the [sensor example](../examples/17_sensor_tracking.probl). Those observations stay local to that simulation; the outer reports do not accumulate their evidence.

## 6. Reports

```probl
report win                                        # the label defaults to the source text
report rolls as "rolls per bet"
report customers * price by month as "MRR ($)"    # one distribution per month
```

What gets printed depends on the type:

| Reported value | Printed as |
|---|---|
| a fact (`bool` or `dist[bool]`) | one percentage: `win  49.29%` |
| number | `mean 3.38 · sd 3.00 · 5% 1 · median 2 · 95% 9`, plus a sparkline when there are few distinct values |
| date | `5% 2027-01-26 · median 2027-02-18 · 95% 2027-03-24` |
| anything else | each value with its probability: `sun 70.00% · rain 30.00%` |
| `… by key` | one row per key: a table, or a fan chart over time |

A report inside a branch describes only the worlds that pass through it, and says what share of the weight that was: `win  100.00% (reached in 1.00% of worlds)`. A report inside a loop needs `by`. A single loop over a directly written integer range, keyed by its own variable (`for month in 1..18 { report mrr by month }`), contributes at most once per world/key. Other cases, including nested loops and collections that might repeat elements, are conservatively labelled `(per visit)`. Every visit counts; no duplicates are removed. `report` is only allowed at the top level of a program, not inside functions or `simulate`. For debugging, `print(x)` prints once per world that runs it (worlds that have merged count once).

When a loop left some weight unresolved, a probability it could visibly change is printed as the range it must lie in, such as `0.00%–99.80%`; means and quantiles get a note instead, since a rare, very large value could move them anywhere. `--fractions` adds the simplest fraction near each probability: `49.29% (≈ 244/495)`. It's a hint for recognizing an answer, not a proof: weights are floating-point numbers.

When sampling, probabilities normally come with a standard error: `33.0% ± 0.2%`. An ordinary unweighted Bernoulli report instead shows a labelled 95% Wilson interval when fewer than 30 runs contributed or all outcomes agree. Two successes from two contributing runs give an interval of about 34.24%–100%. Weighted and repeated-visit reports do not use that interval. A zero computed error is labelled `MC error not estimable` for ordinary sampled probabilities; directly reported finite distributions instead identify their integrated outcomes and zero empirical error. Neither establishes that unseen outcomes are impossible. Reports and table rows with fewer than 30 effective contributions show their contributing-run count and effective sample size.

A mean shows its standard error when visible at the printed precision (`mean 3.40 ± 0.01`); standard deviations and quantiles have none. Small nonzero summaries and table cells use scientific notation instead of rounding to zero, and probabilities just below one retain their nonzero tail. A mean limited by floating-point cancellation is shown as `≈0`, without changing its value. The [reference semantics](semantics.md#14-sampling) details eligibility and reliability limits.

## 7. `simulate`: distributions from code

```probl
let damage = simulate { attack(hero, goblin) }    # a dist[int]
report mean(damage) as "damage per attack"        # 4.35

let game = simulate { race() }                    # turns that one player needs
let mine ~ game
report game >= mine as "first player wins"        # this `game` is a fresh race, independent of `mine`
```

`simulate { … }` runs its block as a separate model, starting from a copy of the current world, and returns the distribution of the block's final value. The current world doesn't split. The result is always a distribution, even a distribution of probabilities (see *Events need identities*). Observations inside the block condition its result and don't count as the program's evidence; if they rule out every world, that's an error. The block is always enumerated, even in sample mode, so its result is exact; sampling inside it isn't supported yet.

It connects Probl's two styles: the imperative one (branch, draw, update) and the distributional one (distributions as values). It also lets a strategy compare its options before acting:

```probl
let if_hit   = simulate { finish(hit(hand), dealer) }
let if_stand = simulate { finish(hand, dealer) }
if mean(if_hit) > mean(if_stand) { hand = hit(hand) }
```

## 8. Execution modes

All modes compute the same model; switching mode changes speed and accuracy, never meaning. `enumerate` and `sample` exist today. The others wait until their estimators are specified: merged samples need statistical bookkeeping, and dropped prior weight doesn't bound a posterior (see the [audit](design/project-audit.md), D2 and D4).

| Mode | How it runs | Use it for | Accuracy shown as |
|---|---|---|---|
| `enumerate` | follows every branch; identical worlds merge | dice, cards, boards, discrete models | rounding only; unresolved weight, with ranges for the probabilities it could change |
| `sample(runs: n, seed: s)` | each run takes one branch at random, and draws one value from each distribution; observations weight the runs | continuous estimates, very large models | standard errors; the effective sample size when observations weight the runs |
| `beam(worlds: n)` | like `enumerate`, but keeps only the n heaviest worlds | discrete models too big to enumerate | not built yet |
| `particles(runs: n, seed: s)` | like `sample`, and resamples the runs after observations | time series with streams of evidence | not built yet |
| `auto` (default) | `enumerate` for now; later, `sample` when a model can't be enumerated, saying so | getting started | as for the mode it picked |

Sampled runs are independent: they never merge, and every call makes its own choices. Evidence weights them (likelihood weighting); the **effective sample size** in the summary line says how many equally weighted runs they're worth, and when it's small, so is the confidence the estimates deserve. The same program, data, execution date, seed and engine version give reproducible successful output across supported hosts.

**Exact updates.** Some priors and observations fit together: a `beta` observed through `binomial` or `bernoulli`, a `gamma` through `poisson`, and a `normal` through `normal` with a known spread (they're *conjugate*). For such a variable, a sampled run doesn't draw it first and weight itself by the data. It keeps the variable's distribution, updates it exactly with each observation, and draws the variable from the result when something first needs its value. The model is the same, but the runs are worth more: in an A/B test with 30 days of data, 100,000 runs are worth 100,000 instead of 863, and the evidence is exact. `probl run --no-conjugate` turns it off, for comparing, and `probl run --stats` shows which variables were updated this way.

Only the observations before the variable's first use update it exactly. The ones after weight the runs as usual. If they disagree with the earlier ones, fewer runs can carry the weight than without exact updates, so use such a variable after all its observations when you can.

Choose a mode with a pragma at the top of a file, or override it on the command line:

```probl
@mode enumerate
@mode sample(runs: 100_000, seed: 42)
```
```sh
probl run model.probl --mode sample --runs 100000 --seed 3   # sample, whatever the program says
probl run model.probl --threads 2                            # sample on at most 2 cores
probl run model.probl --no-conjugate                         # sample without exact updates, for comparing
probl run model.probl --timeout 10                           # stop after 10 seconds
probl check model.probl                                      # parse and check without running
probl check --data model.probl                               # and check its data
probl schema data.csv                                        # suggest a type to read a file with
probl repl
```

The output always starts with a line saying how the numbers were computed, so an enumerated answer is never confused with an estimate:

```
enumerated · evidence 8.87% · unresolved < 1e-12
sample · 200,000 runs · seed 7 · evidence 3.60e-15 (± 0.00%) · effective sample size 200,000
```

When the program observes, the line gives the evidence: the probability of all the observations. Sampling estimates it, with its standard error, which is 0 when every observation is an exact update. When an observation uses a density (`observe 1.5 from normal(mu, 1)`), it gives the evidence's logarithm (`log evidence -1.825 ± 0.003`), which compares models: the difference of two models' log evidence on the same data is the logarithm of their Bayes factor.

Sampling uses every core by default. The runs go in batches of 1,000 with random numbers of their own, combined in order, so the output is the same on any number of cores.

Whoever runs a program sets limits on its worlds, work, loop iterations, call depth and output (`--max-worlds`, `--max-work`, `--timeout`). A program's `@max_worlds` and `@max_iterations` can lower these limits, never raise them.

## 9. Semantics in one table

For a single world with state σ and weight w (the full rules, including evaluation order, calls and reports, are in the [reference semantics](semantics.md)):

| Statement | Produces |
|---|---|
| `x = e` | (σ[x ↦ e], w) |
| `x ~ D` | one world per outcome v of D: (σ[x ↦ v], w · P(D = v)) |
| `if c { A } else { B }` | A runs on (σ, w · t), B on (σ, w · f), where t and f are c's true and false weights |
| `chance { p₁ => A₁ … else => B }` | each Aᵢ run on (σ, w · pᵢ), and B on (σ, w · (1 − Σpᵢ)) |
| `while c { A }` | exits with weight w · f; A runs with weight w · t, then the condition is evaluated again |
| `observe c` | (σ, w) if the boolean c is true; no world if c is false |
| `score p` | (σ, w · p) |
| `observe v from D` | (σ, w · P(D = v)) |
| `A` followed by `B` | B runs on every world A produced |
| join point | (σ, w₁) and (σ, w₂) become (σ, w₁ + w₂) |

For conditions, c may be a boolean, probability, number checked in `[0, 1]`, or boolean distribution. For a probability p, t = p and f = 1 − p; booleans are the 0/1 cases. A distribution's unresolved mass remains unresolved. Bare `observe` accepts only booleans; each pᵢ and `score` argument must be a probability or a number checked in `[0, 1]`.

This is the standard semantics of probabilistic programs as functions from a state to a distribution over states (Kozen, 1981). Merging changes nothing but rounding, because a set of worlds is a weighted sum of states, and equal states simply add. Liveness analysis only lets the engine forget variables that can no longer affect anything. Sample mode estimates the same distribution: instead of splitting a world, probabilistic conditions, `chance` and `~` choose one outcome for each run.

## 10. What the language protects you from

- **A distribution where a value is needed.** `for i in 1..d6` is an error: *"`d6` is a distribution, but a range needs a number. To use one roll, write `let n ~ d6` first."*
- **Drawing when a probability would do.** `let r ~ d20` followed only by `if r + 5 >= 15` creates 20 worlds. Drawing just the event, `let hit ~ d20 + 5 >= 15; if hit { ... }`, creates 2. A planned lint will suggest this form when `r` isn't used again.
- **Confusing a recipe with an outcome.** `storm and storm` composes two independent trials when `storm` is a probability. To test the same event twice, draw it first with `let stormy = ~storm`. Bare `observe storm` rejects the recipe; observe the drawn boolean instead.
- **Matching a distribution.** `match d6 { … }` is an error: each arm would test a fresh roll. Draw the value first.
- **Evidence after a report**, or evidence that rules out every world: both are errors.
- **Unsupported continuous calculations.** Enumeration preserves the identity of a continuous draw through affine arithmetic and threshold conditioning. `let x ~ uniform(0, 2); report x + 1` has mean 2 and support 1–3; adding `observe x > 1` before the report gives mean 2.5 and support 2–3. Nonlinear calculations such as `x*x`, combinations of independent continuous draws, and continuous draws inside `simulate` remain unsupported. See [analytic outcomes](semantics.md#analytic-outcomes-in-enumeration) for the current boundaries.
- **State that never merges.** Worlds merge only when their values are exactly equal. Floats that accumulate (`balance += 0.1`) rarely are, so float-heavy state stops merging and enumeration slows down. Count cents as integers, or `round()`. `probl run --stats` shows the peak number of worlds and how many calls were reused.
- **Loops that never finish.** Uncertain loops stop at ε and report the unresolved weight. A loop that keeps every world inside (`while true` with no `break`) hits an iteration cap and fails with a clear error.

## 11. Where Probl fits

| | Probl | AnyDice, Troll | Squiggle, Guesstimate | WebPPL, Pyro, Stan |
|---|---|---|---|---|
| Programs are | imperative code over weighted worlds | dice expressions and small functions | estimates combined by sampling | generative models for statistical inference |
| Every possibility followed, for discrete models | yes, merging equal states | yes, for dice | no, sampling | limited |
| State and loops | yes | limited | limited | yes |
| Conditioning on evidence | `observe` | not a focus | not a focus | yes, with advanced inference |
| Continuous quantities | sampling and a restricted analytic subset | no | yes | yes |

Probl borrows `a to b` estimates from Squiggle, dice notation from AnyDice and tabletop games, and `observe` from probabilistic programming languages like WebPPL. Weighted states, probabilistic branching and state merging all have prior art; Probl's bet is putting them behind ordinary imperative code, with diagnostics that say how an answer was computed. Merging keeps many game models small. A loop whose states come back is solved as a Markov chain, as PRISM does; the others are followed until the weight still playing is negligible. For exact inference at larger scale, the research language Dice (Holtzen et al., 2020) compiles programs to binary decision diagrams, which could become a later backend.

## 12. Examples

| File | Domain | What it shows |
|---|---|---|
| [`01_tour.probl`](../examples/01_tour.probl) | | every core idea on one page |
| [`02_craps.probl`](../examples/02_craps.probl) | game | draws, a loop with no fixed end, fractions |
| [`03_rpg_duel.probl`](../examples/03_rpg_duel.probl) | game | records, functions, distributions as fields, `simulate`, a balancing sweep |
| [`04_risk_battle.probl`](../examples/04_risk_battle.probl) | game | dice pools; merging collapses a huge tree |
| [`05_snakes_and_ladders.probl`](../examples/05_snakes_and_ladders.probl) | game | maps, long games, independent copies of a distribution |
| [`06_blackjack_dealer.probl`](../examples/06_blackjack_dealer.probl) | game | cards without replacement, tables with `report … by` |
| [`07_launch_forecast.probl`](../examples/07_launch_forecast.probl) | forecasting | `a to b` estimates, regime switching, a fan chart over months |
| [`08_signup_forecast.probl`](../examples/08_signup_forecast.probl) | forecasting | `observe … from`, learning a rate, then forecasting with it |
| [`09_roadmap.probl`](../examples/09_roadmap.probl) | forecasting | risks and dates: an illustrative forecast of the original plan, not a release schedule |
| [`10_quantum_key.probl`](../examples/10_quantum_key.probl) | physics | a classical BB84 intercept–resend model; measurement probabilities and Bayes' rule, without complex amplitudes |
| [`11_delivery_dates.probl`](../examples/11_delivery_dates.probl) | forecasting | delivery uncertainty from `today`, weekends, explicit holidays and deadline probability |
| [`12_invoice_calendar.probl`](../examples/12_invoice_calendar.probl) | forecasting | recurring invoices anchored to a calendar day, payment delays and monthly cash-flow buckets |
| [`13_renewal_dates.probl`](../examples/13_renewal_dates.probl) | forecasting | leap-day renewals, notice periods and uncertain response dates |
| [`14_stock_decision.probl`](../examples/14_stock_decision.probl) | decisions | expected profit, choosing before uncertainty resolves, perfect information and regret |
| [`15_service_queue.probl`](../examples/15_service_queue.probl) | operations | continuous arrivals and service times, shared scenarios for staffing comparisons |
| [`16_predictive_check.probl`](../examples/16_predictive_check.probl) | model checking | replicate a dataset from a fitted rate and compare variation between days |
| [`17_sensor_tracking.probl`](../examples/17_sensor_tracking.probl) | monitoring | noisy observations of a hidden state, incremental exact filtering and next-state prediction |
| [`18_correlated_losses.probl`](../examples/18_correlated_losses.probl) | risk | common hazards, equal marginal risks, different joint tails and reserve shortfalls |
| [`19_analytic_continuous.probl`](../examples/19_analytic_continuous.probl) | arrival times | analytic continuous reports, shared draws and threshold evidence |

Every example ends with the output it should produce, and those outputs are golden tests. The enumerated ones were checked against independent reference calculations, and must be printed exactly. Tests pin `today` to `2026-09-29`; use `--today 2026-09-29` to reproduce date-dependent output. Sampled output is compared within five standard errors for estimates, and 4% for other numbers. Examples 07–09 use independent reference simulations as their baselines; examples 15–16 record seeded engine output, with their headline results also checked independently. See the [use cases](use-cases.md) for the new examples' validation and the limitations they expose.

---

## Appendix A: Grammar

EBNF. `NEWLINE` ends a statement unless the line clearly continues: inside `( )` or `[ ]`, or after a binary operator, a comma or `=>`. Blank lines, and newlines right after `{` or right before `}`, are ignored. A `;` separates statements on the same line.

```ebnf
program      = { pragma } { item } ;
pragma       = "@" IDENT [ expr ] NEWLINE ;      (* @mode enumerate · @mode sample(runs: 1000) · @epsilon 1e-9 *)
item         = fn_decl | type_decl | enum_decl | import | stmt ;

fn_decl      = "fn" IDENT "(" [ param { "," param } ] ")" [ "->" type ] block ;
param        = IDENT [ ":" type ] ;
type_decl    = "type" IDENT "=" type ;
enum_decl    = "enum" IDENT "{" IDENT { "," IDENT } [ "," ] "}" ;
import       = "import" STRING ;
type         = IDENT [ "[" type { "," type } "]" ]            (* int, list[int], dist[int] *)
             | "{" IDENT ":" type { "," IDENT ":" type } [ "," ] "}" ;

block        = "{" { stmt } "}" ;
stmt         = binding | assign | loop | jump | observe | score | report | expr ;
binding      = ( "let" | "var" ) pattern [ ":" type ] ( "=" | "~" ) expr ;
assign       = place ( "=" | "~" | "+=" | "-=" | "*=" | "/=" ) expr ;
place        = IDENT { "." IDENT | "[" expr "]" } ;
loop         = "for" pattern "in" expr block | "while" expr block
             | "repeat" expr block | "loop" block ;
jump         = "break" | "continue" | "return" [ expr ] ;
observe      = "observe" expr [ "from" expr ] ;
score        = "score" expr ;
report       = "report" expr [ "by" expr ] [ "as" STRING ] ;

expr         = lambda | or_expr ;
lambda       = ( IDENT | "(" [ IDENT { "," IDENT } ] ")" ) "->" expr ;
or_expr      = and_expr { "or" and_expr } ;
and_expr     = not_expr { "and" not_expr } ;
not_expr     = "not" not_expr | cmp_expr ;
cmp_expr     = range_expr [ cmp_op range_expr ] ;
cmp_op       = "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" | "not" "in" ;
range_expr   = add_expr [ ( ".." | "..<" | "to" ) add_expr ] ;
add_expr     = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr     = unary { ( "*" | "/" | "div" | "mod" ) unary } ;
unary        = "-" unary | power ;
power        = prefix [ "^" unary ] ;
prefix       = ( "typeof" | "~" ) prefix | postfix ;
postfix      = primary { "." IDENT [ call_args ] | call_args | "[" expr "]" | "with" record } ;
call_args    = "(" [ arg { "," arg } ] ")" ;
arg          = [ IDENT ":" ] expr ;

primary      = INT | FLOAT | PERCENT | DICE | STRING | "true" | "false"
             | IDENT [ record ]                              (* a variable, or Fighter { … } *)
             | "(" expr ")" | list | map | record | block
             | if_expr | chance_expr | match_expr | "simulate" block ;
if_expr      = "if" expr block [ "else" ( if_expr | block ) ] ;
chance_expr  = "chance" "{" arm { sep arm } [ sep ] "}" ;
arm          = ( expr | "else" ) "=>" ( block | stmt ) ;
match_expr   = "match" expr "{" match_arm { sep match_arm } [ sep ] "}" ;
match_arm    = pattern [ "if" expr ] "=>" ( block | stmt ) ;
list         = "[" [ expr { "," expr } [ "," ] ] "]" ;
map          = "[" ":" "]" | "[" expr ":" expr { "," expr ":" expr } [ "," ] "]" ;
record       = "{" field { "," field } [ "," ] "}" ;
field        = IDENT [ ":" expr ] ;                          (* { rounds } means { rounds: rounds } *)
pattern      = alt { "|" alt } ;
alt          = "_" | IDENT | literal | "[" [ pattern { "," pattern } ] "]" ;
literal      = INT | FLOAT | PERCENT | STRING | "true" | "false" ;
sep          = "," | NEWLINE ;
```

Lexical notes:

- `PERCENT` is a number immediately followed by `%` (`30%`, `12.5%`). Because `%` only ever means *percent*, remainder is spelled `mod`.
- `DICE` is `[count] "d" sides`, written with no spaces (`d6`, `2d6`, `10d10`). Names of that shape are reserved: `d2` can't be a variable.
- In the header of an `if`, `while`, `for` or `match`, a record literal needs parentheses, so `if x == (Point { x: 1, y: 2 }) { … }`, as in Rust.
- `{` starts a record when it's followed by `IDENT :` or `IDENT ,`. Otherwise it starts a block.
- In a pattern, a name that refers to an enum variant (`Boom`) matches that variant. Any other name binds a new variable.

## Appendix B: Operator precedence

From loosest to tightest binding:

| Level | Operators | Associativity |
|---|---|---|
| 1 | `x -> e` (lambda) | right |
| 2 | `or` | left |
| 3 | `and` | left |
| 4 | `not` | prefix |
| 5 | `==` `!=` `<` `<=` `>` `>=` `in` `not in` | none |
| 6 | `..` `..<` `to` | none |
| 7 | `+` `-` | left |
| 8 | `*` `/` `div` `mod` | left |
| 9 | `-` (negation) | prefix |
| 10 | `^` | right |
| 11 | `typeof` `~` (draw) | prefix |
| 12 | call `f(x)`, index `a[i]`, field and method `a.b`, `with { … }` | left |

`/` always divides as floats and `div` is integer division, so `7 / 2` is 3.5 and `7 div 2` is 3.

## Appendix C: Keywords

`and` `break` `chance` `continue` `div` `else` `enum` `false` `fn` `for` `if` `import` `in` `let` `loop` `match` `mod` `not` `observe` `or` `repeat` `report` `return` `score` `simulate` `true` `type` `typeof` `var` `while` `with`

`as`, `by`, `from` and `to` are keywords only where the grammar uses them, so they remain usable as variable names.

## Appendix D: Standard library (initial)

| Area | Functions |
|---|---|
| Distributions | `bernoulli` `one_of` `binomial` `poisson` `geometric` `normal` `lognormal` `normal_range` `uniform` `beta` `gamma` `exponential` `triangular` `pert` `mixture`\* `roll` `bag` |
| Distribution helpers | `take` `truncate(d, lo, hi)`\* `bins(d, n)`\* |
| Queries | `P` `mean` `sd` `variance` `median` `median_low` `median_high` `quantile` `support` `cdf` `pmf` `pdf` |
| Probability | `odds(p)` `logit(p)` `inv_logit(x)` |
| Math | `abs` `min` `max` `clamp` `floor` `ceil` `trunc` `round(x, digits?)` `sqrt` `cbrt` `hypot(x, y)` `exp` `exp2` `expm1` `ln` `log1p` `log2` `log10` `sin` `cos` `tan` `asin` `acos` `atan` `atan2(y, x)` `sinh` `cosh` `tanh` `asinh` `acosh` `atanh` |
| Integer and special functions | `choose(n, k)` `factorial(n)` `gcd(a, b)` `lcm(a, b)` `euler_phi(n)` `ln_gamma(x)` `erf(x)` `erfc(x)` |
| Complex numbers | `complex(re, im?)` `real(z)` `imag(z)` `conj(z)` `abs(z)` `abs2(z)` `arg(z)` `cis(theta)` |
| Constants | `pi` `e` `euler_gamma` `today`; a variable, variant or function of the program's with the same name hides one, so `let e = 5` still works |
| Collections | `len` `slice(xs, start, end?)` `push` `pop` `insert` `remove` `get(key, default)` `keys` `values` `map` `filter` `reduce` `sum` `count` `minimum` `maximum` `highest(n, compare?)` `lowest(n, compare?)` `sort` `sort_desc` `reverse` `enumerate` `zip` |
| Text | `str` `upper` `lower` `trim(s, chars?)` `trim_start(s, chars?)` `trim_end(s, chars?)` `starts_with` `ends_with` `chars` `split` `join` |
| Dates | `date("2027-01-31")` or `date(year, month, day)`; `days(n)` `weeks(n)` `add_workdays(d, n, holidays?)` `is_workday(d, holidays?)` `weekday(d)` `add_months(d, n)` `add_years(d, n)` `start_of_month(d)` `end_of_month(d)` `year(d)` `month(d)` `day(d)` |
| Debugging | `print` |

\* Planned, not built yet.
