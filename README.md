# Probl

**A programming language where conditions are probabilities.**

In Probl, `if 30% { … } else { … }` runs *both* branches: one in a world with weight 0.3, the other in a world with weight 0.7. A program doesn't produce one answer; it produces the distribution over every world it can end up in. Worlds that reach the same state merge, which keeps many game simulations small enough to follow every possibility. For models too big for that, or with continuous quantities, the same program runs by sampling instead.

```probl
# Craps, pass line bet: what's the chance of winning?
let come_out ~ 2d6               # one world per total
var win = false
if come_out in [7, 11] {
  win = true
} else if come_out not in [2, 3, 12] {
  loop {                         # keep rolling for the point
    let r ~ 2d6
    if r == come_out { win = true; break }
    if r == 7 { break }
  }
}
report win                       # 49.29%: the known answer, 244/495
```

Probl is aimed at two kinds of work:

- **Game simulation**: dice, cards, boards and fights. Odds, game length, balance.
- **Forecasting**: estimates (`5 to 10`), scenarios, evidence (`observe`), fan charts and dates.

## Status

The engine enumerates and samples. Examples 01–06 print their documented output, checked against independent calculations; the forecasting examples 07–09 sample, and agree with an independent reference simulation within their sampling error. The rules are written down in [the reference semantics](docs/semantics.md). A second, deliberately simple interpreter checks enumeration against them on thousands of generated programs, and sampling is checked against enumeration, standard errors included. When sampling, conjugate priors (a beta with binomial counts, say) are updated exactly instead of weighting each run by the data, so forecasts from data keep every run's worth.

```sh
cargo run --release -p probl-cli -- run examples/02_craps.probl
cargo run --release -p probl-cli -- run examples/02_craps.probl --fractions   # ≈ 244/495
cargo run --release -p probl-cli -- run examples/07_launch_forecast.probl      # sampled
cargo run --release -p probl-cli -- run examples/02_craps.probl --runs 100000  # sampled too
cargo run --release -p probl-cli -- run examples/08_signup_forecast.probl      # reads examples/data/pilot.csv
cargo run --release -p probl-cli -- schema examples/data/pilot.csv            # a type to read it with
cargo run --release -p probl-cli -- repl
cargo test --all
```

- [Language overview](docs/language-overview.md): the model, the syntax, and a tour of the language
- [Reference semantics](docs/semantics.md): the precise rules the engine follows
- [Reading data](docs/data-input.md): CSV, JSON and lines, read with declared types, and its [design review](docs/data-input-review.md)
- [Better inference](docs/inference-proposal.md): exact updates for conjugate priors, what a general method needs first, and its [design review](docs/inference-proposal-review.md)
- [Implementation plan](docs/implementation-plan.md): status, architecture, phases, testing and risks
- [Project audit](docs/project-audit.md): the review that led to the reference semantics
- [Benchmarks](docs/benchmarks.md): realistic models, what they cost, and what to build next (`cargo run --release -p probl-bench`)
- [Examples](examples/): nine sample programs with their expected output, from a tour of the language to a revenue forecast
