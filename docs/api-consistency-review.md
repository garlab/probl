# API consistency review

Reviewed 4 October 2026, against `a31bb27` (`feat: median_low and median_high`).

Follow-up: the manual checks are represented in [Rust regression tests](../crates/probl-engine/tests/api_consistency.rs). See [test coverage and quality](testing.md) for commands, the finding-to-test map, and the coverage assessment. All nine numbered findings are now fixed, and all 102 audit tests are active. The design choices below have also been settled. Broader regression coverage is in [api_contracts.rs](../crates/probl-engine/tests/api_contracts.rs).

The recent statistics changes establish useful rules, but several neighboring APIs still disagree about ordering, identity, and probability mass. Some disagreements silently change answers or discard data. I would address these contracts before adding more built-ins.

This review inspected the 136 public built-in declarations, their dispatch and validation paths, and the relevant collection, numeric, distribution, and type-conversion implementations. It exercised 52 targeted programs through both the native CLI and WASM, in enumeration and sampling modes: 208 executions, with all 104 native/WASM comparisons agreeing after ignoring the CLI's extra trailing newline. Sampling used 20 runs and seed 7 to check dispatch consistency, not estimator accuracy. The release CLI build succeeded. This is a targeted audit, not exhaustive coverage or a new regression suite.

## Findings to fix

### 1. High: order statistics can contradict the language's ordering

**Fixed:** finite medians and quantiles now order a separate population with the language comparator. Storage identity is unchanged. The audit regressions are enabled; [ordering.rs](../crates/probl-engine/tests/ordering.rs) also covers nested int/float/prob combinations and reversed inputs. The reproduction below describes the earlier behavior.

```probl
let xs = [[1.0, 0], [1, 100]]
report sort(xs)             # [[1.0, 0], [1, 100]]
report median_low(xs)       # [1, 100] — the higher item
report median_high(xs)      # [1.0, 0] — the lower item
report quantile(xs, 50%)    # [1, 100]
report cdf(xs, [1.0, 0])    # 50%
```

Language comparisons treat `1` and `1.0` as numerically equal and continue to the next list element. Internal storage ordering breaks that tie by numeric type immediately. The median functions validate with the language comparator, but then walk outcomes sorted by storage order. Quantiles have the same problem. This also breaks the relationship between a CDF and its quantiles.

**Suggested fix:** construct an ordered population using the language's statistical comparator before computing order statistics. Keep the internal typed ordering for storage, hashing, and world merging. Sorting, validation, cumulative weights, and median selection need to use the same public ordering.

**Regression checks:** the example above for both a list and `one_of(xs)`; reversed input order; mixed `int`/`float`/`prob` elements inside lexicographically ordered lists; `median_low(xs) <= median_high(xs)`.

Sources: [statistical comparison, quantile, and median](../crates/probl-engine/src/builtins.rs#L795), [storage ordering](../crates/probl-engine/src/value.rs#L559), [language ordering](../crates/probl-engine/src/ops.rs#L750).

### 2. High: adding a continuous component silently changes probability normalization

**Fixed:** `P`, `cdf`, `pmf` and `pdf` now reject any positive unresolved mass in both modes, including mixtures and truncated count-distribution tails. Reports retain probability bounds; descriptive statistics still describe resolved outcomes. Lifting preserves even sub-epsilon missing mass. The reproductions below describe the earlier behavior.

```probl
@epsilon 0.1
let d = simulate {
    var n = 0
    while 50% { n += 1 }
    n
}
let a = one_of([d, 100])
let b = one_of([d, uniform(100, 101)])

report cdf(a, 200)    # 96.88%
report cdf(b, 200)    # 100%
report pmf(a, 0)      # 25%
report pmf(b, 0)      # 25.81%
report a <= 200       # 96.88%–100.00%
report b <= 200       # 96.88%–100.00%
```

Both mixtures have the same unresolved mass. Both added components are entirely below 200 and cannot equal zero. Their answers to these queries should therefore use the same treatment of unresolved outcomes.

Finite `cdf` and `pmf` return the known, unnormalized mass. The continuous-mixture path divides by resolved mass, silently making the answer conditional on resolution. Neither scalar result carries the report's uncertainty interval. Replacing one component changes the meaning of the query.

**Suggested fix:** specify one unresolved-mass contract for `P`, `cdf`, and `pmf`, and share its implementation across finite and mixed distributions. Prefer retaining bounds or requiring an explicit choice to condition on resolved outcomes. Until that API exists, rejecting a query with material unresolved mass is clearer than displaying an apparently exact scalar. Means and medians can retain their separately documented convention of describing resolved outcomes.

**Regression checks:** the paired mixtures above; equivalent point-mass queries; mixtures with only continuous components and unresolved mass; consistency of CDF, PMF, and integrated density normalization. This sharpens the existing inference-result concern in [use-case gaps](use-case-gaps.md), rather than identifying a need for another distribution family.

Sources: [finite queries](../crates/probl-engine/src/builtins.rs#L551), [continuous queries](../crates/probl-engine/src/builtins.rs#L662), [mixture CDF](../crates/probl-engine/src/continuous.rs#L871).

### 3. High: implicit map-key conversion can silently discard entries

**Fixed:** typed map conversion now rejects key collisions, including annotated map literals. This also protects the new float-to-int conversion. The audit regression is enabled. The reproduction below describes the earlier behavior.

```probl
let original = [1: "int", 1.0: "float"]
let typed: map[prob, str] = original
report original         # [1: "int", 1.0: "float"]
report typed            # [100%: "float"]
report len(original)    # 2
report len(typed)       # 1
```

The annotation converts both keys to the same `prob`, then a map insertion overwrites the first value. The same happens when widening probability keys to floats. A type annotation can lose data without a diagnostic; the winner depends on storage order. An annotated map literal also exhibits this behavior.

**Suggested fix:** detect collisions after key conversion and report an error identifying the conflicting keys. Bags may legitimately merge counts, and distributions may merge probability mass; maps cannot generally merge arbitrary values. The data-input contract already [rejects converted-key collisions](data-input.md), so runtime conversion should offer the same protection.

**Regression checks:** conversions through variable annotations and function parameters, nested maps, annotated literals, and probability-to-float widening. Retain count-preserving bag conversion.

Source: [recursive type conversion](../crates/probl-engine/src/interp.rs#L1437).

### 4. High: membership, lookup, updates, and probability queries disagree about identity

**Fixed:** Map/bag membership, lookup, indexing and updates use exact typed identity. PMF counts exact typed outcomes, including atoms in continuous mixtures, and agrees with finite support. Numeric-equality events remain explicit with `P(d == x)`; list membership and typed structural equality retain their documented semantics. The reproductions below describe the earlier behavior.

```probl
let m = [1: "a"]
report m.contains(1.0)        # true
report m.get(1.0)             # "a"
report remove(m, 1.0)         # [1: "a"] — unchanged
report insert(m, 1.0, "b")    # [1: "a", 1.0: "b"]

let b = bag([1])
report b.contains(1.0)        # true
report b.get(1.0)             # 0
# remove(b, 1.0) errors: 1.0 isn't in the bag
```

Map lookup and membership fall back to language numeric equality. Map insertion/removal and bag counts/removal use exact typed identity. A successful membership check does not establish that the next operation will find the item.

The same distinction creates a statistical trap:

```probl
let d = one_of([1, 1.0])
report support(d)                          # [1, 1.0]
report pmf(d, 1)                           # 100%
report pmf(d, 1.0)                         # 100%
report sum(support(d).map(x -> pmf(d, x)))  # 2.0
```

Here the support exposes typed outcomes, but `pmf` queries numeric equality classes. Iterating over the advertised support double-counts probability. Structural equality also changes with nesting: `1 == 1.0` is true, while `[1] == [1.0]` and `{x: 1} == {x: 1.0}` are false.

**Suggested fix:** keep internal typed identity, which preserves `typeof` and world semantics, but define a consistent public contract. Given the current documented key identity, exact typed lookup/membership/update is the smallest coherent collection rule. Numeric equality searches could remain explicit. Separately, make support/PMF agree on what constitutes an outcome, or expose typed outcome-weight pairs and clearly distinguish equality-event queries. Decide whether user-facing equality should recurse numerically through containers; do not accidentally change world-merging equality while fixing it.

**Regression checks:** the same key expressed as `int`, `float`, `prob`, and purely real `complex`; successful membership followed by get/remove; typed support and its probability total; nested containers. This is a design decision with observable compatibility consequences, not a reason to merge all numeric values in storage.

Sources: [language equality and membership](../crates/probl-engine/src/ops.rs#L720), [get/insert/remove](../crates/probl-engine/src/builtins.rs#L1414), [PMF](../crates/probl-engine/src/builtins.rs#L561), [documented structural identity](semantics.md#1-values-and-types).

### 5. High: built-ins can manufacture an invalid `prob`

**Fixed:** Computed probabilities use a shared checked constructor, compensated summation and resolved total mass. Only boundary roundoff within eight floating-point epsilons is corrected; materially invalid or nonfinite results fail. Explicit conversions, including an existing `prob`, remain strict. The reproductions below describe the earlier behavior.

```probl
let p = cdf([1,2,3,4,5,6,7,8,9,10], 10)
report p > 1    # true
report 1 - p    # -6.66133814775e-16
```

This is ordinary floating-point accumulation error, but it escapes into a type that promises a finite value in `[0, 1]`. Internal queries construct `Value::Prob` directly. Both `to_prob` and explicit `prob(...)` trust an existing probability without revalidation, so the invariant is not restored at subsequent API boundaries.

**Suggested fix:** centralize construction of computed probabilities. Use stable summation and the known total mass; correct only justified rounding excursions at the boundary and reject genuinely invalid results. Keep user-provided out-of-range numbers as errors. This does not require snapping general numerical calculations such as `sin(pi)` to zero.

**Regression checks:** CDFs at the maximum of complete populations of varying sizes, equality probabilities, complements, and use of returned probabilities in branching and distribution constructors. Assert range invariants on the underlying value, not just formatted percentages.

Sources: [CDF/PMF result construction](../crates/probl-engine/src/builtins.rs#L551), [probability conversion](../crates/probl-engine/src/ops.rs#L135), [distribution normalization](../crates/probl-engine/src/dist.rs#L237).

### 6. High: continuous statistics bypass finite-result checks and can return materially wrong answers

**Fixed:** Continuous queries reject nonfinite results, including infinite endpoint quantiles and singular densities. Scaled/centered means and SD formulas avoid intermediate overflow and translation cancellation; finite populations and mixtures share these formulas. Report summaries label unrepresentable fields “out of range”. The reproductions below describe the earlier behavior.

These all succeed:

```probl
report mean(lognormal(1000, 1))                   # inf
report quantile(lognormal(1000, 1), 50%)          # inf
report sd(normal(0, 1e308))                       # inf; expected 1e308
report mean(uniform(1e308, 1.2e308))              # inf; expected about 1.1e308
report mean(beta(1e308, 1e308))                   # 0.0; expected 0.5
report variance(triangular(1000000000,
                           1000000001,
                           1000000002))          # 0.0; expected 1/6
report variance(triangular(0, 1, 2))              # 0.166666666667
```

Yet `median(lognormal(1000, 1))` rejects overflow, as does `variance(one_of([-1e308, 1e308]))`. The continuous query path constructs floats without the finite-result guard used by finite statistics and elementary math.

There are two fixes: consistent overflow handling, and numerically stable formulas where the answer is representable. Squaring a normal's SD and taking its square root needlessly overflows. Adding uniform endpoints overflows before division. The triangular variance loses its spread when translated to a large baseline; this is not a tiny display discrepancy.

**Suggested fix:** apply a shared output contract and use scaled/centered formulas. Test translation and scale invariants. Specify boundary cases separately: infinite endpoint quantiles of unbounded distributions, and genuinely singular densities, may merit an explicit extended-real policy. The examples above involve finite interior quantiles or finite mathematical moments, not those boundary cases.

**Regression checks:** each example above, representable large means/SDs, translated triangular distributions, and parallel behavior for lists, finite distributions, and continuous recipes.

Sources: [continuous query outputs](../crates/probl-engine/src/builtins.rs#L681), [family moments](../crates/probl-engine/src/continuous.rs#L157), [mixture variance](../crates/probl-engine/src/continuous.rs#L852).

### 7. Medium: large valid relative weights turn `one_of` into an empty distribution

**Fixed:** Relative weights are scaled by their largest weight before compensated normalization. Absolute probabilities retain their separate sum-to-one validation. Positive support survives large common scaling. The reproductions below describe the earlier behavior.

```probl
let d = one_of([1: 1e308, 2: 1e308])
report support(d)    # []
report pmf(d, 1)     # zero (currently printed -0%)
```

Replacing both weights with `1` produces the expected `[1, 2]` support and 50% probability. Each original weight is finite and valid, but their sum overflows. Dividing by infinity makes both normalized weights zero; they are then discarded. A later use such as `d == 1` fails with an empty-distribution error far from the cause.

**Suggested fix:** normalize relative weights after scaling by their maximum, with stable summation. Reject any invalid or empty result at construction. Relative weights should be invariant under a common positive scale when the supplied weights are representable.

**Regression checks:** the two equivalent maps, highly unequal weights, tiny finite weights, and all-zero rejection.

Sources: [weighted one_of](../crates/probl-engine/src/builtins.rs#L1510), [discarding zero weights](../crates/probl-engine/src/dist.rs#L215).

### 8. Medium: quantile still has two independent contract holes

**Fixed:** lists and finite distributions share ordering validation, including singletons. Quantiles use compensated sums without a fixed probability tolerance and handle retained minimum/maximum endpoints explicitly. Report percentiles share the finite quantile helper. The audit regressions are enabled. The reproductions below describe the earlier behavior.

**Input validation differs by representation.** Run these separately:

```probl
report quantile([1, "a"], 50%)           # error: can't compare int with str
report quantile(one_of([1, "a"]), 50%)   # 1
```

A list of records likewise fails, but wrapping the same records in `one_of` returns a record. The distribution path preserves legacy categorical storage-order quantiles, while the list path requires comparable elements. Newly added medians reject unordered populations in both forms.

**A fixed tolerance can skip a real tail, even at 100%.**

```probl
let d = one_of([0: 1, 1000000: 1e-13])
report pmf(d, 1000000)     # positive: about 1e-13
report quantile(d, 100%)   # 0
report maximum(support(d))    # 1000000
```

`Dist::quantile` subtracts `1e-12` from every requested cumulative probability. That tolerance can overwhelm the probability being queried. A retained finite maximum must be the 100th percentile.

**Suggested fix:** share admissible-type validation between lists and distributions, while preserving the agreed lower-quantile convention. Handle finite endpoints explicitly and replace the fixed probability tolerance with stable cumulative sums and a narrowly justified rounding rule. Preserve positive tails.

**Regression checks:** list/distribution equivalence for unordered values, singleton records and complex values, q=0/q=1, tiny positive tails, and queries immediately on either side of an exact mass boundary.

Sources: [quantile validation](../crates/probl-engine/src/builtins.rs#L802), [finite quantile selection](../crates/probl-engine/src/dist.rs#L411).

### 9. Medium: `get` can silently mistake a valid index for a missing item

**Fixed:** integer contexts now accept exactly integral finite floats with no rounding. Indexing, list `get`, mutations, and slices share validation; `get` defaults apply only to out-of-range integer positions. The audit regressions are enabled, with broader coverage in [integer_conversion.rs](../crates/probl-engine/tests/integer_conversion.rs). The reproduction below describes the earlier behavior.

```probl
let xs = [10, 20]
report xs[1.0]              # 20
report xs.get(1.0, -1)      # -1
report xs.get("wrong", -1)  # -1
```

Indexing accepts an integral float, while list `get` recognizes only the `int` variant. It treats every unsupported key type as absence, so adding a fallback changes a successful access into a fallback value. `slice` separately requires strict ints.

**Suggested fix:** choose a single sequence-index policy and reuse its validation. A default should handle an absent/out-of-range position, not conceal an invalid index type. Strict integer indices would fit the newer slice and integer APIs; accepting integral floats everywhere is also coherent but needs a deliberate rule.

**Regression checks:** integer, integral-float, fractional, negative, oversized, and nonnumeric indices across indexing, `get`, `insert`, `remove`, and `slice`; distinguish type errors from absence.

Sources: [list get](../crates/probl-engine/src/builtins.rs#L1414), [index validation](../crates/probl-engine/src/ops.rs#L925), [slice](../crates/probl-engine/src/builtins.rs#L1349).

## Settled design choices

- **Comparison and population extrema are separate.** `min(a,b,…)`/`max(a,b,…)` require at least two candidates and retain finite-recipe lifting. `minimum(xs,compare?)`/`maximum(xs,compare?)` select an existing collection element; they never lift collection elements. On a distribution they query closed-support bounds and reject unresolved mass or nonfinite bounds. `highest`/`lowest` require an explicit count and always return lists. [Regression tests](../crates/probl-engine/tests/min_max.rs) cover these contracts.
- **Boolean ordering stays explicit.** Statistical low/high medians, quantiles and CDFs order false before true. Ordinary comparisons, default sort and min/max do not order booleans. Custom sort comparators can order them explicitly.
- **Sequence support is consistent.** `get` supports lists, strings and ranges with checked integer indices and defaults for absence. Strings use Unicode scalar positions. Integer ranges support all population statistics without materialization, except `support`, which creates a list under collection limits. See the [capability table](semantics.md#1-values-and-types).
- **Existing overloads remain explicit.** `round(x)` returns an int. With `digits`, int inputs stay ints and other numeric inputs return floats, including `digits=0`. Bag counts are defined for every typed key, with zero for absence, so `get` always returns a count and does not use its fallback. Maps and sequences use the fallback only for absence; invalid index types still fail.

The spot checks of Unicode scalar indexing/slicing, trim character sets, ordinary date arithmetic and date medians, exact integer functions, principal complex logarithms, and the newly agreed numeric midpoint medians behaved as documented. None of the findings calls for undoing the distinction between a distribution recipe and a drawn value.

## Recommended implementation order

1. Define shared public ordering and key-identity rules; fix order statistics, map conversions, and conflicting lookup/update behavior against those rules.
2. Centralize probability construction and unresolved-mass handling; harden continuous moments and relative-weight normalization.
3. Align quantile validation and index validation, then resolve the lower-priority API choices above.

For regression protection, favor cross-operation properties in addition to individual examples: list versus distribution agreement, CDF/quantile consistency, weight-scale invariance, translation-invariant variance, collision-free map conversion, and range-valid returned probabilities. Keep these small deterministic checks in both execution modes and the native/WASM parity fixtures. Existing example snapshots alone are unlikely to catch these interactions.

To reproduce a snippet, save it as a `.probl` file and run `target/release/probl run FILE --today 2026-10-04`. Error examples should be run separately so an earlier failure does not hide later results. The snippets and comments above record the reviewed behavior; suggested fixes are not implemented by this audit.
