# Proposal: reading data from files and stdin

> Draft, September 2026, revised after review. Nothing here is implemented yet. The review settled the main questions: the program declares the type of what it reads, JSON comes in the first version, and the language as a whole is statically typed, with inference ([implementation plan](implementation-plan.md), section 8). Two questions remain open, at the end.

Models need data: the pilot's daily sign-ups in [example 08](../examples/08_signup_forecast.probl) are typed into the program, which doesn't scale to a year of sales or a list of 300 cards. This proposal adds a built-in function, `read`, for CSV, JSON and plain lines, and a command, `probl schema`, that suggests the types to read them with.

## In two examples

A CSV file, `pilot.csv`:

```csv
day,visitors,signups
2026-09-01,250,9
2026-09-02,250,6
2026-09-03,250,11
```

```probl
@mode sample(runs: 100_000)

type Day = { day: date, visitors: int, signups: int }
let pilot: list[Day] = read("pilot.csv")

let rate ~ beta(2, 40)
for row in pilot {
  observe row.signups from binomial(row.visitors, rate)
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
probl run signups.probl        # reads pilot.csv next to signups.probl
probl schema pilot.csv         # suggests the type Day above
tail -n +2 pilot.csv | cut -d, -f3 | probl run counts.probl   # with: let counts: list[int] = read("-")
```

## The rules

**`let name: T = read(path)`** reads a file as a value of type `T`. `read("-")` reads standard input.

- **The type is required, and says how to read the data.** A `read` is the whole value of a `let` or `var` with a type annotation, at the top level of the program. The type isn't checked after reading: it's what decides how each value is read.
  - The same text `12` is an int in an `int` field and 12.0 in a `float` one.
  - `007` stays `"007"` in a `str` field.
  - `Small` is a variant in a field of an enum type.

  So the program decides what the data means, not the data itself. A stray `2.5` or `n/a` is an error at the boundary, with its line number. It doesn't become a float or a string that fails later, somewhere in the model.
- **The path must be a string literal.** Data is read once, before the program runs, by whoever runs it: the command line, a test, later a playground. The engine itself never touches files. So `read("data_" + month + ".csv")` is a compile error: "the path must be written out, so the data can be read before the program runs".
- **Relative paths are relative to the program's file**, so a model and its data can move together. `probl repl` uses the working directory.
- **Data is a constant.** Every world and every run sees the same value, as if it had been typed in. Reading data isn't evidence: to condition on it, `observe` it, as above.
- **Standard input can be read only once**, and not in the REPL, where it's the session's input.

**The format comes from the extension**, or from a `format:` argument, which `read("-")` needs for anything but lines: `read("-", format: "csv")`.

| Format | Files | Reads as |
|---|---|---|
| `csv` | `.csv` | `list[R]`, for a record type `R`: one record per row after the header |
| `json` | `.json` | any type data can have (below) |
| `lines` | `.txt`, and stdin by default | `list[T]`, for a plain type `T`: one value per non-blank line |

Another extension is an error that suggests `format:`.

**Data has these types:** the plain types `int`, `float`, `prob`, `bool`, `date`, `str` and enums, plus records, `list`, `map` and `bag` of those. Distributions and functions aren't data. `let d: dist[int] = read(…)` is a compile error that suggests reading the numbers and building the distribution from them.

**Names.** A record field matches the CSV column or JSON key with the same name, compared in lowercase with everything but letters and digits dropped. So `daily_sign_ups` matches `Daily sign-ups`, `dailySignUps` and `DAILY_SIGN_UPS`.
- Columns and keys that no field names are ignored, so a program declares only the columns it uses.
- A field that matches no name, or two, is an error that lists the names the file has.

### Plain values

The same rules apply to a CSV cell, a line and a JSON value:

| Type | CSV cells and lines | JSON |
|---|---|---|
| `int` | `12`, `-3` | an integer, like `12` |
| `float` | `2.5`, `1e-3`, `12` | a number |
| `prob` | `30%`, `12.5%`, or a number from 0 to 1 | a number from 0 to 1, or a string like `"30%"` |
| `bool` | `true`, `false`, in any case | `true`, `false` |
| `date` | `2027-01-31` | the same, as a string |
| `str` | the text | a string |
| an enum | a variant's name, like `Small` | the same, as a string |

Errors suggest the likely fix. For example, `30` in a `prob` field is "not between 0 and 1: did you mean `30%`?"

In CSV, unquoted cells are trimmed of surrounding spaces, and quoted cells are kept as written. An empty cell is an error, except in a `str` field, where it reads as `""`. The language has no missing value yet: optional types come with the type checker (see *Later*).

### CSV

CSV follows RFC 4180: cells are separated by commas, with double quotes around cells that contain commas, quotes or line breaks. The first row names the columns, and every row has as many cells as it does.

### JSON

JSON follows RFC 8259.
- **Arrays** read as `list[T]`.
- **Objects** read as records, matched by the name rule above. They can also read as `map[K, V]`, where each key is read as a `K`: `map[date, float]` reads `{"2027-01-01": 1.5}`.
- **Bags:** a `bag[T]` reads an array of items, or an object of counts, like `bag(…)` in a program.
- **Null** is an error for now.

## Writing the type: `probl schema`

Writing the type of a wide file by hand is tedious, so a command suggests one:

```sh
$ probl schema pilot.csv
type Day = { day: date, visitors: int, signups: int }
# let pilot: list[Day] = read("pilot.csv")
```

- **CSV:** for each column, it takes the first of `int`, `float`, `prob`, `bool`, `date` and `str` that fits every value in the column.
- **JSON:** it follows the document. Objects become records, and arrays become lists of the type that fits all their elements.
- **Names:** column names become field names in snake case: `Daily sign-ups` becomes `daily_sign_ups`. A name that can't become a field name is listed in a comment.

The guess is a starting point, to paste into the program and edit. `probl run` never guesses, so the data can't change what the program means.

## Errors and limits

Every problem is a diagnostic at the `read(…)` call. When it's about the contents, it gives the file's line, and for JSON also the path to the value, like `segments[1].share`:

- the file can't be read, or doesn't exist;
- the format is unknown, or can't give the declared type (a CSV file as a `list[int]`, say);
- a declared field matches no name in the file, or two;
- a row has more or fewer cells than the header, a quote isn't closed, or the JSON is malformed;
- a value doesn't fit its type: "pilot.csv, line 12: `signups` is `n/a`, not an int";
- there's an empty cell outside a `str` field, or a JSON `null`;
- stdin is read twice, or in the REPL;
- the data is too large.

The host limits what can be read:
- a new limit on input size: 64 MiB by default, set with `--max-input`;
- the existing limit on collection length, for rows and elements;
- a limit on how deeply JSON nests.

## Types in the language

Probl is statically typed, with inference. Every expression's type is known before the program runs, but annotations are optional: the checker works them out (phase 6 of the plan). Data is the exception, because nothing in a program says what a file contains. So a `read` is where a type has to be written.

This doesn't wait for the checker. Until the checker exists, the declared type drives the reading, and data is checked at the boundary. After that, the checker also takes `read`'s type from the declaration, and checks the rest of the program against it before anything runs.

## How it would be built

- **`probl-sema`:** `read` is recognized at compile time. The lowering:
  - checks that `read` is the value of an annotated top-level binding, with a literal path and a type data can have;
  - records each source (path, format, type, span) in a list on the program;
  - turns the call into a constant that refers to its source.
- **`probl-engine`:** a `data` module reads text as a value of a given type. It has a CSV reader, a JSON parser and the plain values, with no file access and no new dependency. Reading follows the type, so every error has a position. `Options` gets the values of the program's sources, which the engine uses for those constants.
- **`probl-cli`:**
  - reads each source (the file next to the program, or stdin), reads it with the data module, and turns errors into diagnostics;
  - `probl check` reads the data too, so bad data shows up before running;
  - `probl schema FILE` prints a suggested type.
- **Tests:**
  - the CSV reader: quoting, line breaks in cells, trimming;
  - the JSON parser: escapes, Unicode, numbers, nesting;
  - every type, name matching, and every error;
  - the guesser;
  - `read` in enumeration and sampling, with the same value everywhere;
  - the command line, with files and a pipe.

  Example 08 would read its pilot data from a CSV file next to it.

This is about 1,000 lines of code and tests.

## Later

- **Choosing the data on the command line.** Named inputs would let one model run on different data without editing it: `input pilot: list[Day] = "pilot.csv"` in the program, then `--input pilot=june.csv` or `--input pilot=-` when running.
- **Missing values:** optional types (such as `int?`) with the type checker, for empty cells and `null`.
- **More formats:** JSON Lines, other separators (`.tsv`, semicolons), headerless CSV, dates in other formats.
- **Columns whose names can't be field names**, through an explicit mapping.
- **The data in the summary line**, such as `sample · 50,000 runs · seed 11 · data pilot.csv (14 rows)`, so the output says what it was computed from.

## Decided in review

- The program declares the type of what it reads, and the type drives the reading. Nothing is guessed when running, and `probl schema` suggests a declaration.
- JSON is in the first version.
- Names match ignoring case and punctuation, and columns no field names are ignored.
- An empty cell is an error, except in a `str` field. Optional types come with the type checker.
- The language is statically typed, with inference, and annotations are required only where data comes in.

## Open questions

1. **`read` in the program, or named inputs bound on the command line?** This proposal starts with `read`: it's the smallest change and reads like the rest of the language. Named inputs could come later on top of it.
2. **Relative paths:** relative to the program's file (proposed), or to the working directory?
