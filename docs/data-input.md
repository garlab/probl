# Proposal: reading data from files and stdin

> Draft, September 2026. For review: nothing here is implemented. The open questions at the end need a decision before it is.

Models need data: the pilot's daily sign-ups in [example 08](../examples/08_signup_forecast.probl) are typed into the program, which doesn't scale to a year of sales or a list of 300 cards. This proposal adds one built-in function, `read`, and nothing else.

## In one example

```csv
day,visitors,signups
1,250,9
2,250,6
3,250,11
```

```probl
let pilot = read("pilot.csv")           # a list of records: { day, visitors, signups }

let rate ~ beta(2, 40)
for row in pilot {
  observe row.signups from binomial(row.visitors, rate)
}
report rate * 100 as "conversion rate (%)"
```

```sh
probl run signups.probl                                  # reads pilot.csv next to signups.probl
tail -n +2 pilot.csv | cut -d, -f3 | probl run counts.probl   # with read("-"): one value per line
```

## The rules

**`read(path)`** returns the contents of a file as a Probl value. `read("-")` reads standard input.

- **The path must be a string literal.** Data is read once, before the program runs, by whoever runs it (the command line, a test, later a playground); the engine itself never touches files. So `read("data_" + month + ".csv")` is a compile error: "the path must be written out, so the data can be read before the program runs".
- **Relative paths are relative to the program's file**, not the working directory, so a model and its data can move together. `probl repl` uses the working directory.
- **Data is a constant.** Every world and every run sees the same value, as if it had been typed in. Reading data is not evidence: to condition on it, `observe` it, as above.
- **Each path is read once.** Two `read("pilot.csv")` calls share one value. Standard input can be read only once, so `read("-")` may appear with only one format.

**The format comes from the extension**, or from a `format:` argument, which `read("-")` needs for anything but lines:

| Format | Files | Value |
|---|---|---|
| `lines` | `.txt`, and stdin by default | a list: one value per non-blank line |
| `csv` | `.csv` | a list of records, one per row after the header |

Another extension is an error that suggests `format:`.

**CSV** follows RFC 4180: comma-separated, with double quotes around cells that contain commas, quotes or line breaks. The first row names the columns. The names become field names in snake case (`Daily sign-ups` becomes `daily_sign_ups`), and two columns that end up with the same name are an error.

**Values are typed by column** (for `lines`, the file is one column). A column's type is the first of these that fits every cell in it:

| Cells like | Type | Value |
|---|---|---|
| `12`, `-3` | `int` | 12 |
| `2.5`, `1e-3`, `12` mixed with `2.5` | `float` | 2.5 |
| `30%`, `12.5%` | `prob` | 30% |
| `true`, `false` | `bool` | true |
| `2027-01-31` | `date` | 2027-01-31 |
| anything else | `str` | the text, trimmed of surrounding spaces |

Typing whole columns, rather than cells, keeps a column's values the same type: a column of sign-ups is all ints, even if one day had none.

An **empty cell** is an error for now ("pilot.csv, line 12: the `signups` cell is empty"), since the language has no missing value yet.

A **type annotation** checks the data, as it does any value: `let pilot: list[{ day: int, visitors: int, signups: int }] = read("pilot.csv")` fails if a column isn't what the model expects.

## Errors and limits

Every problem is a diagnostic at the `read(…)` call, with the file's line when it's about the contents:

- the file can't be read, or doesn't exist;
- the format is unknown;
- a row has more or fewer cells than the header, a quote isn't closed, a cell is empty;
- stdin is read with two formats, or in the REPL (where it's the session's input);
- the data is too large.

The host limits what can be read: a new limit on input size (64 MiB by default, `--max-input`), and the existing limit on collection length for the number of rows.

## How it would be built

- **`probl-sema`**: `read` is recognized at compile time. The lowering checks that its arguments are literals, records each source (path, format, span) in a list on the program, and turns the call into a constant that refers to it.
- **`probl-engine`**: a `data` module parses text into a value (`parse(format, text)`), with no file access. `Options` gets the values of the program's sources, which the engine uses for those constants.
- **`probl-cli`**: after compiling, it reads each source (the file next to the program, or stdin), parses it, and turns errors into diagnostics. `probl check` reads the data too, so bad data shows up before running.
- **Tests**: the CSV reader (quoting, line breaks in cells, typing columns, every error), `read` in enumeration and sampling (the same value everywhere), type annotations on data, and the command line with files and a pipe. Example 08 would read its pilot data from a CSV file next to it.

This is about 400 lines of code and tests, with no new dependency.

## Later, if needed

- **Choosing the data on the command line.** Named inputs (`input pilot = "pilot.csv"` in the program, `--input pilot=june.csv` or `--input pilot=-` when running) would let one model run on different data without editing it. Until then, `--data pilot.csv=june.csv` could substitute one file for another.
- **JSON**, as records, lists and values, for nested data and configuration.
- **Missing values**, once the language has a way to represent them.
- **Other separators** (`.tsv`, semicolons), headerless CSV, and dates in other formats.
- **The data in the summary line**, such as `sample · 50,000 runs · seed 11 · data pilot.csv (14 rows)`, so the output says what it was computed from.

## Open questions

1. **`read("path")` in the program, or named inputs bound on the command line?** This proposal starts with `read`: it's the smallest change and reads like the rest of the language. Named inputs could come later on top of it.
2. **Column names**: convert headers to snake case (proposed), or keep them as written and require them to be valid names?
3. **Empty cells**: an error (proposed), or a value such as `()` that fails only when used?
4. **JSON**: in the first version, or later (proposed)?
5. **Relative paths**: relative to the program's file (proposed) or to the working directory?
