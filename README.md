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

The playground runs the same engine in the browser, compiled to WebAssembly. Beside the editor are the language guide, with programs to run, and a reference. The editor completes names, describes them on hover, and goes to their definitions with Cmd-click or Ctrl-click. It needs Node, and Rust's WebAssembly target:

```sh
rustup target add wasm32-unknown-unknown
cd web && npm install && npm run serve        # then open http://localhost:8000
npm test                                      # every example in Node, then the page in headless Chrome
```

It's deployed to Cloudflare Pages with wrangler. `npm run deploy` checks the credentials, creates the Pages project the first time, builds, and uploads `web/dist`. It needs Node 22 or later, and two variables, from the environment or from `web/.env`, which git ignores:

- `CLOUDFLARE_API_TOKEN`: an API token with the permission *Account · Cloudflare Pages · Edit*.
- `CLOUDFLARE_ACCOUNT_ID`: the account's ID.
- Optionally, `CLOUDFLARE_PAGES_PROJECT`, the project's name. It's `probl-playground` if unset.

```sh
cd web
npm run deploy                                # production: the project's production branch, main
npm run deploy -- --preview                   # a preview, named after the current git branch
PROBL_URL=https://probl-playground.pages.dev node test/page.mjs   # test the deployed page
```

- [Language overview](docs/language-overview.md): the model, the syntax, and a tour of the language
- [Reference semantics](docs/semantics.md): the precise rules the engine follows
- [Reading data](docs/data-input.md): CSV, JSON and lines, read with declared types, and its [design review](docs/data-input-review.md)
- [Better inference](docs/inference-proposal.md): exact updates for conjugate priors, what a general method needs first, and its [design review](docs/inference-proposal-review.md)
- [Complex values and quantum simulation](docs/quantum-and-complex.md): the implemented scalar foundation and a possible future quantum engine
- [Playground plan](docs/playground-plan.md): Probl in the browser, what it took, and what's left
- [Implementation plan](docs/implementation-plan.md): status, architecture, phases, testing and risks
- [Project audit](docs/project-audit.md): the review that led to the reference semantics
- [Benchmarks](docs/benchmarks.md): realistic models, what they cost, and what to build next (`cargo run --release -p probl-bench`)
- [Examples](examples/): ten sample programs with their expected output, from a tour of the language to a revenue forecast and quantum cryptography
