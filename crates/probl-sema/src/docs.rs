//! Documentation of the built-in functions and the keywords: what an editor
//! shows on hover and when completing, and the playground's reference.

use crate::{Builtin, Constant};

/// How something is used, and what it does, in a sentence or two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Doc {
    /// Like `binomial(n: int, p: prob) -> dist[int]`.
    pub signature: &'static str,
    pub summary: &'static str,
}

const fn doc(signature: &'static str, summary: &'static str) -> Option<Doc> {
    Some(Doc { signature, summary })
}

/// The group a public built-in belongs to, as the reference lists them.
pub fn category(b: Builtin) -> &'static str {
    use Builtin as B;
    match b {
        B::Min
        | B::Max
        | B::Abs
        | B::Floor
        | B::Ceil
        | B::Trunc
        | B::Round
        | B::Sqrt
        | B::Cbrt
        | B::Exp
        | B::Exp2
        | B::Ln
        | B::Log10
        | B::Log2
        | B::Log1p
        | B::Expm1
        | B::Sin
        | B::Cos
        | B::Tan
        | B::Asin
        | B::Acos
        | B::Atan
        | B::Atan2
        | B::Hypot
        | B::Sinh
        | B::Cosh
        | B::Tanh
        | B::Asinh
        | B::Acosh
        | B::Atanh
        | B::BitLength
        | B::BitAnd
        | B::BitOr
        | B::BitXor
        | B::BitNot
        | B::BitCount
        | B::ILog2
        | B::Choose
        | B::Factorial
        | B::Gcd
        | B::Lcm
        | B::EulerPhi
        | B::LnGamma
        | B::Erf
        | B::Erfc
        | B::Complex
        | B::Real
        | B::Imag
        | B::Conj
        | B::Abs2
        | B::Arg
        | B::Cis
        | B::Clamp => "Math",
        B::Str
        | B::Upper
        | B::Lower
        | B::Trim
        | B::TrimStart
        | B::TrimEnd
        | B::StartsWith
        | B::EndsWith
        | B::Chars
        | B::Split
        | B::Join
        | B::Print => "Text",
        B::Len
        | B::Slice
        | B::Sum
        | B::Count
        | B::Map
        | B::Filter
        | B::Reduce
        | B::Sort
        | B::SortDesc
        | B::Reverse
        | B::Keys
        | B::Values
        | B::Get
        | B::Contains
        | B::Highest
        | B::Lowest
        | B::Enumerate
        | B::Zip
        | B::Push
        | B::Insert
        | B::Remove
        | B::Pop
        | B::Take => "Collections",
        B::Bernoulli
        | B::OneOf
        | B::Binomial
        | B::Poisson
        | B::Geometric
        | B::Roll
        | B::Bag
        | B::Normal
        | B::Lognormal
        | B::Uniform
        | B::Beta
        | B::Gamma
        | B::Exponential
        | B::Triangular
        | B::Pert
        | B::NormalRange
        | B::Mixture
        | B::Truncate
        | B::Bins
        | B::To => "Distributions",
        B::P
        | B::Mean
        | B::Sd
        | B::Variance
        | B::Median
        | B::Quantile
        | B::Support
        | B::Cdf
        | B::Pmf
        | B::Pdf
        | B::Odds
        | B::Logit
        | B::InvLogit
        | B::Prob => "Questions about distributions",
        B::Date
        | B::Days
        | B::Weeks
        | B::AddWorkdays
        | B::IsWorkday
        | B::AddMonths
        | B::AddYears
        | B::StartOfMonth
        | B::EndOfMonth
        | B::Year
        | B::Month
        | B::Day
        | B::Weekday => "Dates",
        B::RunDate
        | B::IterItems
        | B::RepeatCount
        | B::IsFalse
        | B::IsTrue
        | B::IsListOfLen
        | B::Settled
        | B::Last
        | B::DropLast
        | B::BooleanLaw
        | B::ScoreLaw
        | B::Typeof => "Internal",
    }
}

/// The documentation of a built-in function; `None` for the internal helpers
/// programs can't call.
pub fn builtin(b: Builtin) -> Option<Doc> {
    use Builtin as B;
    match b {
        // Math
        B::Min => doc(
            "min(a, b, …) or min(xs)",
            "The smallest of its arguments, or of a list or range's items.",
        ),
        B::Max => doc(
            "max(a, b, …) or max(xs)",
            "The largest of its arguments, or of a list or range's items.",
        ),
        B::Abs => doc(
            "abs(x)",
            "The absolute value. An int stays an int; a complex number gives its magnitude as a float.",
        ),
        B::Floor => doc("floor(x) -> int", "`x` rounded down, as an int."),
        B::Ceil => doc("ceil(x) -> int", "`x` rounded up, as an int."),
        B::Trunc => doc(
            "trunc(x) -> int",
            "Drop the fractional part, rounding toward zero: `trunc(-1.9)` is -1. Int inputs stay exact. Non-finite values and results outside the int range are errors.",
        ),
        B::Round => doc(
            "round(x, digits?: int)",
            "Round to the nearest value, halves away from zero. With no `digits`, returns an int. With `digits`, rounds that many decimal places: `round(1.234, 2)` is 1.23; `round(1234, -2)` is 1200. Int inputs stay exact ints (integer size and work limits apply); other numbers return floats. Float scaling is approximate near halfway cases. This changes the value, not its display format.",
        ),
        B::Sqrt => doc(
            "sqrt(x) -> float or complex",
            "The square root. Real inputs must be nonnegative. Complex inputs give the principal root with nonnegative real part: `sqrt(complex(-1))` is `complex(0, 1)`.",
        ),
        B::Cbrt => doc(
            "cbrt(x) -> float or complex",
            "The cube root. Real inputs give the real root: `cbrt(-8)` is -2. Complex inputs give the principal root with phase `arg(x)/3`: `cbrt(complex(-8))` is approximately `complex(1, sqrt(3))`.",
        ),
        B::Exp => doc(
            "exp(x) -> float or complex",
            "e to the power `x`. At exactly 1, the result equals the built-in constant `e` exactly. Complex inputs give `exp(real(x)) * cis(imag(x))`; overflow is an error.",
        ),
        B::Exp2 => doc(
            "exp2(x) -> float or complex",
            "2 to the power `x`, including complex inputs. Overflow is an error; very small results may underflow to zero.",
        ),
        B::Ln => doc(
            "ln(x) -> float or complex",
            "The natural logarithm. Real inputs must be positive. Nonzero complex inputs give the principal value `complex(ln(abs(x)), arg(x))`, with phase +pi on the negative real axis. Other branches are explicit: `ln(x) + complex(0, 2*pi*k)` for integer `k`.",
        ),
        B::Log10 => doc(
            "log10(x) -> float or complex",
            "The base-10 logarithm. Real inputs must be positive; nonzero complex inputs give the principal value `ln(x)/ln(10)`.",
        ),
        B::Log2 => doc(
            "log2(x) -> float or complex",
            "The approximate base-2 logarithm. Real inputs must be positive; nonzero complex inputs give the principal value `ln(x)/ln(2)`. For the exact floor on a positive integer, use `ilog2(n)`.",
        ),
        B::Log1p => doc(
            "log1p(x) -> float or complex",
            "`ln(1 + x)`, preserving tiny real or complex inputs. Real inputs must be above −1. Complex inputs use the principal logarithm; complex −1 is an error.",
        ),
        B::Expm1 => doc(
            "expm1(x) -> float or complex",
            "`exp(x) − 1`, preserving tiny real or complex inputs. For a Poisson process with constant `rate`, the chance of at least one event in time `t` is `-expm1(-rate * t)`.",
        ),
        B::Sin => doc(
            "sin(x) -> float or complex",
            "The sine of a real angle in radians, or its complex extension.",
        ),
        B::Cos => doc(
            "cos(x) -> float or complex",
            "The cosine of a real angle in radians, or its complex extension.",
        ),
        B::Tan => doc(
            "tan(x) -> float or complex",
            "The tangent of a real angle in radians, or its complex extension.",
        ),
        B::Asin => doc(
            "asin(x) -> float or complex",
            "The inverse sine in radians. Real inputs must be from −1 to 1. Complex inputs give the principal value, with real part from −pi/2 to pi/2 and cuts on the real axis outside [−1, 1]. Exact cut values use the upper side.",
        ),
        B::Acos => doc(
            "acos(x) -> float or complex",
            "The inverse cosine in radians. Real inputs must be from −1 to 1. Complex inputs give the principal value, with real part from 0 to pi and cuts on the real axis outside [−1, 1]. Exact cut values use the upper side.",
        ),
        B::Atan => doc(
            "atan(x) -> float or complex",
            "The inverse tangent in radians. Complex inputs give the principal value, with cuts on the imaginary axis beyond ±i; ±i are errors. Exact cut values use the right side.",
        ),
        B::Atan2 => doc(
            "atan2(y, x) -> float",
            "The angle from the x-axis to the point (`x`, `y`), in radians, from −pi to pi. Unlike `atan(y / x)`, it knows the quadrant, and `x` can be 0. Signed zeros are treated alike; `atan2(0, 0)` is 0 by convention.",
        ),
        B::Hypot => doc(
            "hypot(x, y) -> float",
            "`sqrt(x^2 + y^2)`, the distance from the origin to (`x`, `y`), without overflowing on the way.",
        ),
        B::Sinh => doc(
            "sinh(x) -> float or complex",
            "The hyperbolic sine, including complex inputs.",
        ),
        B::Cosh => doc(
            "cosh(x) -> float or complex",
            "The hyperbolic cosine, including complex inputs.",
        ),
        B::Tanh => doc(
            "tanh(x) -> float or complex",
            "The hyperbolic tangent, including complex inputs. Real results lie between −1 and 1.",
        ),
        B::Asinh => doc(
            "asinh(x) -> float or complex",
            "The inverse hyperbolic sine. Complex inputs give the principal value, with cuts on the imaginary axis beyond ±i. Exact cut values use the right side.",
        ),
        B::Acosh => doc(
            "acosh(x) -> float or complex",
            "The inverse hyperbolic cosine. Real inputs must be at least 1. Complex inputs give the principal value with nonnegative real part and a cut on the real axis below 1. Exact cut values use the upper side.",
        ),
        B::Atanh => doc(
            "atanh(x) -> float or complex",
            "The inverse hyperbolic tangent. Real inputs must be strictly between −1 and 1. Complex inputs give the principal value, with cuts on the real axis outside [−1, 1]; ±1 are errors. Exact cut values use the upper side.",
        ),
        B::BitLength => doc(
            "bit_length(n: int) -> int",
            "The number of binary digits in the magnitude of `n`, ignoring its sign and leading zeros. `bit_length(0)` is 0; `bit_length(-7)` is 3. Exact and constant-time for arbitrary-precision integers.",
        ),
        B::BitAnd => doc(
            "bit_and(a: int, b: int) -> int",
            "The bitwise AND of two integers, using infinite two's-complement sign extension. For example, bit_and(-1, 0xff) is 255. Integer size and work limits apply.",
        ),
        B::BitOr => doc(
            "bit_or(a: int, b: int) -> int",
            "The bitwise OR of two integers, using infinite two's-complement sign extension. For example, bit_or(0b1010, 0b0101) is 15. Integer size and work limits apply.",
        ),
        B::BitXor => doc(
            "bit_xor(a: int, b: int) -> int",
            "The bitwise exclusive OR of two integers, using infinite two's-complement sign extension. For example, bit_xor(0b1010, 0b1100) is 6. Integer size and work limits apply.",
        ),
        B::BitNot => doc(
            "bit_not(n: int) -> int",
            "The bitwise complement of an integer, equal to -n - 1. There is no implicit word size: bit_not(0) is -1. Use a mask for a fixed-width result, such as bit_and(bit_not(n), 0xff) for eight bits. Integer size and work limits apply.",
        ),
        B::BitCount => doc(
            "bit_count(n: int) -> int",
            "The number of set bits in an integer's magnitude, ignoring its sign: bit_count(0) is 0 and bit_count(-0b1011) is 3. Works directly on bigints, with work proportional to their bit length.",
        ),
        B::ILog2 => doc(
            "ilog2(n: int) -> int",
            "The exact floor of log base 2 of a positive integer: `ilog2(31)` is 4, and `ilog2(32)` is 5. Zero and negatives are errors. Uses integer bits, with no float conversion; constant-time even for bigints.",
        ),
        B::Choose => doc(
            "choose(n: int, k: int) -> int",
            "The number of ways to choose `k` items from `n`, without order or replacement. Both must be nonnegative; `k > n` gives 0. The result is an arbitrary-precision int; size and work limits apply.",
        ),
        B::Factorial => doc(
            "factorial(n: int) -> int",
            "The product of the integers from 1 to `n`; `factorial(0)` is 1. `n` must be nonnegative. The result is an arbitrary-precision int, subject to size and work limits. Use `ln_gamma(n + 1)` for its logarithm.",
        ),
        B::Gcd => doc(
            "gcd(a: int, b: int) -> int",
            "The nonnegative greatest common divisor. Signs are ignored, and `gcd(0, 0)` is 0. A result too large for an int is an error.",
        ),
        B::Lcm => doc(
            "lcm(a: int, b: int) -> int",
            "The nonnegative least common multiple. Signs are ignored; if either argument is 0, the result is 0. Integer size and work limits apply.",
        ),
        B::EulerPhi => doc(
            "euler_phi(n: int) -> int",
            "Euler's totient: how many integers from 1 through `n` are coprime to `n`. `n` must be positive; `euler_phi(1)` is 1. Factoring large inputs can reach the run's work limit.",
        ),
        B::LnGamma => doc(
            "ln_gamma(x) -> float",
            "The natural logarithm of the gamma function, for positive `x`. For integer `n >= 0`, `ln_gamma(n + 1)` is `ln(n!)`, without forming the factorial. `gamma(shape, scale)` remains the distribution constructor.",
        ),
        B::Erf => doc(
            "erf(x) -> float",
            "The error function: `2 / sqrt(pi)` times the integral of `exp(-t^2)` from 0 to `x`. For the standard normal, `cdf(normal(0, 1), x)` is `(1 + erf(x / sqrt(2))) / 2`.",
        ),
        B::Erfc => doc(
            "erfc(x) -> float",
            "The complementary error function, `1 - erf(x)`, computed directly to preserve small tails. For a standard normal, the probability above `x` is `erfc(x / sqrt(2)) / 2`. `x` must be finite; very small results may underflow to zero.",
        ),
        B::Complex => doc(
            "complex(re, im?) -> complex",
            "Construct a complex number from finite real components. The imaginary part defaults to 0; `complex(z)` also accepts an existing complex number. Complex values are ordinary data, never probabilities or quantum states.",
        ),
        B::Real => doc("real(z) -> float", "The real component of a real or complex number."),
        B::Imag => doc("imag(z) -> float", "The imaginary component; 0 for a real number."),
        B::Conj => doc(
            "conj(z) -> complex",
            "The complex conjugate: negate the imaginary component.",
        ),
        B::Abs2 => doc(
            "abs2(z) -> float",
            "The squared magnitude, `real(z)^2 + imag(z)^2`. This is a nonnegative number, not an implicit probability; overflow is an error.",
        ),
        B::Arg => doc(
            "arg(z) -> float",
            "The phase angle in radians, from −pi to pi. Signed zeros are treated alike: a negative real number has phase pi, and zero has phase 0 by convention.",
        ),
        B::Cis => doc(
            "cis(theta) -> complex",
            "`complex(cos(theta), sin(theta))`, a unit-magnitude phase (up to floating-point rounding). `theta` is a finite real angle in radians.",
        ),
        B::Clamp => doc("clamp(x, lo, hi)", "`x`, kept between `lo` and `hi`."),
        // Text
        B::Str => doc("str(x) -> str", "`x` as text, as `print` shows it."),
        B::Upper => doc(
            "upper(s: str) -> str",
            "Unicode uppercase, independent of locale. May change length: upper(\"ß\") is \"SS\".",
        ),
        B::Lower => doc(
            "lower(s: str) -> str",
            "Unicode lowercase, independent of locale. May change length; this is not case folding or normalization.",
        ),
        B::Trim => doc(
            "trim(s: str, chars: str?) -> str",
            "Remove leading and trailing Unicode whitespace. With chars, remove any of its Unicode scalars instead: \"abbacacb\".trim(\"ab\") is \"cac\". An empty set leaves the text unchanged. Returns a new value.",
        ),
        B::TrimStart => doc(
            "trim_start(s: str, chars: str?) -> str",
            "Remove leading Unicode whitespace, or any leading scalars in chars when supplied. chars is a set, not a literal prefix; an empty set leaves the text unchanged.",
        ),
        B::TrimEnd => doc(
            "trim_end(s: str, chars: str?) -> str",
            "Remove trailing Unicode whitespace, or any trailing scalars in chars when supplied. chars is a set, not a literal suffix; an empty set leaves the text unchanged.",
        ),
        B::StartsWith => doc(
            "starts_with(s: str, prefix: str) -> bool",
            "Whether the text starts with the exact prefix, without normalization or case conversion. The empty prefix always matches.",
        ),
        B::EndsWith => doc(
            "ends_with(s: str, suffix: str) -> bool",
            "Whether the text ends with the exact suffix, without normalization or case conversion. The empty suffix always matches.",
        ),
        B::Chars => doc(
            "chars(s: str) -> list[str]",
            "One string per Unicode scalar value, in order. Combining marks and emoji components may be separate elements. chars(\"\") is []; equivalent to split(s, \"\").",
        ),
        B::Split => doc(
            "split(s: str, sep: str) -> list[str]",
            "The text cut at each literal sep, preserving empty fields. With \"\" as sep, one string per Unicode scalar value (like chars); splitting empty text that way gives [].",
        ),
        B::Join => doc(
            "join(xs: list, sep: str) -> str",
            "The items as text, with `sep` between them.",
        ),
        B::Print => doc(
            "print(a, b, …)",
            "Debug output: prints its arguments, separated by spaces, once for each world that runs it, with the world's weight in brackets when it isn't 100%. A function that prints runs every time it's called.",
        ),
        // Collections
        B::Len => doc(
            "len(xs) -> int",
            "How many items a list, map, range or bag has (repeats count), or how many Unicode scalar values a string has. String length is not a byte count or a count of grapheme clusters.",
        ),
        B::Slice => doc(
            "slice(xs, start: int, end: int?)",
            "A sequence from start (inclusive) to end (exclusive, defaults to length). Requires 0 <= start <= end <= length. Strings count Unicode scalar values and return strings; lists return lists; ranges stay compact ranges. Equal bounds give an empty result. Does not mutate xs.",
        ),
        B::Sum => doc("sum(xs: list)", "The items added up; 0 for an empty list."),
        B::Count => doc(
            "count(xs) -> int or count(xs, test) -> int",
            "How many items there are, or how many pass `test`: `count(rolls, r -> r == 6)`. The test cannot draw, observe or branch on uncertainty outside a local simulate scope, in either execution mode.",
        ),
        B::Map => doc(
            "map(xs, f) -> list",
            "f applied to each element of a list, range or string. String elements are one-scalar strings; the result is always a list. The function cannot draw, observe or branch on uncertainty outside a local simulate scope, in either execution mode.",
        ),
        B::Filter => doc(
            "filter(xs, test) -> list",
            "The elements of a list, range or string for which test is true. String elements are one-scalar strings; the result is always a list. Use join(result, \"\") to rebuild text. The test cannot draw, observe or branch on uncertainty outside a local simulate scope, in either execution mode.",
        ),
        B::Reduce => doc(
            "reduce(xs, start, f)",
            "The elements of a list, range or string combined one by one, starting from start. String elements are one-scalar strings. The function cannot draw, observe or branch on uncertainty outside a local simulate scope, in either execution mode.",
        ),
        B::Sort => doc(
            "sort(xs) -> list",
            "The elements of a list, range or string, smallest first. Text uses Unicode scalar order, not locale collation.",
        ),
        B::SortDesc => doc(
            "sort_desc(xs) -> list",
            "The elements of a list, range or string, largest first.",
        ),
        B::Reverse => doc(
            "reverse(xs)",
            "A list or range reversed into a list, or a string with its Unicode scalars reversed into a string. May separate combining marks and emoji components.",
        ),
        B::Keys => doc("keys(m) -> list", "A map's keys, or a bag's distinct items, in order."),
        B::Values => doc("values(m: map) -> list", "A map's values, in the order of their keys."),
        B::Get => doc(
            "get(m, key) or get(m, key, default)",
            "A map's value for `key`, a list's item at index `key` (from 0), or how many times `key` is in a bag. Without `default`, a missing key is an error.",
        ),
        B::Contains => doc(
            "contains(xs, x) -> bool",
            "Whether a list, range or bag holds `x`, a map has the key `x`, or a string contains the text `x`. `x in xs` is the same.",
        ),
        B::Highest => doc(
            "highest(xs: list) or highest(xs: list, n: int) -> list",
            "The largest item, or the `n` largest, largest first: `roll(4, d6).highest(3)`.",
        ),
        B::Lowest => doc(
            "lowest(xs: list) or lowest(xs: list, n: int) -> list",
            "The smallest item, or the `n` smallest, smallest first.",
        ),
        B::Enumerate => doc(
            "enumerate(xs) -> list",
            "[index, item] pairs for a list, range or string, counting from 0. String positions and elements count Unicode scalar values.",
        ),
        B::Zip => doc(
            "zip(xs, ys) -> list",
            "[x, y] pairs at matching positions of two lists, ranges or strings, as many as the shorter sequence has. String elements are one-scalar strings.",
        ),
        B::Push => doc("xs.push(x)", "Adds `x` at the end of the list variable `xs`."),
        B::Insert => doc(
            "xs.insert(i, x)",
            "Inserts `x` at index `i` of the list variable `xs`, or sets the key `i` to `x` in a map.",
        ),
        B::Remove => doc(
            "xs.remove(i)",
            "Removes the item at index `i` of a list, the key `i` of a map, or one `i` from a bag.",
        ),
        B::Pop => doc(
            "let last = xs.pop()",
            "Removes the last item of the list variable `xs`, and gives it.",
        ),
        B::Take => doc(
            "let card ~ deck.take()",
            "Draws an item from the bag variable `deck` without replacement: one world per distinct item, weighted by how many there are.",
        ),
        // Distributions
        B::Bernoulli => doc(
            "bernoulli(p: prob) -> dist[bool]",
            "`true` with probability `p`: `let rain ~ bernoulli(30%)` is a fact, true in 30% of the worlds.",
        ),
        B::OneOf => doc(
            "one_of(options) -> dist",
            "One of the options: equally likely from a list or a range (`one_of(1..6)`), or weighted from a map, like `one_of([Boom: 20%, Steady: 80%])`. Percentages must add up to 100%; plain numbers are relative weights.",
        ),
        B::Binomial => doc(
            "binomial(n: int, p: prob) -> dist[int]",
            "The number of successes in `n` independent trials, each with probability `p`.",
        ),
        B::Poisson => doc(
            "poisson(rate: float) -> dist[int]",
            "A count of events that happen independently, `rate` on average.",
        ),
        B::Geometric => doc(
            "geometric(p: prob) -> dist[int]",
            "The number of tries up to and including the first success, each with probability `p`.",
        ),
        B::Roll => doc(
            "roll(n: int, die) -> dist[list[int]]",
            "`n` dice, sorted from highest to lowest: `let dice ~ roll(4, d6)`.",
        ),
        B::Bag => doc(
            "bag(counts) -> bag",
            "Items to draw without replacement, with `take`: `bag([\"ace\": 4, \"king\": 4])`, or from a list.",
        ),
        B::Normal => doc(
            "normal(mean: float, sd: float) -> dist[float]",
            "The normal distribution.",
        ),
        B::Lognormal => doc(
            "lognormal(mu: float, sigma: float) -> dist[float]",
            "The distribution whose logarithm is `normal(mu, sigma)`. For an estimate, `a to b` is easier.",
        ),
        B::Uniform => doc(
            "uniform(lo: float, hi: float) -> dist[float]",
            "Every value from `lo` to `hi` equally likely.",
        ),
        B::Beta => doc(
            "beta(a: float, b: float) -> dist[float]",
            "A continuous distribution on [0, 1], such as an unknown rate. Convert a drawn rate explicitly with `prob(rate)`. Observing counts with `observe k from binomial(n, prob(rate))` updates it exactly when sampling.",
        ),
        B::Gamma => doc(
            "gamma(shape: float, scale: float) -> dist[float]",
            "A distribution of positive numbers, such as an unknown rate of events: its mean is `shape × scale`.",
        ),
        B::Exponential => doc(
            "exponential(rate: float) -> dist[float]",
            "The time until an event that happens at `rate` per unit of time: its mean is `1 / rate`.",
        ),
        B::Triangular => doc(
            "triangular(lo, mode, hi) -> dist[float]",
            "From `lo` to `hi`, most likely at `mode`, with straight sides.",
        ),
        B::Pert => doc(
            "pert(lo, mode, hi) -> dist[float]",
            "From `lo` to `hi`, most likely at `mode`, smoother than `triangular`: a beta distribution stretched over the range.",
        ),
        B::NormalRange => doc(
            "normal_range(lo: float, hi: float) -> dist[float]",
            "The normal distribution with a 90% chance of falling between `lo` and `hi`. Unlike `lo to hi`, it can be negative.",
        ),
        B::Mixture => doc("mixture(…)", "Not implemented yet."),
        B::Truncate => doc("truncate(d, lo, hi)", "Not implemented yet."),
        B::Bins => doc("bins(d, n)", "Not implemented yet."),
        B::To => doc(
            "a to b",
            "An estimate: 90% confident it's between `a` and `b`, as a lognormal, so both must be above 0.",
        ),
        // Questions about distributions
        B::P => doc(
            "P(c) -> prob",
            "The probability that a fact, or a distribution of facts, is true: `P(d6 > 4)`.",
        ),
        B::Mean => doc(
            "mean(d) -> float or complex",
            "The weighted arithmetic mean of a distribution. Complex outcomes give a complex mean; their probabilities stay real.",
        ),
        B::Sd => doc("sd(d) -> float", "The standard deviation of a distribution."),
        B::Variance => doc("variance(d) -> float", "The variance of a distribution."),
        B::Median => doc("median(d)", "The value with half the distribution at or below it."),
        B::Quantile => doc(
            "quantile(d, q: prob)",
            "The smallest value with a share `q` of the distribution at or below it: `quantile(d, 95%)`.",
        ),
        B::Support => doc("support(d) -> list", "The outcomes a distribution can have, in order."),
        B::Cdf => doc(
            "cdf(d, x) -> prob",
            "The probability that the distribution is at most `x`.",
        ),
        B::Pmf => doc(
            "pmf(d, x) -> prob",
            "The probability that the distribution is exactly `x`.",
        ),
        B::Pdf => doc("pdf(d, x) -> float", "The density of a continuous distribution at `x`."),
        B::Prob => doc(
            "prob(x) -> prob",
            "Explicitly converts a finite number in [0, 1], or a bool (false = 0, true = 1), to a probability. Out-of-range values are errors. Does not clamp, draw or lift over distributions. Numeric literals also become probabilities when a declared type or parameter expects prob; numeric variables and expressions need this conversion.",
        ),
        B::Odds => doc("odds(p: prob) -> float", "`p / (1 − p)`: 75% is 3 to 1."),
        B::Logit => doc(
            "logit(p: prob) -> float",
            "The logarithm of the odds, `ln(p / (1 − p))`.",
        ),
        B::InvLogit => doc(
            "inv_logit(x: float) -> prob",
            "The probability whose logit is `x`: `1 / (1 + e^−x)`.",
        ),
        // Dates
        B::Date => doc(
            "date(s: str) -> date or date(year: int, month: int, day: int) -> date",
            "An immutable Gregorian calendar date. Parse exactly YYYY-MM-DD or supply three integer components. Years are 1 through 9999; impossible dates are errors. Dates have no time of day or time zone.",
        ),
        B::Days => doc(
            "days(n) -> int",
            "`n` days, rounded to a whole number, to add to or subtract from a date: `start + days(3)`.",
        ),
        B::Weeks => doc("weeks(n) -> int", "`n` weeks, as a whole number of days."),
        B::AddWorkdays => doc(
            "add_workdays(d: date, n: int, holidays: list[date]?) -> date",
            "Move by n Monday–Friday days, excluding any listed holidays. The starting date isn't counted; zero leaves it unchanged even on a closed day. Negative n goes back. Holiday order and duplicates do not matter; no regional holidays are assumed.",
        ),
        B::IsWorkday => doc(
            "is_workday(d: date, holidays: list[date]?) -> bool",
            "Whether d is Monday–Friday and absent from the optional holiday list. Calendars are explicit data; no regional holidays are assumed.",
        ),
        B::AddMonths => doc(
            "add_months(d: date, n: int) -> date",
            "Move by n calendar months, clamping the day to the last valid day of the target month. January 31 plus one month is February 28 or 29. Derive recurring dates from the original anchor to avoid drift after clamping.",
        ),
        B::AddYears => doc(
            "add_years(d: date, n: int) -> date",
            "Move by n calendar years, clamping February 29 to February 28 in a non-leap year. Negative n goes back. The original date is unchanged.",
        ),
        B::StartOfMonth => doc(
            "start_of_month(d: date) -> date",
            "The first day of d's month, as a new date.",
        ),
        B::EndOfMonth => doc(
            "end_of_month(d: date) -> date",
            "The last day of d's month, including leap-year February.",
        ),
        B::Year => doc("year(d: date) -> int", "The Gregorian year, from 1 to 9999."),
        B::Month => doc(
            "month(d: date) -> int",
            "The month number, January = 1 through December = 12.",
        ),
        B::Day => doc("day(d: date) -> int", "The day of the month, from 1 to 31."),
        B::Weekday => doc(
            "weekday(d: date) -> str",
            "The day of the week: `\"Monday\"` to `\"Sunday\"`.",
        ),
        B::RunDate
        | B::IterItems
        | B::RepeatCount
        | B::IsFalse
        | B::IsTrue
        | B::IsListOfLen
        | B::Settled
        | B::Last
        | B::DropLast
        | B::BooleanLaw
        | B::ScoreLaw
        | B::Typeof => None,
    }
}

/// The documentation of a named value.
pub fn constant(c: Constant) -> Doc {
    let (signature, summary) = match c {
        Constant::Pi => (
            "pi = 3.141592653589793",
            "π: a half turn, in radians. `sin(pi / 2)` is 1.",
        ),
        Constant::E => ("e = 2.718281828459045", "Euler's number, the base of `exp` and `ln`."),
        Constant::EulerGamma => (
            "euler_gamma = 0.5772156649015329",
            "The Euler–Mascheroni constant γ: how far `1 + 1/2 + … + 1/n` ends up above `ln(n)`.",
        ),
        Constant::Today => (
            "today = execution date (date)",
            "The immutable date captured once by the host for this execution, shared by every world, sample, function and simulate block. The CLI and playground use UTC by default; --today YYYY-MM-DD or a host-supplied date makes reruns reproducible. This is a value, not a function. A program's own bindings may hide it.",
        ),
    };
    Doc { signature, summary }
}

/// `read`, which isn't a built-in function: it's how a program's data comes
/// in, before it runs.
pub const READ: Doc = Doc {
    signature: "let rows: list[Row] = read(\"data.csv\")",
    summary: "Reads data from a file: CSV, JSON or lines, with the declared type deciding how. `read(\"-\")` reads standard input. The data is the same in every world and every run.",
};

/// The keywords, and the words that act as keywords in their places.
pub const KEYWORDS: &[&str] = &[
    "let", "var", "fn", "return", "if", "else", "for", "in", "while", "loop", "repeat", "break", "continue", "match",
    "chance", "observe", "score", "from", "report", "by", "as", "simulate", "type", "enum", "with", "and", "or", "not",
    "div", "mod", "to", "true", "false", "import", "typeof",
];

pub fn keyword(word: &str) -> Option<Doc> {
    match word {
        "typeof" => doc(
            "typeof expression -> str",
            "The runtime type of a value, such as \"prob\", \"float\", \"dist[int]\" or \"list[int]\". Evaluates the operand once, without drawing from distribution values. Use parentheses around compound expressions: `typeof (d6 > 3)`. Empty containers use `unknown`; mixed element types use `any`. This describes runtime values, not inferred static types.",
        ),
        "let" => doc(
            "let x = e or let x ~ D",
            "Declares a variable. `=` gives it a value; `~` draws from a distribution or probability: `let roll ~ d20` and `let roll = ~d20` are equivalent. A bound outcome has one identity in each world.",
        ),
        "var" => doc(
            "var x = e",
            "Declares a variable that can change, with `=`, `+=` or `~`. A function can only change its own variables.",
        ),
        "fn" => doc(
            "fn name(a, b: int) -> int { … }",
            "Defines a function. Its result is its last expression, or what `return` gives. Calling it can split the caller's world. When enumerating, a result is reused for inputs it has already seen.",
        ),
        "return" => doc("return e", "Leaves the function with this result."),
        "if" | "else" => doc(
            "if c { … } else { … }",
            "Branches on a bool, a prob, or a dist[bool]. Probabilities and boolean recipes request a fresh trial each time. Numeric literals convert contextually: `if 30% { ... }`. Numeric variables need prob(x).",
        ),
        "for" | "in" => doc(
            "for x in xs { … }",
            "Runs the body once per item of a list, range, map (as `[key, value]` pairs), bag or string. `x in xs` also says whether `xs` holds `x`.",
        ),
        "while" => doc(
            "while c { … }",
            "Re-evaluates its condition each round: a bool follows its fact, and a prob or dist[bool] requests a fresh trial. `while ~d6 != 6 { ... }` explicitly draws a new face each time. If the worlds come back to states they were in, it's solved exactly; otherwise it stops once what's left weighs less than ε.",
        ),
        "loop" => doc(
            "loop { … }",
            "Repeats until the body leaves with `break` or `return`, like `while true`.",
        ),
        "repeat" => doc("repeat n { … }", "Runs the body `n` times."),
        "break" => doc("break", "Leaves the innermost loop."),
        "continue" => doc("continue", "Goes on with the innermost loop's next round."),
        "match" => doc(
            "match x { pattern => … }",
            "Runs the first arm whose pattern fits `x`: a value, a variant, a list like `[a, b]`, a name that takes any value, or `_`. `if` adds a guard.",
        ),
        "chance" => doc(
            "chance { 60% => …, 30% => …, else => … }",
            "Weighted branches: each weight must be prob; numeric literals convert contextually. Use prob(x) for a numeric expression. Each branch runs with its weight, and `else` gets the rest.",
        ),
        "observe" | "from" => doc(
            "observe c or observe v from D",
            "Evidence: `observe c` requires bool and discards worlds where it is false. `observe v from D` weighs worlds by the likelihood of D giving v. Use `score p` to apply a probability likelihood, or `observe ~p` to observe an anonymous boolean draw. Reports describe the worlds that fit the evidence.",
        ),
        "score" => doc(
            "score p",
            "Multiplies each world's weight by a prob in [0, 1]. Numeric literals convert contextually, including branch results: `score if sick { 95% } else { 8% }`. Does not draw or mutate the probability. Like observe, it must come before reports and is local inside simulate.",
        ),
        "report" | "by" | "as" => doc(
            "report e by key as \"label\"",
            "Adds `e` to the output: the chance of a fact, the distribution of a value, or one row per `key`. Only at the top level, after the observations.",
        ),
        "simulate" => doc(
            "simulate { … }",
            "Runs a block as a model of its own, and gives the distribution of its result, normalized, without splitting the current world.",
        ),
        "type" => doc(
            "type Name = { field: type, … }",
            "Declares a record type: `Name { field: value }` makes one.",
        ),
        "enum" => doc("enum Name { A, B, C }", "Declares a type whose values are these names."),
        "with" => doc(
            "r with { field: value }",
            "A copy of the record `r` with some fields changed.",
        ),
        "and" | "or" | "not" => doc(
            "a and b, a or b, not a",
            "Combine boolean facts, or compose probability and boolean-distribution recipes independently. `p and p` means two trials; bind `let event = ~p` to reuse one outcome. `not p` complements a recipe. Only an actual boolean false/true short-circuits and/or.",
        ),
        "div" => doc("a div b", "Integer division, rounded down."),
        "mod" => doc("a mod b", "The remainder of `a div b`, with the sign of `b`."),
        "to" => builtin(Builtin::To),
        "true" | "false" => doc("true, false", "The two facts."),
        "import" => doc("import …", "Not supported yet."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_public_built_in_is_documented() {
        for &b in Builtin::ALL {
            let d = builtin(b);
            assert_eq!(d.is_some(), b.is_public() || b == Builtin::To, "{}", b.name());
            assert_eq!(category(b) == "Internal", d.is_none(), "{}", b.name());
        }
        for word in KEYWORDS {
            assert!(keyword(word).is_some(), "{word}");
        }
    }
}
