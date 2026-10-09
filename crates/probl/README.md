# probl

**Compile and run Probl programs from Rust.**

[Probl](https://github.com/garlab/probl) is a language for probabilities, simulations and forecasts. Write models with variables, functions and loops, then inspect their possible outcomes. This crate is the supported embedding API used by the command-line tool and browser playground.

[API reference](https://docs.rs/probl) · [Library guide](https://github.com/garlab/probl/blob/main/docs/library.md) · [Try the playground](https://playground.probl.dev)

To run `.probl` files from a terminal, install [`probl-cli`](https://crates.io/crates/probl-cli), which provides the `probl` command.

## Get started

Requires Rust 1.85 or later. Add the library to your project:

```sh
cargo add probl
```

Compile a model, run it, and read its reports without parsing output text:

```rust
use probl::{Options, compile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
        let total ~ 2d6
        report total >= 10 as "ten or more"
    "#;
    let program = compile("dice.probl", source)?;
    let outcome = program.run(&Options::new())?;
    print!("{}", outcome.text());

    if let Some(chance) = outcome.report("ten or more")
        .and_then(|report| report.groups().first())
        .and_then(|group| group.probability())
    {
        println!("point: {:?}, complete: {}", chance.point(), chance.is_complete());
    }
    Ok(())
}
```

The model enumerates two six-sided dice and reports a probability of 6/36, displayed as 16.67%. Compilation returns a reusable `Program`; it does not run the model or open data files.

## Execution and results

- **Modes:** the program's mode applies unless overridden. The default enumerates; it does not automatically fall back to sampling. For sampling, pass options such as `Options::new().runs(100_000).seed(42)`.
- **Structured results:** `Outcome` exposes reports, evidence, unresolved weight, sampling metadata and execution statistics, as well as formatted text.
- **Data:** models using `read` load their inputs through `Program::load` and a host-chosen `Files` provider. Use `MemoryFiles` for supplied bytes or `LocalFiles` for native filesystem access.
- **Host controls:** options configure resource limits, cancellation, progress callbacks and a fixed date for models using `today`.
- **Errors:** compilation, loading and execution return diagnostics. In partial failure mode, execution still returns an error; `Error::partial()` exposes the available results explicitly.

Interpret a probability's `point()` together with `is_complete()`, `bounds()` and its sampling uncertainty. An available point does not guarantee a complete answer. Enumeration uses floating-point weights, and sampling carries Monte Carlo error.

The [library guide](https://github.com/garlab/probl/blob/main/docs/library.md) explains data loading, errors and result interpretation. The [language guide](https://github.com/garlab/probl/blob/main/docs/language-overview.md) and [examples](https://github.com/garlab/probl/tree/main/examples) introduce model writing.

## Compatibility

Probl is early-stage software; the language and Rust API are evolving. Use this crate to embed Probl. `probl-number`, `probl-syntax`, `probl-sema` and `probl-engine` are internal components with no stable API; the hidden `probl::__internal` adapter is also outside the supported API.

## License

Licensed under either the [MIT license](https://github.com/garlab/probl/blob/main/LICENSE-MIT) or [Apache License, Version 2.0](https://github.com/garlab/probl/blob/main/LICENSE-APACHE), at your option.
