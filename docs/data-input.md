# Reading data from files and stdin

> September 2026. Implemented. This started as a proposal, and a [design review](data-input-review.md) shaped what was built: [how the review was resolved](#how-the-review-was-resolved) is at the end. Two decisions frame it:
> - A program declares the type of what it reads, and that type decides how the data is read.
> - The language as a whole is statically typed, with inference ([implementation plan](implementation-plan.md), section 8).

Models need data: a year of sales, a list of 300 cards, a pilot's daily sign-ups. The built-in `read` loads CSV, JSON and plain lines. The command `probl schema` suggests the types to read them with.

## In two examples

A CSV file, [`examples/data/pilot.csv`](../examples/data/pilot.csv), read by [example 08](../examples/08_signup_forecast.probl):

```csv
day,visitors,signups
2026-09-01,250,9
2026-09-02,250,6
2026-09-03,250,11
```

```probl
@mode sample(runs: 100_000)

type Day = { day: date, visitors: int, signups: int }
let pilot: list[Day] = read("data/pilot.csv")

let rate ~ beta(2, 40)
for day in pilot {
  observe day.signups from binomial(day.visitors, rate)
}
report rate * 100 as "conversion rate (%)"
```

A JSON file, `assumptions.json`:

```json
{
  "price": 12.5,
  "churn": "4%",
  "segments": [
    { "name": "Small", "share": 0.7 },
    { "name": "Large", "share": 0.3 }
  ]
}
```

```probl
enum Size { Small, Large }
type Segment = { name: Size, share: prob }
type Assumptions = { price: float, churn: prob, segments: list[Segment] }
let assumptions: Assumptions = read("assumptions.json")
```

On the command line:

```sh
probl run model.probl                  # reads its data, relative to model.probl
probl check model.probl                # checks the program, without its data
probl check --data model.probl         # and checks the data too
probl schema pilot.csv                 # suggests: type Pilot = { day: date, visitors: int, signups: int }
tail -n +2 pilot.csv | cut -d, -f3 | probl run counts.probl   # with: let counts: list[int] = read("-")
```

## The rules

**`let name: T = read(path)`** reads a file as a value of type `T`. `read("-")` reads standard input.

- **The type is required, and says how to read the data.**
  - `read` is the whole value of a `let` or `var` with a type annotation, at the top level of the program. Anywhere else it's a compile error.
  - The type is not a check made after reading. It decides how each value is read: the text `12` is an int in an `int` field and 12.0 in a `float` one, `007` stays `"007"` in a `str` field, and `Small` is a variant in a field of an enum type.
  - So the program, not the data, decides what the data means. A stray `2.5` or `n/a` is an error at the boundary, with its line number. It doesn't become a float or a string that fails later, somewhere in the model.
- **The path is a string literal.** Data is read once, before the program runs, by whoever runs it: the command line, a test, later a playground. The engine never touches files. So `read("data_" + month + ".csv")` is a compile error.
- **Relative paths are relative to the program's file**, so a model and its data move together (more under [Where data comes from](#where-data-comes-from)).
- **Data is a constant.** Every world and every run sees the same value, as if it had been typed in. Reading data isn't evidence: to condition on it, `observe` it, as above.
- **Standard input can be read only once in a program.** It can't be read at all in the REPL, where it's the session.

**The format comes from the extension**, or from a `format:` argument, which `read("-")` needs for anything but lines: `read("-", format: "csv")`.

| Format | Files | Reads as |
|---|---|---|
| `csv` | `.csv` | `list[R]`, for a record type `R` of plain fields: one record per row after the header |
| `json` | `.json` | any type data can have |
| `lines` | `.txt`, and stdin by default | `list[T]`, for a plain type `T`: one value per non-blank line |

**Data has these types:** the plain types `int`, `float`, `prob`, `bool`, `date`, `str` and enums, and records, `list`, `map` and `bag` of those. The compiler checks the whole declared type, through every named record it refers to:
- A distribution or a function anywhere in it is an error at the field that has it.
- So is a CSV field that isn't plain, or a map from JSON whose keys can't be text (below).

Record types may refer to each other in any order, and to themselves inside a list, map or bag. A record type that contains itself outside those has no values, and is an error.

### Names

A record field matches the CSV column, or JSON key, with the same *key*: its letters and digits, in lowercase, with everything else dropped. `daily_sign_ups`, `Daily sign-ups`, `dailySignUps` and `DAILY_SIGN_UPS` all have the key `dailysignups`. "Letters and digits" means Unicode ones, so `Größe (m²)` has the key `größem²`. The compiler, the loader and `probl schema` share this one function (`probl_sema::data::field_key`).

- Columns and keys that no field names are ignored, so a program declares only what it uses.
- Two fields of a record read as data may not have the same key: `{ ab: int, a_b: int }` is a compile error.
- A field whose key matches no name in the data, or two names, is an error. The error lists the names the data has.
- Map keys are not names: they're kept exactly as they are.

### Plain values

The same rules apply to a CSV cell, a line, and a JSON string read as a date, an enum, a percentage or a map key:

| Type | Text | JSON |
|---|---|---|
| `int` | `12`, `-3` | an integer, like `12`, not `12.0` |
| `float` | `2.5`, `1e-3`, `12` | a number |
| `prob` | `30%`, `12.5%`, or a number from 0 to 1 | a number from 0 to 1, or a string like `"30%"` |
| `bool` | `true`, `false`, in any case | `true`, `false` |
| `date` | `2027-01-31` | the same, as a string |
| `str` | the text, exactly | a string |
| an enum | a variant's name, like `Small` | the same, as a string |

- **Text is kept exactly as written**, whether a CSV cell is quoted or not: ` ACME ` and `" ACME "` are the same string. Other types allow spaces around the value.
- **Integers are read as integers,** never through a float, so `9007199254740993` stays exact. Integers beyond ±2⁶³ remain exact, up to the host's integer size and memory limits (65,536 magnitude bits by default). JSON integers are parsed from their original decimal tokens; the playground passes data files as text, never through JavaScript `Number`. Decimal points and exponent notation still require a `float` field.
- **Floats are finite.** JSON floats are parsed with correct rounding, so any float written by a correct writer reads back as the same number.
- **Dates are Gregorian calendar days**, written exactly `YYYY-MM-DD`, within `0001-01-01` through `9999-12-31`. Invalid dates, times and timezone suffixes are errors. This is the same validation as the `date` constructor.
- **An empty value is an error**, except in a `str` field, where it's `""`. The language has no missing values yet.
- **Errors suggest the fix,** such as "`30` isn't between 0 and 1: did you mean `30%`?".

### CSV, JSON and lines

**CSV** follows RFC 4180. Cells are separated by commas, with double quotes around cells that contain commas, quotes or line breaks, and a quote inside quotes is written twice. The first row names the columns, and every row has as many cells as it does. A quote that's never closed is an error at the line where it opens.

**JSON** follows RFC 8259:
- **Arrays** read as lists.
- **Objects** read as records, by the name rule above. They can also read as `map[K, V]`, where `K` is `str`, `int`, `bool`, `date` or an enum, and each key is read as a `K`: `map[date, float]` reads `{"2027-01-01": 1.5}`.
- **Duplicate keys:** an object may not have the same key twice. Nor may a map have two keys that read as the same `K`: `{"1": …, "01": …}` as a `map[int, str]` is an error.
- **Bags:** a `bag[T]` reads an array of items, or, when `T` can be a key, an object of counts like `{"ace": 4}`.
- **Null** is an error for now.

**Lines** are read one value per line, and blank lines are skipped. `\n` and `\r\n` both end a line.

In every format, a UTF-8 byte-order mark at the start is ignored, and anything that isn't valid UTF-8 is an error.

## Writing the type: `probl schema`

Writing the type of a wide file by hand is tedious, so a command suggests one:

```sh
$ probl schema pilot.csv
type Pilot = { day: date, visitors: int, signups: int }
# let pilot: list[Pilot] = read("pilot.csv")
```

- **CSV and lines:** for each column, it takes the first of `int`, `float`, `prob`, `bool`, `date` and `str` that fits every non-empty value.
- **JSON:** it follows the document:
  - objects become records, or maps when their keys are all dates or all integers;
  - arrays become lists of the type that fits all their elements.
- **Names:** column names become field names in snake case: `Daily sign-ups` becomes `daily_sign_ups`. A keyword gets an underscore, like `type_`. A name that can't become a field name at all, like `2026 total`, is left out with a comment.

It's a starting point, to paste into the program and edit: `probl run` never guesses, so the data can't change what the program means. It reads the file within the same limits as loading data.

## Where data comes from

A program only names paths. What a path refers to, and whether it may be read at all, is the policy of whoever runs the program, and a program can't widen it. In the library, that policy is a `Files` (`probl::Files`; the engine's `Resolver` underneath). It turns each path into an identity, such as a full file name, and opens it, or refuses.

The command line runs your own programs, so its policy (`probl::LocalFiles`) is broad, but it reads only files:
- **Relative paths** are relative to the directory of the program's file, as it was named. A program reached through a link reads data next to the link, not next to its target. The REPL uses the working directory.
- **Absolute paths** are used as they are, and links in data paths are followed.
- **Only regular files are read.** That's checked before opening, so a program can't make the command wait on a pipe or read a device.
- **Standard input** (`-`) is read on a thread of its own, so `--timeout` ends a wait for data that doesn't come.

The playground gives programs no files at all, only what users add, through `probl::MemoryFiles`. The same policy serves `run`, `check --data` and the REPL.

## Limits

The host limits what a program may read, for all its data together:

| Limit | Default | Command line |
|---|---|---|
| bytes read, from all the files together | 64 MiB | `--max-input 64M` |
| values made, from all the files together: every number, string, record and element | 10,000,000 | |
| elements of one list, map or bag | the engine's collection limit | |
| how deeply JSON nests | 64 | |

Limits are charged as the data is read and decoded, before memory is allocated:
- **Bytes** are read in chunks, and reading stops at the limit.
- **Values** are counted before they're made. No intermediate tree is built: JSON is decoded straight into values, directed by the declared type.
- **Cancellation** is checked while reading and decoding. The command line's `--timeout` covers loading as well as running.

The values are loaded once, before the program runs, then shared, unchanged, by every world and every sampling batch. Each sampling thread takes its own copy: threads reading the same values would compete for their reference counts, which made example 08 five times slower on 12 threads.

## Checking, snapshots and provenance

- **Checking:** `probl check` checks the program alone, without its data, so it works on a machine without the data, and never waits on standard input. `probl check --data` also reads and checks the data, with the same loader as `probl run`. `probl run` always reads and checks the data it runs on: an earlier check can't vouch for a file that has changed since.
- **Snapshots:** a run reads each file once, even when several `read`s name it, possibly with different types. The REPL keeps what it has read for the rest of the session, until `:reload`: it runs the whole session again after each input, and earlier bindings shouldn't change under it.
- **Provenance:** each result keeps, for every file read, its identity, its size and the SHA-256 of its bytes. `probl run --stats` prints them.

## How it's built

- **`probl-sema`:**
  - Record types keep their fields' declared types. Named types are resolved once every type's name is known.
  - `read` is lowered into the program's *manifest*, `Program::inputs`. Each input has its variable's name (a stable identity for hosts, and later for `--input name=path`), its path, format, type and source span. The binding refers to its input by number.
  - `data.rs` has the rules shared with the loader: formats, plain types, key types, and the name key.
- **`probl-engine`:** `data::load(program, resolver, snapshots, limits, cancel)` reads every input, through the host's resolver. It returns `Inputs`, which only `load` makes. The engine refuses to run a program whose inputs aren't loaded, or were loaded for another program.
  - The [`csv`](https://docs.rs/csv) crate splits CSV files, and [`serde_json`](https://docs.rs/serde_json) parses JSON, with correctly rounded floats. Probl's own layer does the typing, the strictness, the budgets and the error positions.
  - `data::suggest` is the guesser behind `probl schema`.
- **`probl`:** `Program::load`, and the data as `Data`, which any number of runs can share; `LocalFiles`, the command line's policy; `MemoryFiles`; `Snapshots`, which keep what was read until they're cleared ([Probl as a library](library-proposal.md)).
- **`probl-cli`:** `--max-input`; `check --data`; `schema`; the REPL's snapshots and `:reload`; `--stats`.

## Tests

- **Compiling:**
  - where `read` may appear, and its paths, formats and arguments;
  - what each format can be read as;
  - forbidden types nested in named records, with the error at the field;
  - field-name collisions;
  - record types that refer to each other, or contain themselves.
- **Reading:** every plain type and its errors; CSV quoting, line breaks in cells, and text kept as written; JSON's nested named types, maps, bags, duplicate keys, converted-key collisions and error paths; large integers kept exact.
- **Limits:**
  - bytes, values and elements counted across all the data;
  - JSON nesting;
  - cancellation;
  - a stream that fails midway;
  - a host that refuses a path.
- **Snapshots:** one file read once for several inputs; a snapshot kept between loads; `:reload`'s clearing.
- **Running:** data is the same in every world, every batch, and on any number of threads, and a program can't run on missing or foreign inputs.
- **Command line:**
  - data next to the program, read from another directory;
  - a pipe;
  - static `check` and `check --data`;
  - `schema`;
  - `--max-input`;
  - an error at the `read`, with the data's line.
- **Round trips and fuzzing:**
  - random tables written as CSV, JSON and lines read back as the same values, including strings with commas, quotes and line breaks, and extreme integers and floats. This found `serde_json`'s default float parsing off by one ulp, hence correct rounding;
  - mutated files never make the readers or the guesser panic or report an internal error, with small limits and without. `PROBL_DATA_CASES` runs more.
- **Example 08** reads its pilot data from `examples/data/pilot.csv`, with the same output as before.

## Later

- **Choosing the data on the command line.** Named inputs, `--input pilot=june.csv` or `--input pilot=-`, would let one model run on different data without editing it. The manifest already gives each input a name to bind.
- **Missing values:** optional types (such as `int?`) with the type checker, for empty cells and `null`.
- **More formats:** JSON Lines, other separators (`.tsv`, semicolons), headerless CSV, dates in other formats.
- **Columns whose names can't be field names**, through an explicit mapping.
- **The data in the summary line**, such as `sample · 50,000 runs · seed 11 · data pilot.csv (14 rows)`.

## Decisions

In the proposal's review:
- The program declares the type of what it reads, and the type drives the reading. Nothing is guessed when running.
- JSON is in the first version.
- Names match ignoring case and punctuation, and undeclared columns are ignored.
- An empty value is an error, except in a `str` field.
- The language is statically typed, with inference; data is where a type must be written.

From the design review, with the two questions the proposal left open:
- `read` is the syntax for now, compiled into a manifest that hosts load through. Named inputs can come later without changing what programs mean.
- Relative paths are relative to the program's file.

## How the review was resolved

1. **A complete, resolved schema.** Record types keep their fields' types, resolved after every type's name is known. The whole declared type is validated at compile time, through named records, with cycles handled. The format/type combinations are checked there too. Runtime checks of named records now check their fields as well. Inputs are validated bindings tied to the program's manifest, not a vector of values.
2. **Read authority.** A host's `Resolver` decides what a path means and whether it may be read, and a program can't widen that. The command line's policy is documented above: regular files only, with standard input explicit.
3. **Bounded ingestion.** Bytes and decoded values are limited for all the data together. They're charged while reading and before allocating, with cancellation and the command line's deadline, and no intermediate tree is built. `probl schema` works within the same limits.
4. **Name collisions both ways.** Fields that would match the same names are rejected when compiling. Ambiguous names in the data, duplicate JSON keys, and keys that convert to the same map key are rejected when loading. Map keys aren't normalized, and key types are restricted.
5. **Static `check`.** `probl check` doesn't read data, and `check --data` does. Runs always read their data, a run reads each file once, and the REPL keeps snapshots until `:reload`. Each result keeps the identity, size and SHA-256 of what it read.
6. **Parsers.** The `csv` and `serde_json` crates tokenize, and Probl's layer types, limits and locates. Text is kept regardless of quoting, integers never go through floats, and floats are correctly rounded. The readers are tested with round trips and fuzzing.
