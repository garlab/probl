# Probl

**A programming language where conditions are probabilities.**

In Probl, `if 30% { … } else { … }` runs *both* branches: one in a world with weight 0.3, the other in a world with weight 0.7. A program doesn't produce one answer; it produces the distribution over every world it can end up in. Worlds that reach the same state merge, so game simulations stay small and their answers are exact. When exact answers are out of reach, the same program runs by sampling instead.

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
report win                       # 49.29%, exactly 244/495
```

Probl is aimed at two kinds of work:

- **Game simulation**: dice, cards, boards and fights. Odds, game length, balance.
- **Forecasting**: estimates (`5 to 10`), scenarios, evidence (`observe`), fan charts and dates.

## Status

v0.1 is in progress. The exact engine works: examples 01–06 run and print their documented output, and those numbers were checked against independent calculations. Sampling mode, needed for the forecasting examples 07–09, is next (v0.2).

```sh
cargo run --release -p probl-cli -- run examples/02_craps.probl
cargo run --release -p probl-cli -- run examples/02_craps.probl --fractions   # 244/495
cargo run --release -p probl-cli -- repl
cargo test --all
```

- [Language overview](docs/language-overview.md): the model, a syntax proposal, semantics and grammar
- [Implementation plan](docs/implementation-plan.md): architecture, phases, testing and risks
- [Examples](examples/): nine sample programs with their expected output, from a tour of the language to a revenue forecast
