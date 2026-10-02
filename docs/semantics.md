# Probl reference semantics

> Version 0.2, September 2026. This document is normative for the engine. Where it disagrees with the [language overview](language-overview.md), this document wins. It resolves findings D1–D3, I1 and I2 of the [project audit](project-audit.md). Continuous distributions (D6) and sampling (D4), with exact updates for conjugate priors, are in sections 13 and 14, and data read from files in section 15; loops and recursion that cycle are solved as in sections 6 and 10 (D5); what is still open, such as particles, is listed in section 16.

## 1. Values and types

| Type | Values | Notes |
|---|---|---|
| `bool` | `true`, `false` | **Facts.** Comparisons of settled values, `and`/`or`/`not`, `in`, pattern tests |
| `prob` | a number from 0 to 1 | **Probabilities.** Percentage literals from `0%` to `100%`. A probability is a parameter, not an event |
| `int` | arbitrary-precision whole numbers | Exact arithmetic, bounded by host resource limits |
| `float` | IEEE 754 binary64 numbers | Arithmetic on probabilities gives floats |
| `complex` | `complex(re, im)` | Two finite floating-point components; never an implicit probability |
| `str`, `date`, `list[T]`, `map[K, V]`, `bag[T]`, records, enums, functions | | |
| `dist[T]` | a distribution over `T` | Dice, `one_of`, `bernoulli`, counts, `roll`, `simulate`, continuous distributions (section 13), and operations on distributions |

All values are immutable: assigning or passing a collection gives an independent copy.

Wherever a probability is expected, a `float` from 0 to 1 is accepted too. Nothing else converts implicitly: in particular, a `prob` never turns into an event, and an `int` is never a condition.

**Percentage representation.** The lexer divides percentage literals by 100. A nonnegative literal from `0%` through `100%` becomes a runtime `prob`; a larger percentage becomes a `float`. Negation and ordinary arithmetic on probabilities produce floats: `typeof 33%` is `"prob"`, while `typeof 0.33`, `typeof (-33%)`, `typeof 150%` and `typeof (33% + 1%)` are `"float"`. The numeric comparison `33% == 0.33` is true. An annotation such as `let p: prob = 0.33` checks the range without changing the float's representation. This is the current behavior, not a new probability/event identity rule. A visible consequence of the tag is that `one_of` requires weights all tagged `prob` to sum to one, whereas ordinary numeric weights are relative.

**Runtime type inspection.** `typeof e` evaluates `e` once and returns a `str` describing the resulting value's runtime type. It is a reserved prefix operator, not a function; `typeof(e)` works by grouping its operand. Calls, indexing and field access bind more tightly; every binary operator binds less tightly. Thus `typeof x == "int"` compares the type string, and `typeof (x + 1)` inspects the sum. An operand's errors and effects still occur: `typeof (1 / 0)` fails, and `typeof f()` calls `f`.

The operator inspects distribution objects themselves, without drawing, normalizing or lifting over their outcomes. `typeof d6` is `"dist[int]"`, `typeof (d6 > 3)` is `"dist[bool]"`, and `typeof normal(0, 1)` is `"dist[float]"` even in enumeration mode. A direct inspection of a delayed conjugate variable, `typeof p`, returns `"float"` without forcing its draw or preventing later exact updates. Computing an operand such as `typeof (p + 0)` still needs `p`'s value; a function call that reads `p` has its ordinary effects too.

| Value | Type description |
|---|---|
| Primitive values | `bool`, `int`, `float`, `prob`, `complex`, `str`, `date` |
| An integer range, a function, the unit value | `range`, `fn`, `()` |
| Lists, maps and bags | `list[T]`, `map[K, V]`, `bag[T]`, recursively describing their current contents |
| Finite distributions | `dist[T]`, describing retained outcomes; unresolved mass provides no additional type information |
| Continuous distributions | `dist[float]`; a continuous component of a finite mixture contributes `float` to the mixture's outcome type |
| Named records and enum variants | Their declared type name, such as `Fighter` or `Market` |
| Anonymous records | Field descriptions in name order, such as `{a: int, b: dist[int]}` |

An empty collection has `unknown` in each unobserved type position: `typeof []` is `"list[unknown]"`. If observed element descriptions differ, that position becomes `any`, without numeric promotion: `typeof [1, 2.0]` is `"list[any]"`; `typeof [[], [1]]` is also `"list[any]"`. These markers are descriptive strings, not new annotation types. `list[dist[int]]` and `dist[list[int]]` remain distinct. Inspection uses runtime contents, not annotations or a static type inference pass, so type strings may differ between worlds or after a collection changes. Traversal respects work and string budgets, and is limited to 64 levels of value nesting.

**Dates and the execution snapshot.** A `date` is an immutable day in the proleptic Gregorian calendar, from **0001-01-01 through 9999-12-31**, stored as an integer day offset from 1970-01-01. It contains no time, timezone or locale. `date(s)` requires exactly ten ASCII characters in `YYYY-MM-DD` form; `date(year, month, day)` requires three ints. Both reject invalid dates instead of normalizing them. File readers use the same date validation, after their usual trimming of non-string fields. Dates compare chronologically and have exact identity as map keys and distribution outcomes.

`today` is a shadowable constant of type `date`, supplied by the host once per execution. All worlds, sampling batches, threads, function calls and nested `simulate` evaluations see that same value. The engine and compiler never read the clock. The CLI and JavaScript host capture the UTC date before execution; `probl run --today YYYY-MM-DD` or the JavaScript request's `today` field replaces that snapshot for replay or a scenario. The REPL retains one snapshot for its entire session, including input reloads. `Options.today` supplies the engine's integer day offset; a host that omits it gets a language error if the program evaluates `today`. The returned `Outcome.today` and WASM response's ISO `today` field record the snapshot, and CLI `--stats` prints it. Reproducibility includes this input as well as source, data, seed and engine version. `today()` and assignment to an unshadowed `today` are compile errors.

Date transformations return values without changing their arguments and lift over finite distributions. Results outside the supported date range are errors.

| Operation | Contract |
|---|---|
| `d + n`, `d - n` | Shift by `n` calendar days, where `n` is an int. `n + d` also works. |
| `a - b` for two dates | An exact signed integer day count. |
| `days(n)`, `weeks(n)` | Round `n` or `7*n` to an int, halves away from zero; these are day counts, not a separate duration type. |
| `add_months(d, n)`, `add_years(d, n)` | Shift by an integer number of calendar months or years, clamping the day to the target month's last valid day. Negative and zero counts are allowed. |
| `start_of_month(d)`, `end_of_month(d)` | First or last date of the same calendar month. |
| `year(d)`, `month(d)`, `day(d)` | Integer components; month and day are one-based. |
| `weekday(d)` | An English name from `"Monday"` through `"Sunday"`. |
| `is_workday(d, holidays?)` | Whether `d` is Monday–Friday and absent from the optional holiday list. |
| `add_workdays(d, n, holidays?)` | Move by `n` open weekdays, excluding the starting date. Negative counts go backwards; zero returns the starting date even if closed. |

The holiday argument must be a list of dates. Order and duplicate dates do not matter, weekend entries do not remove extra weekdays, and no national or regional holidays are implicit. Workday counts must be ints. Weekday-only shifts take constant time; explicit holiday calendars require work proportional to sorting and scanning their entries, charged to the work and collection budgets.

Clamping makes month/year arithmetic non-invertible and non-associative: January 31 plus one month plus one month can give March 28, whereas January 31 plus two months gives March 31. Recurrences should derive each occurrence from the original anchor. There is no generic fixed-length `months(n)` or `years(n)` duration. This follows the default constrained arithmetic of [Temporal.PlainDate](https://tc39.es/proposal-temporal/docs/plaindate.html#add); the supported year range matches [Python's date](https://docs.python.org/3/library/datetime.html#date-objects).

**Text and sequences.** Strings are immutable, valid Unicode text stored as UTF-8. Source files and text data use UTF-8; malformed input is an error. There is no separate character type. String length, indexing, iteration, slicing, `chars`, `split(s, "")` and `reverse` all use **Unicode scalar values**, with each element represented by a one-scalar string. This is neither byte indexing nor grapheme-cluster indexing: `len("🙂")` is 1, `len("🇫🇷")` is 2, and `len("é")` is 2 (an `e` followed by a combining acute accent). Slicing and reversal can separate combining marks or emoji components while always preserving valid UTF-8. Grapheme segmentation is not currently exposed; see [Unicode text segmentation](https://www.unicode.org/reports/tr29/) for the distinction.

Equality, hashing, substring search and prefix/suffix tests compare exact text, without normalization or case folding. Precomposed `"é"` and decomposed `"é"` are distinct map keys and distribution outcomes. Ordering is lexicographic Unicode scalar order, not locale collation. `upper` and `lower` use the Rust standard library's Unicode mappings, independent of locale, and can change length: `upper("ß")` is `"SS"`, and `lower("İ")` is an `i` followed by a combining dot. Native and WASM builds use the same Rust Unicode tables; updating the toolchain can update those tables. No transformation mutates its input.

Strings, lists and ranges are ordered sequences. `slice(xs, start, end)` selects positions from `start` inclusive to `end` exclusive; `slice(xs, start)` defaults `end` to `len(xs)`. Bounds must be actual `int` values with `0 <= start <= end <= len(xs)`. Invalid types or bounds are errors: there are no negative offsets, clamping or step arguments. Equal bounds produce an empty result, including at the sequence's length. A string slice is a string, a list slice is a list, and a range slice stays a compact range without materializing its elements. Empty range slices are represented as `0..-1`. Maps and bags do not support slicing. String indexing and slicing scan UTF-8 in linear time with constant auxiliary space; they do not allocate a character array. Existing scalar `[]` indexing continues to accept whole-valued floats for compatibility; the new slice bounds require ints.

| Text function | Contract |
|---|---|
| `chars(s)` | A list containing one string per Unicode scalar; `chars("")` is `[]`. |
| `trim(s, chars?)` | Remove matching scalars repeatedly from both ends. Without `chars`, match Unicode `White_Space`, including newlines. With `chars`, match any scalar in that string, with no implicit whitespace. |
| `trim_start(s, chars?)`, `trim_end(s, chars?)` | The same trimming at only the start or end. These names refer to sequence order, independent of display direction. |
| `starts_with(s, prefix)`, `ends_with(s, suffix)` | Exact literal matching; the empty prefix or suffix always matches. |
| `split(s, sep)` | Split on a literal separator, preserving empty fields. An empty separator gives `chars(s)`. Thus `split("", ",")` is `[""]`, while `split("", "")` is `[]`. |
| `join(xs, sep)` | Format each list item as text and insert the separator between items. An empty list gives `""`. |

Trim's optional argument is a **set of scalars**, not a substring: `"abbacacb".trim("ab")` is `"cac"`. Repeats and order in the set do not matter, and an empty set leaves the string unchanged. The interior is preserved. All text arguments must be strings, with no implicit conversion. [Rust's trim](https://doc.rust-lang.org/std/primitive.str.html#method.trim) uses the same default whitespace property.

`map`, `filter`, `reduce`, `count(xs, test)`, `sort`, `sort_desc`, `enumerate` and `zip` accept strings as sequences of one-scalar strings, as well as lists and ranges. `map` and `filter` always return lists; use `join(result, "")` to rebuild text. Sorting also returns a list. `reverse` returns a string for a string and a list for a list or range. `zip` stops at the shorter sequence. These operations materialize their input elements and respect collection limits. Their existing callback restrictions remain in force. `slice` and text functions lift over finite distributions, as do these sequence operations; callbacks in `map` and `filter` receive individual scalar strings.

**Integers.** There is one `int` type, with decimal, binary (`0b111`) and hexadecimal (`0xfab101`) literals and automatic promotion beyond machine integers; no suffix or separate `bigint` type. Integer `+`, `-`, `*`, `div`, `mod`, and powers with nonnegative integer exponents are exact. `div` rounds toward negative infinity; a nonzero remainder has the divisor's sign. Zero to zero is 1; zero to a negative power is an error. `/` and negative powers return approximate floats. Integer division with `/` converts the ratio jointly, so `(10^400) / (10^400)` is 1.0. Mixed float or complex arithmetic and real math functions convert integers to finite binary64 values, with rounding; an integer too large for a finite conversion is an error. Floats, complex components and probability weights do not gain arbitrary precision.

Binary and hexadecimal prefixes accept either case (`0b`/`0B`, `0x`/`0X`), and hex digits accept `a`–`f` or `A`–`F`. At least one digit is required. Single underscores may separate digits, as in `0b1010_0110` or `0xFA_B101`; they cannot immediately follow the prefix or end the literal. Invalid digits and suffixes are errors. A leading minus is the ordinary unary operator: `-0xff` is −255. Leading zeros carry no width or sign information. The literal's base affects neither its type nor its default decimal display. Prefixes apply only to integer source literals: floats, percentage and dice literals retain their decimal syntax, and file readers still require decimal integers. For example, `0x2d6` is the integer 726, while `2d6` is a dice distribution.

Integer comparisons are exact, including against the exact value represented by a float: `9007199254740993 > 9007199254740992.0` is true. Promoting or shrinking an integer does not change its identity in maps, bags, distributions, caches or merged worlds. Integer reports, strings and interpolation retain every digit; automatic numeric summaries fall back to categorical output if converting integer outcomes to floats would lose information or produce nonfinite statistics.

The parser and runtime impose a hard ceiling of **65,536 magnitude bits** per integer (at most 19,729 decimal digits); hosts may lower it through `Limits.max_integer_bits` and `InputLimits.max_integer_bits`. Arithmetic work is charged in proportion to operand size, with a conservative quadratic charge for multiplication and division. A separate cumulative allowance for large integer results is 256 MiB by default and 64 MiB in the playground, configurable through `max_integer_bytes`. It conservatively counts payloads and metadata, can count shared results again, and is not a measurement of live memory. Input loading has its own allowance. Oversized operations fail with a resource-limit diagnostic rather than converting to floats. Collection lengths, dice counts, bag counts and distribution parameters retain their checked implementation bounds; a huge `int` does not authorize a huge allocation. Ranges may have large endpoints and can be indexed or measured without materializing them. Factorization such as `euler_phi` can still exhaust the work budget on large primes.

**Math.** `pi`, `e` and `euler_gamma` are ordinary `float` constants (π, Euler's number, and the Euler–Mascheroni constant γ). They are names, not reserved words: a program's variables, parameters, variants and functions can hide them. Assigning to an unshadowed constant is an error.

`exp(1)` returns exactly the same float as the built-in `e`, the nearest representable value to Euler's number. This exact-input convention is shared by real and complex exponentials and the engine's probability calculations. Other floating-point identities still require tolerances; results are not automatically snapped to nearby constants or zero. Scalar float reports round their displayed significant digits, so identical printed values do not generally imply identical stored floats.

For real inputs, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2(y, x)`, `hypot(x, y)`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, `log2`, `log1p` and `expm1` return floats. Complex extensions are described below. They act on each outcome of a finite distribution, like the existing math functions (section 2); draw from a continuous distribution first. Angles are in radians, including inverse-function results. `asin` and `acos` accept −1 through 1; `asinh` accepts any finite number, `acosh` requires at least 1, and `atanh` requires −1 < x < 1 (both endpoints are errors). `log2` requires a positive argument, and `log1p` requires an argument above −1. Arguments and results must be finite; invalid domains and overflow are errors. `atan2` takes the vertical coordinate first and returns an angle from −π to π; signed zeros are treated alike, and at `(0, 0)` it returns 0 by convention. These are floating-point approximations: `sin(pi)` need not be exactly zero, and `tan(pi / 2)` evaluates at the rounded argument rather than an exact pole. `log1p(x)` and `expm1(x)` preserve accuracy near zero; `hypot` avoids overflow or underflow in intermediate squares.

**Rounding.** `round(x)` returns the nearest `int`, with halves away from zero, as before. `round(x, digits)` requires an `int` for `digits`: positive values select decimal places, zero selects whole numbers, and negative values select tens, hundreds, and so on. With explicit `digits`, an `int` input returns an exact `int` (subject to integer resource limits); a `float` or `prob` input returns a `float`, including when `digits` is zero. For example, `round(1.125, 2)` is 1.13, `round(-1250, -2)` is −1300, and `round(1.5, 0)` is the float 2.0. Float rounding scales by powers of ten and rounds halves away from zero; binary representation and scaling can affect results near halfway cases. Arguments and float results must be finite. Arbitrarily large positive `digits` leave a finite value unchanged, and sufficiently negative `digits` give zero, without unbounded work. Both arguments lift over finite distributions. Rounding changes the value, not the number of digits printed; it is not a fixed-decimal type or a formatting function.

`trunc(x)` drops the fractional part toward zero and returns an `int`: `trunc(-1.9)` is −1. Like `floor`, `ceil` and one-argument `round`, it preserves integer inputs exactly, accepts finite numeric inputs, and returns the exact whole number represented by the rounded float, subject to integer resource limits. It lifts over finite distributions.

For real inputs, `cbrt(x)` returns the real cube root as a float, including for negative inputs; `cbrt(-8)` is −2. `exp2(x)` returns 2ˣ as a float and is the inverse of `log2`. Both accept finite numbers and lift over finite distributions. Overflow is an error; very small floating-point results may underflow to zero.

**Integer and special functions.** The following functions also act on each outcome of a finite distribution. The integer functions require `int` arguments (not floats, even whole-valued ones), and return exact `int` results. Results grow beyond machine integers; exceeding the integer size or work limit is an error, never a rounded float.

| Function | Domain and result |
|---|---|
| `bit_length(n)` | Integer, signs ignored. The number of binary digits in its magnitude, excluding leading zeros and any sign bit; `bit_length(0) = 0`, `bit_length(-7) = 3`. This is not a machine word size or a two's-complement width. |
| `bit_and(a, b)` | Two integers. Bitwise AND: `bit_and(0b1010, 0b1100) = 8`. |
| `bit_or(a, b)` | Two integers. Bitwise OR: `bit_or(0b1010, 0b1100) = 14`. |
| `bit_xor(a, b)` | Two integers. Bitwise exclusive OR: `bit_xor(0b1010, 0b1100) = 6`. |
| `bit_not(n)` | Integer. Bitwise complement, equal to `-n - 1`: `bit_not(0) = -1`. |
| `bit_count(n)` | Integer, signs ignored. The number of set bits in its magnitude: `bit_count(0) = 0`, `bit_count(-0b1011) = 3`. |
| `ilog2(n)` | Positive integer. The exact floor of its base-2 logarithm; `ilog2(1) = 0`, `ilog2(31) = 4`, `ilog2(32) = 5`. Zero and negatives are errors. |
| `choose(n, k)` | Nonnegative integers. The binomial coefficient, counting selections without order or replacement. `k > n` gives 0; `choose(n, 0)` is 1. |
| `factorial(n)` | Nonnegative integer. The product 1 × … × n, with `factorial(0) = 1`. `factorial(30)` is exactly 265252859812191058636308480000000; size and work limits apply. |
| `gcd(a, b)` | Two integers, signs ignored. The nonnegative greatest common divisor; `gcd(0, 0) = 0`. |
| `lcm(a, b)` | Two integers, signs ignored. The nonnegative least common multiple; either argument being zero gives 0. |
| `euler_phi(n)` | Positive integer. The number of integers from 1 through n coprime to n, with `euler_phi(1) = 1`. Factorization consumes the host's work budget and may reach its limit for large inputs. |
| `ln_gamma(x)` | Positive finite number. Returns a float approximating ln Γ(x), with `ln_gamma(n + 1) = ln(n!)` for nonnegative integers n. Nonpositive arguments and non-finite results are errors. This does not change the `gamma(shape, scale)` distribution constructor. |
| `erf(x)` | Finite number. Returns the error function as a float: (2 / √π) ∫₀ˣ exp(−t²) dt, from −1 to 1. |
| `erfc(x)` | Finite number. Returns the complementary error function as a float, from 0 to 2. Computes 1 − erf(x) directly to preserve small tails; extremely small results may underflow to zero. For the standard normal, the probability above x is `erfc(x / sqrt(2)) / 2`. |

`bit_length` and `ilog2` inspect the stored integer magnitude in constant time, including for bigints, and each charges one unit of work for that inspection. For positive integers, `ilog2(n) = bit_length(n) - 1`. Both require an actual `int` and lift over finite distributions. They never convert through a float: `ilog2(2^100 - 1)` is 99 even though `floor(log2(2^100 - 1))` rounds to 100. The approximate `log2` function retains its float and complex behavior.

`bit_and`, `bit_or`, `bit_xor` and `bit_not` operate as if signed integers had infinitely many two's-complement sign bits. There is no implicit word size: `bit_and(-1, 0xff)` is 255, while `bit_not(0x000f)` is −16. For a fixed-width complement, mask the result: `bit_and(bit_not(0b1010), 0xff)` is 245. `bit_count` instead counts the ones in the absolute value, consistent with `bit_length`; it does not count sign-extension bits. All five functions work directly on bigints and charge work proportional to operand bit length. Integer size and memory limits also apply: a signed bit operation can increase the magnitude by one bit, and exceeding the limit is an error. The `^` operator remains exponentiation.

The selection, factorial, sign and zero conventions follow the corresponding [Python integer math functions](https://docs.python.org/3/library/math.html#number-theoretic-functions), with Probl's integer resource limits and two-argument `gcd`/`lcm`. Mathematical definitions: [NIST's gamma function](https://dlmf.nist.gov/5.2) and [Euler's totient](https://dlmf.nist.gov/27.2).

**Complex values.** `complex(re, im)` takes finite real numeric components; the imaginary component defaults to zero. `complex(z)` also accepts a complex value. Both components use `f64`, so converting very large integers can round them. A `complex` annotation requires a complex value; use the constructor to convert explicitly. No imaginary literal syntax or built-in `i` name is introduced.

Complex values support `+`, `-`, unary `-`, `*`, `/`, and integer powers `z ^ n`. Mixed real/complex arithmetic promotes the real operand; results stay complex, even if their imaginary component is zero. Negative integer powers take the reciprocal; zero to zero is one. Division by zero, non-finite components and overflowing results are errors; underflow is possible. The `^` operator still requires an integer exponent for complex operands; for a nonzero complex `z`, a principal general power can be written `exp(w * ln(z))`. `div` and `mod` are not defined for complex operands.

`real(z)` and `imag(z)` return float components, `conj(z)` returns the complex conjugate, `abs(z)` returns the magnitude, and `abs2(z)` returns its square as a float. These functions accept real inputs too. `arg(z)` returns the phase in radians from −pi to pi; zero has phase zero by convention. `cis(theta)` takes a finite real angle and returns `complex(cos(theta), sin(theta))`. Signed zeros are canonicalized, so a negative real complex value has phase +pi.

**Complex elementary functions.** `sqrt`, `cbrt`, `exp`, `exp2`, `expm1`, `ln`, `log2`, `log10`, `log1p`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh` and `atanh` accept complex inputs and return complex values. Real inputs retain their existing real domains and float results: `sqrt(-1)` is an error, while `sqrt(complex(-1))` is `complex(0, 1)`. There is no automatic promotion after a domain error.

Multivalued mathematical functions return **one principal value**, never a distribution. `ln(z)` is `complex(ln(abs(z)), arg(z))` mathematically (the implementation avoids overflowing the intermediate magnitude). Its other branches are `ln(z) + complex(0, 2*pi*k)` for integer `k`. They are mathematical choices, without inherent probability weights. `exp(ln(z))` recovers nonzero `z` up to rounding; `ln(exp(z))` wraps the imaginary part into the principal phase range and need not recover `z`. Paths that wind around zero need caller-managed phase unwrapping; a scalar logarithm does not remember a path.

| Functions | Principal complex convention and cuts |
| --- | --- |
| `ln`, `log2`, `log10` | Cut on the negative real axis; zero is an error. Base-2/10 results are `ln(z)/ln(base)`. |
| `log1p` | Principal `ln(1+z)`, preserving tiny inputs; cut below −1 on the real axis, error at −1. |
| `sqrt` | Nonnegative real part, with positive imaginary part on the negative real axis. Zero maps to zero. |
| `cbrt` | Phase `arg(z)/3`, with the same cut as `ln`; zero maps to zero. Thus `cbrt(complex(-8))` is approximately `complex(1, sqrt(3))`, while real `cbrt(-8)` is −2. |
| `asin`, `acos` | Real-axis cuts outside [−1, 1]; real parts in [−pi/2, pi/2] and [0, pi], respectively. |
| `atan` | Imaginary-axis cuts beyond ±i; ±i are errors. |
| `asinh` | Imaginary-axis cuts beyond ±i. |
| `acosh` | Real-axis cut below 1; nonnegative real part. |
| `atanh` | Real-axis cuts outside [−1, 1]; ±1 are errors. |

On a cut, canonical positive-zero input components select the upper side of a real-axis cut or the right side of an imaginary-axis cut, matching [Python `cmath`](https://docs.python.org/3/library/cmath.html) for positive-zero inputs. Unlike `cmath`, Probl does not use negative zero to select the other side: equality, world merging and memoization treat signed zeros as identical. Nonzero components can approach either side. For example, `ln(complex(-1))` has imaginary part +pi, while `ln(complex(-1, -1e-20))` has imaginary part approximately −pi. Identities involving conjugation may differ exactly on a cut because conjugation canonicalizes zero too.

All these functions require finite inputs and finite results; overflow and singularities are errors. Small results may underflow. `atan2`, `hypot`, rounding, integer functions, `ln_gamma`, `erf` and `erfc` remain real-only.

`==` and `!=` compare complex components numerically, including equality with a real number when the imaginary component is zero. Integer comparisons do not round the integer operand into a float. Structural equality for world merging, keys and memoization remains exact and typed, as for existing numeric types; approximate comparisons are explicit, such as `abs(a - b) < 1e-12`. Complex values have no ordering: comparisons, ordering-dependent queries and sorting cannot order them. The deterministic internal ordering used to store outcomes is not a mathematical order.

Complex arithmetic and helper functions lift over finite distributions. Such distributions remain classical mixtures: outcomes with opposite phases do not cancel. `sum` supports complex elements; `mean` returns a complex weighted arithmetic mean when any outcome is complex. `variance`, `sd`, real-only math functions and distribution parameters do not accept complex arguments. Reports present complex values as categorical outcomes instead of applying real summary statistics. Complex values cannot be conditions or probability weights, even with zero imaginary component. `abs2` is an ordinary float, potentially above 1; it is not automatically normalized or interpreted as a probability.

The data-file schema does not add a complex encoding; read real and imaginary fields and construct values explicitly. The [complex and quantum design note](quantum-and-complex.md) distinguishes these implemented scalar rules from a possible future quantum engine.

## 2. Events and identity

A **distribution is a recipe**. Each occurrence of a distribution in an expression is an independent draw: `d6 + d6` is two dice, and `let die = d6` followed by `die + die` is also two dice.

A **drawn value is a fact**. `let x ~ d6` gives `x` one value in each world, and every use of `x` refers to that value.

Operators and built-in functions applied to distributions act on every outcome and return a distribution. Comparing a distribution gives a `dist[bool]`: `d6 > 4` is the distribution of the fact "this roll is above 4", which is true a third of the time. A distribution stays a distribution even when only one outcome is possible (so `d1` is `dist[int]`, not `int`).

A distribution never has distributions as outcomes: where one would, they are mixed in. `one_of([d2, 10])` is 1 or 2 a quarter of the time each and 10 half the time, so a value drawn from it is settled. The same holds for the results of operators and of `simulate` (section 8).

**Logical operators work on facts.** `and`, `or` and `not` accept `bool` operands. They also accept a `dist[bool]`, but `and` and `or` accept at most one uncertain operand: with two, Probl can't know whether they describe the same event (`e and e`) or two independent ones, so it reports an error. Probabilities are never accepted: `30% and 30%` is an error, because a probability has no identity.

To combine uncertain events, give them identities by drawing them:

```probl
let rain ~ bernoulli(30%)     # a fact: true in 30% of worlds
let late ~ bernoulli(20%)
report rain and late          # 6%
report rain and rain          # 30%: the same event
```

**Drawing needs a distribution.** `x ~ D` splits each world by the outcomes of `D`. Drawing from a probability is an error (write `bernoulli(p)`). Drawing from any other plain value leaves it unchanged, so that `x ~ if c { d6 } else { 0 }` works.

**`match` needs a settled subject.** Matching a distribution is an error; draw it first. Patterns are tests on a single value, and `|` between patterns is an ordinary `or` on facts.

## 3. Conditions and branching

`if c`, `while c`, the weights of `chance`, and `observe c` accept:

| Condition | Meaning |
|---|---|
| `bool` | no split: the world goes one way |
| `prob` (or a float from 0 to 1) | a fresh, independent trial with that probability |
| `dist[bool]` | a trial with the probability that it's true |

`if c { A } else { B }` sends each world into `A` with its weight multiplied by p and into `B` multiplied by 1 − p, where p is the condition's probability in that world. Branches with weight zero are skipped.

`chance { p₁ => A₁ … pₙ => Aₙ else => B }` evaluates every weight first, then splits. Each weight is a condition from the table above, and counts with the probability that it holds. The weights must not add up to more than 1 (a tolerance of 10⁻⁹ allows for rounding). The remainder goes to `else`. Without `else`, the remainder continues after the statement; when the `chance` is used as a value, a remainder above 10⁻⁹ is an error.

## 4. Evaluation order

Evaluation is **left to right**, and every operand is evaluated exactly once:

- operands of binary operators, arguments of calls, elements of lists, maps and records, and the parts of string interpolation are evaluated in the order they're written;
- `place = value` evaluates `value`, then the indices in `place`, then assigns;
- `place += value` (and `-=`, `*=`, `/=`) reads the current value of `place` first, then evaluates `value`;
- `and` doesn't evaluate its right side when the left side is certainly false, and `or` when it's certainly true.

An assignment made while evaluating an operand (for example inside a block expression) is visible to the operands evaluated after it, and not to those evaluated before it:

```probl
var x = 1
report x + { x = 2; 0 }       # 1
report [x, { x = 3; x }]      # [2, 3]
```

## 5. Worlds, merging and liveness

A running program is a finite set of worlds, each a program state σ with a weight w. A statement maps each world to a set of worlds:

| Statement | From (σ, w) |
|---|---|
| `x = e` | (σ[x ↦ e], w) |
| `x ~ D` | (σ[x ↦ v], w · P(D = v)) for every outcome v of D |
| `if c { A } else { B }` | A on (σ, w · p), B on (σ, w · (1 − p)) |
| `observe c` | (σ, w · p) |
| `observe v from D` | (σ, w · P(D = v)) |

Where branches rejoin, worlds with equal states are merged by adding their weights. Before comparing, variables that can't be read again are cleared (liveness analysis), so worlds that differ only in such variables merge. Merging changes nothing but rounding: a set of worlds denotes the sum of its weighted states, and equal states add.

Weights are unnormalized: they include every observation so far. Nothing is normalized except where section 8 and section 9 say so.

## 6. Functions and effects

A function may read any variable in scope; the values are copied in when it's called. It may assign only its own variables. A call runs the function's body as a set of worlds of its own and returns the distribution of results, **unnormalized**: splits and observations inside the function become part of the caller's weights. An ordinary call is not an inference boundary.

Because a function's result depends only on its arguments and the values it reads, the engine may compute it once and reuse it (memoization) when enumerating. It does so only when the function has no debug output: a function that calls `print`, directly or through other functions, runs every time it's called. When sampling, every call runs (section 14).

**Collection callbacks.** Callbacks passed to `map`, `filter`, `count` and `reduce` cannot execute draws (including bag draws), observations, `chance`, or an `if` that splits on uncertainty. The same runtime effect boundary applies in enumeration and sampling, including through helper calls and nested callbacks. Ordinary deterministic conditions are allowed. An explicit `simulate` creates a local inference scope, so a callback may compute or return its distribution. Returning a distribution value does not draw it. A loop is the way to traverse a collection with random effects in the caller's worlds. Cached function results are not reused across this validation boundary: a single returned outcome does not prove that no effect occurred.

**Recursion.** When enumerating, a call can come back to itself while it's running: the same function, with the same arguments, directly or through other calls. An example is `fn f() { if 50% { 1 } else { f() } }`. Its result is then the least solution of what the calls say about each other, found by iteration:
- **Rounds.** In the first round, a call that comes back gets nothing: all of its weight waits. In each later round, it gets the results of the round before.
- **Stopping.** The outermost call that another call came back to runs again, round after round, with everything it calls. It stops once the weight still waiting is below ε (section 10), and that weight is unresolved, as for an unrolled loop.
- **Calls that never return.** If a round leaves as much weight waiting as the round before, part of the call never returns, and that's an error.
- **Print.** A function that prints can't come back to itself. It would print once per round.

When sampling, each run follows its own path, however deep (section 14).

`print` is debug output, not part of the model. It prints once for each world that executes it; worlds that have merged count once, so the number of lines depends on how the engine merges.

## 7. Evidence

`observe c` multiplies each world's weight by the probability of `c` (section 3). `observe v from D` multiplies it by P(D = v), or, when sampling, by the density of a continuous `D` at `v` (section 13). Every factor is between 0 and 1, except densities.

Observations condition worlds, without mutating distribution recipes or linking their independent uses (section 2). For example, `let x = 3d8; observe x > 10; report x` has evidence 392/512 and still reports the original distribution over 3–24: the same likelihood factor applies to every world. With `let x ~ 3d8`, the observation instead removes worlds where the bound total is at most 10, and the report has support 11–24 with each remaining outcome's prior probability divided by 392/512. The evidence is the same in both programs. A recipe depending on a drawn parameter can still have a different reported mixture after observation, because the weights of its parameter worlds change.

The **evidence** of a program is the total final weight of its worlds: the probability of all its observations, including those made inside ordinary function calls. Observations inside `simulate` are not program evidence (section 8).

If the evidence is zero and no weight is unresolved, the evidence is **impossible**: the run fails with an error pointing at an `observe`. This is different from a report that is never reached, which is ordinary control flow.

**No `observe` may follow a `report`.** The compiler rejects a program in which an `observe`, or a call to a function that observes, can run after a `report` on the same path, including through a loop. So every report sees all the evidence its worlds will ever get, and describes the posterior. (Reports that update as evidence arrives, as in filtering, are future work.)

## 8. `simulate`

`simulate { B }` runs `B` as a separate model, starting from a copy of the current world with weight 1. It returns the distribution of `B`'s value, **normalized**: conditioned on the observations made inside `B`. Where `B`'s value is itself a distribution, its outcomes are mixed in (section 2). It is an inference boundary:

- the current world doesn't split;
- the result is always a `dist[T]`, even when only one outcome is possible, and a distribution over probabilities stays a distribution over probabilities (it is never averaged into a single probability);
- observations inside `B` condition the result and don't count as program evidence;
- if all of `B`'s weight is ruled out by its observations, it's an error;
- weight that `B` leaves unresolved (section 10) becomes the result's **missing mass**.

## 9. Reports

`report` is only allowed at the top level of the program. When a world reaches a report, the report records the world's value (and key, with `by`) and its weight.

- The **denominator** of a report is the weight that reached it: the result is conditional on reaching it.
- The **reach** of a report is that weight divided by the program's total weight. When it is below 100%, the output says so ("reached in 1.00% of worlds").
- A `bool` or `dist[bool]` value is reported as the probability that it's true. Numbers are summarized (mean, standard deviation, quantiles); other values are listed with their probabilities. A `dist[T]` value contributes each of its outcomes.

**Repeated reports.** A report outside any loop runs at most once per world, so it describes a distribution over worlds. A report inside a loop must have `by`. The compiler proves at-most-once-per-world/key only for a single enclosing `for` over a directly written integer range, keyed by that loop's own binding. Nested loops, shadowed bindings, arbitrary collections and range aliases are conservatively labelled "per visit". Every visit counts; the engine never silently deduplicates repeated keys. A collection can repeat values even when its loop variable supplies the key.

**Small values.** Numeric summaries, quantile table cells and percentages switch to scientific notation when a nonzero value would otherwise round to zero. A probability below one retains enough decimal places to distinguish it from 100%. A mixed-sign mean at floating-point summation resolution is displayed as `≈0`: the threshold is `γ(n+2) * Σ pᵢ |xᵢ| / Σ pᵢ`, with `γ(k) = k ε / (1 − k ε)` and machine epsilon ε. This distinguishes cancellation noise from genuinely small values; it is a display rule, not a statistical confidence bound, and does not change computed values.

## 10. Termination and approximation

**Unbounded loops.** `while` and `loop` repeat until no world is left inside. `for` and `repeat` loops are never cut short.

- **Solved, when they cycle.** When enumerating, a `while` or `loop` whose worlds come back to a state they were in at the start of an earlier round is solved as an absorbing Markov chain from that round on. A state is the values of the variables read later. The engine finds every state reachable from the worlds inside, runs the body once from each, and solves for the expected number of visits to each state. That gives how much weight leaves by each way out: `break`, `return`, and observations that rule worlds out. The answer is exact up to floating-point rounding, and nothing is left unresolved. If some worlds can never leave, because no way out can be reached from their state, it's an error.
- **Unrolled, otherwise.** Some loops are unrolled instead:
  - one that never comes back to a state, such as one that counts its rounds;
  - one whose body reports or prints, which happen once per visit;
  - one whose chain would have more states than the host allows (50,000 by default).

  An unrolled loop stops early once the weight still inside is less than ε times the weight that entered it (ε is 10⁻¹² by default, set with `@epsilon`). The weight left inside is **unresolved**.

**Infinite supports.** Distributions with infinitely many outcomes (`poisson`, `geometric`) drop outcomes whose probability is below 10⁻¹⁸. The dropped probability is the distribution's missing mass; drawing from it, or using it as a condition, adds (weight × missing mass) to the unresolved weight, because the missing outcomes could go either way. Combining distributions combines their missing mass (for independent draws, 1 − Π(1 − mᵢ)).

**Bounds.** Every observation factor is at most 1, so unresolved weight U can only shrink with further observations. For an event with weight a among the weight Z that reached a report, the true probability therefore lies in

  [ a / (Z + U), (a + U) / (Z + U) ]

where U is all the weight left unresolved by the program. Reports print this range when it is visible at two decimals, and a single number otherwise. Means and quantiles are computed on the resolved part; when unresolved or missing weight is visible, the output says so, but no bound is claimed for them: a rare, very large value can move a mean arbitrarily.

**Numbers.** Weights are binary floating-point numbers with an extended exponent, so repeated observations can't underflow to zero. Sums are subject to rounding (about 10⁻¹⁵ relative). `--fractions` shows the simplest fraction within 10⁻¹³ of a probability, marked "≈": it's a hint for recognizing an answer, not a proof that the answer is exact. The engine's mode is called **enumeration**, not "exact", for these reasons.

## 11. Resource limits

Text limits count UTF-8 bytes, independently of the scalar positions used by the language. `Limits.max_string_bytes` bounds one string (16 MiB by default, 4 MiB in the playground); `max_string_alloc_bytes` bounds cumulative string payloads produced during a run (256 MiB by default, 64 MiB in the playground), shared across sampled workers. The latter is an allocation allowance, not live memory: results are not refunded when dropped, temporary capacity and metadata are excluded, and shared values are not charged again merely for being referenced. Input loading has separate byte/value limits; loaded string sizes are also checked against the runtime's per-string limit before execution.

String scans charge work in proportion to UTF-8 length. Literal construction, concatenation, interpolation, formatting with `str`/`join`, case conversion, trimming, slicing, reversal and character extraction check result sizes and charge payloads before allocation or incremental buffer growth. `chars`, `split` and string iteration also check collection sizes. Large integer ranges can be sliced without allocating their elements. Limit failures produce resource-limit diagnostics.

The host running a program sets upper limits on: worlds per statement (when enumerating; sampled runs don't multiply), outcomes per distribution, total work, loop iterations, call depth, cached results, length of materialized collections, and output. A program's `@max_worlds` and `@max_iterations` can lower these limits, never raise them. Exceeding a limit, or cancelling a run, stops it with an error; so do all language errors. The engine never crashes on a program: a crash is a bug in Probl, and it is reported as an internal error.

## 12. How this document is checked

The crate `probl-oracle` is a second implementation of sections 1–9, for the discrete part of the language whose loops end. It shares only the parser with the engine and is deliberately naive: it follows every path on its own (no merging, no liveness analysis, no memoization), evaluates operands left to right one world at a time, and computes weights as exact fractions.

Its tests generate thousands of programs from that part of the language, run each one through both implementations, and require the same evidence, the same unnormalized weight for every value at every report, or an error from both. The engine runs with merging and memoization on and off. A hand-written corpus covers each rule above, and a fuzzing test feeds mutated programs to the parser, the compiler and the engine under small limits, which must never crash (section 11).

Sampling (section 14) is checked against enumeration on the same generated programs: every estimate must be within six standard errors of the exact value, and the standard errors must be calibrated (across thousands of estimates, the mean squared error in standard errors is close to 1). The samplers of section 13 are checked against their CDFs with Kolmogorov–Smirnov and chi-square tests.

## 13. Continuous distributions

`normal(mean, sd)`, `lognormal(mu, sigma)`, `uniform(lo, hi)`, `beta(a, b)`, `gamma(shape, scale)`, `exponential(rate)`, `triangular(lo, mode, hi)` and `pert(lo, mode, hi)` are continuous distributions. Two more describe an estimate by its 90% interval:

- `a to b` is the lognormal whose 5% and 95% quantiles are `a` and `b`. Both must be positive, as in Squiggle.
- `normal_range(lo, hi)` is the normal whose 5% and 95% quantiles are `lo` and `hi`, for quantities that can be negative.

A continuous distribution is a `dist[float]`, and a recipe like any other distribution (section 2). What can be done with one:

- **Draw from it** with `~`, when sampling (section 14). Enumeration can't list its outcomes, so a continuous draw is an error there.
- **Compare it with a number.** `normal(0, 1) > 1.96` is a `dist[bool]`, true with probability 1 − F(1.96), where F is the distribution's CDF; this works in both modes. `==` is never true.
- **Ask about it.** `mean`, `sd`, `variance`, `median`, `quantile`, `cdf` and `pdf` use the formulas.
- **Choose among them.** A choice with continuous options, such as `one_of([normal(0, 1), 5])` or `if 35% { 1 to 3 } else { 0 }` used as a value, is a mixture: drawing from it chooses an option with its probability, then draws from that option.
- **Use it as evidence.** When sampling, `observe v from D` with a continuous `D` multiplies the weight by D's density at `v`, which can be more than 1.

Anything else, such as arithmetic (`normal(0, 1) * 2`) or comparing two continuous distributions, is an error for now: draw a value first (`let x ~ normal(0, 1)`), then compute with it.

## 14. Sampling

`@mode sample(runs: n, seed: s)` estimates the same model as sections 1–13 by following `n` random paths through the program, called **runs**, instead of every path.

- **A run is a single world.** Where enumeration would split a world (`if`, `chance`, `~`, taking a card, calling a function), a run takes one branch, chosen with that branch's probability, and its weight doesn't change. A `dist[bool]` condition picks a branch with the probability that it's true, without drawing the distribution.
- **Weights come from evidence.** `observe` multiplies a run's weight as in section 7 (likelihood weighting). A run that is ruled out has weight zero.
- **Runs are independent.** They don't merge, and calls aren't memoized, so every call makes its own choices. A function may call itself with the same arguments: each call chooses its own path. Unbounded loops aren't cut short; the iteration limit still applies.
- **`simulate` is enumerated.** Inside each run, a `simulate` block is computed exactly, by enumeration, as in section 8, so its result has no sampling error. A block that enumeration can't compute (because it draws from a continuous distribution, say) is an error, even when sampling: estimates inside estimates aren't supported yet.
- **Reproducible.** The same program, input data, execution-date snapshot, seed and version of Probl give the same output on any machine, with any number of threads. Runs go in batches of 1,000, each with a random stream of its own, derived from the seed and the batch's number. Batches may run at the same time, but they're combined in order: the estimates, what `print` shows and the first error are those of running them one after another. Only whether a run reaches a host's limit on work, which the threads share, can depend on timing.
- The missing mass of infinite discrete distributions (below 10⁻¹⁸, section 10) is ignored.

**Estimates.** Let the runs end with weights w₁…wₙ. A run's weight when it reaches a report is its final weight, since no observation may follow a report (section 7). A report's estimates are averages over the runs, weighted and normalized:

- A **probability** is estimated as p̂ = Σ wᵢ aᵢ / Σ wᵢ bᵢ, where bᵢ is the number of times run i reached the report (0 or 1, except for per-visit reports), and aᵢ adds up, over those times, the probability that the reported fact was true: 1 or 0 for a fact, P(true) for a `dist[bool]`. A reported finite distribution isn't drawn: each of its outcomes counts with its probability. (A continuous one can't list its outcomes, so it's drawn once.) The probabilities of other values are estimated the same way.
- Its **standard error** is √(Σ wᵢ² (aᵢ − p̂ bᵢ)²) / Σ wᵢ bᵢ (the delta method; with equal weights, this is √(p̂(1 − p̂)/n)). Probabilities are printed with it, rounded to its precision: `46.1% ± 0.3%`.
- A **mean** is estimated the same way, with aᵢ adding up the reported values, and printed with its standard error when that shows at the printed precision. Standard deviations and quantiles are those of the weighted runs, with no error estimate.
- A report's **reach** is the weight of the runs that reached it, divided by the weight of all runs.
- The **effective sample size** (Σ wᵢ)² / Σ wᵢ² is printed when observations made the weights unequal. It says how many equally weighted runs the estimates are worth; when it's small, the estimates and their standard errors are unreliable.

**Reliability of individual reports.** Each report/key records how many independent runs contributed; repeated visits from one run count once. Its effective sample size uses that estimator's denominator contributions: `(Σ wᵢ bᵢ)² / Σ (wᵢ bᵢ)²`. A report/key below 30 effective contributions displays a low-support note with both counts, including in numeric and categorical tables. Thirty is a display heuristic, not a guarantee that an estimate above it is accurate.

For an ordinary Bernoulli report with at most one contribution per world/key, no possible outer observations, equal weights and no integrated distributions, the output uses a labelled **two-sided 95% Wilson score interval** when fewer than 30 runs contributed or all outcomes agree. It replaces the usual standard-error display in those cases; it is not a ± one-standard-error interval. See the [NIST formula](https://www.itl.nist.gov/div898/handbook/prc/section2/prc241.htm). With two successes in two trials the interval is approximately 34.24%–100%, rather than `100% ± 0%`.

Wilson intervals are not applied to weighted, per-visit or integrated reports. When the computed empirical error is zero for an ordinary sampled probability, the display says the MC error is not estimable. A finite distribution reported directly integrates its outcomes rather than drawing them; a zero error is labelled `zero empirical MC error; integrated outcomes`, without a binomial interval. These statements describe the estimator on the observed runs, not proof that unvisited outcomes are impossible. The raw delta-method standard-error accessors retain their value, including zero when the empirical variance vanishes or is below numerical resolution.

**Evidence.** When the program observes, the evidence (section 7) is estimated by the average final weight of the runs, Ẑ = Σ wᵢ / n, counting the runs ruled out as 0. Ẑ is unbiased. Its standard error, relative to Ẑ, is √((n / ESS − 1) / (n − 1)), where ESS is the effective sample size.

- **Probabilities only.** When every observation is of a probability (facts, and values of discrete distributions), Ẑ estimates the probability of the evidence. It's printed like enumeration's, with its standard error: `evidence 3.24% ± 0.05%`. Below 0.01%, it's printed in scientific notation with its relative error: `evidence 1.23e-14 (± 0.6%)`.
- **Densities.** An observation of a continuous value multiplies by a density (section 13), so then Ẑ estimates a density, whose scale depends on the units. It's printed as its natural logarithm, whose standard error is Ẑ's relative one: `log evidence -42.31 ± 0.01`. The difference between two models' log evidence on the same data is the logarithm of their Bayes factor.

When few runs carry the weight (a small effective sample size), this estimate is unreliable too, and more often too low than too high. If every run is ruled out, it's an error: the evidence is impossible, or too unlikely for this number of runs.

**Exact updates for conjugate priors.** Unless whoever runs the program turns them off (`probl run --no-conjugate`), some variables are updated exactly instead of being drawn and weighted by the evidence. This changes which random numbers a seed gives, and the variance of the estimates, not what they estimate.

- **Delayed draws.** A draw `x ~ D` into a whole variable, outside `simulate`, is *delayed* when the function it's in also observes `x` in one of the forms below, and `D`'s value is a single beta, gamma or normal distribution. `D` is evaluated and checked at the draw, as usual. `x` then holds `D` internally: no expression can read it in that state.
- **Exact updates.** Each of these observations of a delayed `x` multiplies the run's weight by the probability of what's observed (a density for `normal`) given `x`'s current distribution. It then replaces that distribution with `x`'s distribution given the observation:

  | `x`'s distribution | Observation | Afterwards | The weight multiplies by |
  |---|---|---|---|
  | `beta(α, β)` | `observe k from binomial(n, x)` | `beta(α + k, β + n − k)` | C(n, k) B(α + k, β + n − k) / B(α, β) |
  | `beta(α, β)` | `observe v from bernoulli(x)`, or `observe bernoulli(x)` (v is `true`) | `beta(α + 1, β)` if v, `beta(α, β + 1)` if not | α / (α + β), or β / (α + β) |
  | `gamma(s, θ)` | `observe k from poisson(x)` | `gamma(s + k, θ / (1 + θ))` | Γ(s + k) / (Γ(s) k!) · (θ / (1 + θ))ᵏ (1 + θ)⁻ˢ |
  | `normal(μ, σ)` | `observe y from normal(x, τ)` | `normal(μ + g(y − μ), στ / √(σ² + τ²))`, where g = σ² / (σ² + τ²) | the normal density of `y` with mean μ and standard deviation √(σ² + τ²) |

  `x` must be the parameter, written as itself, and appear nowhere else in the observation. The observation's other parts must be plain values; they're evaluated and checked as when `x` is drawn, with the same errors. A value the distribution can't produce, such as a count above `n`, makes the run impossible.
- **Drawing.** Any other statement that reads a delayed `x` draws it first, from its current distribution, and `x` is an ordinary number from then on. That includes every expression, a call or closure that captures `x`, and an observation in another form, or of a family that doesn't pair. A type check draws `x` only if some of its distribution's values could fail it, so `let p: prob ~ beta(2, 3)` stays delayed. A delayed variable that's assigned, or never read again, is never drawn.
- **The same model.** Drawing `x` from its updated distribution, with the weight multiplied by the probability of each observation given the distribution before it, gives the same expected weight to every outcome as drawing `x` at `~` and weighting by each observation given `x`. So every report and the evidence estimate the same quantities, and runs are still independent: the estimators and standard errors above apply unchanged. When every observation is an exact update and nothing else random affects the weights, every run ends with the same weight. The effective sample size is then n, and Ẑ is the evidence itself, with a standard error of 0. The reports still have sampling error.
- **Observations after the draw.** An observation made after `x` is drawn weights the runs as usual, and `x` was drawn from its distribution given the earlier observations only. When the later observations disagree with the earlier ones, that can leave fewer effective runs than drawing `x` from its prior would. Reading `x` only after all its observations avoids it.
- **Limits.** An exact update doesn't draw `x` or list the observed distribution's outcomes, so it doesn't reach the limits that drawing would, such as `poisson`'s rate above 10¹⁵. Probabilities are computed as logarithms, so an observation's probability can be far below the smallest floating-point number.

## 15. Data

`let name: T = read(path)` binds data read from outside the program, such as a CSV or JSON file ([reading data](data-input.md)).

- **Before running.** The data is read before the program runs, as a value of the declared type `T`. Data that doesn't fit `T` is an error, and the program doesn't run.
- **A constant.** The value is the same in every world and every run, as if it had been written in the program. Reading it splits nothing and weighs nothing: it isn't evidence. To condition on data, `observe` it.
- **Part of the input.** The same program, data, execution-date snapshot and seed give the same output for a given engine version.

## 16. Not specified yet

- **Particles, beam search and merged runs** (audit D4): how merged samples keep their statistical bookkeeping, and when particles resample.
- **A general method for models that aren't conjugate**, such as MCMC: its target, its moves and its report estimators ([inference proposal](inference-proposal.md), section 2).
- **Nested estimates** (D4): `simulate` blocks that must be sampled, and how their error affects decisions.
- **Arithmetic on continuous distributions**, beyond comparing them with numbers.
- **Reports that update with later evidence** (filtering and smoothing).
