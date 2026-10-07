# Probl

**A programming language for probabilities, simulations and forecasts.**

Write a model with ordinary variables, functions and loops. Probl follows its possible outcomes and reports their probabilities. Use it to explore game balance, compare decisions, forecast uncertain quantities or update a model with evidence.

[Try the playground](https://playground.probl.dev) · [Language guide](docs/language-overview.md) · [Examples](examples/) · [Documentation](docs/README.md)

```probl
let total ~ 2d6
observe total >= 7
report total >= 10 as "ten or more"
```

```text
enumerated · evidence 58.33%

ten or more    28.57%
```

Here `~` draws two dice, `observe` keeps worlds where their total is at least seven, and `report` asks how often the total is ten or more in those remaining worlds: 6 of 21 equally likely dice pairs.

## How it works

In enumeration mode, `if 30% { … } else { … }` follows both branches, with weights 0.3 and 0.7. A program doesn't produce a single answer but a distribution over its possible outcomes. Worlds with the same remaining state merge, which keeps many game models small. Certain cyclic processes are solved as Markov chains, and a restricted class of continuous calculations stays analytic.

For larger models and broader continuous computations, sampling follows one random path per run and reports estimates with uncertainty information. Switching modes preserves the model; it changes how its answers are computed. The default mode enumerates and does **not** automatically fall back to sampling.

A distribution is a recipe; a drawn value has an identity:

```probl
let die = d6
report die + die          # two independent rolls

let roll ~ die
report roll + roll        # the same roll used twice
```

Other building blocks include arbitrary-precision integers, complex numbers, immutable Unicode strings and dates, collections, typed CSV/JSON input, and local models with `simulate`. Supported conjugate priors update analytically when sampling. See the [language guide](docs/language-overview.md) for the rules and boundaries.

## Get started

The [browser playground](https://playground.probl.dev) runs locally in your browser; no installation is needed.

To build the CLI from a checkout, install Rust 1.85 or later and run these commands from the repository root:

```sh
cargo install --path crates/probl-cli --locked
probl run examples/01_tour.probl
probl run examples/02_craps.probl --fractions
probl run examples/07_launch_forecast.probl
```

The package is named **`probl-cli`** and the installed command is **`probl`**. You can also run without installing:

```sh
cargo run --release -p probl-cli -- run examples/02_craps.probl
```

Useful commands:

```sh
probl run model.probl --runs 100000 --seed 42  # select sampling
probl run model.probl --stats                 # inspect execution and effective sample size
probl check model.probl                       # compile without running or opening data
probl check --data model.probl                # also validate declared input files
probl schema examples/data/pilot.csv          # suggest an input type
probl repl
```

Models using `today` get one immutable UTC date snapshot from the host. Add `--today 2026-09-29` to reproduce the calendar examples' documented output. Successful runs are reproducible for the same program, data, date, seed and engine version.

## Explore the examples

| Use case | Start here |
|---|---|
| Learn the language | [Tour](examples/01_tour.probl) |
| Games and odds | [Craps](examples/02_craps.probl), [RPG duel](examples/03_rpg_duel.probl), [Risk](examples/04_risk_battle.probl), [blackjack dealer](examples/06_blackjack_dealer.probl) |
| Forecast with evidence | [Signup forecast from CSV](examples/08_signup_forecast.probl), [predictive model check](examples/16_predictive_check.probl) |
| Calendar uncertainty | [Delivery dates](examples/11_delivery_dates.probl), [invoice cash flow](examples/12_invoice_calendar.probl) |
| Compare decisions | [Stock levels](examples/14_stock_decision.probl), [service queues](examples/15_service_queue.probl) |
| Monitoring and risk | [Sensor tracking](examples/17_sensor_tracking.probl), [correlated losses](examples/18_correlated_losses.probl) |
| Analytic continuous inference | [Arrival times and threshold evidence](examples/19_analytic_continuous.probl) |

Each example includes its assumptions and expected output. The [full list](docs/language-overview.md#12-examples) and [use-case assessment](docs/use-cases.md) explain what they demonstrate and what remains difficult.

## Embed in Rust

The **`probl`** crate is the library used by the CLI and playground. Compile once, run repeatedly, and read structured results without parsing text:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = "let total ~ 2d6\nreport total >= 10 as \"ten or more\"";
    let program = probl::compile("dice.probl", source)?;
    let outcome = program.run(&probl::Options::new())?;

    if let Some(chance) = outcome.report("ten or more")
        .and_then(|r| r.groups().first())
        .and_then(|g| g.probability())
    {
        println!("point: {:?}, complete: {}", chance.point(), chance.is_complete());
    }
    Ok(())
}
```

Results retain unresolved bounds, sampling-error status and evidence metadata. An available point estimate is not necessarily a complete or exact answer. See the [library guide](docs/library.md) for setup, data loading and result interpretation, or run `cargo run -p probl --example craps` from this checkout.

## Status and limits

Probl is early-stage software at version `0.1.0`. The language and Rust API are evolving.

- Enumeration uses floating-point weights and can retain unresolved mass; displayed fractions are approximations. State spaces can still grow exponentially.
- Sampling can be unreliable with rare events or concentrated evidence. Inspect effective sample size and uncertainty; zero empirical variation is not proof of certainty.
- Analytic continuous enumeration supports affine calculations and threshold conditions on a shared draw. General nonlinear combinations need sampling, and `simulate` always enumerates.
- Some type errors are detected only at runtime. Modules, optional/null values, general MCMC, particles and beam search are not implemented.
- Complex scalar arithmetic is available; quantum-amplitude simulation is not.

The [reference semantics](docs/semantics.md) documents these contracts. Verification includes known answers, regression tests, an independent rational-arithmetic interpreter, generated sampling comparisons and native/WASM parity. The [testing guide](docs/testing.md) explains their scope and remaining automation gaps.

## Development

See [contributing](CONTRIBUTING.md) for bug reports, where tests go, updating examples and commit messages. The checks CI runs:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all --locked
```

To work on the playground, install [Bun](https://bun.sh), then:

```sh
rustup target add wasm32-unknown-unknown
cd web
bun install --frozen-lockfile
bun run serve
```

Open `http://localhost:8000`. See [playground development](docs/playground.md) for browser tests and deployment, [architecture](docs/architecture.md) for the crate layout and priorities, and the [documentation index](docs/README.md) for design notes and measured benchmarks.

## License

Licensed under either the [MIT license](LICENSE-MIT) or [Apache License, Version 2.0](LICENSE-APACHE), at your option (`MIT OR Apache-2.0`). Both permit commercial use under their terms.

Probl is provided **“as is,” without warranty**. The licenses disclaim liability for damages arising from use of the software, subject to their terms and applicable law. Independently validate models and results before relying on them for financial or other consequential decisions. This notice summarizes the licenses; it does not add a restriction on use.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
