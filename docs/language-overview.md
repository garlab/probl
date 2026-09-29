# Probl: language overview

> Draft 0.2, September 2026. Enumeration and sampling are implemented; particles, beam search and a few functions marked below are designed but not built yet. The precise rules are in the [reference semantics](semantics.md), which wins where the two disagree.
> See also: [implementation plan](implementation-plan.md) · [examples](../examples/)

Probl is a small programming language where **conditions are probabilities instead of booleans**. An `if` doesn't choose a branch. It runs both, each in its own *world*, weighted by how likely that branch is. A program doesn't produce one answer: it produces the distribution over every world it could end up in.

```probl
let weather = if 30% { "rain" } else { "sun" }
report weather
```
```
weather    sun 70.00% · rain 30.00%
```

It is built for two kinds of work:

- **Game simulation**: dice, cards, boards and fights, with odds computed over every possibility. *What's the chance this attack takes the territory? Does going first matter? How many hit points make this fight fair?*
- **Forecasting**: estimates, scenarios and evidence. *When will we ship? Will revenue reach $100k a month? Given the pilot's numbers, how many sign-ups should we expect next month?*

**Contents:** [1 The model](#1-the-model-weighted-worlds) · [2 Five rules](#2-five-rules) · [3 Syntax tour](#3-syntax-tour) · [4 Distributions](#4-distributions) · [5 Evidence](#5-evidence) · [6 Reports](#6-reports) · [7 simulate](#7-simulate-distributions-from-code) · [8 Execution modes](#8-execution-modes) · [9 Semantics](#9-semantics-in-one-table) · [10 Pitfalls](#10-what-the-language-protects-you-from) · [11 Where Probl fits](#11-where-probl-fits) · [12 Examples](#12-examples) · [Appendices](#appendix-a-grammar)

---

## 1. The model: weighted worlds

A running Probl program is a set of **worlds**. Each world is an ordinary program state, in which every variable has one plain value, plus a **weight**: the probability of being in that world. A program starts as a single world with weight 1.

- **Branching splits worlds.** `if p` sends a copy of each world into the `then` branch with its weight multiplied by p, and another copy into `else` with its weight multiplied by 1 − p. When p is 0% or 100% nothing splits, so deterministic code runs exactly as it would in any other language.
- **Drawing splits worlds too.** `let r ~ 2d6` turns each world into eleven, one per total, each weighted by the chance of that total.
- **Identical worlds merge.** Where branches rejoin, worlds that have reached the same state become one, and their weights add up. This is what makes the approach practical:

  ```
  var pos = 0
  repeat 3 { pos += if 50% { 1 } else { -1 } }

  after step 1:          -1 (½)                +1 (½)
  after step 2:    -2 (¼)         0 (¼ + ¼)          +2 (¼)      ← two paths reach 0: one world
  after step 3:  -3 (⅛)    -1 (⅜)          +1 (⅜)          +3 (⅛)
  ```

  A thousand steps have 2¹⁰⁰⁰ paths but only 1,001 possible positions, and positions are all the engine keeps. Many games behave the same way: the tree of possible games is astronomically large, but the set of distinct board states is small. Merging only helps when histories stop mattering, though: a model that keeps a whole hand of cards or a trajectory keeps its worlds apart.
- **Evidence re-weights worlds.** `observe` multiplies each world's weight by the probability of what was observed. Worlds that contradict it disappear.
- **Results are aggregated.** `report x` collects `x` from every world that reaches it and prints its weighted distribution.

## 2. Five rules

Everything else follows from these five rules.

1. **Branching follows every possibility.** `if c { A } else { B }` runs A in worlds weighted by the probability p that `c` holds, and B weighted by 1 − p. A condition can be a fact (`hp > 0`), a probability (`30%`) or an uncertain fact (`d20 + 5 >= 15`).
2. **A program is a set of weighted worlds.** Inside one world, every variable holds a single ordinary value. The uncertainty is in how many worlds there are and how much each one weighs.
3. **`~` settles a value; `=` keeps a distribution.** `let r ~ 2d6` gives `r` one number per world: a fact. `let d = 2d6` names the distribution itself, and every use of `d` is a fresh, independent roll.
4. **Identical worlds merge.** When branches rejoin, worlds with the same state combine, ignoring variables that will never be read again. Merging changes nothing but rounding. Draws of distributions written out, like `let pump ~ bernoulli(95%)`, are taken just before their first use, so drawing everything at the top of a model costs nothing.
5. **Output looks across worlds.** `report` prints distributions over all worlds, and `observe` conditions them on evidence. Code running inside a world only ever sees that world.

## 3. Syntax tour

Probl reads like a small modern scripting language: braces, no semicolons, `#` comments and optional type annotations.

### Literals

```probl
42   1_000_000   3.14   2.5e-3       # int and float
30%   12.5%   -5%                     # percentages: 30% is 0.3
"hello, {name}"                       # strings, with interpolation
true   false                          # facts
[1, 2, 3]                             # list
["sun": 70%, "rain": 30%]             # map;  [:] is the empty map
{ hp: 12, ac: 16 }                    # record
1..6   0..<n                          # ranges: both ends included / end excluded
d6   2d6   d20   3d8                  # dice: these are distributions
5 to 10                               # an estimate: 90% sure it's between 5 and 10
```

### Math

Math functions use radians: `sin(pi / 2)` is 1 and `atan2(1, -1)` is `3 * pi / 4`. The constants `pi`, `e` and `euler_gamma` are floats; they can be hidden by your own names. `euler_gamma` is the Euler–Mascheroni constant, distinct from Euler's number `e` and the `gamma(shape, scale)` distribution. Results are floating-point approximations, so test identities with a tolerance rather than exact equality.

```probl
@mode sample(runs: 1000, seed: 1)
let angle ~ uniform(-pi, pi)
report sin(angle)               # a settled angle, transformed in each run
report cos(d6)                 # a finite distribution, transformed outcome by outcome
```

Use `log1p(x)` for `ln(1 + x)` and `expm1(x)` for `exp(x) - 1` when `x` may be tiny, and `hypot(x, y)` for a distance without squaring large or tiny numbers. Arguments outside a function's domain, such as `asin(2)`, and overflowing results are errors. Continuous distributions must be drawn before applying these functions.

Integer mathematics stays exact: `choose`, `factorial`, `gcd`, `lcm` and `euler_phi` take ints and return ints, with an error if the result is too large. `factorial` supports 0 through 20; `ln_gamma(n + 1)` gives the logarithm of a larger factorial without forming it. `ln_gamma(x)` requires positive `x`. `erf(x)` is the error function used in normal probabilities.

```probl
report choose(52, 5)       # 2,598,960 five-card hands
report factorial(6)        # 720 orders of six items
report gcd(54, 24)         # 6
report lcm(6, 8)           # 24
report euler_phi(12)       # 4: the coprime integers are 1, 5, 7, 11
report ln_gamma(101)       # ln(100!), approximately 363.74
report erf(1)              # approximately 0.8427
```

### Variables

```probl
let price = 49              # immutable
var customers = 0           # mutable
customers += 120

let roll ~ 2d6              # a draw: one world per possible total
var market ~ one_of([Boom: 20%, Steady: 60%, Slump: 20%])
```

All values have **value semantics**, including lists, maps and records: assigning or passing one gives an independent copy. Copies are cheap because the collections are persistent data structures. Nothing is shared between worlds, which is what makes splitting and merging safe.

### Branching

```probl
if 30% {                               # splits 30% / 70%
  umbrella = true
}

let damage = if d20 + 5 >= 15 { 8 } else { 0 }    # `if` is also an expression

chance {                               # several weighted branches
  50% => pos += 1
  30% => pos -= 1
  else => {}                           # the remaining 20%: nothing happens
}

let weather = chance { 60% => "sun", 30% => "rain", else => "snow" }

match weather {
  "sun" => mood += 1
  "rain" | "snow" => mood -= 1
}
```

A condition can be any of these:

| Condition | Example | Effect on worlds |
|---|---|---|
| a fact | `hp > 0`, `true` | no split: it's true or false in each world |
| a probability | `30%`, `hit_chance` | splits in two: a fresh trial |
| an uncertain fact | `d20 + 5 >= 15` | splits in two, with p = 55% |

The last row matters. `if d20 + 5 >= 15` never draws the die, so it creates 2 worlds instead of 20. Use `let r ~ d20` only when you need the number itself afterwards.

The weights in a `chance` block can be any conditions, such as `P(2d6 == point)` or `hit - 5%`, and `else` takes whatever is left. Weights that add up to more than 100% are an error, and so is a `chance` used as a value whose weights leave something over with no `else` to take it.

### Loops

```probl
for month in 1..18 { … }
repeat 10 { … }
while hp > 0 and foe_hp > 0 { … }
loop { …; if done { break } }
```

A loop with an uncertain condition runs until every world has left it. Some loops never end with certainty: `while d6 != 6` could in principle roll forever.

- **Loops that come back are solved.** If the worlds come back to states they were in before, as a tennis game returns to deuce, Probl solves the loop exactly, as a Markov chain.
- **Other loops stop at ε.** A loop that counts its rounds never comes back to a state. Probl stops such a `while` or `loop` once the worlds still inside weigh less than ε times the weight that entered it (ε is 10⁻¹² by default; change it with `@epsilon 1e-9`). That remainder is reported as *unresolved* weight rather than silently dropped.

`for` and `repeat` always run to the end.

Recursion works the same way. A function can call itself, even with the same arguments, as in "roll again on a 6":

```probl
fn explode() -> int {
  let r ~ d6
  if r == 6 { return 6 + explode() }
  return r
}
report explode()          # mean 4.20 · … · a 7 is a 6 then a 1: 2.78%
```

Probl works out such a call by rounds, each using the previous round's result, until the weight still waiting is below ε. A call that can never return is an error.

```probl
var rolls = 1
while d6 != 6 {        # a fresh roll each time: 5/6 chance of going round again
  rolls += 1
}
report rolls           # mean 6.00 · sd 5.48 · 5% 1 · median 4 · 95% 17
```

### Functions

```probl
fn attack(a: Fighter, target: Fighter) -> int {
  let roll ~ d20
  if roll > 1 and roll + a.to_hit >= target.ac {
    let dmg ~ a.dice + a.bonus
    dmg                                   # the last expression is the result
  } else {
    0
  }
}

let doubled = [3, 5, 8].map(x -> x * 2)   # lambdas; x.f(y) is the same as f(x, y)
```

Functions can branch, draw and observe, so calling one can split the caller's world. A call doesn't normalize anything: the splits and observations inside a function become part of the caller's weights. There is one restriction: **a function can read anything in scope, but it can only assign its own local variables.** It returns whatever it changes. That keeps every function a *probabilistic function of its inputs*, so the engine can compute the distribution of `attack(hero, goblin)` once and reuse it in every world and every round. (A function that calls `print`, directly or not, runs every time instead: its output is debug output, one line per world that runs it.)

### Types

```probl
type Fighter = { hp: int, ac: int, to_hit: int, dice: dist[int], bonus: int }
enum Market { Boom, Steady, Slump }

let hero = Fighter { hp: 12, ac: 16, to_hit: 5, dice: 1d8, bonus: 3 }
let tougher = hero with { hp: 20 }       # a copy with some fields changed
```

The built-in types are `bool`, `int`, `float`, `prob`, `str`, `date`, `list[T]`, `map[K, V]`, `bag[T]` and `dist[T]`, plus records and enums. Note `dice: dist[int]`: distributions are ordinary values you can store, pass and return.

Probl is statically typed, with inference: every expression's type is known before the program runs, but you rarely write one. Annotations are optional, and checked when they're there: `let p: prob = "high"` is an error. Data read from a file is the exception, since nothing in the program says what the file contains, so for data the type is required. Until the type checker arrives in v0.3, annotations are checked as the program runs.

### Data

```probl
type Day = { day: date, visitors: int, signups: int }
let pilot: list[Day] = read("data/pilot.csv")      # next to the program
let counts: list[int] = read("-")                  # standard input, one value per line
```

`read` loads CSV (as a list of records), JSON (as any type data can have) and plain lines. The declared type decides how the data is read. Fields match columns and keys ignoring case and punctuation, and unused columns are ignored. Data that doesn't fit the type is an error before the program runs, with the line it's on. The data is read once, and is the same in every world and every run. `probl schema FILE` suggests a type. The details are in [reading data](data-input.md).

Three of these types describe uncertainty, and keeping them apart is what lets `and` and `or` mean what they say:

- `bool` is a **fact**: `true` or `false` in each world. Comparisons of settled values give facts, and so do `and`, `or`, `not` and `in` on facts.
- `prob` is a **probability**: a number from 0% to 100%, such as a success rate. It's a parameter, not an event, so branching on it is a fresh trial every time. Arithmetic on it (`p * 2`, `1 - p`) gives a `float`, and a float from 0 to 1 is accepted wherever a probability is expected.
- `dist[T]` is a **distribution**: `d6`, `bernoulli(30%)`, or `d6 > 4`, a `dist[bool]` that is an uncertain fact.

## 4. Distributions

### Making them

| Kind | Examples |
|---|---|
| Dice | `d6`, `2d6`, `d20 + 5`; `roll(4, d6)` gives the individual dice, sorted high to low |
| Choices | `one_of(["rock", "paper", "scissors"])`, `one_of(1..10)`, `one_of([Boom: 20%, Steady: 60%, Slump: 20%])` |
| Yes/no | `bernoulli(30%)`: `true` with probability 30%, so `let rain ~ bernoulli(30%)` is a fact |
| Counts | `binomial(n, p)`, `poisson(rate)`, `geometric(p)` |
| Cards | `bag([card: count, …])`, then `let c ~ deck.take()` draws without replacement |
| Continuous | `normal(mean, sd)`, `lognormal(mu, sigma)`, `uniform(lo, hi)`, `beta(a, b)`, `gamma(shape, scale)`, `exponential(rate)`, `triangular(lo, mode, hi)`, `pert(lo, mode, hi)` |
| Estimates | `5 to 10`: 90% confident it's between 5 and 10, as a lognormal, so both ends must be positive (as in Squiggle). For a quantity that can be negative, say which shape you mean: `normal_range(-8%, 0%)` is a normal with that 90% interval |
| From code | `simulate { … }`: the distribution of a block's result (section 7) |

`deck.take()` is shorthand for `one_of(deck)` followed by removing the drawn card from `deck` in that world.

Continuous distributions can't list their outcomes, so what you can do with them depends on the mode. Drawing one (`let x ~ normal(0, 1)`) needs sampling (section 8). Comparing one with a number works in both modes, from its CDF: `normal(0, 1) > 1.96` is 2.50%. `mean`, `sd`, `median`, `quantile`, `cdf` and `pdf` use the formulas. A choice among options that include one, such as `if 35% { 1 to 3 } else { 0 }`, is a mixture, and drawing from it picks an option first. Arithmetic on a continuous distribution (`normal(0, 1) * 2`) isn't supported yet: draw a value, then compute with it.

### Computing with them

Operators and built-in functions accept distributions and return new ones:

```probl
d20 + 5                             # 6 … 25
2d6 * 10                            # 20, 30, …, 120
roll(4, d6).highest(3).sum()        # "4d6, drop the lowest": mean 12.24
roll(5, d10).count(x -> x >= 8)     # successes in a dice pool: mean 1.5
```

**Every occurrence of a distribution is an independent draw:**

```probl
let die = d6
report die + die      # two independent rolls: 2 … 12, peaking at 7 (the same as 2d6)
report 2 * die        # one roll, doubled: 2, 4, …, 12
let r ~ d6
report r + r          # r is settled, so this is also 2, 4, …, 12
```

### Comparing them gives uncertain facts

```probl
2d6 >= 10             # 16.67%
d20 + 5 >= 15         # 55.00%
normal(0, 1) > 1.96   # 2.50%
2d6 > 2d6             # 44.37%: two independent rolls
```

Each of these is a `dist[bool]`: a fact that is true with some probability. `if` branches on it with that probability, and `report` prints it.

### Events need identities

`and`, `or` and `not` work on facts. Draw an event with `~` and it becomes a fact, true in some worlds and false in the others, and ordinary logic applies:

```probl
let rain ~ bernoulli(30%)     # a fact: true in 30% of worlds
let late ~ bernoulli(20%)
report rain and late          # 6%: two independent events
report rain and rain          # 30%: one event, checked twice
```

A probability has no identity, so `30% and 30%` is an error: nothing says whether that's one event checked twice (30%) or two independent ones (9%). For the same reason, `and` and `or` accept at most one uncertain fact: `(d6 > 3) and (d6 > 3)` is an error. Draw the events first (`let high ~ d6 > 3`), then combine the facts.

A probability that is itself uncertain stays a distribution. Here a success rate is 10% or 90%, equally likely, and two trials share it:

```probl
let rate ~ simulate { if 50% { 10% } else { 90% } }   # one rate per world
let a ~ bernoulli(rate)
let b ~ bernoulli(rate)
report a and b                                        # 41%: ½ × 0.1² + ½ × 0.9²
```

### Asking about them

`P(e)`, `mean(d)`, `sd(d)`, `median(d)`, `quantile(d, 0.9)`, `support(d)` and `cdf(d, x)` work on distribution *values*: `mean(1d8 + 3)` is 7.5. They never look across worlds. That's what `report` and `simulate` are for.

## 5. Evidence

```probl
let sick ~ bernoulli(1%)                      # 1 in 100 people has the disease
let positive = if sick { 95% } else { 8% }    # chance that their test comes back positive
observe positive                              # …and it did
report sick                                   # 10.71%
```

- `observe c` multiplies each world's weight by the probability that `c` holds. A fact such as `observe total == 7` deletes the worlds where it's false; a probability or an uncertain fact re-weights them. This is Bayes' rule, applied one world at a time.
- `observe v from D` multiplies by the probability of seeing the value `v` under `D`. For example: `observe 11 from binomial(250, rate)`. When sampling, a continuous `D` contributes its density at `v`.
- The **evidence** is the weight that survives: the probability of all the observations. Reports are normalized over it, and the run summary shows it (8.87% above). Observations inside a function count; those inside `simulate` don't (section 7).
- If the observations rule out every world, the evidence is impossible, and the run stops with an error rather than printing reports that mean nothing.

**No `observe` may follow a `report`.** The compiler rejects a program in which one could, including through a loop or a function that observes, so every report sees all the evidence. (Reports that update as evidence arrives, as in filtering, are future work.)

## 6. Reports

```probl
report win                                        # the label defaults to the source text
report rolls as "rolls per bet"
report customers * price by month as "MRR ($)"    # one distribution per month
```

What gets printed depends on the type:

| Reported value | Printed as |
|---|---|
| a fact (`bool` or `dist[bool]`) | one percentage: `win  49.29%` |
| number | `mean 3.38 · sd 3.00 · 5% 1 · median 2 · 95% 9`, plus a sparkline when there are few distinct values |
| date | `5% 2027-01-26 · median 2027-02-18 · 95% 2027-03-24` |
| anything else | each value with its probability: `sun 70.00% · rain 30.00%` |
| `… by key` | one row per key: a table, or a fan chart over time |

A report inside a branch describes only the worlds that pass through it, and says what share of the weight that was: `win  100.00% (reached in 1.00% of worlds)`. A report inside a loop needs `by`. When the key is the loop's variable (`for month in 1..18 { report mrr by month }`), each world reports once per key; with any other key, every visit counts, and the label says `(per visit)`. `report` is only allowed at the top level of a program, not inside functions or `simulate`. For debugging, `print(x)` prints once per world that runs it (worlds that have merged count once).

When a loop left some weight unresolved, a probability it could visibly change is printed as the range it must lie in, such as `0.00%–99.80%`; means and quantiles get a note instead, since a rare, very large value could move them anywhere. `--fractions` adds the simplest fraction near each probability: `49.29% (≈ 244/495)`. It's a hint for recognizing an answer, not a proof: weights are floating-point numbers.

When sampling, each probability comes with its standard error, rounded to its precision: `33.0% ± 0.2%`. A mean shows its standard error when it's visible at the printed precision (`mean 3.40 ± 0.01`); standard deviations and quantiles have none.

## 7. `simulate`: distributions from code

```probl
let damage = simulate { attack(hero, goblin) }    # a dist[int]
report mean(damage) as "damage per attack"        # 4.35

let game = simulate { race() }                    # turns that one player needs
let mine ~ game
report game >= mine as "first player wins"        # this `game` is a fresh race, independent of `mine`
```

`simulate { … }` runs its block as a separate model, starting from a copy of the current world, and returns the distribution of the block's final value. The current world doesn't split. The result is always a distribution, even a distribution of probabilities (see *Events need identities*). Observations inside the block condition its result and don't count as the program's evidence; if they rule out every world, that's an error. The block is always enumerated, even in sample mode, so its result is exact; sampling inside it isn't supported yet.

It connects Probl's two styles: the imperative one (branch, draw, update) and the distributional one (distributions as values). It also lets a strategy compare its options before acting:

```probl
let if_hit   = simulate { finish(hit(hand), dealer) }
let if_stand = simulate { finish(hand, dealer) }
if mean(if_hit) > mean(if_stand) { hand = hit(hand) }
```

## 8. Execution modes

All modes compute the same model; switching mode changes speed and accuracy, never meaning. `enumerate` and `sample` exist today. The others wait until their estimators are specified: merged samples need statistical bookkeeping, and dropped prior weight doesn't bound a posterior (see the [audit](project-audit.md), D2 and D4).

| Mode | How it runs | Use it for | Accuracy shown as |
|---|---|---|---|
| `enumerate` | follows every branch; identical worlds merge | dice, cards, boards, discrete models | rounding only; unresolved weight, with ranges for the probabilities it could change |
| `sample(runs: n, seed: s)` | each run takes one branch at random, and draws one value from each distribution; observations weight the runs | continuous estimates, very large models | standard errors; the effective sample size when observations weight the runs |
| `beam(worlds: n)` | like `enumerate`, but keeps only the n heaviest worlds | discrete models too big to enumerate | not built yet |
| `particles(runs: n, seed: s)` | like `sample`, and resamples the runs after observations | time series with streams of evidence | not built yet |
| `auto` (default) | `enumerate` for now; later, `sample` when a model can't be enumerated, saying so | getting started | as for the mode it picked |

Sampled runs are independent: they never merge, and every call makes its own choices. Evidence weights them (likelihood weighting); the **effective sample size** in the summary line says how many equally weighted runs they're worth, and when it's small, so is the confidence the estimates deserve. The same seed always gives the same output.

**Exact updates.** Some priors and observations fit together: a `beta` observed through `binomial` or `bernoulli`, a `gamma` through `poisson`, and a `normal` through `normal` with a known spread (they're *conjugate*). For such a variable, a sampled run doesn't draw it first and weight itself by the data. It keeps the variable's distribution, updates it exactly with each observation, and draws the variable from the result when something first needs its value. The model is the same, but the runs are worth more: in an A/B test with 30 days of data, 100,000 runs are worth 100,000 instead of 863, and the evidence is exact. `probl run --no-conjugate` turns it off, for comparing, and `probl run --stats` shows which variables were updated this way.

Only the observations before the variable's first use update it exactly. The ones after weight the runs as usual. If they disagree with the earlier ones, fewer runs can carry the weight than without exact updates, so use such a variable after all its observations when you can.

Choose a mode with a pragma at the top of a file, or override it on the command line:

```probl
@mode enumerate
@mode sample(runs: 100_000, seed: 42)
```
```sh
probl run model.probl --mode sample --runs 100000 --seed 3   # sample, whatever the program says
probl run model.probl --threads 2                            # sample on at most 2 cores
probl run model.probl --no-conjugate                         # sample without exact updates, for comparing
probl run model.probl --timeout 10                           # stop after 10 seconds
probl check model.probl                                      # parse and check without running
probl check --data model.probl                               # and check its data
probl schema data.csv                                        # suggest a type to read a file with
probl repl
```

The output always starts with a line saying how the numbers were computed, so an enumerated answer is never confused with an estimate:

```
enumerated · evidence 8.87% · unresolved < 1e-12
sample · 200,000 runs · seed 7 · evidence 3.60e-15 (± 0.00%) · effective sample size 200,000
```

When the program observes, the line gives the evidence: the probability of all the observations. Sampling estimates it, with its standard error, which is 0 when every observation is an exact update. When an observation uses a density (`observe 1.5 from normal(mu, 1)`), it gives the evidence's logarithm (`log evidence -1.825 ± 0.003`), which compares models: the difference of two models' log evidence on the same data is the logarithm of their Bayes factor.

Sampling uses every core by default. The runs go in batches of 1,000 with random numbers of their own, combined in order, so the output is the same on any number of cores.

Whoever runs a program sets limits on its worlds, work, loop iterations, call depth and output (`--max-worlds`, `--max-work`, `--timeout`). A program's `@max_worlds` and `@max_iterations` can lower these limits, never raise them.

## 9. Semantics in one table

For a single world with state σ and weight w (the full rules, including evaluation order, calls and reports, are in the [reference semantics](semantics.md)):

| Statement | Produces |
|---|---|
| `x = e` | (σ[x ↦ e], w) |
| `x ~ D` | one world per outcome v of D: (σ[x ↦ v], w · P(D = v)) |
| `if c { A } else { B }` | A run on (σ, w · p) and B run on (σ, w · (1 − p)) |
| `chance { p₁ => A₁ … else => B }` | each Aᵢ run on (σ, w · pᵢ), and B on (σ, w · (1 − Σpᵢ)) |
| `while c { A }` | exits with (σ, w · (1 − p)); A runs on (σ, w · p), then the loop repeats |
| `observe c` | (σ, w · p) |
| `observe v from D` | (σ, w · P(D = v)) |
| `A` followed by `B` | B runs on every world A produced |
| join point | (σ, w₁) and (σ, w₂) become (σ, w₁ + w₂) |

Here p is the probability that the condition holds in that world: 0% or 100% for a fact, anything in between for a probability or an uncertain fact.

This is the standard semantics of probabilistic programs as functions from a state to a distribution over states (Kozen, 1981). Merging changes nothing but rounding, because a set of worlds is a weighted sum of states, and equal states simply add. Liveness analysis only lets the engine forget variables that can no longer affect anything. Sample mode will estimate the same distribution: instead of splitting a weight, `if c` sends each run one way with probability p.

## 10. What the language protects you from

- **A distribution where a value is needed.** `for i in 1..d6` is an error: *"`d6` is a distribution, but a range needs a number. To use one roll, write `let n ~ d6` first."*
- **Drawing when a probability would do.** `let r ~ d20` followed only by `if r + 5 >= 15` creates 20 worlds where `if d20 + 5 >= 15` creates 2. A planned lint will suggest the shorter form when `r` isn't used again.
- **An event without an identity.** `storm and storm` is an error when `storm` is a probability, and so is `(d6 > 3) and (d6 > 3)`: is that one event or two? Draw it first with `let stormy ~ bernoulli(storm)`.
- **Matching a distribution.** `match d6 { … }` is an error: each arm would test a fresh roll. Draw the value first.
- **Evidence after a report**, or evidence that rules out every world: both are errors.
- **Continuous draws when enumerating.** `let x ~ normal(0, 1)` can't be enumerated, so the error suggests `@mode sample`. Comparisons such as `if normal(0, 1) > 1.96` can be enumerated, because they use the CDF. (Turning a continuous distribution into bins, `bins(d, 50)`, is planned.)
- **State that never merges.** Worlds merge only when their values are exactly equal. Floats that accumulate (`balance += 0.1`) rarely are, so float-heavy state stops merging and enumeration slows down. Count cents as integers, or `round()`. `probl run --stats` shows the peak number of worlds and how many calls were reused.
- **Loops that never finish.** Uncertain loops stop at ε and report the unresolved weight. A loop that keeps every world inside (`while true` with no `break`) hits an iteration cap and fails with a clear error.

## 11. Where Probl fits

| | Probl | AnyDice, Troll | Squiggle, Guesstimate | WebPPL, Pyro, Stan |
|---|---|---|---|---|
| Programs are | imperative code over weighted worlds | dice expressions and small functions | estimates combined by sampling | generative models for statistical inference |
| Every possibility followed, for discrete models | yes, merging equal states | yes, for dice | no, sampling | limited |
| State and loops | yes | limited | limited | yes |
| Conditioning on evidence | `observe` | not a focus | not a focus | yes, with advanced inference |
| Continuous quantities | yes, by sampling | no | yes | yes |

Probl borrows `a to b` estimates from Squiggle, dice notation from AnyDice and tabletop games, and `observe` from probabilistic programming languages like WebPPL. Weighted states, probabilistic branching and state merging all have prior art; Probl's bet is putting them behind ordinary imperative code, with diagnostics that say how an answer was computed. Merging keeps many game models small. A loop whose states come back is solved as a Markov chain, as PRISM does; the others are followed until the weight still playing is negligible. For exact inference at larger scale, the research language Dice (Holtzen et al., 2020) compiles programs to binary decision diagrams, which could become a later backend.

## 12. Examples

| File | Domain | What it shows |
|---|---|---|
| [`01_tour.probl`](../examples/01_tour.probl) | | every core idea on one page |
| [`02_craps.probl`](../examples/02_craps.probl) | game | draws, a loop with no fixed end, fractions |
| [`03_rpg_duel.probl`](../examples/03_rpg_duel.probl) | game | records, functions, distributions as fields, `simulate`, a balancing sweep |
| [`04_risk_battle.probl`](../examples/04_risk_battle.probl) | game | dice pools; merging collapses a huge tree |
| [`05_snakes_and_ladders.probl`](../examples/05_snakes_and_ladders.probl) | game | maps, long games, independent copies of a distribution |
| [`06_blackjack_dealer.probl`](../examples/06_blackjack_dealer.probl) | game | cards without replacement, tables with `report … by` |
| [`07_launch_forecast.probl`](../examples/07_launch_forecast.probl) | forecasting | `a to b` estimates, regime switching, a fan chart over months |
| [`08_signup_forecast.probl`](../examples/08_signup_forecast.probl) | forecasting | `observe … from`, learning a rate, then forecasting with it |
| [`09_roadmap.probl`](../examples/09_roadmap.probl) | forecasting | risks and dates: a forecast of this project's own plan |
| [`10_quantum_key.probl`](../examples/10_quantum_key.probl) | physics | quantum key distribution: a measurement as branching worlds, `observe` inside `simulate`, Bayes' rule for an eavesdropper |

Every example ends with the output it should produce, and those outputs are golden tests. The enumerated ones were checked against independent reference calculations, and must be printed exactly. The sampled ones come from an independent reference simulation, so the engine's numbers must agree with them within their sampling error: estimates within five standard errors, other numbers within 4%.

---

## Appendix A: Grammar

EBNF. `NEWLINE` ends a statement unless the line clearly continues: inside `( )` or `[ ]`, or after a binary operator, a comma or `=>`. Blank lines, and newlines right after `{` or right before `}`, are ignored. A `;` separates statements on the same line.

```ebnf
program      = { pragma } { item } ;
pragma       = "@" IDENT [ expr ] NEWLINE ;      (* @mode enumerate · @mode sample(runs: 1000) · @epsilon 1e-9 *)
item         = fn_decl | type_decl | enum_decl | import | stmt ;

fn_decl      = "fn" IDENT "(" [ param { "," param } ] ")" [ "->" type ] block ;
param        = IDENT [ ":" type ] ;
type_decl    = "type" IDENT "=" type ;
enum_decl    = "enum" IDENT "{" IDENT { "," IDENT } [ "," ] "}" ;
import       = "import" STRING ;
type         = IDENT [ "[" type { "," type } "]" ]            (* int, list[int], dist[int] *)
             | "{" IDENT ":" type { "," IDENT ":" type } [ "," ] "}" ;

block        = "{" { stmt } "}" ;
stmt         = binding | assign | loop | jump | observe | report | expr ;
binding      = ( "let" | "var" ) pattern [ ":" type ] ( "=" | "~" ) expr ;
assign       = place ( "=" | "~" | "+=" | "-=" | "*=" | "/=" ) expr ;
place        = IDENT { "." IDENT | "[" expr "]" } ;
loop         = "for" pattern "in" expr block | "while" expr block
             | "repeat" expr block | "loop" block ;
jump         = "break" | "continue" | "return" [ expr ] ;
observe      = "observe" expr [ "from" expr ] ;
report       = "report" expr [ "by" expr ] [ "as" STRING ] ;

expr         = lambda | or_expr ;
lambda       = ( IDENT | "(" [ IDENT { "," IDENT } ] ")" ) "->" expr ;
or_expr      = and_expr { "or" and_expr } ;
and_expr     = not_expr { "and" not_expr } ;
not_expr     = "not" not_expr | cmp_expr ;
cmp_expr     = range_expr [ cmp_op range_expr ] ;
cmp_op       = "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" | "not" "in" ;
range_expr   = add_expr [ ( ".." | "..<" | "to" ) add_expr ] ;
add_expr     = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr     = unary { ( "*" | "/" | "div" | "mod" ) unary } ;
unary        = "-" unary | power ;
power        = postfix [ "^" unary ] ;
postfix      = primary { "." IDENT [ call_args ] | call_args | "[" expr "]" | "with" record } ;
call_args    = "(" [ arg { "," arg } ] ")" ;
arg          = [ IDENT ":" ] expr ;

primary      = INT | FLOAT | PERCENT | DICE | STRING | "true" | "false"
             | IDENT [ record ]                              (* a variable, or Fighter { … } *)
             | "(" expr ")" | list | map | record | block
             | if_expr | chance_expr | match_expr | "simulate" block ;
if_expr      = "if" expr block [ "else" ( if_expr | block ) ] ;
chance_expr  = "chance" "{" arm { sep arm } [ sep ] "}" ;
arm          = ( expr | "else" ) "=>" ( block | stmt ) ;
match_expr   = "match" expr "{" match_arm { sep match_arm } [ sep ] "}" ;
match_arm    = pattern [ "if" expr ] "=>" ( block | stmt ) ;
list         = "[" [ expr { "," expr } [ "," ] ] "]" ;
map          = "[" ":" "]" | "[" expr ":" expr { "," expr ":" expr } [ "," ] "]" ;
record       = "{" field { "," field } [ "," ] "}" ;
field        = IDENT [ ":" expr ] ;                          (* { rounds } means { rounds: rounds } *)
pattern      = alt { "|" alt } ;
alt          = "_" | IDENT | literal | "[" [ pattern { "," pattern } ] "]" ;
literal      = INT | FLOAT | PERCENT | STRING | "true" | "false" ;
sep          = "," | NEWLINE ;
```

Lexical notes:

- `PERCENT` is a number immediately followed by `%` (`30%`, `12.5%`). Because `%` only ever means *percent*, remainder is spelled `mod`.
- `DICE` is `[count] "d" sides`, written with no spaces (`d6`, `2d6`, `10d10`). Names of that shape are reserved: `d2` can't be a variable.
- In the header of an `if`, `while`, `for` or `match`, a record literal needs parentheses, so `if x == (Point { x: 1, y: 2 }) { … }`, as in Rust.
- `{` starts a record when it's followed by `IDENT :` or `IDENT ,`. Otherwise it starts a block.
- In a pattern, a name that refers to an enum variant (`Boom`) matches that variant. Any other name binds a new variable.

## Appendix B: Operator precedence

From loosest to tightest binding:

| Level | Operators | Associativity |
|---|---|---|
| 1 | `x -> e` (lambda) | right |
| 2 | `or` | left |
| 3 | `and` | left |
| 4 | `not` | prefix |
| 5 | `==` `!=` `<` `<=` `>` `>=` `in` `not in` | none |
| 6 | `..` `..<` `to` | none |
| 7 | `+` `-` | left |
| 8 | `*` `/` `div` `mod` | left |
| 9 | `-` (negation) | prefix |
| 10 | `^` | right |
| 11 | call `f(x)`, index `a[i]`, field and method `a.b`, `with { … }` | left |

`/` always divides as floats and `div` is integer division, so `7 / 2` is 3.5 and `7 div 2` is 3.

## Appendix C: Keywords

`and` `break` `chance` `continue` `div` `else` `enum` `false` `fn` `for` `if` `import` `in` `let` `loop` `match` `mod` `not` `observe` `or` `repeat` `report` `return` `simulate` `true` `type` `var` `while` `with`

`as`, `by`, `from` and `to` are keywords only where the grammar uses them, so they remain usable as variable names.

## Appendix D: Standard library (initial)

| Area | Functions |
|---|---|
| Distributions | `bernoulli` `one_of` `binomial` `poisson` `geometric` `normal` `lognormal` `normal_range` `uniform` `beta` `gamma` `exponential` `triangular` `pert` `mixture`\* `roll` `bag` |
| Distribution helpers | `take` `truncate(d, lo, hi)`\* `bins(d, n)`\* |
| Queries | `P` `mean` `sd` `variance` `median` `quantile` `support` `cdf` `pmf` `pdf` |
| Probability | `odds(p)` `logit(p)` `inv_logit(x)` |
| Math | `abs` `min` `max` `clamp` `floor` `ceil` `round` `sqrt` `hypot(x, y)` `exp` `expm1` `ln` `log1p` `log2` `log10` `sin` `cos` `tan` `asin` `acos` `atan` `atan2(y, x)` `sinh` `cosh` `tanh` |
| Integer and special functions | `choose(n, k)` `factorial(n)` `gcd(a, b)` `lcm(a, b)` `euler_phi(n)` `ln_gamma(x)` `erf(x)` |
| Constants | `pi` `e` `euler_gamma`; a variable, variant or function of the program's with the same name hides one, so `let e = 5` still works |
| Collections | `len` `push` `pop` `insert` `remove` `get(key, default)` `keys` `values` `map` `filter` `reduce` `sum` `count` `highest(n)` `lowest(n)` `sort` `sort_desc` `reverse` `enumerate` `zip` |
| Text | `str` `upper` `lower` `split` `join` |
| Dates | `date("2027-01-31")` `today()`\* `days(n)` `weeks(n)` `add_workdays(d, n)` `weekday(d)`; dates can be compared, and adding or subtracting them works in days |
| Debugging | `print` |

\* Planned, not built yet.
