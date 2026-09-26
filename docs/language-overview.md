# Probl: language overview

> Draft 0.1, September 2026. This is a proposal. Nothing is implemented yet, and any decision here can still change.
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

- **Game simulation**: dice, cards, boards and fights, with exact odds. *What's the chance this attack takes the territory? Does going first matter? How many hit points make this fight fair?*
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

  A thousand steps have 2¹⁰⁰⁰ paths but only 1,001 possible positions, and positions are all the engine keeps. Games behave the same way: the tree of possible games is astronomically large, but the set of distinct board states is small.
- **Evidence re-weights worlds.** `observe` multiplies each world's weight by the probability of what was observed. Worlds that contradict it disappear.
- **Results are aggregated.** `report x` collects `x` from every world and prints its weighted distribution.

## 2. Five rules

Everything else follows from these five rules.

1. **A condition is a probability.** `true` and `false` are simply 100% and 0%. `if p { A } else { B }` runs A with weight p and B with weight 1 − p.
2. **A program is a set of weighted worlds.** Inside one world, every variable holds a single ordinary value. The uncertainty is in how many worlds there are and how much each one weighs.
3. **`~` settles a value; `=` keeps a distribution.** `let r ~ 2d6` gives `r` one number per world. `let d = 2d6` names the distribution itself, and every use of `d` is a fresh, independent roll.
4. **Identical worlds merge.** When branches rejoin, worlds with the same state combine, ignoring variables that will never be read again. Merging is exact, not an approximation.
5. **Output looks across worlds.** `report` prints distributions over all worlds, and `observe` conditions them on evidence. Code running inside a world only ever sees that world.

## 3. Syntax tour

Probl reads like a small modern scripting language: braces, no semicolons, `#` comments and optional type annotations.

### Literals

```probl
42   1_000_000   3.14   2.5e-3       # int and float
30%   12.5%   -5%                     # percentages: 30% is 0.3
"hello, {name}"                       # strings, with interpolation
true   false                          # the same as 100% and 0%
[1, 2, 3]                             # list
["sun": 70%, "rain": 30%]             # map;  [:] is the empty map
{ hp: 12, ac: 16 }                    # record
1..6   0..<n                          # ranges: both ends included / end excluded
d6   2d6   d20   3d8                  # dice: these are distributions
5 to 10                               # an estimate: 90% sure it's between 5 and 10
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
| a certain comparison | `hp > 0` | no split: it is 0% or 100% |
| a probability | `30%`, `hit_chance` | splits in two |
| a comparison involving a distribution | `d20 + 5 >= 15` | splits in two, with p = 55% |

The last row matters. `if d20 + 5 >= 15` never draws the die, so it creates 2 worlds instead of 20. Use `let r ~ d20` only when you need the number itself afterwards.

The weights in a `chance` block can be any probability expressions, such as `P(2d6 == point)` or `hit - 5%`, and `else` takes whatever is left. Weights that add up to more than 100% are an error.

### Loops

```probl
for month in 1..18 { … }
repeat 10 { … }
while hp > 0 and foe_hp > 0 { … }
loop { …; if done { break } }
```

A loop with an uncertain condition runs until every world has left it. Some loops never end with certainty (`while d6 != 6` could in principle roll forever), so Probl stops once the worlds still inside weigh less than ε (10⁻¹² by default; change it with `@epsilon 1e-9`). That remainder is reported as *unresolved* probability rather than silently dropped.

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

Functions can branch and draw, so calling one can split the caller's world. There is one restriction: **a function can read anything in scope, but it can only assign its own local variables.** It returns whatever it changes. That keeps every function a pure *probabilistic function of its inputs*, so the engine can compute the distribution of `attack(hero, goblin)` once and reuse it in every world and every round.

### Types

```probl
type Fighter = { hp: int, ac: int, to_hit: int, dice: dist[int], bonus: int }
enum Market { Boom, Steady, Slump }

let hero = Fighter { hp: 12, ac: 16, to_hit: 5, dice: 1d8, bonus: 3 }
let tougher = hero with { hp: 20 }       # a copy with some fields changed
```

The built-in types are `int`, `float`, `prob`, `str`, `date`, `list[T]`, `map[K, V]`, `bag[T]` and `dist[T]`, plus records and enums. Annotations are optional. Note `dice: dist[int]`: distributions are ordinary values you can store, pass and return.

`prob` is the type of conditions. Its values are percentages between 0% and 100%, `true` and `false`, and the results of comparisons and of `and`, `or` and `not`. Arithmetic on a probability (`p * 2`, `1 - p`) gives a plain `float`. A float between 0 and 1 is accepted wherever a probability is expected.

## 4. Distributions

### Making them

| Kind | Examples |
|---|---|
| Dice | `d6`, `2d6`, `d20 + 5`; `roll(4, d6)` gives the individual dice, sorted high to low |
| Choices | `one_of(["rock", "paper", "scissors"])`, `one_of(1..10)`, `one_of([Boom: 20%, Steady: 60%, Slump: 20%])` |
| Yes/no | any probability: `let rain ~ 30%` |
| Counts | `binomial(n, p)`, `poisson(rate)`, `geometric(p)` |
| Cards | `bag([card: count, …])`, then `let c ~ deck.take()` draws without replacement |
| Continuous | `normal(mean, sd)`, `lognormal(mu, sigma)`, `uniform(lo, hi)`, `beta(a, b)`, `gamma(shape, scale)`, `exponential(rate)`, `triangular(lo, mode, hi)`, `pert(lo, mode, hi)` |
| Estimates | `5 to 10`: 90% confident it's between 5 and 10. Lognormal when both ends are positive, normal otherwise (as in Squiggle). |
| From code | `simulate { … }`: the distribution of a block's result (section 7) |

`deck.take()` is shorthand for `one_of(deck)` followed by removing the drawn card from `deck` in that world.

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

### Comparing them gives probabilities

```probl
2d6 >= 10             # 16.67%
d20 + 5 >= 15         # 55.00%
normal(0, 1) > 1.96   # 2.50%
2d6 > 2d6             # 44.37%: two independent rolls
```

This extends the core idea beyond `if 30%`: a condition on an uncertain value is simply a probability, and `if` branches on it.

### Chances and facts

A probability kept with `=` is a *chance*: each time you branch on it, it's a new trial. Settling it with `~` gives a *fact*, which in every world is simply true or false:

```probl
let rain = 30%
report rain and rain          # 9%: two independent 30% events
let raining ~ rain
report raining and raining    # 30%: one fact, checked twice
```

On chances, `p and q` is p·q, `p or q` is 1 − (1 − p)(1 − q) and `not p` is 1 − p, the rules for independent events. On facts they reduce to ordinary boolean logic.

### Asking about them

`P(e)`, `mean(d)`, `sd(d)`, `median(d)`, `quantile(d, 0.9)`, `support(d)` and `cdf(d, x)` work on distribution *values*: `mean(1d8 + 3)` is 7.5. They never look across worlds. That's what `report` and `simulate` are for.

## 5. Evidence

```probl
let sick ~ 1%                                 # 1 in 100 people has the disease
let positive = if sick { 95% } else { 8% }    # chance that their test comes back positive
observe positive                              # …and it did
report sick                                   # 10.71%
```

- `observe p` multiplies each world's weight by p. A certain condition such as `observe total == 7` deletes the worlds where it's false. An uncertain one re-weights them. This is Bayes' rule, applied one world at a time.
- `observe v from D` multiplies by the probability of seeing the value `v` under `D` (the density, if `D` is continuous). For example: `observe 11 from binomial(250, rate)`.
- Reports are normalized over the weight that survives, and the run summary shows the probability of the evidence itself (8.87% above).

A report is a **snapshot**: it reflects the evidence observed before it runs. Put reports after your `observe` statements; the checker warns when a report is followed by an `observe` it can't see.

## 6. Reports

```probl
report win                                        # the label defaults to the source text
report rolls as "rolls per bet"
report customers * price by month as "MRR ($)"    # one distribution per month
```

What gets printed depends on the type:

| Reported value | Printed as |
|---|---|
| `prob` | one percentage: `win  49.29%` |
| number | `mean 3.38 · sd 3.00 · 5% 1 · median 2 · 95% 9`, plus a sparkline when there are few distinct values |
| date | `5% 2027-01-26 · median 2027-02-18 · 95% 2027-03-24` |
| anything else | each value with its probability: `sun 70.00% · rain 30.00%` |
| `… by key` | one row per key: a table, or a fan chart over time |

A report inside a branch describes only the worlds that pass through it, and says what share of the weight that was. `report` is only allowed at the top level of a program, not inside functions or `simulate`. For debugging, `print(x)` prints once per world, prefixed with the world's weight.

In sample modes each probability comes with a standard error (`33.0% ± 0.2%`). In exact mode, `--fractions` prints exact rationals instead (`244/495`).

## 7. `simulate`: distributions from code

```probl
let damage = simulate { attack(hero, goblin) }    # a dist[int]
report mean(damage) as "damage per attack"        # 4.35

let game = simulate { race() }                    # turns that one player needs
let mine ~ game
report game >= mine as "first player wins"        # this `game` is a fresh race, independent of `mine`
```

`simulate { … }` runs its block as a separate simulation starting from the current world, and returns the distribution of the block's final value. The current world does not split. If the block's value is a probability, the result is a probability.

It connects Probl's two styles: the imperative one (branch, draw, update) and the distributional one (distributions as values). It also lets a strategy compare its options before acting:

```probl
let if_hit   = simulate { finish(hit(hand), dealer) }
let if_stand = simulate { finish(hand, dealer) }
if mean(if_hit) > mean(if_stand) { hand = hit(hand) }
```

## 8. Execution modes

All modes share the semantics above. Switching mode changes speed and accuracy, never meaning.

| Mode | How it runs | Use it for | Accuracy shown as |
|---|---|---|---|
| `exact` | follows every branch; identical worlds merge | dice, cards, boards, discrete models | exact, ± unresolved mass |
| `beam(worlds: n)` | like `exact`, but keeps only the n heaviest worlds | discrete models too big for `exact` | bounds from the pruned mass |
| `sample(runs: n, seed: s)` | each run takes one branch at random; runs that reach the same state still merge | continuous estimates, very large models | standard errors |
| `particles(runs: n, seed: s)` | like `sample`, and resamples the runs after each `observe` | time series with streams of evidence | standard errors, effective sample size |
| `auto` (default) | `exact` if every draw is discrete and the world count stays under a budget, otherwise `sample`, and it says which | getting started | as for the mode it picked |

Choose a mode with a pragma at the top of a file, or override it on the command line:

```probl
@mode exact
@mode sample(runs: 100_000, seed: 42)
```
```sh
probl run model.probl --mode sample --runs 1000000
probl check model.probl          # parse, resolve and lint without running
probl repl                       # the prompt shows how many worlds are live
```

The output always starts with a line saying how the numbers were computed, so an exact answer is never confused with an estimate:

```
exact · unresolved < 1e-12
sample · 50,000 runs · seed 11 · effective sample size 50,000
```

## 9. Semantics in one table

For a single world with state σ and weight w:

| Statement | Produces |
|---|---|
| `x = e` | (σ[x ↦ e], w) |
| `x ~ D` | one world per outcome v of D: (σ[x ↦ v], w · P(D = v)) |
| `if p { A } else { B }` | A run on (σ, w · p) and B run on (σ, w · (1 − p)) |
| `chance { p₁ => A₁ … else => B }` | each Aᵢ run on (σ, w · pᵢ), and B on (σ, w · (1 − Σpᵢ)) |
| `while p { A }` | exits with (σ, w · (1 − p)); A runs on (σ, w · p), then the loop repeats |
| `observe p` | (σ, w · p) |
| `observe v from D` | (σ, w · P(D = v)), or the density of D at v if D is continuous |
| `A` followed by `B` | B runs on every world A produced |
| join point | (σ, w₁) and (σ, w₂) become (σ, w₁ + w₂) |

Here p is the value of the condition in that world: 0% or 100% for a certain comparison, anything in between for a chance or a comparison involving distributions.

This is the standard semantics of probabilistic programs as functions from a state to a distribution over states (Kozen, 1981). Merging is exact because a distribution is a weighted sum of states, and equal states simply add. Liveness analysis only lets the engine forget variables that can no longer affect anything. Sample mode estimates the same distribution: instead of splitting a weight, `if p` sends each run one way with probability p.

## 10. What the language protects you from

- **A distribution where a value is needed.** `for i in 1..d6` is an error: *"`d6` is a distribution, but a range needs a number. To use one roll, write `let n ~ d6` first."*
- **Drawing when a probability would do.** `let r ~ d20` followed only by `if r + 5 >= 15` creates 20 worlds where `if d20 + 5 >= 15` creates 2. The linter suggests the shorter form when `r` isn't used again.
- **One chance, tested twice.** `if storm and storm` gets a warning: those are two independent trials of the same chance. If it's one event, settle it first with `let stormy ~ storm`.
- **Continuous draws in exact mode.** `let x ~ normal(0, 1)` can't be enumerated, so the error suggests `@mode sample` or `normal(0, 1).bins(50)`. Comparisons such as `if normal(0, 1) > 1.96` are fine in exact mode, because they use the CDF.
- **State that never merges.** Worlds merge only when their values are exactly equal. Floats that accumulate (`balance += 0.1`) rarely are, so float-heavy state stops merging and exact mode slows down. Count cents as integers, or `round()`. `probl run --stats` shows which variables keep worlds apart.
- **Reports before evidence.** A report that runs before an `observe` can't see it, and the checker warns about it.
- **Loops that never finish.** Uncertain loops stop at ε and report the unresolved mass. A loop that keeps every world inside (`while true` with no `break`) hits an iteration cap and fails with a clear error.

## 11. Where Probl fits

| | Probl | AnyDice, Troll | Squiggle, Guesstimate | WebPPL, Pyro, Stan |
|---|---|---|---|---|
| Programs are | imperative code over weighted worlds | dice expressions and small functions | estimates combined by sampling | generative models for statistical inference |
| Exact answers for discrete models | yes, with state merging | yes, for dice | no, sampling | limited |
| State and loops | yes | limited | limited | yes |
| Conditioning on evidence | `observe` | not a focus | not a focus | yes, with advanced inference |
| Continuous quantities | yes, by sampling | no | yes | yes |

Probl borrows `a to b` estimates from Squiggle, dice notation from AnyDice and tabletop games, and `observe` from probabilistic programming languages like WebPPL. Its distinctive bet is the execution model: follow every branch and merge identical states. Games are Markov chains in disguise, so this gives exact answers from ordinary imperative code. For exact inference at larger scale, the research language Dice (Holtzen et al., 2020) compiles programs to binary decision diagrams, which could become a later backend.

## 12. Examples

| File | Domain | What it shows |
|---|---|---|
| [`01_tour.probl`](../examples/01_tour.probl) | | every core idea on one page |
| [`02_craps.probl`](../examples/02_craps.probl) | game | draws, a loop with no fixed end, exact fractions |
| [`03_rpg_duel.probl`](../examples/03_rpg_duel.probl) | game | records, functions, distributions as fields, `simulate`, a balancing sweep |
| [`04_risk_battle.probl`](../examples/04_risk_battle.probl) | game | dice pools; merging collapses a huge tree |
| [`05_snakes_and_ladders.probl`](../examples/05_snakes_and_ladders.probl) | game | maps, long games, independent copies of a distribution |
| [`06_blackjack_dealer.probl`](../examples/06_blackjack_dealer.probl) | game | cards without replacement, tables with `report … by` |
| [`07_launch_forecast.probl`](../examples/07_launch_forecast.probl) | forecasting | `a to b` estimates, regime switching, a fan chart over months |
| [`08_signup_forecast.probl`](../examples/08_signup_forecast.probl) | forecasting | `observe … from`, learning a rate, then forecasting with it |
| [`09_roadmap.probl`](../examples/09_roadmap.probl) | forecasting | risks and dates: a forecast of this project's own plan |

Every example ends with the output it should produce. The exact ones were checked against independent reference calculations; the sampled ones come from a reference simulation, so their last digits will differ. These outputs become the first golden tests.

---

## Appendix A: Grammar

EBNF. `NEWLINE` ends a statement unless the line clearly continues: inside `( )` or `[ ]`, or after a binary operator, a comma or `=>`. Blank lines, and newlines right after `{` or right before `}`, are ignored. A `;` separates statements on the same line.

```ebnf
program      = { pragma } { item } ;
pragma       = "@" IDENT [ expr ] NEWLINE ;      (* @mode exact · @mode sample(runs: 1000) · @epsilon 1e-9 *)
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
| Distributions | `one_of` `binomial` `poisson` `geometric` `normal` `lognormal` `uniform` `beta` `gamma` `exponential` `triangular` `pert` `mixture` `roll` `bag` |
| Distribution helpers | `take` `truncate(d, lo, hi)` `bins(d, n)` |
| Queries | `P` `mean` `sd` `variance` `median` `quantile` `support` `cdf` `pmf` `pdf` |
| Probability | `odds(p)` `logit(p)` `inv_logit(x)` |
| Math | `abs` `min` `max` `clamp` `floor` `ceil` `round` `sqrt` `exp` `ln` `log10` |
| Collections | `len` `push` `pop` `insert` `remove` `get(key, default)` `keys` `values` `map` `filter` `reduce` `sum` `count` `highest(n)` `lowest(n)` `sort` `sort_desc` `reverse` `enumerate` `zip` |
| Text | `str` `upper` `lower` `split` `join` |
| Dates | `date("2027-01-31")` `today()` `days(n)` `weeks(n)` `add_workdays(d, n)` `weekday(d)`; dates can be compared, and adding or subtracting them works in days |
| Debugging | `print` |
