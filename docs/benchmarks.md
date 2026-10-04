# Benchmarks: what to build next

> September 2026. Step 5 of the [project audit](project-audit.md): "use realistic game and forecast benchmarks to choose further work: Markov-chain solving, symbolic inference, persistent collections, or a faster interpreter." This document reports the benchmarks and recommends an order. Measured on an Apple M2 Pro (8 performance cores), release build.

## Summary

1. **Parallel sampling batches first.** Every sampled forecast takes 0.5–2.6 s on one core, and runs are independent. Running example 07's runs as 8 processes takes 0.27 s instead of 1.77 s (6.6×). This is the cheapest large win, and it meets the plan's target for example 07 (under 2 s on 8 cores) several times over. *Done: 6–8× on 12 cores ([below](#since-parallel-batches)).*
2. **Then cheaper merging, and moving draws to their first use.** Enumeration hits a wall when states are large or many facts stay live together: blackjack from a real deck takes 5.2 s and 952 MB, and a 20-component reliability model 1.4 s and 847 MB. Merging (hashing and comparing world states) takes 34–70% of that time; caching the hashes of collections cuts most of it. Separately, moving each draw to just before its first use turns the reliability model's 1,048,576 worlds into 256, with the same answer. *Both are done: cheaper merging makes enumeration 1.4–2.9× faster ([below](#since-cheaper-merging)), and moving draws takes the reliability model from 1.2 s to 0.6 ms ([below](#since-moving-draws)).*
3. **Then better inference for evidence-heavy forecasts.** Weighting runs by the evidence (likelihood weighting) degrades as data accumulates: an A/B test's 100,000 runs are worth 852 after 30 days of data, 467 after 60 and 211 after 120, and it gets exponentially worse with more unknown parameters. Forecasts with real data need conjugate updates, MCMC or particles; each needs its contract specified first (audit D4). *Done for conjugate priors: the A/B test's runs are worth 100,000 instead of 863 ([below](#since-exact-updates-for-conjugate-priors)). A general method waits for a benchmark that needs it.*
4. **Not now, for the original benchmark suite:** symbolic inference, persistent collections and a bytecode interpreter. No benchmark establishes a need for them that a cheaper change cannot address first. Markov-chain solving would make cyclic loops exact and allow recursion to the same call, but no model is slow because of them. *Done for loops: those that cycle are solved exactly ([below](#since-solving-loops-that-cycle)). October follow-up: a [symbolic inference review](symbolic-inference-review.md) identifies a different workload, many posterior reports over live discrete unknowns, and recommends a bounded prototype.*

## Running them

```sh
cargo run --release -p probl-bench              # every model in benches/ and examples/ (about 3½ minutes)
cargo run --release -p probl-bench -- tennis    # models whose name contains "tennis"
cargo run --release -p probl-bench -- --quick   # one timed run each
cargo run --release -p probl-bench -- --threads=1   # sample on one thread
cargo run --release -p probl-bench -- --no-conjugate   # sample without exact updates for conjugate priors
```

For each model, the runner reports the median time of up to five runs, the engine's statistics, the peak heap (from a separate run that counts allocations, and isn't timed), and for enumerated models the same run without merging worlds. Sampling uses every core unless `--threads=` says otherwise. A run that takes more than 60 seconds is cancelled and reported as such. `cargo test` checks that every model still compiles.

## The models

The twelve models in [`benches/`](../benches/) are realistic uses of the language, each chosen to stress one of the candidate improvements; the ten examples run alongside them.

| Model | What it computes | What it stresses |
|---|---|---|
| `tennis` | a best-of-three-sets match, point by point | loops that cycle (deuce, tie-breaks): Markov chains |
| `blackjack_deck` | one hand from a single 52-card deck | state explosion: the deck keeps worlds apart |
| `blackjack_shoe` | the same hand from an endless shoe | nothing: the baseline for `blackjack_deck` |
| `yahtzee` | going for five of a kind in one turn | dice pools, higher-order functions |
| `board_game` | a Monopoly-like board after 100 turns | long horizons over a few hundred states |
| `snakes_three` | snakes and ladders, three players, written naturally | state explosion: up to 100³ positions |
| `regimes` | a hidden market regime, from five years of monthly data | a long chain of evidence |
| `reliability` | a 20-component system, written naturally | many independent facts live together: 2²⁰ worlds |
| `reliability_staged` | the same system, computed stage by stage | nothing: the baseline for `reliability` |
| `inventory` | a shop's stock over a year (sampled) | long loops in every run: interpreter speed |
| `epidemic` | a stochastic SIR epidemic in a town (sampled) | large counts drawn every day |
| `ab_test` | an A/B test with 30 days of data (sampled) | evidence-heavy sampling |

## Results

Measured before sampling ran in parallel: sampled models used one core.

| model | mode | time | peak worlds | world-steps | per step | calls | peak heap | without merging | notes |
|---|---|--:|--:|--:|--:|--:|--:|---|---|
| benches/ab_test | sample | 1.96 s | 1,000 | 25,700,000 | 76 ns |  | 12 MB |  | effective sample size 852 |
| benches/blackjack_deck | enumerate | 5.16 s | 699,168 | 7,273,009 | 710 ns | 2,754,170 (100% reused) | 952 MB | gave up: more than 10,000,000 worlds |  |
| benches/blackjack_shoe | enumerate | 27 ms | 5,130 | 63,257 | 422 ns | 33,050 (99% reused) | 3 MB | gave up: too slow | unresolved < 1e-12 |
| benches/board_game | enumerate | 171 ms | 4,536 | 2,220,290 | 77 ns |  | 2 MB | gave up: more than 10,000,000 worlds |  |
| benches/epidemic | sample | 1.42 s | 1,000 | 19,044,454 | 74 ns |  | 2 MB |  |  |
| benches/inventory | sample | 2.59 s | 1,000 | 40,753,365 | 64 ns |  | 1 MB |  |  |
| benches/regimes | enumerate | 789 µs | 3 | 1,364 | 578 ns | 366 (98% reused) | 17 KB | gave up: too slow | evidence 2.9e-23% |
| benches/reliability | enumerate | 1.39 s | 1,048,576 | 8,388,607 | 166 ns |  | 847 MB | the same (nothing merges) |  |
| benches/reliability_staged | enumerate | 187 µs | 128 | 356 | 524 ns | 5 | 54 KB | 420 steps |  |
| benches/snakes_three | enumerate | over 60 s |  |  |  |  | over 1.1 GB |  | **cancelled** |
| benches/tennis | enumerate | 2 ms | 210 | 6,262 | 270 ns | 69 (94% reused) | 88 KB | gave up: too slow | unresolved 2.3e-11 |
| benches/yahtzee | enumerate | 5 ms | 750 | 14,797 | 345 ns | 12,420 (97% reused) | 611 KB | 157× the steps, 351 ms |  |
| examples/01_tour | enumerate | 331 ms | 2,000 | 3,509,941 | 94 ns | 1,001 (100% reused) | 1 MB | gave up: more than 10,000,000 worlds | unresolved < 1e-12 |
| examples/02_craps | enumerate | 3 ms | 186 | 14,775 | 214 ns |  | 231 KB | gave up: more than 10,000,000 worlds | unresolved < 1e-12 |
| examples/03_rpg_duel | enumerate | 816 ms | 4,080 | 1,289,480 | 632 ns | 161,937 (100% reused) | 12 MB | gave up: too slow | unresolved < 1e-12 |
| examples/04_risk_battle | enumerate | 51 ms | 8,232 | 530,403 | 97 ns |  | 3 MB | gave up: too slow |  |
| examples/05_snakes_and_ladders | enumerate | 232 ms | 682 | 1,105,771 | 210 ns | 2 | 328 KB | gave up: more than 10,000,000 worlds | unresolved < 1e-12 |
| examples/06_blackjack_dealer | enumerate | 14 ms | 3,196 | 58,534 | 246 ns |  | 5 MB | 9× the steps, 54 ms |  |
| examples/07_launch_forecast | sample | 1.79 s | 1,000 | 22,067,620 | 81 ns | 900,000 | 7 MB |  |  |
| examples/08_signup_forecast | sample | 1.16 s | 1,000 | 13,600,000 | 85 ns |  | 25 MB |  | effective sample size 31,194 |
| examples/09_roadmap | sample | 557 ms | 1,000 | 5,200,000 | 107 ns | 500,000 | 2 MB |  |  |
| examples/10_quantum_key | enumerate | 1 ms | 32 | 1,290 | 1,083 ns | 170 (83% reused) | 75 KB | 1,450 steps (1×), 2 ms |  |

"Without merging" repeats an enumerated run with merging off, stopping at the world limit or at 20 times the merged run's time. A world-step is one statement run in one world; "per step" divides the time by them.

Three things stand out:

- **Merging is what makes enumeration work.** Without it, 10 of the 14 enumerated models that finish give up, including a hidden-regime model over 60 months (3⁶⁰ paths, 3 worlds when merged: merging is the forward algorithm).
- **When states can't merge, memory runs out first.** A world costs about 850 bytes in `reliability` and 1.4 KB in `blackjack_deck`, so a million worlds take about a gigabyte. `snakes_three` reached 1.1 GB within its minute.
- **Sampled models cost about 65–110 ns per statement per run**, so every sampled forecast takes seconds on one core.

## Where the time goes

Profiles of three models (macOS `sample`, share of samples on top of the stack):

| | `inventory` (sampled) | `blackjack_deck` (large states) | `reliability` (a million worlds) |
|---|--:|--:|--:|
| hashing world states for merging | | 41% | 22% |
| the rest of merging: comparing states, clearing dead slots, hash tables | 4% | 29% | 12% |
| evaluating expressions | 23% | 2% | 19% |
| copying, dropping and allocating values | 26% | 17% | 23% |
| drawing counts (`poisson`) | 14% | | |
| everything else: arithmetic, logic, indexing, reading slots, statements | 32% | 11% | 24% |

Large states are dominated by merging: a world's slots, including its deck (a map) and its hands (records), are hashed at every merge point, and nothing about a collection's hash is remembered. Sampled runs are dominated by the interpreter's general overhead: walking expression trees, and copying and freeing values. In `inventory`, drawing a Poisson count recomputes the probability at the mode (`lgamma`) on every draw, although the rate stays the same for a whole run.

## The candidates

**Parallel sampling batches.** Runs are independent, and the engine already processes them in batches of 1,000. Measured with 8 processes over 07's runs: 6.6× on this machine. It needs a random stream per batch (derived from the seed and the batch number, so that the output doesn't depend on the number of threads) and a way to add up the reports of several engines, which the per-batch sums of sampled reports already make simple. The sampling-against-enumeration check covers it. *Recommended first.*

**Cheaper merging.** Caching each collection's hash when it's built (records, bags, lists and maps are immutable and shared) would make hashing a world cost one step per slot instead of one per element, and comparing hashes first makes most equality checks cheap. Clearing dead slots can use a precomputed list per merge point instead of testing every slot. Merging is where state-heavy enumeration spends 34–70% of its time, so this should roughly halve it. The oracle and the merging-on-and-off tests check that results don't change. *Recommended second.*

**Moving draws to their first use.** A compiler pass could move each `let x ~ D` down to just before the first statement that reads `x`, when nothing in between reads or changes what `D` depends on. Draws are independent, so this doesn't change the model; it lets the facts a statement combines die, and their worlds merge, before the next draws split them again. Done by hand, it turns `reliability` from 1,048,576 worlds and 847 MB into 256 worlds, with the same 99.53%. The pass must not move a draw across an `observe`, a call, or anything that could fail, so that errors stay where the semantics puts them. *Recommended with cheaper merging.*

**Better inference for evidence-heavy forecasts.** In `ab_test`, likelihood weighting still gets the headline right (P(B better) is 99.33% against the exact 99.37%, and the mean lift 20.03 against 20.11), but the tails already drift (the 5% quantile of the lift is 5.92 against 6.34), and its effective sample size falls from 852 to 211 as the data grows from 30 to 120 days. With more unknown parameters, it collapses exponentially. Forecasting with data needs conjugate updates for the common cases (beta–binomial, gamma–Poisson), and a general method such as Metropolis–Hastings over a run's choices, or particles with rejuvenation. Each needs its estimator contract written first (audit D4). *Recommended third: the most valuable feature for forecasting, and the largest. Conjugate updates are done ([below](#since-exact-updates-for-conjugate-priors)).*

**Markov-chain solving.** The models with loops that cycle (`tennis`, craps, snakes and ladders, the tour's `while d6 != 6`) run in milliseconds; what unrolling costs them is exactness (unresolved weight up to 2.3 × 10⁻¹¹ in `tennis`), not time. Solving absorbing chains would make them exact, and would allow recursion that returns to the same call, which enumeration rejects today. *Worth doing for exactness and expressiveness, after the above. Done for loops ([below](#since-solving-loops-that-cycle)).*

**Symbolic inference** (decision diagrams, as in Dice). Moving draws fixes the original `reliability` model more cheaply. The large concrete states in `blackjack_deck` and `snakes_three` motivate sampling (`snakes_three` samples 20,000 games in 1.4 s), but their size alone does not establish whether a symbolic representation would help. This suite did not measure that question. *Deferred for these workloads. The October [symbolic inference review](symbolic-inference-review.md) adds an exploratory posterior-reporting workload that still reaches `2^n` worlds with current optimizations and proposes a bounded Boolean prototype; no symbolic speedup has yet been measured.*

**Persistent collections.** No benchmark spends significant time copying large collections: the collections in these models are small (a deck of 10 counts, a pipeline of 3 orders, hands as records), and each world's copy differs from the others. What costs is hashing them, which cached hashes fix. *Not now; revisit if a model with large shared collections shows up.*

**A bytecode interpreter.** Interpretation overhead (evaluating expressions, copying values) is about half of a sampled run's time, so a faster design could give 2–4×, at the cost of rewriting the engine's core. Parallel batches give more for much less, and should come first; then profile again. *Not now.*

## Since: parallel batches

Sampling now runs its batches of 1,000 runs on every core. Each batch has a random stream of its own, and the batches are combined in order, so the output is the same on any number of threads (docs/semantics.md, section 14). The same machine:

| model | runs | 1 thread | 8 threads | 12 threads |
|---|--:|--:|--:|--:|
| benches/ab_test | 100,000 | 1.95 s | 289 ms (6.7×) | 247 ms (7.9×) |
| benches/epidemic | 10,000 | 1.39 s | 294 ms (4.7×) | 178 ms (7.8×) |
| benches/inventory | 10,000 | 2.53 s | 553 ms (4.6×) | 347 ms (7.3×) |
| examples/07_launch_forecast | 50,000 | 1.80 s | 277 ms (6.5×) | 260 ms (6.9×) |
| examples/08_signup_forecast | 200,000 | 1.14 s | 187 ms (6.1×) | 169 ms (6.7×) |
| examples/09_roadmap | 100,000 | 553 ms | 82 ms (6.7×) | 75 ms (7.4×) |

One thread is as fast as before. The M2 Pro has 8 performance and 4 efficiency cores, so the last four threads add less than the first eight. A model can't use more threads than it has batches: `epidemic` and `inventory` have 10, which take two rounds on 8 threads. Peak heap grows with the threads, since each has an engine and a batch of its own: example 07 takes 55 MB on 12 threads instead of 10 MB.

## Since: cheaper merging

Merging now works like this:

- **Live slots only.** It hashes and compares only the slots that may still be read.
- **Cached hashes.** Lists, maps, bags and records keep their hash once computed, and forget it when they change.
- **Clearing where slots die.** Each statement clears the slots that die in it, which liveness works out, instead of every join testing every slot. Values are freed sooner, so peak memory went down too.
- **Cheaper internals.** The merge index no longer allocates per world, and comparing and hashing small values is inlined.
- **Bags as sorted vectors,** whose hash `take` updates without rehashing the bag.

The oracle, and the engine with merging on and off, still agree on 5,000 generated programs. Enumerated models, on the same machine:

| model | before | after |
|---|--:|--:|
| benches/blackjack_deck | 5.01 s, 952 MB | 2.55 s (2.0×), 818 MB |
| benches/board_game | 171 ms | 124 ms (1.4×) |
| benches/reliability | 1.41 s, 847 MB | 1.23 s (1.15×), 762 MB |
| benches/regimes | 665 µs | 289 µs (2.3×) |
| examples/01_tour | 329 ms | 159 ms (2.1×) |
| examples/03_rpg_duel | 808 ms | 276 ms (2.9×) |
| examples/04_risk_battle | 52 ms | 36 ms (1.4×) |
| examples/05_snakes_and_ladders | 235 ms | 97 ms (2.4×) |
| examples/06_blackjack_dealer | 14 ms | 7 ms (2.0×) |

What's left in `blackjack_deck` is mostly the cost of worlds themselves: copying, dropping and allocating them takes about a quarter of the time. Merging still spends 16% in its hash table (up to 700,000 worlds at once), and 8% comparing the decks of worlds that turn out equal, which has to be exact. `reliability` barely changed, since none of its million worlds merge: moving draws to their first use is what fixes it. `snakes_three` still runs out of time.

## Since: moving draws

A compiler pass (`crates/probl-sema/src/draws.rs`) now moves each `let x ~ D` down its block, to just before the first statement that reads or writes `x`.

It only moves draws of distributions written out: dice, `bernoulli` of a probability, and `one_of` a list of values. Those can't fail, read no variable and add up to 1. So the pass can move them past more than the sketch above allowed:
- **Observations** multiply weights in either order.
- **Reports** add up the same weights.
- **Calls** that don't use `x` do the same thing in every world the draw would have split.

No weight, error or unresolved bound changes. A draw stops only at:
- a statement that uses `x`;
- anything that prints, since output is once per world;
- anything that can leave the block early.

A type-annotated draw moves with its check.

| model | before | after |
|---|--:|--:|
| benches/reliability | 1.23 s · 1,048,576 worlds · 8,388,607 world-steps · 762 MB | 0.55 ms · 256 worlds · 1,643 world-steps · 384 KB |

The other models already draw where they use the values: only craps' first roll moved, past two assignments. The oracle, which interprets the source directly, agrees with the engine on 20,000 generated programs, and sampling is still calibrated.

## Since: exact updates for conjugate priors

When sampling, a beta, gamma or normal prior observed through a conjugate form (binomial or Bernoulli counts, Poisson counts, normal values with a known spread) is no longer drawn first and weighted by the data. Each run updates its distribution exactly with each observation and draws it from the result when first needed (docs/semantics.md, section 14; the [inference proposal](inference-proposal.md)). In `ab_test` and example 08, every run then ends with the same weight, the probability of the data. So the effective sample size is the number of runs, and the evidence is exact. The same machine, seed and 12 threads, with `--no-conjugate` for before:

| model | effective sample size | mean lift's standard error | evidence | time |
|---|--:|--:|---|--:|
| benches/ab_test, 30 days | 863 → 100,000 | 0.21 → 0.03 | 4.02e-62 ± 3.4% → 4.20e-62 (exact: 4.2045e-62) | 249 → 240 ms |
| the same, 60 days | 417 → 100,000 | 0.21 → 0.02 | 1.88e-121 ± 4.9% → 2.01e-121 | |
| the same, 120 days | 204 → 100,000 | 0.21 → 0.01 | 8.45e-240 ± 7.0% → 9.10e-240 | |
| examples/08_signup_forecast | 31,244 → 200,000 | | 3.61e-15 ± 0.52% → 3.60e-15 (exact: 3.598e-15) | 189 → 186 ms |

The standard errors fall 7–21×, so the same precision takes about 50 to 440 times fewer runs. The time per run doesn't change: an exact update costs about what the likelihood it replaces did. The tails are right now. Over 12 seeds, `ab_test`'s 5% quantile of the lift is 6.27–6.42 against the exact 6.34, and P(B better) is 99.32–99.42% against 99.37%. Without exact updates, seed 1 gives 6.19 and 99.30% ± 0.06%. The evidence without them was 4–7% low on all three data sets, by 1.0–1.3 of its standard errors: with few runs carrying the weight, it's more often too low than too high.

## Since: solving loops that cycle

A `while` or `loop` whose states come back is now solved as an absorbing Markov chain, as soon as a state comes back (docs/semantics.md, section 10; `crates/probl-engine/src/chain.rs`). It's exact, and a loop some worlds can never leave is an error at once, instead of after ten million rounds.

| model | before | after |
|---|--:|--:|
| benches/tennis | 4 ms, unresolved 2.3e-11 | 1 ms, exact: 3 loops solved, 20 states |
| benches/blackjack_shoe | 12 ms, 63,257 world-steps, unresolved < 1e-12 | 8 ms, 34,142 world-steps, exact: 2 loops solved, 1,080 states |

Without merging, both now finish: solving tells states apart itself. Nothing else changed.
- **Loops that count their rounds.** Craps, snakes and ladders, the tour's `while d6 != 6` and the duel all count, and one of those counts is reported, so their states never come back. They're unrolled as before, and still end with `unresolved < 1e-12`.
- **Loops that end within a few rounds.** The dealer is unrolled as before, exactly. In Risk, different paths reach the same armies after different numbers of rounds, which counts as coming back, so it's solved from there. It prints the same.

Hashing the states costs nothing measurable. A loop that ends once in 10⁹ rounds, which unrolling couldn't finish, takes microseconds.

Recursion that comes back to the same call now works too (semantics §6). Such a call is solved by rounds, with every call in the cycle run at most once per round. A recursive tennis game takes 35 rounds and gives the loop's 81.26%. Exploding dice take 15 rounds.

## Smaller findings

- `poisson` draws in sampled runs recompute `lgamma` at the mode every time (14% of `inventory`); caching it for the last rate is a small fix.
- A sampled report of a continuous quantity keeps every distinct value for exact quantiles: 25 MB for example 08's 200,000 runs. A quantile sketch would bound it.
- The effective sample size is the right warning for likelihood weighting, but at a few hundred its standard errors are unreliable too (the semantics says so). A warning in the output when it's small would help.
- Tiny evidence is printed as a percentage in scientific notation (`evidence 2.9e-23%`); plain scientific notation, `2.9e-25`, would read better. *Done: it's `evidence 2.92e-25` now, and sampling prints its estimate the same way.*
- A local variable can hide a built-in function (a `count` variable made `count(…)` fail in `yahtzee` with "can't call an int"). A warning, or a better error, would help.
