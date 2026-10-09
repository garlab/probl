# Probl reference semantics

> Reference contract, updated October 2026. This document is normative for the engine. Where it disagrees with the [language overview](language-overview.md), this document wins. It resolves findings D1–D3, I1 and I2 of the [project audit](design/project-audit.md). Continuous distributions (D6) and sampling (D4), with exact updates for conjugate priors, are in sections 13 and 14, and data read from files in section 15; loops and recursion that cycle are solved as in sections 6 and 10 (D5); what is still open, such as particles, is listed in section 16.

## 1. Values and types

| Type | Values | Notes |
|---|---|---|
| `bool` | `true`, `false` | **Facts.** Comparisons of settled values, `and`/`or`/`not`, `in`, pattern tests |
| `prob` | a number from 0 to 1 | **Probabilities.** Checked finite values in `[0, 1]`; a parameter, not an event |
| `int` | arbitrary-precision whole numbers | Exact arithmetic, bounded by host resource limits |
| `float` | IEEE 754 binary64 numbers | Arithmetic on probabilities gives floats |
| `complex` | `complex(re, im)` | Two finite floating-point components; never an implicit probability |
| `str`, `date`, `list[T]`, `map[K, V]`, `bag[T]`, records, enums, functions | | |
| `dist[T]` | a distribution over `T` | Dice, `one_of`, `bernoulli`, counts, `roll`, `simulate`, continuous distributions (section 13), and operations on distributions |

All values are immutable: assigning or passing a collection gives an independent copy.

**Percentages and conversion.** `33%` is numeric notation for `0.33`: both have type `float`. This applies to all percentages, including `0%`, `100%`, negative percentages and percentages above 100%. There is no type selection based on a number's value.

`prob(x)` explicitly constructs a probability from a finite number in `[0, 1]`, a boolean (`false` becomes 0, `true` becomes 1), or an existing probability. Invalid ranges, nonfinite values and other types are errors. It neither clamps nor draws nor lifts over distributions. `P(d)` queries a boolean distribution (`dist[bool]`) only. Scalar booleans, probabilities and numbers are errors: use `prob(fact)` for a 0/1 conversion and `report fact` to aggregate across worlds.

A numeric value is implicitly converted wherever `prob` is expected: `let p: prob = rate`, `fn f(p: prob) { ... }; f(rate)`, and `binomial(n, rate)` all accept a numeric `rate`. Conversion checks that the value is finite and within `[0, 1]`; it never clamps. Numeric literals can be checked during compilation; variables, named constants and calculations are checked when evaluated. The source value keeps its type: `let rate = 33%; let p: prob = rate` leaves `rate` a float and makes `p` a probability.

The same context applies to typed return values, record fields, collection element types, branching conditions, `chance` weights and `score`. Container annotations convert their contents recursively without modifying the source container. Expected types propagate through the result branches of `if`, `chance`, `match` and blocks; for example, `score if sick { 95% } else { 8% }` supplies probability context to both branches. Booleans still require explicit `prob(fact)` in a probability parameter. A distribution is not converted to a scalar probability; existing lifting rules for built-ins still apply to its individual outcomes. Data schemas validate and construct their declared types when loading input.

Probabilities widen to floats in numeric contexts. Ordinary arithmetic (`p + q`, `p * q`, `1 - p`, negation) returns numbers; it never chooses a probability type based on the result's range. Booleans never implicitly become probabilities or numbers. Neither probabilities nor numbers implicitly become booleans. Logical operators do not convert raw numbers: `0.33 and 0.5` is an error. Actual probabilities compose as independent Bernoulli recipes: `prob(0.33) and prob(0.5)` is `prob(0.165)`. This specifies independent trials, rather than inferring a joint probability for previously bound events.

`one_of` maps have two explicit modes: all-numeric weights are relative (including percentage notation), while all-`prob` weights are absolute and must sum to one. Relative weights are normalized after scaling by their largest weight, so a finite common scale does not erase the distribution when the unscaled sum overflows. At least one weight must be positive. Extremely small normalized weights can still underflow in floating-point arithmetic. Mixing the modes is an error. Thus `one_of([true: 33%, false: 33%])` is a fair choice; `one_of([true: prob(33%), false: prob(33%)])` fails.

**Checked integer conversion.** Wherever an `int` is expected, an existing integer is preserved and a finite, exactly integral `float` is converted to the integer it represents. Thus `let n: int = 1.0` succeeds, while `let n: int = 1.4` fails. Conversion never rounds, truncates, or uses a tolerance: `1.0000000000000002` is rejected too. This applies to declared bindings, parameters, returns, record fields, recursively typed containers, integer built-in arguments, sequence positions, range bounds, repeat/roll/bag counts, and date components and offsets. Booleans, strings, probabilities and complex values do not implicitly convert to ints. Percentage notation is a float, so `100%` can convert to 1; `prob(1)` cannot.

Conversion is requested by the receiving context, not inferred from a value's magnitude. `let x = 1.0; let n: int = x` leaves `x` a float and makes `n` an int. Ordinary mixed numeric arithmetic retains its existing float behavior. Invalid literals in declared integer contexts are diagnosed during compilation where possible; computed values and built-in arguments are checked at runtime. A failed conversion stops execution with an error, including when it occurs in one probabilistic world or a lifted distribution outcome, and in either failure mode (section 11): a declared type is a contract, not a fault of the values. It does not condition the model by discarding that outcome. Integer size, memory, work and operation-specific bounds still apply. Exactness refers to the stored binary64 value, which may already have rounded during parsing or earlier arithmetic. Input-file schemas retain their format-specific parsing rules (section 15).

**Public identity.** Map keys and bag elements use exact typed identity for membership, indexing, lookup, insertion and removal. Thus `[1: "a"].contains(1.0)` is false and `get([1: "a"], 1.0, "absent")` returns "absent". Scalar numeric equality remains numeric (`1 == 1.0`), as does list membership (`[1].contains(1.0)`). Container equality remains structural and typed: `[1] != [1.0]` and `{x: 1} != {x: 1.0}`. Internal storage, hashing, memoization and world merging keep that same typed identity.

Typed map conversion rejects keys that become identical instead of silently overwriting an entry. Typed bag conversion combines counts for identical converted elements; distribution conversion combines their weights. Original containers keep their original types and values.

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

The holiday argument must be a list of dates. Order and duplicate dates do not matter, weekend entries do not remove extra weekdays, and no national or regional holidays are implicit. Workday counts use checked integer conversion. Weekday-only shifts take constant time; explicit holiday calendars require work proportional to sorting and scanning their entries, charged to the work and collection budgets.

Clamping makes month/year arithmetic non-invertible and non-associative: January 31 plus one month plus one month can give March 28, whereas January 31 plus two months gives March 31. Recurrences should derive each occurrence from the original anchor. There is no generic fixed-length `months(n)` or `years(n)` duration. This follows the default constrained arithmetic of [Temporal.PlainDate](https://tc39.es/proposal-temporal/docs/plaindate.html#add); the supported year range matches [Python's date](https://docs.python.org/3/library/datetime.html#date-objects).

**Text and sequences.** Strings are immutable, valid Unicode text stored as UTF-8. Source files and text data use UTF-8; malformed input is an error. There is no separate character type. String length, indexing, iteration, slicing, `chars`, `split(s, "")` and `reverse` all use **Unicode scalar values**, with each element represented by a one-scalar string. This is neither byte indexing nor grapheme-cluster indexing: `len("🙂")` is 1, `len("🇫🇷")` is 2, and `len("é")` is 2 (an `e` followed by a combining acute accent). Slicing and reversal can separate combining marks or emoji components while always preserving valid UTF-8. Grapheme segmentation is not currently exposed; see [Unicode text segmentation](https://www.unicode.org/reports/tr29/) for the distinction.

Equality, hashing, substring search and prefix/suffix tests compare exact text, without normalization or case folding. Precomposed `"é"` and decomposed `"é"` are distinct map keys and distribution outcomes. Ordering is lexicographic Unicode scalar order, not locale collation. `upper` and `lower` use the Rust standard library's Unicode mappings, independent of locale, and can change length: `upper("ß")` is `"SS"`, and `lower("İ")` is an `i` followed by a combining dot. Native and WASM builds use the same Rust Unicode tables; updating the toolchain can update those tables. No transformation mutates its input.

Strings, lists and ranges are ordered sequences. `slice(xs, start, end)` selects positions from `start` inclusive to `end` exclusive; `slice(xs, start)` defaults `end` to `len(xs)`. Bounds use checked integer conversion and must satisfy `0 <= start <= end <= len(xs)`. Invalid types or bounds are errors: there are no negative offsets, clamping or step arguments. Equal bounds produce an empty result, including at the sequence's length. A string slice is a string, a list slice is a list, and a range slice stays a compact range without materializing its elements. Empty range slices are represented as `0..-1`. Maps and bags do not support slicing. String indexing and slicing scan UTF-8 in linear time with constant auxiliary space; they do not allocate a character array. Sequence `[]` access and assignment, sequence `get`, list `insert`/`remove`, and sequence `slice` share checked integer conversion. For example, `[10, 20][1.0]` and `[10, 20].get(1.0, -1)` both return 20. A sequence `get` fallback handles an out-of-range integer position, including a negative position; a fractional or nonnumeric index is always an error, even for an empty list. Access without a fallback rejects out-of-range positions. Map keys and bag elements retain their own key semantics.

**Sequence capabilities.** `get(xs, index, default?)` works for lists, ranges and strings. Strings return a one-scalar string, matching `[]`; ranges retain exact integer indexing without materialization. Map lookups use typed keys. Bag lookup is a count: every key has a defined count, zero when absent, so a supplied default is never used.

| Operation | List | Range | String | Map / bag |
|---|---|---|---|---|
| `len`, `contains` | Yes | Yes | Yes | Yes; membership uses typed keys |
| `get` | Indexed item | Indexed integer | Indexed Unicode scalar | Value / count |
| `slice` | List | Compact range | String | Unsupported |
| `sort`, `sort_desc` | List result | List result | List of scalar strings | Unsupported |
| `minimum`, `maximum` | Ordered element | Endpoint | Scalar string | Unsupported |
| Population statistics | Nonempty, admissible elements | Nonempty integer range | Unsupported | Unsupported |

**Extrema.** `min(a, b, …)` and `max(a, b, …)` require at least two candidates. They retain ordinary finite-distribution lifting: `max(d6, 3)` is a distribution of outcomes. Recipes combine independently, including repeated uses of the same recipe; previously drawn values retain their correlations. Missing mass is preserved, and candidates are combined incrementally. They never unpack a list argument.

`minimum(xs, compare?, default?)` and `maximum(xs, compare?, default?)` select an original element of a nonempty list, range or string. They use the same ordering and optional numeric comparator as `sort`; ties retain the first element. Without a comparator, even singleton elements must support language ordering. Nested lists remain lexicographic values. Collection elements are never implicitly drawn or lifted: `maximum([d6,d8])` is an error, while `maximum([d6,d8], (a,b)->mean(a)-mean(b))` selects the `d8` recipe. To combine recipes explicitly, use `[d6,d8].reduce(max)`.

An optional default handles an empty list, range or string: `maximum(xs, default: value)` uses natural ordering, and `maximum(xs, compare, default: value)` uses a comparator. The third positional form `maximum(xs, compare, value)` is equivalent. A default never competes with elements: `maximum([-8,-3], default: 0)` is -3. It is returned unchanged only for an empty collection; invalid elements, malformed comparators and unresolved or unbounded distributions still error. Supplied comparators are checked for type and arity even for empty collections. Distribution support queries never use the default. Without a default, empty collections remain errors.

Arguments, including defaults, are evaluated eagerly from left to right. `maximum([1], default: 1/0)` therefore errors even though the fallback would not be selected. Named arguments follow positional arguments, cannot repeat, and currently only `default` on `minimum`/`maximum` is supported. This also works through function values: `let greatest=maximum; greatest([], default: 0)`. User functions and other builtins do not accept named arguments.

**Reduction.** `reduce(xs, f, initial?)`, also `xs.reduce(f, initial?)`, folds left to right with `f(accumulator, element)`. With an initial value, all elements participate and an empty collection returns that value. Without one, the first element becomes the accumulator, a singleton returns its element unchanged, and an empty collection errors. The callback must accept two arguments even when it need not run. Lists, integer ranges and strings are supported; strings iterate Unicode scalars. Callback effect restrictions and finite-distribution lifting of the collection remain unchanged.

The seed participates in every reduction, unlike an extrema default: `[-8,-3].reduce(max,0)` is 0, while `[-8,-3].reduce(max)` is -3. The signature changed from `reduce(xs, initial, f)`; migrate existing seeded calls by putting the callback before the seed. There is no automatic detection of the former order.

`minimum(d)` and `maximum(d)` instead inspect a distribution's whole support. `maximum(3d8)` is 24; `maximum(one_of([[1,9],[2,0]]))` is `[2,0]`, not a distribution of collection maxima. Comparators are accepted only for collections. Distribution queries reject any positive unresolved mass rather than selecting from a truncated support. They retain exact finite outcome values, including bigints. Continuous distributions use the endpoints of their **closed support**, so `minimum(exponential(1))` is 0 and `maximum(uniform(0,1))` is 1. A requested unbounded or unrepresentable endpoint is an error; Probl does not expose infinity values. These queries do not summarize an individual drawn scalar across worlds.

`highest(xs, n, compare?)` and `lowest(xs, n, compare?)` always return a list, sorted largest/smallest first, with up to `n` elements. The nonnegative count is mandatory; zero yields an empty list and counts beyond the collection's size return all its elements. Ties remain in input order. Comparators follow `sort`, including callback restrictions. As with sorting, a finite distribution of collections lifts, but recipes inside a collection need an explicit comparator.

There are currently no separate `inf`/`sup` functions. For finite ordered collections they would duplicate minimum/maximum; distribution queries already return closed-support bounds. A future API for open intervals or general sets could distinguish an attained extremum from an infimum/supremum without changing this distribution contract.

**Function values.** Named functions, builtin functions and lambdas can be stored, returned, passed to callbacks and called through a variable. `[-2,3].map(abs)` and `[1,2,3].reduce(max)` work without wrappers. Referencing a named function captures the current values of its free variables, just like a lambda; direct named calls continue to receive the current environment. Aliased calls preserve declared parameter/result checks, builtin arities and probability parameter validation. Callback effect restrictions and print-aware caching/draw scheduling also apply through function values. Mutation syntax such as `deck.take()` still requires a mutable receiver; a function reference does not capture a mutable location.

**Sorting.** `sort(xs, compare?)` and `sort_desc(xs, compare?)` return a new list from a list, range or string. Both are stable: ties retain their input order, including in descending order. Without a comparator, elements must support the language's ordering, including in singleton and nested lists. Complex values, records and distribution recipes need a custom comparator. Empty inputs return an empty list. Strings are sorted as Unicode scalar strings, without locale collation.

The optional two-argument comparator returns an `int` or finite `float`: negative puts its first argument before the second, zero means a tie, and positive puts it after. `sort_desc` reverses this order, preserving ties. Booleans, probabilities, complex values and distribution results are errors. For example, `xs.sort((a, b) -> abs(a) - abs(b))` orders complex values by magnitude, and `rows.sort((a, b) -> a.priority - b.priority)` orders records by a field. The function and its arity are checked even for empty or singleton inputs; its body runs only when a comparison is needed. A comparator must provide a consistent, transitive ordering; results from an inconsistent comparator are unspecified, but it cannot make the sorting algorithm panic or loop without bound. Comparison order and call counts are not part of the API.

Sorting callbacks follow the same effect rules as `map` and `filter`: they cannot draw, observe, score or branch on uncertainty outside a local `simulate`. Callback errors stop sorting immediately. Existing finite-distribution lifting applies to the collection argument without drawing it. Sorting and callback work obey execution limits.

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

**Integer and special functions.** The following functions also act on each outcome of a finite distribution. The integer functions use checked integer conversion for their arguments and return exact `int` results. Results grow beyond machine integers; exceeding the integer size or work limit is an error, never a rounded float.

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

`bit_length` and `ilog2` inspect the stored integer magnitude in constant time, including for bigints, and each charges one unit of work for that inspection. For positive integers, `ilog2(n) = bit_length(n) - 1`. Both accept ints or exactly integral finite floats and lift over finite distributions. Integer inputs never convert through a float: `ilog2(2^100 - 1)` is 99 even though `floor(log2(2^100 - 1))` rounds to 100. The approximate `log2` function retains its float and complex behavior.

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

The data-file schema does not add a complex encoding; read real and imaginary fields and construct values explicitly. The [complex and quantum design note](design/quantum-and-complex.md) distinguishes these implemented scalar rules from a possible future quantum engine.

## 2. Events and identity

A **distribution is a recipe**. Each occurrence of a distribution in an expression is an independent draw: `d6 + d6` is two dice, and `let die = d6` followed by `die + die` is also two dice.

A **drawn value is a fact**. `let x ~ d6` gives `x` one value in each world, and every use of `x` refers to that value.

Operators and built-in functions applied to distributions act on every outcome and return a distribution. Comparing a distribution gives a `dist[bool]`: `d6 > 4` is the distribution of the fact "this roll is above 4", which is true a third of the time. A distribution stays a distribution even when only one outcome is possible (so `d1` is `dist[int]`, not `int`).

A distribution never has distributions as outcomes: where one would, they are mixed in. `one_of([d2, 10])` is 1 or 2 a quarter of the time each and 10 half the time, so a value drawn from it is settled. The same holds for the results of operators and of `simulate` (section 8).

**Logical operators compose facts and independent recipes.** `and`, `or` and `not` accept `bool`, `prob` and `dist[bool]`. Each use of a recipe means an independent trial, conditional on the current world, consistently with numeric distribution arithmetic. Thus `let d = d6 > 4; report d and d` gives 1/9. Drawing `let a = ~d` and reporting `a and a` gives 1/3: both uses name the same bound outcome.

For booleans, the result is boolean. When both operands are evaluated, combinations of probabilities and booleans with at least one probability return `prob`: conjunction multiplies rates, disjunction is `p + (1-p)*q`, and negation is `1-p`. When a distribution is involved, the result is `dist[bool]`, retaining unresolved mass. Only an actual left-hand boolean short-circuits (`false and ...`, `true or ...`), returning that boolean; probability endpoints and singleton distributions remain recipes and do not suppress evaluation of the right operand.

```probl
let p: prob = 30%
let rain = ~p
let late = ~prob(20%)
report rain and late          # 6%
report rain and rain          # 30%: the same event
report p and p                # 9%: independent trials
```

**Prefix drawing.** `~D` draws once each time it is evaluated and returns a settled outcome in each world. It binds more tightly than every binary operator, but after calls, indexing and field access: `~d6 != 6` means `(~d6) != 6`, `~d6 ^ 2` means `(~d6) ^ 2`, and `~f()[0]` draws from the indexed result. Use `~(d6 > 3)` to draw the comparison's boolean result. Operands still run left to right, once each, and boolean short-circuiting skips a draw on an unevaluated right side.

`let x ~ D` and `let x = ~D` have the same meaning; assignment has the same equivalence. A `prob` draws a Bernoulli boolean directly. A distribution over probabilities still draws a probability value, without automatically performing a second trial. Drawing any other plain value keeps the existing point-value behavior, so `x ~ if c { d6 } else { 0 }` works. In particular, `%` is still numeric notation: `~30%` produces the float 0.3; use `~prob(30%)` or a `prob` binding to request a trial.

**Bag extraction is an operation.** `deck.take()` selects an item, removes exactly one copy from the mutable bag in that world, and returns the item unchanged. Enumeration branches by item counts; sampling chooses one item. An empty bag is an error. The operation works anywhere an expression is accepted, runs once per evaluation, and respects left-to-right evaluation and short-circuiting. It also consumes an item when its result is unused. The receiver must be a mutable local variable, field or element, under the same mutation restrictions as other collection methods.

A probability or distribution stored in the bag remains a recipe after extraction. `let p = rates.take()` returns a probability from a bag of probabilities; `let event = ~rates.take()` takes a probability and then draws a boolean from it. The binding form `let event ~ rates.take()` does the same two operations. There is no special draw syntax for bags. Use `one_of(deck)` to construct a distribution without removal.

**`match` needs a settled subject.** Matching a distribution is an error; draw it first. Patterns are tests on a single value, and `|` between patterns is an ordinary `or` on facts.

## 3. Conditions and branching

`if c`, `while c` and match guards accept **bool**, **prob** or **dist[bool]**. A boolean follows an existing fact. A probability or boolean distribution requests a fresh trial each time the condition is evaluated: enumeration splits world weights, and sampling chooses a branch. Unresolved distribution mass remains unresolved. This is a branching operation, not a general conversion to boolean.

Numeric conditions are checked as probabilities. `if 30% { "rain" } else { "sun" }`, `if 0.3 { "rain" } else { "sun" }`, and `let p = 30%; if p { "rain" } else { "sun" }` agree. Calculations and named constants follow the same rule, without changing their source types. This is not numeric truthiness: nonfinite values and values outside `[0, 1]`, including `2` and `-1`, are errors. General type and range checks happen when reached; invalid literals can be diagnosed during compilation.

`if d6 > 5` branches on the boolean recipe directly. To give an event a reusable identity, draw it: `let hit = ~(d20 + 5 >= 15); if hit { ... }`. A loop can write `while ~d6 != 6 { ... }`, drawing a face on every test, or `while d6 != 6 { ... }`, branching directly on the boolean recipe. Neither syntax reuses a trial from an earlier iteration.

`chance { p₁ => A₁ … pₙ => Aₙ else => B }` is explicit weighted branching. It evaluates every weight first; each must be a probability or a number checked in `[0, 1]`. Use `prob(fact)` for booleans and `P(d)` for a queried probability. The weights must not add up to more than 1 (a tolerance of 10⁻⁹ allows for rounding). The remainder goes to `else`. Without `else`, the remainder continues after the statement; when `chance` is used as a value, a remainder above 10⁻⁹ is an error.

## 4. Evaluation order

Evaluation is **left to right**, and every operand is evaluated exactly once:

- operands of binary operators, arguments of calls, elements of lists, maps and records, and the parts of string interpolation are evaluated in the order they're written;
- `place = value` evaluates `value`, then the indices in `place`, then assigns;
- `place += value` (and `-=`, `*=`, `/=`) reads the current value of `place` first, then evaluates `value`;
- `and` doesn't evaluate its right side when the left side is the boolean `false`, and `or` when it is the boolean `true`.

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
| `if c { A } else { B }` | A on (σ, w · t), B on (σ, w · f), where t and f are c's true and false weights; missing mass stays unresolved |
| `observe c` | (σ, w) if c is true; discarded otherwise |
| `score p` | (σ, w · p) |
| `observe v from D` | (σ, w · P(D = v)) |

Where branches rejoin, worlds with equal states are merged by adding their weights. Before comparing, variables that can't be read again are cleared (liveness analysis), so worlds that differ only in such variables merge. Merging changes nothing but rounding: a set of worlds denotes the sum of its weighted states, and equal states add.

Weights are unnormalized: they include every observation so far. Nothing is normalized except where section 8 and section 9 say so.

## 6. Functions and effects

A function may read any variable in scope; the values are copied in when it's called. It may assign only its own variables. A call runs the function's body as a set of worlds of its own and returns the distribution of results, **unnormalized**: splits and observations inside the function become part of the caller's weights. An ordinary call is not an inference boundary.

Because a function's result depends only on its arguments and the values it reads, the engine may compute it once and reuse it (memoization) when enumerating. It does so only when the function has no debug output: a function that calls `print`, directly or through other functions, runs every time it's called. When sampling, every call runs (section 14).

**Collection callbacks.** Callbacks passed to `map`, `filter`, `count` and `reduce` cannot execute draws (including bag draws), observations, scores or `chance`. The same runtime effect boundary applies in enumeration and sampling, including through helper calls and nested callbacks. Ordinary deterministic conditions are allowed. An explicit `simulate` creates a local inference scope, so a callback may compute or return its distribution. Returning a distribution value does not draw it. A loop is the way to traverse a collection with random effects in the caller's worlds. Cached function results are not reused across this validation boundary: a single returned outcome does not prove that no effect occurred.

**Recursion.** When enumerating, a call can come back to itself while it's running: the same function, with the same arguments, directly or through other calls. An example is `fn f() { chance { 50% => 1, else => f() } }`. Its result is then the least solution of what the calls say about each other, found by iteration:
- **Rounds.** In the first round, a call that comes back gets nothing: all of its weight waits. In each later round, it gets the results of the round before.
- **Stopping.** The outermost call that another call came back to runs again, round after round, with everything it calls. It stops once the weight still waiting is below ε (section 10), and that weight is unresolved, as for an unrolled loop.
- **Calls that never return.** If a round leaves as much weight waiting as the round before, part of the call never returns, and that's an error.
- **Print.** A function that prints can't come back to itself. It would print once per round.

When sampling, each run follows its own path, however deep (section 14).

`print` is debug output, not part of the model. It prints once for each world that executes it; worlds that have merged count once, so the number of lines depends on how the engine merges.

## 7. Evidence

`observe c` retains worlds where the boolean `c` is true and discards the others. `observe v from D` weighs each world by P(D = v), or, when sampling, by the density of a continuous `D` at `v` (section 13). `score p` applies a probability likelihood by multiplying each world's weight by `p`. Its operand must be `prob` or a numeric value checked in `[0, 1]`, including variables, calculations and branch results (`score if sick { 95% } else { 8% }`). A zero score rules out the world, a unit score leaves its weight unchanged, and invalid types or ranges are errors. `score` never draws, clamps or changes the probability value. It is equivalent to `observe true from bernoulli(p)`, preserving exact conjugate updates for `score rate` after a beta draw.

`observe ~p` explicitly observes an anonymous boolean draw. The engine integrates out that unnamed trial and applies its likelihood, retaining unresolved mass instead of treating it as false. This also works for boolean distribution recipes. It rejects non-boolean laws; it does not turn `observe ~normal(mu, sd)` into a continuous equality observation. A bag draw is actually consumed before the observation. To retain the event's identity, bind it first: `let event = ~p; observe event; report event` reports 100%.

`let x = 3d8; observe x > 10; report x` is an error: the comparison is a distribution, not a fact. With `let x ~ 3d8`, the observation removes worlds where the bound total is at most 10, leaving support 11–24 and evidence 392/512. Similarly, `let p = prob(5%); observe p` fails. Draw `let e ~ bernoulli(p); observe e; report e` to report the observed event as 100%. An explicit likelihood observation of a recipe, `observe true from (x > 10)`, remains valid and does not mutate the recipe.

The **evidence** of a program is the total final weight of its worlds: the probability of all its observations, including those made inside ordinary function calls. Observations inside `simulate` are not program evidence (section 8).

If the evidence is zero and no weight is unresolved, the evidence is **impossible**: the run fails with an error pointing at an `observe`. This is different from a report that is never reached, which is ordinary control flow.

**No `observe` or `score` may follow a `report`.** The compiler rejects a program in which an `observe` or `score`, or a call to a function that applies evidence, can run after a `report` on the same path, including through a loop. So every report sees all the evidence its worlds will ever get, and describes the posterior. (Reports that update as evidence arrives, as in filtering, are future work.)

## 8. `simulate`

`simulate { B }` runs `B` as a separate model, starting from a copy of the current world with weight 1. It returns the distribution of `B`'s value, **normalized**: conditioned on the observations made inside `B`. Where `B`'s value is itself a distribution, its outcomes are mixed in (section 2). It is an inference boundary:

- the current world doesn't split;
- the result is always a `dist[T]`, even when only one outcome is possible, and a distribution over probabilities stays a distribution over probabilities (it is never averaged into a single probability);
- observations inside `B` condition the result and don't count as program evidence;
- if all of `B`'s weight is ruled out by its observations, it's an error;
- weight that `B` leaves unresolved (section 10) becomes the result's **missing mass**.

**Statistical queries.** `mean`, `variance`, `sd`, `median`, `median_low`, `median_high`, `quantile`, `cdf`, `pmf` and `support` accept explicit distributions, nonempty lists or nonempty integer ranges. Scalars are rejected in both execution modes, including bound analytic outcomes; there is no implicit singleton promotion. `pdf` requires a continuous distribution, and `P` requires `dist[bool]`. To query a singleton, construct `[x]` or `one_of([x])`. Queries evaluate in the current world: they neither aggregate worlds nor replay the model that produced a bound value, even inside `simulate { x }`.

`P`, `cdf`, `pmf` and `pdf` require a fully resolved distribution: any positive missing mass is an error, including tiny tails retained as unresolved by `poisson`, `geometric` or a pruned count distribution. They never silently condition on resolution or return an unlabelled lower bound. Use `report d <= x` (or another distribution comparison) to retain probability bounds. Means, medians and other descriptive summaries retain their convention of describing resolved outcomes. Computed probabilities use compensated summation and total-mass normalization; a shared boundary permits correction of at most eight floating-point epsilons outside `[0,1]`. Larger excursions and nonfinite results fail. Explicit probability conversions never apply that correction.

`pmf` and finite `support` share exact typed outcome identity. `pmf(one_of([1,1.0]), 1)` is 50%, while `P(one_of([1,1.0]) == 1)` is 100% because the comparison uses numeric equality. The distinction also applies to `prob`, real-valued complex numbers and nested containers. PMF in a mixed distribution counts typed point atoms; continuous components contribute zero point mass. An absent typed outcome has probability zero, regardless of its type. CDF uses the language's ordering and numeric comparisons.

Integer ranges give every integer from lo through hi equal weight. Mean, population variance/SD, medians, quantiles, CDF and PMF use direct formulas; `support` alone materializes the integers and obeys collection limits. Medians and ranks preserve bigint precision. Counts up to 2^53 use the same rounded cumulative weights as a finite uniform population; above that, percentile ranks are computed from the exact stored probability and integer count, without converting the count to float. Empty ranges fail. Approximate means/SDs must fit in a finite float, while selected integer endpoints remain exact.

Lists give each element weight 1/n, including repetitions, and do not implicitly draw or flatten distribution elements. `mean` accepts real or complex numbers, or calendar dates; `variance` and `sd` accept only real numbers and use population variance (division by n). Numeric means and variances use finite floating-point arithmetic. Empty lists and incompatible element types are errors.

`median_low` and `median_high` return the endpoints of the median interval of the resolved population. For an even-length list, these are its two middle elements. For a weighted finite distribution, they differ only if the cumulative resolved weight reaches exactly half between distinct outcomes; otherwise both select the same outcome. Repeated list elements retain their weight. Finite-weight comparisons allow four floating-point epsilons relative to the total to absorb summation roundoff, using compensated sums. For continuous mixtures, a half-mass gap in the support gives the endpoints; otherwise both return the usual 50% quantile.

`median` accepts only real numeric or date populations and returns the midpoint of these endpoints. A unique median preserves its type. Two ints with an integral midpoint produce an exact int; otherwise the midpoint is a float, with an error if its magnitude is outside the finite float range. Strings, bools and enums require the explicit low/high functions, even for odd lengths or singleton populations. These ordered medians preserve the selected element's type and exact integer precision. All three medians reject complex elements. Booleans are ordered false before true for low/high medians, quantiles and CDFs.

Date means average day offsets; date midpoint medians average the two endpoints. Results round to the nearest calendar day, with exact half-day ties choosing the earlier day. List date means use exact integer sums; distribution date means use compensated weighted offsets from the earliest date, avoiding dependence on distance from the epoch. Dates never gain a hidden time component. Low/high date medians preserve actual endpoints without rounding.

Finite medians and quantiles sort a separate population using language comparisons, including lexicographic list comparisons with numeric equality across int/float/prob. The typed order used for storage, keys, hashing and world merging is unchanged. Lists and distributions have the same ordering validation, including singleton populations.

`quantile` retains its lower-quantile convention: it selects the smallest finite outcome whose cumulative fraction reaches q, without interpolation. Thus `quantile([1,4], 50%)` is 1 while `median([1,4])` and `median(one_of([1,4]))` are 2.5. At 0% and 100%, finite quantiles select the minimum and maximum retained outcomes with positive weight. Other boundaries use compensated sums of resolved weights without a fixed probability tolerance, so a small positive tail is not treated as zero. Queries and numeric report percentiles share this rule. Weights remain floating-point approximations. `cdf(xs, x)` and `pmf(xs, x)` give the fractions at most x and exactly typed-identical to x, respectively; `support(xs)` gives distinct elements in storage order.

## 9. Reports

`report` is only allowed at the top level of the program. When a world reaches a report, the report records the world's value (and key, with `by`) and its weight.

- The **denominator** of a report is the weight that reached it: the result is conditional on reaching it.
- The **reach** of a report is that weight divided by the program's total weight. When it is below 100%, the output says so ("reached in 1.00% of worlds").
- A `bool` or `dist[bool]` value is reported as the probability that it's true. Numbers are summarized (mean, standard deviation, quantiles); other values are listed with their probabilities. A `dist[T]` value contributes each of its outcomes.

**Median summaries.** Numeric and date summaries and numeric table columns labelled `median` use the same midpoint rule as `median(d)`. Other percentiles retain the lower-quantile convention. A fractional integer midpoint beyond the finite float range is labelled "out of range" rather than replaced with an endpoint.

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

**Infinite supports.** Distributions with infinitely many outcomes (`poisson`, `geometric`) drop outcomes whose probability is below 10⁻¹⁸. The dropped probability is the distribution's missing mass; drawing from it adds (weight × missing mass) to the unresolved weight, because the missing outcomes could go either way. Combining distributions combines their missing mass (for independent draws, 1 − Π(1 − mᵢ)).

**Bounds.** Every observation factor is at most 1, so unresolved weight U can only shrink with further observations. For an event with weight a among the weight Z that reached a report, the true probability therefore lies in

  [ a / (Z + U), (a + U) / (Z + U) ]

where U is all the weight left unresolved by the program. Reports print this range when it is visible at two decimals, and a single number otherwise. Means and quantiles are computed on the resolved part; when unresolved or missing weight is visible, the output says so, but no bound is claimed for them: a rare, very large value can move a mean arbitrarily.

**Numbers.** Weights are binary floating-point numbers with an extended exponent, so repeated observations can't underflow to zero. Sums are subject to rounding (about 10⁻¹⁵ relative). `--fractions` shows the simplest fraction within 10⁻¹³ of a probability, marked "≈": it's a hint for recognizing an answer, not a proof that the answer is exact. The engine's mode is called **enumeration**, not "exact", for these reasons.

## 11. Resource limits

Text limits count UTF-8 bytes, independently of the scalar positions used by the language. `Limits.max_string_bytes` bounds one string (16 MiB by default, 4 MiB in the playground); `max_string_alloc_bytes` bounds cumulative string payloads produced during a run (256 MiB by default, 64 MiB in the playground), shared across sampled workers. The latter is an allocation allowance, not live memory: results are not refunded when dropped, temporary capacity and metadata are excluded, and shared values are not charged again merely for being referenced. Input loading has separate byte/value limits; loaded string sizes are also checked against the runtime's per-string limit before execution.

String scans charge work in proportion to UTF-8 length. Literal construction, concatenation, interpolation, formatting with `str`/`join`, case conversion, trimming, slicing, reversal and character extraction check result sizes and charge payloads before allocation or incremental buffer growth. `chars`, `split` and string iteration also check collection sizes. Large integer ranges can be sliced without allocating their elements. Limit failures produce resource-limit diagnostics.

The host running a program sets upper limits on: worlds per statement (when enumerating; sampled runs don't multiply), outcomes per distribution, total work, loop iterations, call depth, cached results, length of materialized collections, and output. A program's `@max_worlds` and `@max_iterations` can lower these limits, never raise them. Exceeding a limit, or cancelling a run, stops it with an error; so do language errors, except a fault in partial mode (below). The engine never crashes on a program: a crash is a bug in Probl, and it is reported as an internal error.

### Failure modes

A **fault** is a language error that depends on the values a world computes with, not on the program being wrong:

- `DivisionByZero`: division or remainder by zero;
- `DomainError`: a value outside what an operation is defined for: `sqrt(-1)`, `logit(0%)`, a negative count, chances that add up to more than 100%, a distribution's parameter (`normal(0, 0)`, `bernoulli(1.5)`);
- `IndexOutOfBounds`: an index past the end, or a slice that doesn't fit;
- `MissingKey`: a key that isn't in a map, or an item that isn't in a bag;
- `EmptyCollection`: a collection or a bag with nothing in it, where an element is needed;
- `ConversionError`: an explicit conversion that can't represent its value: `prob(1.5)`, an invalid `date`;
- `NumericOverflow`: a result too large to represent: a float that isn't finite, a date out of range.

Other errors are not faults, and always stop the run: wrong types or arities, a declared type that a value fails, effects a callback isn't allowed, impossible evidence, unsupported features, limits, cancellation and internal errors.

A world stops at its first fault that no `catch` takes (below). The **failure mode** decides what happens to the others:

- **total**: the fault stops the run, with its diagnostic, as any other error does.
- **partial**: the world that failed ends there, and the other worlds finish; when sampling, so do the other runs. Nothing takes the failed world's place: no placeholder value, and no run drawn again to replace it.

The default follows the mode the run uses, whatever the program's `@mode` says: total when enumerating, partial when sampling. `@on_error total` or `@on_error partial` chooses for a program, and a host's choice (`--on-error`, `Options::on_error`) wins over the program's.

**What fails together.** A fault fails the world that ran the operation. A call fails in the caller's worlds that took its failing paths: a function that branches can fail in some of them only. An operation on a recipe that can't be built, like `1 / (d6 - 1)`, fails its whole world, and so do a `simulate` block one of whose local paths fails and a collection callback that fails on any element: their results are built whole or not at all.

**Reports.** A report describes the worlds that reached it, as it does without failures. A world that fails after a report keeps its contribution there, and it doesn't reach later reports, like a world that took another branch. Since no evidence can follow a report (section 7), a failure never changes a report already made.

**How much failed.** A failed world's weight is the weight it had when it failed. When no evidence can be applied after its fault (later on its path, in a later round of a loop around the fault, or after the call it failed in), that weight is final and adds to the finished worlds': the result gives the share that failed, reach counts the failed worlds in its denominator, and the evidence includes them. Otherwise, the failed worlds never met evidence that the finished ones did, and the two can't be added up: the failed share and reach are unavailable, and the summary gives only the finished worlds' contribution to the evidence, saying so. The same goes when enumerating if the failed worlds' weight includes another number of densities than the finished worlds' (section 13). When sampling, the result also counts the runs that failed, and names the first in batch order.

**Partial results.** A run where some worlds failed is a partial result. Its summary line says `partial result`, with the share that failed (when enumerating) or the number of runs that failed (when sampling). After the reports, a `failed` section lists each place and kind of fault, with how much failed there:

```text
enumerated · partial result · 16.67% failed

y    mean 0.46 · sd 0.29 · 5% 0.20 · median 0.33 · 95% 1.00 (reached in 83.33% of worlds)

failed
  line 2: division by zero    16.67%
```

`probl run` prints it, then a diagnostic for each place, and exits with status 3 (status 1 when no world finished). The library returns it in the run's error, as `Error::partial`, so a caller that only handles success doesn't mistake it for a complete answer. A partial result is reproducible as any other: the same program, data, date, seed and engine version give the same output and the same failures on any number of threads.

### Catching faults

`try { body } catch F { … } catch { … }` is an expression. In each world it runs the body, and its value is the body's. A world where the body faults goes on in the first `catch`, in source order, that names the fault, or that names none: a `catch` without a name takes every fault. The `try`'s value is then the catch's.

```probl
let x ~ d6 - 1
let y = try { 1 / x } catch DivisionByZero { 0 }
report y        # mean 137/360: the worlds where x is 0 give 0
```

- **What the caught world keeps.** It goes on from its fault: the statements before it stay done, draws made before it aren't made again, and its weight includes the evidence applied before it. The statement that faulted has no effect: an assignment whose value faulted doesn't happen. Variables declared in the body aren't visible in the catch; the variables around the `try` are, with the values the body gave them.
- **Where faults come from.** Anywhere in the body, including the functions it calls, at any depth. A call's paths that fault reach the caller's `try`, which catches them in the caller's world as it was at the call, with those paths' weight; its other paths go on as usual. `simulate` blocks and collection callbacks fault whole, as in partial mode, and a `try` inside them catches locally.
- **What isn't caught.** Only faults: a `catch` without a name takes every fault, not other errors. A fault that none of a `try`'s catches takes goes on to the `try` around it, then up the calls, then to the failure mode. A fault in a catch is for the `try`s around this one, not for its other catches.
- **Caught faults aren't failures.** A caught world finishes as any other world: it counts in no failed share, and doesn't make a result partial, in either failure mode.
- **Evidence.** A catch can observe as any code can: `catch DivisionByZero { observe false; 0 }` leaves out, as evidence, the worlds where the body divided by zero. A catch can run after anything in its body, so the rule that no evidence follows a report (section 7) treats it as coming after the body.
- **Names.** Naming a fault that doesn't exist is a compile error, and a catch that an earlier one already covers is a warning. A catch can't read the fault's message, and there's no `finally`.

## 12. How this document is checked

The crate `probl-oracle` is a second implementation of sections 1–9, for the discrete part of the language whose loops end. It shares only the parser with the engine and is deliberately naive: it follows every path on its own (no merging, no liveness analysis, no memoization), evaluates operands left to right one world at a time, and computes weights as exact fractions.

Its tests generate thousands of programs from that part of the language, run each one through both implementations, and require the same evidence, the same unnormalized weight for every value at every report, or an error from both. The engine runs with merging and memoization on and off. A hand-written corpus covers each rule above, and a fuzzing test feeds mutated programs to the parser, the compiler and the engine under small limits, which must never crash (section 11).

Sampling (section 14) is checked against enumeration on the same generated programs: every estimate must be within six standard errors of the exact value, and the standard errors must be calibrated (across thousands of estimates, the mean squared error in standard errors is close to 1). The samplers of section 13 are checked against their CDFs with Kolmogorov–Smirnov and chi-square tests.

## 13. Continuous distributions

`normal(mean, sd)`, `lognormal(mu, sigma)`, `uniform(lo, hi)`, `beta(a, b)`, `gamma(shape, scale)`, `exponential(rate)`, `triangular(lo, mode, hi)` and `pert(lo, mode, hi)` are continuous distributions. Two more describe an estimate by its 90% interval:

- `a to b` is the lognormal whose 5% and 95% quantiles are `a` and `b`. Both must be positive, as in Squiggle.
- `normal_range(lo, hi)` is the normal whose 5% and 95% quantiles are `lo` and `hi`, for quantities that can be negative.

A continuous distribution is a `dist[float]`, and a recipe like any other distribution (section 2). What can be done with one:

- **Draw from it** with `~`. Sampling produces a concrete float (section 14). Enumeration retains an analytic float with a unique draw identity, subject to the operations below.
- **Compare it with a number.** `normal(0, 1) > 1.96` is a `dist[bool]`, true with probability 1 − F(1.96), where F is the distribution's CDF; this works in both modes. `==` is never true.
- **Ask about it.** `mean`, `sd`, `variance`, `median`, `quantile`, `cdf` and `pdf` use the formulas. Reporting the recipe directly in enumeration displays its numeric summary.
- **Choose among them.** A choice with continuous options, such as `one_of([normal(0, 1), 5])` or `chance { 35% => 1 to 3, else => 0 }` used as a value, is a mixture: drawing from it chooses an option with its probability, then draws from that option.
- **Use it as evidence.** `observe v from D` with a continuous `D`, or a mixture of continuous distributions, multiplies the weight by D's density at `v`, which can be more than 1. Enumeration computes it as a logarithm, so that a density far into a tail doesn't round to 0, and the summary gives the evidence as a logarithm too: `log evidence -1.1380`. A density is per unit of what it observes, so when enumerating, weights are only compared and added up when they include as many densities, and some programs are rejected as unsupported: one where worlds observe different numbers of continuous values (in a branch, or a loop that runs a varying number of times), one that observes a continuous value inside a function, and one with unresolved weight, which could have any density at the value. In partial mode, the failed weight counts in the evidence and in reach only when it includes as many densities as the finished worlds' (section 11). A loop whose body observes a continuous value is unrolled, not solved as a Markov chain, whose transitions are probabilities.

Continuous statistical queries return only finite results. A mathematically unbounded 0%/100% quantile, a singular density, numerical failure or overflow produces an error; there is no extended-real infinity value from these queries. Densities may exceed one. Means and spreads use centered/scaled formulas, and SD is computed directly rather than by overflowing a variance first. Underflow and finite rounding remain possible. Numeric report summaries label unrepresentable fields "out of range".

Arithmetic on an undrawn continuous recipe (`normal(0, 1) * 2`) and comparisons between continuous recipes remain unsupported. Bind an outcome first.

### Analytic outcomes in enumeration

```probl
let x ~ uniform(0, 2)
let y = x + 1
observe x > 1
report y                 # uniform(2, 3): mean 2.5, sd ≈0.289
report x + x             # uniform(2, 4), using the same x twice
report x - x             # 0.0
report y > 2.5           # 50%
```

The observation has 50% evidence. `typeof x` remains `"float"`, and `typeof (x > 1)` is `"bool"`. The internal analytic representation does not turn a bound outcome back into a distribution recipe. Assigning or drawing an already bound scalar (`let alias ~ x`) retains its identity. Separate `~` operations on recipes create independent identities, including draws inside separate function calls.

Supported arithmetic is affine in a single continuous draw: addition/subtraction of settled numbers, multiplication/division by settled numbers, negation, and addition/subtraction of expressions derived from the same draw. Multiplication by zero and cancellation produce ordinary floats. Division by zero is an error. Finite drawn values can supply the coefficients, so enumeration can produce mixtures of affine marginals.

`abs`, `min`, `max`, `clamp` with settled bounds, and `minimum`/`maximum` of a list stay exact too: their result is affine on each piece of the draw's range, as `if x < 0 { -x } else { x }` is, without splitting the world. A piece can be constant. `max(x, 0)` is 0 with the probability that `x` is negative, so `max(x, 0) == 0` is an event with that probability, and a report's CDF and quantiles include the jump. The result is a float, even where a bound is an int. Arithmetic combines the pieces of one draw, as in `abs(x) - x`; a product or quotient needs a factor or divisor that is constant on each piece. An outcome has at most 100,000 pieces: folding one again and again, as `y = abs(2 * y - 1)` in a loop does, reaches that limit.

Comparisons against numbers, or between affine expressions of the same draw, produce boolean events. `observe` restricts the underlying draw, and `if`/`while` split and restrict it in each branch. Conditions on one draw can use `and`, `or`, `not`, and boolean equality; disjoint accepted intervals are retained. A stored event refers to that same draw, so observing it twice applies its evidence once. Equality to a single number has probability zero where a continuous outcome isn't constant; identity comparisons such as `x == x` are true. A point likelihood `observe v from D` with a continuous `D` weighs finite worlds by densities (section 13); `v` and D's parameters can't be analytic outcomes, except as the exact updates below.

**Exact updates.** A conjugate observation of a draw updates the draw's distribution in its world, as the sampler's delayed draws do (section 14): `observe k from binomial(n, p)`, `observe b from bernoulli(p)` and `score p` for a `p` drawn from a beta, `observe k from poisson(r)` for an `r` drawn from a gamma, and `observe y from normal(mu, sd)` for a `mu` drawn from a normal, where the parameter is the variable itself and the rest are settled. A uniform probability within [0, 1] counts as a beta(1, 1) on its range, and an exponential rate as a gamma with shape 1. The draw keeps its identity: aliases, stored events and closures see its posterior. A draw already restricted, by `observe p > 0.5` say, stays restricted, and the evidence includes the ratio of the posterior's and the distribution's probability there before the update, so a restriction and an update give the same result in either order. `if p`, `~bernoulli(p)` and `~binomial(n, p)` for a `p` drawn from a beta split the world by outcome, each with its own posterior: `if p` takes its first branch with weight E[p] and `p` updated as by a success, so two branches on the same `p` stay correlated.

Restrictions follow aliases in lists, records, map values, and closures, and flow back from ordinary function calls even when the function returns a plain value. Collection callbacks can perform affine calculations but retain their existing effect restrictions. Built-ins that move values without looking at them keep outcomes as they are: `get` with a settled index or key, `slice`, `reverse`, `push`, `pop`, `insert`, `remove`, `zip`, `enumerate`, `keys`, `values` and `len`. `sum` and `mean` of a list of outcomes of one draw are affine in it: `mean([x, x + 1])` is `x + 0.5`. Reports combine conditional continuous marginals and numeric point masses using their weights, and display mean, standard deviation, and quantiles; a distribution of outcomes, such as `max(x, d6)`, reports as their mixture. Grouping by a settled discrete key works, and so does grouping by an event of a draw: `report x by x > 1` has two groups, each with its part of `x`. The CDF of a bound outcome can be reported as an event: `report y <= threshold`. These results use floating-point formulas and numerical CDF inversion, with no Monte Carlo error; they are not symbolic exact real arithmetic.

This implementation deliberately rejects unsupported uses with a sampling diagnostic: nonlinear arithmetic, expressions combining independent continuous draws, continuous distribution parameters or probability conditions derived from a draw (other than the exact updates above, so `2 * mu` as a normal's mean or `p` as a weight in `chance` still needs sampling), analytic collection keys/indices, built-ins that compare or order outcomes (`sort`, `contains`, `highest`, …), grouping by a continuous outcome (`report true by x` would have infinitely many groups), and reports of whole aggregates containing analytic values (report their fields separately). Reports cannot mix continuous marginals with nonnumeric outcomes. Built-ins that need concrete scalar outcomes, including conversions and text formatting, require sampling. `mean(x)` and `P(x > 1)` instead produce ordinary type errors in both modes: `x` is a scalar and `x > 1` is a fact in each world, not an explicit population. Use reports to summarize across worlds; distribution queries such as `mean(uniform(0, 2))` retain their existing meaning.

Continuous draws inside `simulate`, and capturing analytic outcomes into it, remain unsupported. A reusable joint distribution recipe needs a separate representation so that subsequent draws receive fresh identities while correlations within each draw survive. Ordinary functions can already return analytic outcomes. Cyclic loops involving analytic state use bounded unrolling and the usual iteration/work limits rather than the finite-state Markov solver.

## 14. Sampling

`@mode sample(runs: n, seed: s)` estimates the same model as sections 1–13 by following `n` random paths through the program, called **runs**, instead of every path.

- **A run is a single world.** Where enumeration would split a world (probabilistic conditions, `chance`, `~`, taking a card, calling a function), a run takes one branch, chosen with that branch's probability, and its weight doesn't change. Boolean conditions follow the already-drawn outcome.
- **Weights come from evidence.** `observe` multiplies a run's weight as in section 7 (likelihood weighting). A run that is ruled out has weight zero.
- **Runs are independent.** They don't merge, and calls aren't memoized, so every call makes its own choices. A function may call itself with the same arguments: each call chooses its own path. Unbounded loops aren't cut short; the iteration limit still applies.
- **`simulate` is enumerated.** Inside each run, a `simulate` block is computed exactly, by enumeration, as in section 8, so its result has no sampling error. A block outside the supported local inference subset (including a continuous draw or a captured analytic outcome) is an error, even when sampling: estimates inside estimates aren't supported yet.
- **Reproducible.** The same program, input data, execution-date snapshot, seed and version of Probl give the same output on any machine, with any number of threads. Runs go in batches of 1,000, each with a random stream of its own, derived from the seed and the batch's number. Batches may run at the same time, but they're combined in order: the estimates, what `print` shows and the first error are those of running them one after another. Only whether a run reaches a host's limit on work, which the threads share, can depend on timing.
- The missing mass of infinite discrete distributions (below 10⁻¹⁸, section 10) is ignored.

**Estimates.** Let the runs end with weights w₁…wₙ. A run's weight when it reaches a report is its final weight, since no observation may follow a report (section 7). A report's estimates are averages over the runs, weighted and normalized:

- A **probability** is estimated as p̂ = Σ wᵢ aᵢ / Σ wᵢ bᵢ, where bᵢ is the number of times run i reached the report (0 or 1, except for per-visit reports), and aᵢ adds up, over those times, the probability that the reported fact was true: 1 or 0 for a fact, its true-outcome weight for a `dist[bool]`. A reported finite distribution isn't drawn: each of its outcomes counts with its probability. (A continuous one can't list its outcomes, so it's drawn once.) The probabilities of other values are estimated the same way.
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
  | `beta(α, β)` | `observe v from bernoulli(x)` | `beta(α + 1, β)` if v, `beta(α, β + 1)` if not | α / (α + β), or β / (α + β) |
  | `gamma(s, θ)` | `observe k from poisson(x)` | `gamma(s + k, θ / (1 + θ))` | Γ(s + k) / (Γ(s) k!) · (θ / (1 + θ))ᵏ (1 + θ)⁻ˢ |
  | `normal(μ, σ)` | `observe y from normal(x, τ)` | `normal(μ + g(y − μ), στ / √(σ² + τ²))`, where g = σ² / (σ² + τ²) | the normal density of `y` with mean μ and standard deviation √(σ² + τ²) |

  `x` must be the parameter, written as itself or with an explicit `prob(x)` for beta likelihoods, and appear nowhere else in the observation. The observation's other parts must be plain values; they're evaluated and checked as when `x` is drawn, with the same errors. A value the distribution can't produce, such as a count above `n`, makes the run impossible.
- **Drawing.** Any other statement that reads a delayed `x` draws it first, from its current distribution, and `x` is an ordinary number from then on. That includes every expression, a call or closure that captures `x`, and an observation in another form, or of a family that doesn't pair. A type check draws `x` only if some of its distribution's values could fail it, so `let p: float ~ beta(2, 3)` stays delayed. A beta draw is a float and converts at a probability-consuming boundary. A `prob` annotation currently materializes and converts the draw at that annotation; leave the draw unannotated to retain exact updates before its first ordinary use. A delayed variable that's assigned, or never read again, is never drawn.
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
- **A general method for models that aren't conjugate**, such as MCMC: its target, its moves and its report estimators ([inference guide](inference.md), section 2).
- **Nested estimates** (D4): `simulate` blocks that must be sampled, and how their error affects decisions.
- **General continuous composition**, beyond the supported affine single-draw arithmetic and threshold conditioning in §13; nonlinear transforms and combinations of independent continuous draws need further representation and inference contracts.
- **Reports that update with later evidence** (filtering and smoothing).
- **A fault's details in a catch**, such as its message, and a `finally` block: see the [error-handling proposal](design/error-handling.md).
