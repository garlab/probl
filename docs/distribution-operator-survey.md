# Distribution operator API survey

Reviewed 1 October 2026 against `92b11a6`. This is a design survey, not a change to the language reference or runtime. It covers all 18 examples, the 133 public built-ins, operators, distribution consumption, and the compiler boundaries that would enforce the proposed rules.

**The proposed separation is viable for every existing example.** Twelve need only draw-syntax migration; six also need expression changes under the candidate rules below. Executable equivalents of those changes produce identical CLI output to the originals, including the seeded sampled examples. The main implementation prerequisite is static checking: the current compiler cannot generally reject distribution/outcome mismatches before execution. The remaining design decisions concern which operations explicitly accept distributions, transformations of joint outcomes, and the supported scope of continuous distribution algebra.

`DRAW(D)` below is metanotation for the proposed operator. It does not select a name, spelling, precedence, or callable-distribution design. The accompanying [verification script](distribution-operator-survey.py) uses today's `~` bindings to execute equivalent models. It does **not** implement or validate a new parser or type checker.

## Rules used for the survey

The agreed direction is ordinary assignment, an explicit distribution-to-outcome operator, distribution–distribution addition, and rejection of mixed distribution/outcome arithmetic. For the survey, a literal such as `3` and a previously drawn integer are both ordinary `int` values; neither is implicitly promoted to a distribution.

The following additional rules are **candidate recommendations**, not decisions already made:

| Operation | Candidate rule |
|---|---|
| `let d = D`, passing or returning `D` | Store or copy a distribution without drawing. |
| `DRAW(D)` | Require a top-level distribution and return one outcome per world. Do not silently accept a plain value or recursively draw fields and elements. |
| `D + E` | Compose two finite numeric distributions through independent outcome combinations, returning a distribution. Other distribution algebra operators need explicit signatures. |
| `D + x`, `x + D` | Reject, including literals, math constants, and analytical query results. |
| `D > E` | If retained, return `dist[bool]`; never a scalar condition. Keeping this overload is a separate choice from keeping addition. |
| `if`, `while`, match guards, `observe`, `chance` weights | Reject a bare `dist[bool]`. Continue accepting the existing scalar facts and probability weights. Use the operator for a fact or an explicit probability query for a likelihood. |
| Ordinary math, text, date and collection operations | Require their documented value types, unless an explicit distribution overload has been chosen. Remove blanket automatic lifting. |
| `mean(D)`, `cdf(D, x)`, `observe x from D`, `report D` | Keep explicit distribution consumers. Their different argument roles are intentional and do not constitute mixed arithmetic. |
| Ordinary functions containing draws or branches | Continue producing outcomes in the caller's worlds. Returning a distribution object does not itself draw it. |
| `simulate { ... }` | Keep a local inference boundary that returns a distribution and retains today's finite-enumeration limitation. |

The implementation only distinguishes a value in the current world from a distribution recipe; it need not track whether every scalar originally came from a draw. A rule allowing `D + 3` but rejecting `D + drawn_integer` would introduce a different dependency/provenance system and would be sensitive to refactoring.

## Distribution algebra and conditions

Finite `D + D` is already supported by the engine's Cartesian-product evaluation in [ops.rs](../crates/probl-engine/src/ops.rs). It requires no new inference algorithm. The two uses are independent **conditional on the current world's parameters**; sharing a drawn parameter can still induce dependence across worlds.

This deliberately retains an important distinction:

```probl
# Proposed operator metanotation
let d = d6
report d + d             # sum of independent rolls
report d - d             # not always zero
report d == d            # 1/6 if outcome-comparison overload is retained

let x = DRAW(d)
report x - x             # zero
report x == x            # true
```

`D == E` currently compares independently chosen outcomes; it does not compare distribution objects structurally. Decide whether to keep that meaning, disallow these comparisons, or provide separate distribution equality before users rely on it. Compiler simplifications such as `d - d` to zero would be invalid. Boolean `and`/`or` should initially remain operations on drawn facts: extending them to independent distribution algebra would need a separate decision, including short-circuit effects.

For explicit point distributions, the existing `one_of([3])` already works:

```probl
let damage = 2d6 + one_of([3])
report mean(damage)
```

A shorter point-distribution constructor would improve ergonomics but is not needed to migrate the examples. It must have a defined policy for a distribution-valued argument: `one_of([D])` currently flattens to `D`, so it cannot preserve `D` as a nested distribution outcome.

Rejecting only mixed arithmetic is insufficient to prevent the observation trap. This currently succeeds:

```probl
let e = d6 > d6
observe e
report e
```

Both the evidence and reported probability are 41.67%. The comparison created a recipe, and observation did not bind its outcome. Under the candidate rules, `observe e` is a type error; bind `let event = DRAW(e)` before observing and reusing it. `observe P(e)` can remain a deliberate likelihood calculation, without changing `e`.

Keeping `if 30%` preserves Probl's probabilistic branching. The restriction concerns implicitly consuming distribution objects, not eliminating every probabilistic control-flow construct.

## Survey of the public API

The [built-in catalog](../crates/probl-sema/src/builtins.rs) marks **117 of 133 public built-ins as `Lift` and 16 as `Raw`**. Some marked `Lift` use special interpreter paths, particularly collection callbacks. Therefore neither flag is a sufficient type contract for the redesign. The implementation needs argument roles and chosen overloads, rather than a universal rule that lifts any distribution argument.

| API family | Current behavior | Candidate treatment and impact |
|---|---|---|
| Numeric, trigonometric, complex, integer and bit functions; `min`, `max`, `clamp`, rounding; probability conversions | Automatically apply to finite outcomes, with independent combinations for multiple distribution arguments. | Keep scalar signatures. Decide explicitly which, if any, receive distribution overloads. `abs(D)` and `round(D, 2)` are not settled by the agreement on `D + E`. |
| Text functions, string conversion, date construction and manipulation | The same lifting mechanism applies, including scalar options such as delimiters, digit counts, dates and holiday lists. | Ordinary string/date operations remain unchanged for outcomes. Transform a whole distribution explicitly when needed. Rejecting every function call containing both a distribution and a scalar would wrongly reject legitimate typed APIs such as `cdf`. |
| `bernoulli`, `binomial`, `poisson`, `geometric`, `normal`, `lognormal`, `uniform`, `beta`, `gamma`, `exponential`, `triangular`, `pert`, `normal_range`, `a to b` | Construct distributions; finite distribution parameters are automatically lifted and resulting distributions flattened. | Require scalar parameters, which may be drawn values in the current world. Shared latent parameters must be drawn once. Explicit mixture construction can cover marginalization. |
| `one_of` | Accepts lists, ranges, weights or a bag. Distribution-valued options are flattened into a mixture; scalar options become point components. | Preserve or refine this as an explicit mixture/choice constructor, with a documented signature. Do not confuse explicit mixture construction with automatic arithmetic promotion. |
| `roll(n, D)` | Builds a finite distribution of dice pools sorted highest first; also accepts an integer number of sides. It does not bind an outcome. | Preserve its distribution-producing role until naming is decided. `DRAW(roll(n, D))` binds the whole pool. This is the only existing built-in name collision if the new operator is called `roll`. |
| `bag`, `take`, collection mutation | A bag is a collection. `deck.take()` selects and removes one item atomically, returning it unchanged; write `let card = deck.take()`. | Implemented as an effectful method on a mutable receiver. An outer `~` draws from the returned item, including probability or distribution items. |
| `P`, `mean`, `sd`, `variance`, `median`, `quantile`, `support`, `cdf`, `pmf`, `pdf` | Explicit distribution queries. Several also silently turn a scalar into a point distribution; `P` additionally accepts facts/probabilities. | Keep distribution query signatures and scalar query parameters. Decide scalar overloads explicitly. Prefer rejecting `mean(x)`/`sd(x)` when `x` is a scalar so they cannot be mistaken for aggregation over worlds. |
| `len`, `slice`, `sum`, one-argument `count`, sorting, reversing, keys/values, `get`, `contains`, extrema, `enumerate`, `zip` | Accept ordinary collections; most also lift over distributions of collections. `sum([d6, d6])` additionally builds a sum distribution through element arithmetic. | Distinguish `list[T]`, `list[dist[T]]`, and `dist[list[T]]`. Choose explicit overloads or require transformation. `sum(list[dist[number]])` can be retained as deliberate distribution algebra, but its current scalar-zero accumulator must change. |
| `map`, `filter`, `reduce`, predicate `count` | Map over each collection outcome when the input is `dist[collection]`. Callbacks cannot draw, observe or branch probabilistically outside local `simulate`. | Keep ordinary collection callbacks subject to their effect rule. A separate operation for transforming distribution outcomes needs its own contract. Existing `map` does not provide that operation for `dist[int]`. |
| `push`, `insert`, `remove`, `pop`; field/index/update syntax | Mixture of ordinary persistent-value updates, receiver lifting and expression lowering. | Preserve ordinary collections containing distribution objects as data. Decide distribution-receiver transforms explicitly; do not recursively draw or update distributions by accident. |
| `print`, interpolation, `str` | `print` and interpolation describe values without drawing; `str` currently lifts over finite distributions. | Distinguish displaying a recipe from transforming its outcomes into strings. Keep debugging nonprobabilistic. |
| `mixture`, `truncate`, `bins` | Names and documentation exist, but calls report “not implemented”. | They are unavailable as migration solutions today. In particular, the existing `truncate` stub cannot yet replace the user's conditional-dice example. |

`read`, `today`, numeric constants, enum variants, and ordinary immutable data retain their roles. Data declarations still produce values, not new choices. A constant appearing alongside a distribution in arithmetic follows the same explicit-conversion rule as any other scalar.

### Constructors can hide loss of dependence

The following current programs differ:

```probl
let p = one_of([10%, 90%])
let d = bernoulli(p)                 # automatically marginalizes p
report simulate { let a ~ d; let b ~ d; a and b }  # 25%
```

```probl
let p ~ one_of([10%, 90%])           # one shared parameter per world
let d = bernoulli(p)
report simulate { let a ~ d; let b ~ d; a and b }  # 41%
```

Requiring scalar constructor parameters catches the first form and makes its author choose whether the rate should be shared. All existing hierarchical forecasting examples already draw their parameters and fit that rule.

### Joint transformations need an explicit path

The existing `.map` maps collection elements. `d6.map(x -> x + 1)` currently fails because each outcome is an integer, not a collection. It cannot simply be advertised as the replacement for lifting.

Projection is also consequential:

```probl
let pair = simulate { let x ~ d2; { a: x, b: x } }
report pair.a == pair.b              # currently 50%: independent marginals
report simulate {
    let p ~ pair
    p.a == p.b
}                                   # 100%: one joint outcome
```

The candidate design should require a draw before ordinary record-field access on a distribution, or provide a visibly explicit projection/transform operation. Accessing `fighter.dice` remains ordinary field access because `fighter` is a record whose field stores a distribution; it is not a distribution of records.

For finite distributions, `simulate` already expresses a transformation without splitting the outer model:

```probl
# Proposed operator metanotation
let excess = simulate {
    let loss = DRAW(loss_distribution)
    max(loss - 100, 0)
}
report mean(excess)
```

A dedicated finite transform could shorten this and require a pure callback. Define whether a distribution returned by that callback is preserved or flattened; today's `combine` flattens it. This is an API decision, not a prerequisite for retaining finite expressiveness.

## Impact on every example

The table assumes strict top-level draws, scalar built-in arguments, explicit point distributions for constants, and no implicit distribution conditions. “Syntax only” means existing `~` bindings can become the new operator; the programs already keep distribution recipes and outcomes separate.

| Example | Migration beyond draw syntax | Result |
|---|---|---|
| [01 Tour](../examples/01_tour.probl) | Promote comparison thresholds and the attack bonus explicitly; make the loop's probability query explicit. Keep `die + die`. | Equivalent rewritten output matches. |
| [02 Craps](../examples/02_craps.probl) | Syntax only. All comparisons use already drawn totals. | Current output retained. |
| [03 RPG duel](../examples/03_rpg_duel.probl) | Draw `a.dice + a.dice` before adding the scalar bonus; draw `a.dice` before the ordinary-hit bonus. Keep record fields containing recipes. | Equivalent rewritten output matches. |
| [04 Risk battle](../examples/04_risk_battle.probl) | Syntax only. Draw each whole sorted dice pool once; compare its ordinary elements. | Current output retained; pool API naming remains open. |
| [05 Snakes and ladders](../examples/05_snakes_and_ladders.probl) | Replace scalar comparisons against `game` with explicit point-distribution operands or suitable analytical queries. Preserve the independent opponent and tie rule. | Equivalent rewritten output matches. |
| [06 Blackjack dealer](../examples/06_blackjack_dealer.probl) | Preserve `take` draw/removal behavior while migrating its syntax. | Current model retained; new `take` surface form must be specified. |
| [07 Launch forecast](../examples/07_launch_forecast.probl) | Syntax only. Branches selecting growth recipes all return distributions. | Current seeded output retained. |
| [08 Signup forecast](../examples/08_signup_forecast.probl) | Syntax only. `observe value from binomial(n, rate)` intentionally takes an outcome and a distribution. | Current seeded output retained; preserve conjugate lowering. |
| [09 Roadmap](../examples/09_roadmap.probl) | Three draws currently accept `if probability { distribution } else { 0 }`. With a strict operator, draw inside the selected branch or use an explicit point distribution for zero. | Moving draws into branches preserves seeded output. |
| [10 Quantum key](../examples/10_quantum_key.probl) | Explicitly convert the zero in `binomial(compared, error) > 0`, or use a tail query. | Equivalent rewritten output matches. |
| [11 Delivery dates](../examples/11_delivery_dates.probl) | Syntax only. Date operations receive drawn delays and ordinary dates. | Current output retained with pinned date. |
| [12 Invoice calendar](../examples/12_invoice_calendar.probl) | Syntax only. One delay is shared across all receipts; callbacks process ordinary records. | Current output retained. |
| [13 Renewal dates](../examples/13_renewal_dates.probl) | Syntax only. | Current output retained. |
| [14 Stock decision](../examples/14_stock_decision.probl) | Syntax only. Expected-value queries use local simulations; choose before drawing demand. | Current output retained. |
| [15 Service queue](../examples/15_service_queue.probl) | Syntax only. Both policies already share the same drawn arrivals and services. | Current seeded output retained. |
| [16 Predictive check](../examples/16_predictive_check.probl) | Syntax only. Replicated counts share one drawn posterior rate. | Current seeded output retained. |
| [17 Sensor tracking](../examples/17_sensor_tracking.probl) | Syntax only. Recipes are intentionally carried between local inference scopes and queried with `P`. | Current output retained. |
| [18 Correlated losses](../examples/18_correlated_losses.probl) | Explicitly lift the multiplier and thresholds; express expected excess through local simulation of a whole loss outcome. | Equivalent rewritten output matches. |

The comparison thresholds can often use existing `cdf`/`pmf`, but inequalities must be translated correctly: for discrete `X`, `P(X >= x) = 1 - cdf(D, x) + pmf(D, x)`. Replacing it with `1 - cdf(D, x)` changes the tie rule. Explicit point-distribution comparisons avoid this arithmetic and retain a `dist[bool]` for the reporting API.

## Blockers and implementation constraints

### Static rejection requires compiler work

The [compiler](../crates/probl-sema/src/lib.rs) currently resolves names, lowers control flow, and analyzes effects/liveness. It does not implement general static type inference. Type annotations largely become runtime checks in [lower.rs](../crates/probl-sema/src/lower.rs).

The verification script confirms that `probl check` accepts both `let x: int = d6` and an unreachable branch containing that binding. Running the first fails; running the unreachable case succeeds. Runtime guards alone therefore cannot satisfy “this must not compile”.

Before claiming compile-time rejection, implement inference/checking for distribution versus value types through variables, parameters, return values, containers and branch joins. A focused checker may be sufficient initially, but it must also check unexecuted branches and reject unresolved uses where the distinction cannot be established. Raw host inputs and resource limits still need runtime checks.

### A global mixed-argument ban would be incorrect

`cdf(D, threshold)`, `quantile(D, probability)`, `roll(count, D)` and `observe value from D` intentionally combine different argument types. Ordinary constructors such as `normal(mean, sd)` intentionally accept scalar outcomes as parameters and return a distribution. These APIs do not implicitly convert those parameters into independent distributions.

Define each overload by its argument roles. Do not implement the redesign by checking that every argument of every call is either a distribution or a scalar.

There are less visible mixed operations inside helpers too: `sum([d6, d6])` currently starts with scalar `0` and repeatedly invokes addition. Once mixed addition is rejected, this helper needs either a distribution-aware initial accumulator, a defined list-of-distributions overload, or rejection. An empty sum also needs a result-type rule.

### Continuous algebra has a narrower implementation today

`normal(0, 1) + normal(0, 1)` already fails. Distribution addition currently composes enumerable outcomes; arbitrary continuous composition is not implemented. `simulate` always enumerates, even under sample mode, so it cannot serve as the general replacement for continuous transformations.

This does not block migrating any existing example. It does block promising that every pair of `dist[number]` values supports addition. Initially scope distribution algebra to supported finite distributions with useful diagnostics, or separately design sampleable continuous compositions and their query capabilities. A special normal-sum formula alone would not settle the general API.

Continuous probability queries remain useful without implicit comparisons: `1 - cdf(normal(0, 1), 1.96)` works today without sampling.

### Preserve evidence and approximation when migrating

For a fully resolved recipe, a probability query can replace some distribution conditions. It is not a general mechanical rewrite. With this current finite simulation:

```probl
@epsilon 0.1
let e = simulate {
    var n = 0
    while 50% { n += 1 }
    n == 0
}
```

Branching on `e`, or first drawing a boolean from it, preserves unresolved mass and reports an event range of **50%–56.25%**. Branching on `P(e)` reports **50%** without that unresolved range. Similarly, `observe e` retains an evidence range while `observe P(e)` reduces it to a scalar likelihood. The present probability query cannot carry the uncertainty metadata of the recipe.

Migration should preserve distribution metadata through an explicit draw where a condition previously consumed a recipe. Analytical rewrites are safe only when their assumptions and uncertainty accounting hold. The tour's rewritten dice condition is fully resolved; the verification does not generalize that replacement to arbitrary `simulate` results.

In sample mode, replacing an integrated `report D` or likelihood calculation with a drawn value also changes estimator variance and may consume additional random numbers. Do not turn `observe value from D` into a draw-and-equality test: continuous observations need a density, and discrete likelihood weighting need not reject sampled runs. Keep report integration and observation likelihoods as explicit distribution consumers.

### Expression draws and effects

The existing IR already has a `Draw` statement, and expression lowering already hoists calls and branches that split worlds. The operator can use that mechanism without replacing the engine. It must preserve left-to-right evaluation, short-circuiting, loop reevaluation, local `simulate` scope, and atomic bag removal.

For example, `while DRAW(d6) != 6` must draw on every condition evaluation, whereas `let x = DRAW(d6); while x != 6 { ... }` reuses `x`. Dead-code removal and common-subexpression elimination cannot merge two independent draws. Memoizing a function's outcome distribution can remain valid; memoizing one realized sampled outcome across calls is not.

Lower directly into the existing draw machinery rather than implementing the operator as an ordinary helper function. Wrapper calls can interfere with delayed/conjugate parameter handling, as already documented in [the use-case review](use-case-gaps.md). Update effect checks so an expression draw cannot slip into a pure collection callback, and keep restrictions identical in both inference modes.

## Decisions needed before syntax

1. **Distribution overloads:** addition is agreed. Specify the rest of numeric algebra, comparisons, equality, unary functions and collection operations; avoid automatically retaining all 117 lifted built-ins.
2. **Condition boundaries:** reject bare distribution conditions, including boolean distributions produced by valid distribution algebra. Keep explicit probability weighting and likelihood observation.
3. **Point values and transforms:** existing `one_of([x])` and finite `simulate` suffice for the examples. Decide whether shorter explicit APIs are warranted, and prevent transforms from losing joint dependence accidentally.
4. **Strict draws and bag extraction:** determine whether the operator requires a distribution in every branch and how atomic `take` appears. The roadmap and blackjack examples exercise these decisions directly.
5. **Compile-time and capability guarantees:** plan static distribution/value checking, and state the finite scope of current distribution algebra. Broad continuous composition can remain separate work.

No new choice of operator name or symbol is required to settle these points. There is no evidence from the current examples that the explicit-operator direction must be abandoned.

## Validation and reproduction

The release CLI built successfully. The accompanying script compared all 18 examples at `today = 2026-09-29`, with their existing inference modes and seeds. Six candidate rewrites and twelve unchanged bodies produced byte-identical output to their baselines. The fixtures retain `~` to represent the not-yet-implemented operator and keep existing API spellings where naming remains open. This establishes model/output compatibility for these rewrites, not future grammar acceptance, static-checker correctness, or identical performance.

Thirty focused probes characterize current behavior, including five expected runtime failures. Every probe passes `probl check` today, including those runtime failures. Cases cover draw identity, distribution composition, the original observation trap, implicit boolean conditions, scalar query promotion, container shapes, field dependence, parameter sharing, continuous limitations, callbacks, bag removal, and unresolved mass. The script records each source, output and diagnostic in `results.json` inside its printed temporary directory.

Run from the repository root:

```sh
cargo build --release -p probl-cli
python3 docs/distribution-operator-survey.py
```

The script does not modify example files or compiler/engine code. Its current-behavior probes are review evidence, not a regression contract requiring the old implicit behavior to remain after the redesign. A future implementation should replace them with positive and negative compile-time cases and inference-equivalence checks for the finalized rules.
