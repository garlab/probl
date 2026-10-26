# Date literals, and literals to come

> Draft proposal, 26 October 2026. Nothing here is implemented. Today's dates are in the [reference semantics](../semantics.md#1-values-and-types). This follows from the [annotations proposal](annotations-and-unions.md#building-collections)'s rule that a type's name builds a value from its parts.

## Recommendation

1. **A date is written in single quotes:** `'2027-01-31'`. It's a date anywhere a value can be written: `if today > '2027-01-01'`, `add_months('2027-01-31', 1)`, `['2027-01-01', '2027-03-01']`, or a `match` pattern. What's inside the quotes decides the type, not a declaration, so `let d: date = '2027-01-31'` only checks it, as for any literal.
2. **It's a constant, checked during compilation.** `'2027-02-30'` is an error before the program runs. Nothing inside is computed, and nothing is parsed when the program runs.
3. **`date(y, m, d)` stays, and `date("…")` goes.** A date's parts are a year, a month and a day, so `date` builds from those. Text with a date in it isn't one of them. A constant date becomes a literal, and text that's only known when the program runs, like an environment variable, goes through `parse_date(s)`.
4. **Each later kind gets a shape no other kind has:** date-times, times of day and durations, following ISO 8601, as TOML does.
5. **Single quotes never hold text.** `'hello'` is an error during compilation, whose help says that text uses double quotes.

## The question

`date` has two forms today. `date(2027, 1, 31)` builds a date from its parts, and `date("2027-01-31")` parses text. The annotations proposal settles that a type's name builds a value from its parts: `set(1, 2, 3)` is a set of three, so `set(xs)` is a set of one, and a list becomes a set by spreading, `set(...xs)`. By that rule, `date("2027-01-31")` is a conversion from text, like a `date(timestamp)` would be.

But the text form is the one people use: 27 calls, against 9 that pass a year, a month and a day. 26 of them pass a constant; one reads an environment variable. `2027-01-31` is how a date is written, and it's easier to read than `date(2027, 1, 31)`. It can't be written bare, though, since `2027-01-31` is already 1995, by subtraction.

## The proposal

### Date literals

A date literal is `'YYYY-MM-DD'`: exactly ten characters, with a year from 0001 to 9999, as `date("…")` requires today. A date that doesn't exist is an error during compilation:

```
Error: '2027-02-30' isn't a date: February 2027 has 28 days
```

It can be written wherever a value can, and is a pattern in a `match`:

```probl
# Proposed.
let start = '2027-01-31'
let deadline: date = '2027-06-30'
let reviews = ['2027-03-01', '2027-06-01', '2027-09-01']
report today > '2027-01-01'
report '2024-02-29'.add_years(1)

match day {
  '2027-12-25' => closed += 1
  else => {}
}
```

A collection literal takes its kind from the declared type, since its brackets can't tell a list from a set. A date literal can tell by itself, so it doesn't need a declaration, and works where the compiler knows no type: in a comparison, as an argument to a function value, or in `one_of(['2027-01-01', '2027-02-01'])`.

The annotations proposal's requirement that "no automatic conversion parses text" still holds. `'2027-01-31'` is a date, as `33%` is a number, and nothing turns text into a date automatically when the program runs.

### Text that's known only when the program runs

A literal can't hold what the program reads. The language overview's example of the environment does that today:

```probl
let start = date(env.get("START", "2026-01-01"))
```

So text becomes a date explicitly, with `parse_date(s)`. It accepts exactly what `date(s)` accepts today, ten characters in `YYYY-MM-DD` form, and fails when the program runs if the text isn't a date:

```probl
# Proposed.
let start = parse_date(env.get("START", "2026-01-01"))
```

Its name says that it parses, which a type's name shouldn't. Text read from a file still becomes a date through a data schema, as today.

### Single quotes aren't text

Python and JavaScript write text in single quotes, so `'hello'` will be written out of habit. It isn't a date, or any other kind, so it's an error during compilation:

```
Error: 'hello' isn't a date
  Help: text is written in double quotes: "hello"
```

The habit is caught, and never silently read as something else. Text that happens to look like a date becomes a date, and a declared `str` then rejects it.

### Printing

A report prints a date as it does today, `2027-01-31`, as it prints text without quotes. Inside a collection, a date prints in single quotes, `['2027-01-31']`, as text prints in double quotes, `["a"]`. Today it prints `[2027-01-31]`, which reads back as a list holding 1995. `str(d)` is still `"2027-01-31"`.

### Literals to come

Each kind of literal has a shape of its own, so what's inside the quotes always decides the type. ISO 8601 gives the shapes, as TOML uses them:

| Kind | Shape | Example |
|---|---|---|
| Date | `YYYY-MM-DD` | `'2027-01-31'` |
| Date and time | A date, `T`, and a time of day | `'2027-01-31T09:30:00'` |
| Instant | A date and time with `Z` or an offset | `'2027-01-31T09:30:00Z'`, `'2027-01-31T09:30:00+01:00'` |
| Time of day | `HH:MM`, or `HH:MM:SS` | `'09:30'` |
| Duration | Anything but a time of day's shape | ISO's `'P3DT2H'`, or something like `'3d 2h'` |

None of these types exist yet, apart from `date`. Each needs its own design: time zones, what `today` becomes, and arithmetic between them. This proposal only reserves their shapes. A duration can't be written `'10:30'`, since that's a time of day. If two kinds ever need the same shape, the later one gets a prefix, as Elixir writes `~D[2027-01-31]`.

## How other languages do it

- **TOML** writes dates, date-times and times bare, and tells them apart by shape: `start = 2027-01-31`. TOML has no arithmetic, so that can't be a subtraction.
- **SQL** puts a keyword before a quoted string: `DATE '2027-01-31'`, `TIMESTAMP '2027-01-31 09:30:00'`, `INTERVAL '3' DAY`.
- **Elixir** has sigils: `~D[2027-01-31]`, `~T[09:30:00]`, `~N[2027-01-31 09:30:00]` and `~U[2027-01-31 09:30:00Z]`.
- **Clojure** has tagged literals: `#inst "2027-01-31"`.
- **Visual Basic** writes `#1/31/2027#`.
- **Python, JavaScript, Kotlin and Swift** have no date literals: `date(2027, 1, 31)`, `new Date("2027-01-31")`, `LocalDate.parse("2027-01-31")`.

## What changes for programs

| Today | Proposed |
|---|---|
| `date("2027-01-31")` | An error, whose help suggests `'2027-01-31'` |
| `date(env.get("START", "2026-01-01"))` | An error, whose help suggests `parse_date(…)` |
| `date(2027, 1, 31)` | Unchanged |
| `[date(2027, 1, 31)]` prints `[2027-01-31]` | `['2027-01-31']` |
| `'a'` is an "unexpected character" | An error, whose help says that text uses double quotes |

## What would break

`date("…")` with one argument, found by searching: 27 lines. 26 pass a constant, and each becomes a literal. One, in the language overview's example of the environment, reads `env`, and becomes `parse_date(…)`.

- `web/test/dates.probl`: 17, the playground's test of dates.
- `examples/09_roadmap.probl`: 3, `examples/11_delivery_dates.probl`: 2, and `examples/13_renewal_dates.probl`: 1.
- The language overview: 4, in its section on dates, its table of built-ins, and its example of the environment. The reference semantics describes `date(s)` too.

## Alternatives considered

### Keep `date("…")`

It reads well, and isn't ambiguous: a date has three parts, so one string can't be one of them. But it's a type's name that converts, which the annotations proposal removes for collections. And it parses when the program runs, though its text is almost always a constant, so a mistyped date fails late.

### `parse_date("…")` for constants too

`parse_date` is needed anyway, for text read when the program runs. Using it for constants would need no new syntax. But it's long for the most common way to write a date, and a typo in a constant would still fail only when the program runs.

### A literal that takes its type from the declaration

`let d: date = '2027-01-31'` would work, as a collection literal takes its kind. But dates are mostly compared and passed as arguments, where nothing is declared and the compiler knows no type: `if today > '2027-01-01'` couldn't work.

### Bare dates, as in TOML

`2027-01-31` is already arithmetic.

### Prefixes, like `d'2027-01-31'` or `~D[2027-01-31]`

Not needed while each kind's shape is its own. One is kept in reserve for a kind whose shape would clash.

## Open questions

- **The other kinds:** date-times, instants, times of day and durations each need a design before their literals mean anything.
- **Other literals** beyond time could follow the same rule, if their shape is their own and can be checked during compilation.

## Implementation

1. **Lexer:** a `'…'` token, up to the closing quote on the same line, with no escapes and no interpolation. An unclosed quote is an error.
2. **Compiler:** read the text's shape. A date becomes a constant date; anything else is an error, with the help for text. A date literal is also a `match` pattern.
3. **`date`** takes exactly three ints, and `parse_date(s)` takes text. `date("…")` is an error whose help suggests the literal for a constant, and `parse_date` otherwise: `Builtin::Date`'s arity becomes `3..=3`, and `check_arity` loses its special case for two arguments.
4. **Printing:** a date inside a collection prints in single quotes.
5. **Programs and documents:** move the 26 constant calls to literals, and the overview's example of the environment to `parse_date`; update the reference semantics (section 1's paragraph on dates), the language overview's section on dates and its table of built-ins, and the playground's highlighting in `web/src/probl-lang.js`.
6. **Tests:** the first and last dates, 0001-01-01 and 9999-12-31; 29 February in a leap year and not; dates that don't exist, as errors during compilation; `parse_date` of valid and invalid text, and of an environment variable, with its errors when the program runs; `'hello'` with its help; an unclosed quote; dates in comparisons, lists, map keys, `match` patterns and arguments to function values; and printed collections that read back as the same value.
