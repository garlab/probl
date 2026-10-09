# probl-cli

**The command-line tool for Probl, a language for probabilities, simulations and forecasts.**

Run models, check programs and explore expressions interactively. The package is named `probl-cli`; the installed executable is **`probl`**.

[Try the playground](https://playground.probl.dev) · [Language guide](https://github.com/garlab/probl/blob/main/docs/language-overview.md) · [Examples](https://github.com/garlab/probl/tree/main/examples) · [Repository](https://github.com/garlab/probl)

To embed Probl in a Rust application, use the [`probl`](https://crates.io/crates/probl) library.

## Install

Requires Rust 1.85 or later:

```sh
cargo install probl-cli --locked
```

## Run a model

Save this as `dice.probl`:

```probl
let total ~ 2d6
observe total >= 7
report total >= 10 as "ten or more"
```

```sh
probl run dice.probl
```

```text
enumerated · evidence 58.33%

ten or more    28.57%
```

`~` draws the dice, `observe` keeps worlds whose total is at least seven, and `report` gives the probability of ten or more among those remaining worlds.

## Useful commands

```sh
probl check dice.probl                       # compile without running or opening data
probl check --data dice.probl                # also validate declared input files
probl run dice.probl --runs 100000 --seed 42  # select sampling
probl run dice.probl --stats                 # inspect execution statistics
probl run dice.probl --fractions             # also show approximate fractions
probl run dice.probl --on-error total        # stop at the first fault
probl repl                                  # interactive session
probl run --help                             # all execution options
```

For a model using `today`, pass `--today 2026-09-29` to fix its immutable UTC date snapshot. Successful runs are reproducible for the same program, data, date, seed and engine version.

The [examples](https://github.com/garlab/probl/tree/main/examples) cover games, forecasts, calendars and operational decisions. To run them locally, clone the repository and run `probl run examples/01_tour.probl` from its root.

## Modes and limitations

Probl enumerates by default and does not automatically switch to sampling. Enumeration follows weighted possible worlds and merges equivalent states, but large state spaces may need sampling through `--runs` or a program's `@mode sample(...)`.

Enumeration uses floating-point weights and may retain unresolved mass; displayed fractions are approximations. Sampling reports estimates with uncertainty information. Check warnings, unresolved weight and effective sample size when interpreting results.

A fault stops the run by default when enumerating. Sampling defaults to partial failure mode, where other runs can finish and the output identifies failures; the command still exits with a failure status. `--on-error total` stops at the first fault in either mode. See the [failure-mode semantics](https://github.com/garlab/probl/blob/main/docs/semantics.md#failure-modes).

Probl is early-stage software and the language is evolving. See the [project overview](https://github.com/garlab/probl#status-and-limits) for current limits.

## License

Licensed under either the [MIT license](https://github.com/garlab/probl/blob/main/LICENSE-MIT) or [Apache License, Version 2.0](https://github.com/garlab/probl/blob/main/LICENSE-APACHE), at your option.
