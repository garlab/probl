# Extending enumeration beyond explicit worlds

> Surveyed October 9, 2026 against revision `f12e583` (`0.2.0`). This document describes current restrictions and proposes implementation directions; it does not change the language contract. Effort estimates are engineering judgments, not measured implementation times. The [high-level plan](#high-level-implementation-plan) separates near-term work from research.
>
> Phases 0 to 3 are built since: see [phase 0](#phase-0-as-built), [phase 1](#phase-1-as-built), [phase 2](#phase-2-as-built) and [phase 3](#phase-3-as-built). The survey's sections still describe the revision it surveyed.

## Recommendation

Extend enumeration incrementally through compact representations of distributions and queries. Probl already does this for continuous marginals and finite-state loops; enumerating every possible outcome is not necessary to answer every question about a model.

The most useful next steps are:

1. Close gaps that the existing analytic representation can already express: collection access and reductions, piecewise affine functions, and finite event grouping.
2. Allow continuous likelihood observations over otherwise finite, fully resolved models, with correct density and failure accounting.
3. Reuse the existing conjugate formulas in enumeration, preserving the identity of the latent variable and its posterior through aliases and branches.
4. Represent count distributions as laws with queries, instead of always materializing their outcomes.
5. Add selected nonlinear transformations and joint Gaussian calculations, then consider reusable continuous `simulate` results.

General deterministic integration is a useful later option, but needs approximation metadata before it can be presented as an enumeration result. A grid with many points is still an approximation; it must not silently replace a continuous model.

There is no universal extension that makes arbitrary Probl programs efficiently enumerable. Infinite output, arbitrary loops, difficult integrals, and exponentially large dependencies remain possible. The practical objective is to support useful classes of models and queries with explicit boundaries.

## What is already supported

The present implementation is more capable than a discrete path enumerator:

- All implemented continuous families have analytic distribution queries: normal, lognormal, uniform, beta, gamma, exponential, triangular and PERT, including the `to` and `normal_range` constructors.
- A continuous draw can remain an analytic scalar. Affine operations on the same draw, threshold comparisons, interval conditioning and ordinary function calls preserve its identity.
- Multiple independent continuous variables can coexist and be reported or restricted separately. Combining them in one expression is the restriction.
- Finite choices can select continuous families or affine coefficients. Numeric reports already combine continuous marginals with numeric point masses.
- Conditions can produce piecewise affine results. Several apparent math limitations can therefore be expressed manually today.
- Poisson and geometric distributions already enumerate a finite portion of their support and retain missing mass. Binomial construction can also retain omitted tails.
- Eligible cyclic loops are solved as finite absorbing Markov chains. Recursive calls that revisit the same call state use fixed-point iteration with unresolved weight. Neither loops nor recursion are categorically sampling-only.

These contracts are described in [continuous semantics](../semantics.md#13-continuous-distributions), [termination and approximation](../semantics.md#10-termination-and-approximation) and [architecture](../architecture.md).

For example, this works now:

```probl
let x ~ uniform(-1, 1)
report if x < 0 { -x } else { x }
```

But `report abs(x)` fails. Similarly, `[x][0]` and `[x, x+1].reduce((a,b) -> a+b, 0)` work, while `[x].get(0)` and `[x, x+1].sum()` fail. Those are implementation gaps, not infinite-world problems.

## Survey method and effort conventions

The survey inspected mode-specific execution paths and analytic rejection sites, then ran 60 small programs in both modes, including working controls and invalid-call controls. Sampled probes used 200 runs, seed 7, total failure mode, a two-second timeout and a two-million-unit work limit. Their purpose was to check capabilities and diagnostics, not numerical accuracy or performance. A successful sampled probe is not a proof that every run of that program succeeds.

The existing `analytic`, `sampling`, `conjugate`, `chains` and `recursion` integration suites were also run: **73 tests passed**. No inference changes were made during this survey.

Estimates below are **focused person-days for one engineer familiar with Probl**, including implementation, meaningful regression tests, documentation and checking native/WASM behavior. They exclude unrelated features and release work. Small extensions have medium confidence; new inference representations have low confidence until prototyped. Dependencies are stated explicitly. Feature estimates overlap; use the phased estimates for scheduling rather than summing every row.

## Capability inventory

“Sampled” describes the representative valid drawn-value case, not every possible input. “Both” means switching modes does not remove the restriction.

| ID | Current restriction | Present status | Direction for enumeration | Initial effort |
|---|---|---|---|---:|
| A | Continuous point likelihoods with concrete parameters | Sampled only | Weight finite worlds by densities; audit density evidence and unresolved bounds | 4–7 days for a restricted first version |
| B | Analytic values passed through collection built-ins | Sampled in many cases; equivalent indexing/reduction already works | Replace the blanket rejection with operation-specific handling | 2–4 days |
| C | `abs`, scalar `min`/`max`, clamp-like piecewise operations | Sampled; manual `if` often works | Split the latent domain into affine pieces | 4–7 days |
| D | `floor`, `ceil`, `round`, derived finite indices | Sampled | Partition the input into output bins using CDF differences | 5–8 days for bounded output |
| E | `x*x`, powers, `exp`, `ln`, `sqrt`, trigonometry and related functions | Sampled for valid concrete inputs | Selected transform nodes with inverse images and supported moments | 10–18 days for the first univariate subset |
| F | Boolean combinations involving independent analytic events | Sampled; nested `if` works for simple cases | Preserve event expressions and partition only when their values are needed | 4–7 days for independent interval events |
| G | Conjugate observations of a continuous latent | Sampled only | Update an identity-preserving posterior in the world context | 12–20 days including the first posterior-context refactor |
| H | A continuous draw used as a probability or likelihood | Sampled only | Integrate each branch's likelihood and keep its conditional posterior | 6–10 days after G |
| I | Independent continuous arithmetic, comparisons and hierarchical draws | Sampled | Joint Gaussian representation first; other families need convolution/integration | 15–25 days for a bounded Gaussian subset |
| J | General non-conjugate continuous likelihoods and transformations | Sampled, often with poor effective sample size | Budgeted deterministic integration over a small latent dimension | 20–35 days for a one-dimensional numerical backend after E/G |
| K | Continuous draws or analytic captures in `simulate` | Local continuous draws fail in both modes; analytic captures fail in enumeration | Close finite-output local models first; then introduce joint recipes with scoped identities | 3–5 days for a restricted finite-output subset; 18–30 more for joint recipes |
| L | Whole aggregates containing analytic values; mixed numeric/nonnumeric reports | Sampled | Structured marginal summaries and explicit joint-result capabilities | 5–10 days for presentation; joint export depends on K |
| M | Analytic keys, filtering, sorting and aggregate equality | Sampled for ordinary concrete values | Finite case partitioning and lazy structures; preserve callback purity | 8–15 days for selected finite collection operations after C/F |
| N | Analytic text output and infinitely many report groups | Sampled, with finite output per run | Symbolic debug rendering or explicit finite grouping; retain errors for requests for infinite tables | 2–4 days for diagnostics/debug policy; broader text values deferred |
| O | Very broad/infinite count supports and queries over truncated count recipes | Some draws work only through the sampling fast path; some queries fail in both | Lazy count laws, exact-form PMF/CDF queries, optional materialization | 10–16 days for law queries; 8–15 more for selected symbolic count outcomes |
| P | Unbounded counters, analytic-state loops, recursive effects | Often approximate, resource-limited or rejected | Reward/phase-type summaries and selected symbolic recurrences | 15–25 days for a narrow reward subset; 20–40 for a phase-type extension |
| Q | Large finite joint state spaces | Supported until budgets are exhausted | Factorization and a finite symbolic backend | 20–40 days for a restricted prototype; broader integration is separate |
| R | Faults on only part of an analytic domain | Operation generally rejected before its fault region is known | Partition successful and failing regions; preserve total/partial/catch semantics | 4–8 days for supported univariate operations, alongside C–E |

### A. Density observations over finite worlds

This currently requires sampling even though there are only two model states:

```probl
let mu ~ one_of([0, 1])
observe 0 from normal(mu, 1)
report mu == 0
```

The posterior is `1 / (1 + exp(-0.5))`, approximately 62.2459%. Each finite world can simply receive its likelihood density. No continuous draw or integration is required.

The rejection is explicit in `likelihood` in [interp.rs](../../crates/probl-engine/src/interp.rs). Its diagnostic says density is not a probability and therefore enumeration cannot use it. The first statement is true; the conclusion is not a mathematical limitation.

Do not implement this by removing that check alone:

- Evidence is a density, potentially greater than one, and must retain its density label and log value.
- Existing unresolved-weight accounting assumes that later probability likelihoods cannot increase omitted mass. A density can increase it arbitrarily. Missing mass needs a valid likelihood envelope before it can bound missing evidence or posterior probabilities.
- The absorbing-chain solver's transitions are probabilities. Density-weighted cycles cannot automatically use the same solver; a general weighted fixed point also needs convergence checks.
- Failure-share denominators and evidence after divergent control flow need the same scrutiny as successful reports. Mixing discrete observations and density observations needs a compatible observation model; do not add atom probabilities to densities.
- Compute likelihoods in log space where possible. Extended-exponent `Weight` cannot recover a density that already underflowed before conversion.

**First release:** support fully resolved finite models and finite mixtures of continuous likelihoods with concrete parameters and concrete observations. Reject unsupported unresolved/density combinations and density-bearing cycles before presenting results. A statically bounded subset is a reasonable initial gate; data-dependent termination should not be treated as a proof of completeness. Keep the existing rejection of a single observation recipe mixing continuous densities and discrete atoms.

**Acceptance:** the two-state posterior above, density evidence above one, very small likelihoods, repeated observations, aliases of likelihood constructors, impossible evidence, and deliberate rejection of unsupported unresolved/cyclic cases. The 4–7-day estimate assumes this conservative scope.

### B–D. Operations that reduce to existing interval reasoning

[builtin_values](../../crates/probl-engine/src/interp.rs) currently rejects almost every built-in whose arguments contain an analytic value. The exceptions include `map`, `filter`, `reduce` and `len`, but even these can hit narrower callback restrictions.

**B: representation-preserving operations.** Start with `get` using a concrete index/key, structural operations that only rearrange values, and `sum`/`mean` of lists whose combination stays affine in one latent. List statistics must keep their ordinary within-world meaning: `mean([x, x+1])` can be `x+0.5`; `mean(x)` must remain invalid. Do not remove the generic guard until each admitted operation handles analytic values deliberately.

**C: piecewise affine math.** Implement `abs(x)` as the two domains separated by zero. `min(x,c)` and `max(x,c)` similarly create an affine part and a point mass; clamping adds a second threshold. Preserve atoms when calculating CDFs and quantiles. Dispatch through a shared domain-partition operation rather than giving each built-in its own ad hoc branching machinery. Built-in argument evaluation order and fault behavior must remain unchanged.

**D: rounding and finite indices.** For `x ~ uniform(0,3)`, `floor(x)` has values 0, 1 and 2, each with probability 1/3. CDF differences give these weights. Preserve the source restriction in each resulting world so a later `observe floor(x) == 1` also restricts aliases of `x` to `[1,2)`.

`round(x,digits)` requires the existing tie and return-type rules; ties have zero measure for a purely continuous law but not for mixtures containing atoms. A normal input produces infinitely many rounded integers: use a lazy discrete law or an explicitly unresolved tail, not an unlabelled finite list. Enumerating billions of bins is still a budget failure even when the range is finite.

A symbolic integer obtained this way can eventually select among a finite collection's valid indices. Out-of-range regions are faults, not evidence to discard. A direct `let n: int = x` for a continuously distributed nonintegral value must not start rounding implicitly.

### E. Nonlinear transforms of one latent

Store `y = g(x)` as an expression tied to `x`'s identity. For a monotone `g`, CDFs and threshold observations can use its inverse; for finitely many monotone pieces, collect the corresponding inverse intervals.

Good first targets are squares, square roots on valid domains, `exp`/`ln`, reciprocal on domains away from zero, and selected family-preserving identities. Examples include exponentiating a normal and taking the logarithm of a lognormal. Repeated uses must refer to the same latent: computing `x*x` is not the same operation as multiplying two independent draws.

For `x ~ uniform(-1,1)`, `x*x` has CDF `sqrt(y)` on `[0,1]`, mean 1/3 and variance 4/45. This gives a compact, independently checkable acceptance case.

Inverse-image CDFs do not automatically provide every moment. Each transform needs either supported analytic moments or a separately identified numerical integration method. A query can be available while another is not. In particular, a principal-value integral is not an ordinary expectation: `1/x` for a normal `x` does not acquire a mean of zero just because the law is symmetric.

For `sin` and `cos` on a bounded domain, partition at turning points. An unbounded input can have infinitely many inverse pieces; that needs a wrapped-law formula or a bounded tail-summation method. It is outside the initial 10–18-day subset. Other libm functions, general user functions and complex-valued transforms should enter through the same capability checks, not a promise that any scalar function now lifts analytically.

Domain errors and overflow regions are part of the design. `sqrt(x)` with `x ~ uniform(-1,1)` has a positive-mass invalid region. Approximation nodes must not accidentally miss it; see R.

### F. Boolean expressions over several analytic events

Today `(x>0) and (y>0)` fails for independent normal draws, although this works:

```probl
let x ~ normal(0, 1)
let y ~ normal(0, 1)
report if x > 0 { if y > 0 { true } else { false } } else { false }
```

Represent composite events as Boolean expression nodes. For independent latents constrained only by their own intervals, conjunctions correspond to rectangles and disjunctions to finite unions of rectangles. Existing world branching can implement a first version without a general multivariate integration engine.

Do not replace a stored event with its marginal probability: observing it later must update the originating latent variables. Overlapping unions must not be counted twice. Independence must be checked in the current posterior, not assumed from originally separate draws.

Combining an analytic event with a `prob` or Boolean recipe needs the existing `bool`/`prob`/`dist[bool]` result-type and fresh-draw rules. It is not permission to silently turn every event into an independent Bernoulli trial.

### G. Conjugate posterior updates in enumeration

The engine already has the needed formulas in [conjugate.rs](../../crates/probl-engine/src/conjugate.rs): beta–binomial, beta–Bernoulli, gamma–Poisson and normal–normal. The execution path enables them only while sampling, using `Value::Delayed`; its next ordinary use draws a number.

Enumeration should instead keep an analytic posterior with the **same latent identity**:

```probl
let p ~ beta(2, 3)
observe 3 from binomial(5, p)
report p
```

This should report the full `beta(5,5)` posterior, mean 0.5, with evidence 4/21, and no Monte Carlo error. Returning an independently drawn posterior at each access would break the model.

The difficult part is the state representation. [Analytic](../../crates/probl-engine/src/analytic.rs) currently embeds a family in each value; a world's `Constraints` stores intervals in that family's **CDF coordinates**. Updating a family without updating aliases and restrictions makes old coordinates mean different events.

Introduce a world-local latent context keyed by identity, with expressions referring to that identity. Restrictions need a representation whose meaning survives posterior changes, such as intervals in the latent's value coordinates. Stored events, closures, container aliases and function-return constraints must all resolve against the current context. Keep previously accumulated report contributions immutable; the language's existing report/evidence ordering rules still apply.

Start with the four existing pairs, concrete observations and concrete remaining parameters, and reject unsupported interactions explicitly. Then support threshold-truncated conjugate priors: update the underlying family, retain the allowed region, and include the ratio of posterior and prior truncation normalizers in the evidence. A beta prior restricted to `p>0.5` does not become an untruncated beta after a success.

This uses ideas from [delayed sampling](https://proceedings.mlr.press/v84/murray18a.html), but enumeration requires analytic continuation where the sampler currently materializes a value. It is not a one-line change to the mode gate. The 12–20 days include the first context change and existing conjugate pairs; broad affine likelihood recognition and truncated-family closure are follow-up scope.

### H. Probabilities, scores and dependent choices

These currently require sampling:

```probl
let p ~ beta(2, 3)
let hit = if p { true } else { false }
report hit
report p
```

The true branch has weight `E[p]=2/5` and posterior `beta(3,3)` for `p`; the false branch has weight 3/5 and posterior `beta(2,4)`. Their mixture recovers the original prior when reporting `p` without further evidence. Using only `E[p]` for each branch while leaving the posterior untouched would give incorrect correlations for later trials or observations.

`score p`, `bernoulli(p)`, binomial outcomes and weights in `chance` are related likelihood updates. Begin with recognized beta and affine-uniform cases. General weights require integrating the weight function against the current posterior and retaining the tilted density, which leads to J.

Probability annotations/conversions also need domain checks. A beta-supported draw can satisfy `prob` without being sampled. A normal draw cannot be silently clamped or have its invalid region dropped. Distinguish annotation/type-contract failures from recoverable domain faults according to the existing error semantics.

### I. Joint continuous values and hierarchical models

There is no infinite-world obstacle to adding two independent normals: the sum is normal. However, storing only that marginal loses information needed by later code:

```probl
let x ~ normal(0, 1)
let y ~ normal(0, 1)
let sum = x + y
observe sum > 0
report x
report y
```

After the observation, `x` and `y` are correlated. A replacement `sum ~ normal(0,sqrt(2))` independent of the original draws is incorrect.

**First subset:** represent Gaussian latents with means/covariances, and affine expressions as coefficient vectors. Support unconditioned affine marginals, terminal comparisons, and normal likelihood observations with concrete noise. `let mu ~ normal(0,1); let y ~ normal(mu,1)` then has `Var(y)=2` and `Cov(mu,y)=1`. Conditioning on a normal observation uses the standard joint Gaussian update.

**Boundary:** a hard inequality generally produces a truncated joint Gaussian, not another ordinary Gaussian. A terminal `report sum>0` is much cheaper than `observe sum>0` followed by arbitrary queries. Initially reject unsupported joint restrictions; never discard them or keep only independently truncated marginals. Joint truncation would add roughly 15–30 days for a carefully limited low-dimensional implementation, depending on J and the required queries.

Other useful closures include sums of compatible gamma laws and selected uniform convolutions. They need their own conditions and identity handling. General products, ratios and non-Gaussian hierarchies belong to a later integration backend. Complex values could eventually be represented as correlated real/imaginary expressions; that is separate from quantum amplitudes and from the initial Gaussian milestone.

### J. General deterministic integration

For a one-dimensional posterior, keep an unnormalized density expression and evaluate its normalizer and requested integrals with adaptive quadrature. This can handle non-conjugate observations, products of likelihoods and transformations without sampling thousands of model executions.

A production subset should require:

- Known integration dimension, domain, discontinuities and supported scalar operations.
- Absolute/relative tolerances, evaluation/work budgets, cancellation and explicit convergence failure.
- Separate status for numerical integration error. An estimated quadrature error is not a rigorous probability bound or a Monte Carlo standard error.
- Tail treatment and existence checks for requested moments. Agreement between successive approximations is not proof that an integral exists.
- Shared evaluation/caching of normalizers, CDFs and moments; quantiles invert a computed CDF and inherit its error.
- Fault-region handling outside mere quadrature-node evaluation.

The [GSL integration documentation](https://www.gnu.org/software/gsl/doc/html/integration.html) is useful algorithmic prior art for adaptive quadrature, infinite intervals and error estimates. It is not a recommendation to add a GSL dependency.

One dimension is a realistic starting point. Low-dimensional cubature is additional work, roughly 20–40 days for a bounded supported subset after the one-dimensional backend; arbitrary dimension is not an incremental guarantee. Factorization and analytic elimination should reduce dimension first.

Recommendation: make numerical integration opt-in until the public result API and renderer expose its accuracy contract. Analytic/formula-based extensions can continue to run under the current enumeration mode. Do not silently change an enumeration request into sampling.

### K. Continuous `simulate` and local model boundaries

There are three different problems:

1. **Finite local output.** `simulate { let x ~ normal(0,1); x>0 }` currently fails in both modes, even though its result has only two outcomes. A restricted first implementation can integrate the local event and return an ordinary finite distribution, provided no local latent escapes. Verify normalization, local evidence, failures and the absence of escaped latent IDs. This is the 3–5-day opportunity.
2. **Continuous or joint local output.** `simulate { let x ~ normal(0,1); {x:x, twice:2*x} }` needs a recipe that owns a joint measure. Each draw must freshen its owned latent IDs while preserving correlation between the returned fields. Merely storing `Value::Analytic` in today's `Dist` would reuse identities across independent draws. Recipe arithmetic, caching, type checks, queries and reports must understand the representation.
3. **Captured outer latents.** A captured `x` is fixed within each caller world. A local model is conditional on it; it must not resample the captured variable or condition the outer population. Freshen only IDs owned by the local recipe. Normalization can depend on the capture, and some captures may make local evidence impossible. Defer these cases until the conditional-model contract is explicit.

Sample mode already allows a concrete outer sampled value to be captured. It does not enable continuous draws *inside* `simulate`, because that scope always enumerates.

The 18–30-day joint-recipe estimate assumes the latent-expression/context infrastructure exists and starts with supported analytic families. Passing through captured identities without locally reweighting them is a further 5–8-day target; restricted conditional kernels with capture-dependent normalization are approximately 15–25 additional days after the joint representation. Arbitrary nested numerical inference has no reliable production estimate before a separate prototype. [Hakaru's disintegration](https://hakaru-dev.github.io/transforms/disintegrate/) is relevant to the conditional-measure boundary, not a drop-in implementation for Probl.

### L–N. Reports, collections, keys and text

**L: structured reports.** Reporting `{x:x, y:x+1}` is a presentation restriction when both marginals are already known. A first release can show each field's marginal and identify the shared joint model, without claiming that separate marginals determine the joint distribution. The public result schema needs to distinguish a structured summary from a finite table of joint outcomes. Mixtures containing a string such as `"unknown"` can show its discrete mass separately from a conditional numeric summary, with denominators stated explicitly.

**M: operations on finite collections.** Membership and structural equality can reduce to Boolean expressions. Sorting or filtering finite lists partitions the latent space by comparisons. Analytic predicates are facts within each possible world; internal region partitioning need not authorize source-level random effects in callbacks. A callback must still be unable to draw, observe or score outside its permitted local scope. This requires adapting `call_pure`/higher-order results rather than merely treating an unresolved event as true or false.

Natural and custom comparator sorting must preserve the existing numeric comparator, tie and error contracts. The number of orderings can grow factorially, so world/region budgets still apply. Map and bag keys add typed equality, hashing and collision rules: an internal latent ID must never become a user-visible key comparison. Full symbolic dictionaries are substantially broader than the 8–15-day finite collection subset; defer them or budget a separate 20–40-day prototype after joint expressions.

**N: grouping and text.** `report x by x>1` is unnecessarily blocked today; the key has only two possible values. The equivalent `let group = if x>1 {true} else {false}; report x by group` works. Split finite event keys explicitly. By contrast, `report true by x` for a continuous `x` asks for infinitely many groups; do not pretend a finite table can contain them. Offer explicit bins or a future conditional-summary interface.

`print(x)` could deliberately render a symbolic description for debugging, once that output policy is agreed. It must not print one representative number as though it were the world's concrete value. `str(x)` and interpolation return language values and therefore need a different contract: symbolic strings or finite partitions induced by explicit formatting. Arbitrary analytic string conversion is not a useful first target. For recursive debug output, see P.

### O. Lazy discrete laws, broad supports and infinite tails

`Dist` currently stores a vector of weighted outcomes plus missing mass. `Counts` already knows binomial, Poisson and geometric parameters, PMFs and samplers, but `direct_counts` is a sampling-only optimization for recognized constructor expressions.

This creates two separate limitations:

```probl
let n ~ geometric(0.000000000001)
report n
```

The direct sampled draw works; enumeration exceeds the outcome limit. Moving the constructor into `let d = geometric(...)` causes even sampling to materialize it and hit that limit. This is a representation/dispatch problem, not a difference in the mathematical law.

Also, `pmf(geometric(0.5),1)` and `cdf(poisson(3),2)` fail in both modes because their materialized recipes retain a tail. The existing rule rejecting scalar queries about an arbitrary incomplete distribution is correct. The improvement is to retain the original complete law so these queries never become queries about a truncated table.

**First milestone:** a count-law value with stable log PMF, CDF/survival, moments, quantiles and sampling, plus explicit budgeted materialization. This removes construction-time truncation from scalar law queries and makes aliases behave like direct constructors. Preserve typed PMF identity: a law over integers must not start matching a float query merely because `Counts::pmf` currently takes an `f64`. Observational equality has its own existing contract.

**Next milestone:** analytic count outcomes that retain identity, support threshold partitions and selected affine arithmetic, and answer reports without listing every integer. This also enables useful closed convolutions, such as sums of independent Poisson laws. Unbounded means require analytic moments or justified tail-moment bounds; missing probability alone is insufficient.

**Later:** probability-generating functions can represent broader infinite-support discrete computations. [Genfer](https://arxiv.org/abs/2305.17058) uses generating functions and automatic differentiation for probabilities and moments in a supported programming language. It motivates a separate prototype if ordinary count-law closures stop being sufficient; it does not establish that arbitrary Probl operations or loops become tractable.

### P. Loops, recurrence and output effects

Current loops can already terminate with a small unresolved tail. For example, counting failures before a six is enumerated approximately today. The live counter prevents a finite repeating state even though the outcome has a simple geometric law.

Useful directions, in increasing scope:

- Recognize safe count/renewal patterns and represent their resulting law symbolically. This should be an IR/dataflow optimization with a fallback, not a syntax-specific rewrite that misses aliases or changes faults.
- Separate an accumulated reward from the finite control state when it does not affect transitions. Solve expected reward and, where supported, higher moments. For expected value, the familiar system is `(I-Q)m=r`. This does not automatically yield quantiles or the full distribution.
- Represent hitting times and bounded transition rewards as discrete phase-type laws or generating functions. Query CDFs/quantiles through budgeted matrix methods; do not materialize all times.
- Keep an unchanged analytic parameter outside a finite chain and, where tractable, integrate a parameterized solution. Continuous evolving state needs additional integral-equation or model-specific methods; it is not a routine extension of the finite solver.

[PRISM's reward properties](https://www.prismmodelchecker.org/manual/PropertySpecification/Reward-basedProperties) illustrate the distinction between expected accumulated reward and a full output distribution. [Generating functions for probabilistic programs](https://arxiv.org/abs/2007.06327) studies a class of infinite-state programs with rational generating functions and discrete phase-type distributions.

The report contract must expose unavailable quantities. A backend that computes a mean must not fabricate a median or histogram. Almost-sure termination also does not prove a finite expected running time or finite reported moments.

Other restrictions are about effects and local normalization:

- A recursive function that revisits its call state and `print`s is rejected in enumeration; sampled executions can print their finite traces. Buffering output during fixed-point iterations prevents duplicates, but does not make an infinite family of traces finitely printable. Keep the restriction initially; an aggregated diagnostic needs its own output policy.
- Loops with reports or prints are unrolled instead of chain-solved. Visit-weighted report summaries could be accumulated as rewards for a restricted finite set of report values, preserving per-visit rather than per-world semantics. Budget roughly 8–15 additional days after reward accounting; ordered print traces remain separate.
- A `simulate` that re-enters a still-running call is rejected in both modes. Its local normalization can create a nonlinear recursive inference problem. Defer general support; a 10–20-day research spike could characterize a useful restricted subset, but is not a production estimate.
- Explicitly stochastic collection callbacks remain invalid in both modes. Enumeration improvements should not repeal that language rule.

### Q. Finite state explosion

A model can have finitely many outcomes and still exhaust the enumerator. Multiple live Boolean variables, retained histories and dependent reports prevent merging. This is not an unsupported language feature, but it often has the same practical effect for users.

The existing [symbolic inference report](symbolic-inference.md) provides a separate plan for a finite Boolean/enum backend using decision diagrams and weighted model counting, informed by [Dice](https://arxiv.org/abs/2005.09089). Preserve that project as a separate benchmark-driven track. Decision-diagram size can also be exponential; it is not a general cure for state explosion or a replacement for continuous integration.

The 20–40-day estimate is for a restricted prototype with evidence, Boolean reports, equivalence tests and budgets. Supporting broad Probl control flow, mixed arithmetic, collections and production diagnostics could take another 40–80 days or more and needs re-estimation after the prototype.

### R. Faults and analytic domains

Every new analytic operator must preserve error handling, including `try`/`catch`. An unsupported inference operation remains a fatal capability error; it must not be converted into a recoverable mathematical fault.

For a supported operation such as `sqrt(x)`, determine the valid and invalid regions under the current posterior. In total mode, a positive-mass fault must fail the model. In partial mode, retain its failed weight and diagnostic; inside a matching catch, continue the corresponding region with the statement's rollback rules. Do not treat it as an observation or renormalize it away invisibly.

Measure-zero exceptions need an explicit continuous-semantics contract. Existing equality-to-a-point behavior uses the ideal continuous law, rather than enumerating all IEEE floating-point bit patterns. Follow that contract consistently and test sampled/analytic differences at representational boundaries. Density reweighting, missing tails, numeric overflow and evidence after failure complicate denominators; reuse the [error-handling design](error-handling.md) rather than creating a second failure model.

## Restrictions that are not sampling-only features

These should appear in capability documentation, but not be counted as successes obtained merely by switching to enumeration:

| Restriction | Current behavior | Recommendation and effort |
|---|---|---|
| Arithmetic/comparisons between undrawn continuous recipes | Rejected in both modes; drawing first permits supported operations | Add lazy recipe transforms/composition, with fresh IDs on every draw and ordinary recipe independence. An affine-only wrapper is roughly 4–7 days; arbitrary composition depends on E/I/K. |
| `mixture(...)` built-in | Stub in both modes; `one_of`/`chance` already make mixtures | Define it as an explicit constructor over existing mixture behavior, or retire the redundant placeholder. About 1–3 days after choosing the contract. |
| `truncate(d,lo,hi)` | Stub in both modes | A restricted continuous recipe can reuse domains and normalization. Preserve atoms and impossible/unknown normalizers. About 3–6 days for supported families. |
| `bins(d,...)` | Stub in both modes | Specify edges, endpoints, outside bins and output type; integrate mass between edges. About 3–5 days for explicit finite bins over existing CDFs. This returns bin masses, not permission to substitute bin centers for continuous draws. |
| `support` of a continuous or infinite-support law | Cannot return its outcomes as a finite collection | Keep the rejection. A separate lazy support/range abstraction would be a collection API project, not an enumeration fix. |
| `mean(x)`, `median(x)` for a drawn scalar | Invalid in both modes | Keep invalid. Use reports for cross-world summaries and statistics on explicit laws/collections. |
| Unbounded extrema and nonfinite scalar query results | No exposed infinity value; query fails | Keep the current scalar contract. A richer query result can distinguish divergent, unbounded, numerically unrepresentable and unsupported. |
| Particle/beam modes, general MCMC, modules and other unimplemented language features | Not an enumeration-versus-sampling distinction | Outside this plan. |

## Architecture to grow into

Avoid creating a separate execution engine for every new family. Evolve a small set of shared concepts while retaining the current fast path:

1. **Law versus realization.** A recipe describes a law; drawing instantiates owned latent identities. An analytic scalar is an expression in those identities, never an implicitly resampled recipe.
2. **World-local posterior context.** Store restrictions and likelihood updates by latent identity, with persistent sharing and posterior versions for cache keys. A context can start with one-dimensional families; a full factor graph is not required for the first release.
3. **Explicit capabilities per representation.** Ask whether a law/expression supports CDF, PMF/PDF, moments, inverse images, sampling or materialization. “Cannot list support” must not automatically mean “cannot answer any query.” Known support and known moments are distinct capabilities.
4. **A common partition operation.** Conditions, piecewise math, finite bins, finite collection operations and fault regions should return weighted restricted contexts consistently. Ordinary expressions that partition inference state need integration with lowering and statement execution; they cannot always remain functions returning one `Value`.
5. **Accuracy and resource metadata.** Distinguish floating-point formula evaluation, unresolved probability, failed mass, deterministic numerical integration error and Monte Carlo uncertainty. Put shared calculations in the report-result layer so CLI, library and playground agree.

Keep restrictions in latent-value coordinates when posterior updates can change CDFs. Preserve correlations across aliases, calls and memoized templates. Freshening applies only to a recipe's owned draws; a captured outer value retains its identity.

Use lightweight enum variants for the first capabilities, rather than committing to a universal symbolic algebra framework immediately. Expression-node count, region count, cache size and integration evaluations need host budgets in addition to the existing world and outcome budgets.

## High-level implementation plan

The phases below are independently reviewable milestones. Estimates include their stated integration work and may share work across inventory items; do not add them to the inventory estimates. Stop after each milestone to reassess against real models.

| Phase | Deliverable | Dependencies | Effort | Exit criterion |
|---|---|---|---:|---|
| 0 | Executable capability corpus and agreed accuracy/fault contracts | None | 2–3 days | Representative programs classify as supported, intentionally invalid, or a specific capability gap; no generic sampling advice for both-mode failures |
| 1 | Representation-preserving built-ins, piecewise affine math, finite event keys, restricted finite-world density evidence | Phase 0 | 10–18 days | B/C and the bounded part of A pass closed-form checks; known unsafe density/unresolved combinations still reject |
| 2 | Posterior context, existing conjugate pairs in enumeration, supported probability-driven branching | Phases 0–1 | 20–32 days | G/H preserve aliases, repeated observations and branch correlation; unsupported truncation/likelihood cases fail explicitly |
| 3 | Lazy count laws and bounded rounding/partitioning | Phase 1; reuse phase 2's context where useful | 15–24 days | Direct and aliased count recipes agree; PMF/CDF do not need support materialization; bounded rounded outputs condition their source correctly |
| 4 | Selected univariate nonlinear transforms | Phase 2 and common partitioning | 10–18 days | E's supported CDFs/moments/observations agree with independent references; divergent or unsupported queries are identified |
| 5 | Composite independent events and joint Gaussian expressions/likelihoods | Phase 2; phase 4 not mandatory | 19–32 days | F and I's first subset preserve covariance; hard joint constraints beyond the subset reject |
| 6 | Continuous joint `simulate` recipes and structured summaries | Phase 2; extend supported expressions from 4/5 as available | 23–40 days | Independent recipe draws are fresh; correlated returned fields stay correlated; local evidence never leaks to the outer population |
| Optional J | One-dimensional numerical integration | Phases 2/4 plus accuracy metadata | 20–35 days | Explicit opt-in, convergence/error status, tail handling, cancellation and fault-region tests |
| Optional P | Loop reward/phase-type prototype | Count/query capabilities and a selected benchmark | 15–25 days for rewards, or 20–40 for the broader phase-type track | Computes only claimed queries, detects nontermination/divergence, and beats bounded unrolling on the chosen model |
| Optional Q | Finite symbolic backend prototype | Existing symbolic-inference plan; largely independent | 20–40 days | Correct evidence/posteriors and measurable improvement against the optimized current enumerator |

The restricted finite-output `simulate` improvement in K can be a separate 3–5-day change once partitioning and local-result validation are ready; it does not require waiting for all of phase 6. Phase 6 starts with recipes whose captures are concrete; the analytic-capture extensions are costed separately in K. `truncate`, explicit `bins`, and the affine-only recipe wrapper are small follow-ups, not prerequisites for the main sequence.

**Recommended commitment:** phases 0–1 first, approximately **12–21 person-days**. With a 25% planning reserve, that is roughly **3–6 working weeks for one person**. Phase 2 is the next substantial investment: all three together are **32–53 person-days before reserve**. Do not commit to the whole table as a fixed release schedule; representation and numerical work have enough uncertainty to warrant re-estimation after phase 2.

Within phase 1, implement B/C before A if the density-accounting audit expands its scope. Within phase 2, start with one latent and the existing conjugate observations; do not make general multivariate integration a prerequisite for useful Bayesian models.

### Where the work lands

| Area | Existing implementation to extend |
|---|---|
| Latent identity, domains, events and transforms | [analytic.rs](../../crates/probl-engine/src/analytic.rs), [world.rs](../../crates/probl-engine/src/world.rs), [value.rs](../../crates/probl-engine/src/value.rs) |
| Families, moments, densities and mixtures | [continuous.rs](../../crates/probl-engine/src/continuous.rs) |
| Conjugate formulas and recognition | [engine conjugacy](../../crates/probl-engine/src/conjugate.rs), [compiler conjugacy](../../crates/probl-sema/src/conjugate.rs) |
| Draws, likelihoods, callbacks, local models and calls | [interp.rs](../../crates/probl-engine/src/interp.rs) |
| Built-in capability dispatch and recipe algebra | [builtins.rs](../../crates/probl-engine/src/builtins.rs), [ops.rs](../../crates/probl-engine/src/ops.rs), [text.rs](../../crates/probl-engine/src/text.rs) |
| Lazy counts and loop queries | [dist.rs](../../crates/probl-engine/src/dist.rs), [chain.rs](../../crates/probl-engine/src/chain.rs) |
| Expression partitioning and effect checks | [lower.rs](../../crates/probl-sema/src/lower.rs), [effects.rs](../../crates/probl-sema/src/effects.rs) |
| Shared report results, uncertainty and API | [report/results.rs](../../crates/probl-engine/src/report/results.rs), [library results](../../crates/probl/src/outcome.rs), [WASM adapter](../../crates/probl-wasm/src/lib.rs) |

### Verification required at each milestone

Use exact or independently computed targets first, then sampled comparisons as supplementary evidence:

- **Formula checks:** the finite normal-likelihood posterior in A; beta(5,5) and evidence 4/21 in G; uniform square moments in E; Gaussian covariance and posterior updates in I; geometric/Poisson PMF/CDF identities in O.
- **Identity checks:** aliases in lists/records/closures, two fresh draws versus reuse of one draw, ordinary calls, recipe copies, local captures and repeated observations.
- **Conditioning order:** equivalent observation orders and algebraic rewrites where valid, including a threshold before versus after a conjugate observation. Test actual joint events, not only marginal means.
- **Fault checks:** positive-mass invalid domains, zero-measure exceptions, total and partial mode, catches and statement rollback. Unsupported inference stays fatal.
- **Accuracy checks:** extreme tails, narrow truncations, near-zero evidence, density evidence above one, underflow/overflow, undefined moments and numeric integration nonconvergence.
- **Resource checks:** region/node/output limits, cancellation, many finite bins and sorts with many possible orderings. The exact backend must not silently sample when it runs out of budget.
- **Independent references:** the rational oracle for finite partitions; separately derived family formulas and high-precision numerical integration for analytic cases. Native/WASM parity checks portability, not mathematical correctness by itself.
- **Metamorphic checks:** analytic versus sampled estimates with calibrated tolerances; merging/memoization enabled and disabled; direct built-ins versus method/alias forms; results through CLI, Rust library and playground.

Keep new acceptance programs in the relevant engine and library suites. Add small illustrative examples only when their functionality exists; do not populate `examples/` with programs that currently fail by design.

## Phase 0, as built

**The corpus.** [capabilities.rs](../../crates/probl-engine/tests/capabilities.rs) runs 59 programs in both modes, with the survey's settings: 200 sampled runs, seed 7, total failure mode and a work limit of two million. Each program names the inventory item that would change it (A–R), or is a control: 12 that work in both modes, 3 that the language rejects on purpose, and 4 recipes and built-ins that aren't built in either mode. For each mode it expects one of: works, `Unsupported` (a capability that isn't built), a limit, or rejected (a language rule or a fault). The test also checks that no error suggests sampling for a program that sampling can't run either. Building a capability flips its expectations to "works". `PROBL_CAPABILITIES=print` prints what every program does now. It replaces the table in [reproducing the current boundaries](#reproducing-the-current-boundaries), which records the survey's revision.

**Reclassified errors.** Two capability gaps were reported as language errors, as if the program were invalid. They are now `Unsupported`: observing a value from a continuous distribution when enumerating (A), whose help no longer claims that a density makes enumeration impossible, and arithmetic on an undrawn continuous distribution such as `normal(0, 1) * 2`. Two messages were fixed as well: `score` on an analytic outcome named an internal built-in, and the aggregate-report message mixed its advice into the sentence.

**Contracts for what follows** (agreed before phase 1):

- **A capability that isn't built is an `Unsupported` error.** It is never a fault: no `catch` takes it, and partial mode doesn't end only its world, so the run stops. Its help suggests sampling only where sampling runs the program, and enumeration never falls back to sampling by itself.
- **Faults on part of a domain fault that part.** An analytic operation whose input has a region of positive probability where it faults, like `sqrt` over a draw that can be negative, fails that region as a world would fail: the run stops in total mode, the region's probability is failed weight in partial mode, and a matching `catch` continues the region, restricted to it. An operation that doesn't separate its fault region yet stays `Unsupported` for inputs that reach it. A region of probability zero doesn't fault, by the equality-to-a-point convention.
- **Analytic results are exact up to floating point.** Formulas and numerical CDF inversion are reported as complete, without Monte Carlo error. A method that approximates beyond rounding, such as quadrature, grids or bins, reports its own error status in the results and is opt-in. It's never shown as an exact answer, and a grid never silently replaces a continuous draw.
- **Densities keep their units.** Evidence that includes a density is labelled as a density and kept as its logarithm. Unresolved-weight bounds hold only for probability likelihoods, at most 1: a capability that applies densities where weight is unresolved must bound the missing likelihood, or reject the program.

## Phase 1, as built

The corpus's A, B, C and finite-key N programs now work in both modes: 11 of them.

**B and C share one representation.** An analytic outcome is a function of one draw, affine on each piece of the draw's CDF coordinates ([analytic.rs](../../crates/probl-engine/src/analytic.rs)). `abs`, `min`, `max`, `clamp` with settled bounds, and `minimum`/`maximum` of a list go through one partition, `select`: one function where an event holds and another elsewhere. Comparisons give events of the draw itself, and arithmetic on the same draw refines both operands' pieces, so `abs(x) - x` is 0 above zero and `-2x` below. The world doesn't split: what an `if` would make two worlds is one value, which pieces outside the draw's restriction leave, so after `observe x > 0`, `abs(x)` is `x`. A constant piece is an atom. CDFs, quantiles, moments and median bounds include it, `max(x, 0) == 0` is an event with its probability, and `pdf` of a mixture with an atom is an error, as for a number. An outcome with one piece keeps its formulas, so results that phase 0 could compute haven't changed. With several pieces, a quantile inverts the CDF by bisection, then takes one piece's own formula where only one piece has values around the answer: bisection alone printed a median of 0 as `-5.55e-17`.

A rule per built-in replaces the blanket rejection of analytic arguments. Built-ins that move values without looking at them keep outcomes; `get`, `insert` and `remove` need a settled key, and an outcome as an index is a type error, as a float is when sampling. `sum` and `mean` take a list of numbers and outcomes of one draw. Sorting, membership, text, `count` with a predicate and outcomes as keys stay unsupported (M, N). A distribution of outcomes, such as `max(x, d6)` lifted over the die, reports as their mixture. Pieces count against the work limit, and an outcome has at most 100,000 of them: a fold like `y = abs(2 * y - 1)` doubles them each time.

Left out: `clamp` with analytic bounds, whose order could fault on part of the domain (R); division by a value that is zero on part of its domain (R); and int-valued pieces. `max(x, 0)` is a float where sampling gives the int 0, but `typeof` and conversions of outcomes are unsupported anyway.

**N: an event key splits its report.** `report x by x > 1` restricts the reported value to each side, with that side's probability of the world, as `let g = if x > 1 { true } else { false }` did. A key that is a list or record of events stays unsupported, as does a continuous key (`report true by x`), which would need infinitely many groups.

**A: densities over finite worlds.** `observe v from D` with a continuous `D`, or a mixture of continuous distributions, multiplies a world's weight by `exp(ln_pdf)`, so a density far into a tail stays representable (the acceptance test observes two values whose densities are each about e^-800). The summary prints `log evidence -1.1380`, and the library's `Evidence::kind` is `Density`. Each world counts the densities in its weight. Worlds only merge with worlds that have as many, and the reports and finished worlds must all have as many, or the program is rejected: a density observed in one branch and not the other, or in a loop that runs a varying number of times, would add weights in different units. Also rejected, as the contracts require: any unresolved weight with densities, and a density observation inside a function, whose frames don't carry the count back to the caller. The Markov-chain loop solver stands down for a loop whose body observes a density, since its transitions must be probabilities, and the loop is unrolled. In partial mode a failure records its world's count (the calling world's, inside a call), and failed weight is added to the finished worlds' only with as many densities.

Left out: densities inside functions and `simulate`, which need the count in call results; a likelihood envelope that would bound unresolved weight instead of rejecting it; and analytic values or parameters in the observation, which are G and H.

## Phase 2, as built

The corpus's G and H programs now work in both modes: 5 of them. Two more rows record what's next: an affine mean in a conjugate likelihood, and a drawn weight in `chance`.

**The posterior context.** A world's restriction on a draw is a `Latent`: the draw's distribution in that world, and the intervals in its CDF coordinates ([analytic.rs](../../crates/probl-engine/src/analytic.rs)). A value carries the coordinates of the distribution it was made under, and reading it moves it to the world's: each coordinate c becomes F'(Q(c)), the same values of the draw. So a stored event, a piece's boundary or a restriction keeps meaning the same values after an update. The context came in a commit of its own, which changed no results.

**G: exact updates.** The compiler's recognition of conjugate observations, which the sampler uses for delayed draws, also serves enumeration now, with the same formulas ([conjugate.rs](../../crates/probl-engine/src/conjugate.rs)): `binomial`, `bernoulli` and `score` with a beta, `poisson` with a gamma, and `normal` with a normal mean, whose evidence is a density and counts as one. A uniform probability within [0, 1] is a beta(1, 1) on its range, and an exponential rate a gamma with shape 1. The parameter must be the variable itself, and the rest settled. After an update, the world's values that hold the draw are read again, so aliases in lists, stored events and closures see the posterior, and a call returns the posterior with its restrictions. A draw already restricted stays restricted: its posterior is the updated family on the same values, and the evidence includes the ratio of that family's mass there to the previous one's. A restriction and an update therefore give the same evidence and joint events in either order, which a test checks for `p > 0.5` and a binomial. Where the updated family's mass on the restriction rounds to zero in CDF coordinates, the update is unsupported, rather than impossible evidence.

**H: branches and draws on a drawn probability.** `if p`, `~bernoulli(p)` and `~binomial(n, p)`, for a `p` drawn from a beta, split the world by outcome, each with the weight and the posterior of observing that outcome. The branches of `if p` weigh E[p] and 1 − E[p], and together give back the prior; two branches on the same `p` agree with probability E[p²] + E[(1 − p)²], 0.6 for beta(2, 3), not the 0.52 of independent ones. `~binomial(5, p)` has the beta-binomial's outcomes, and `p` after each.

Left out:

- A parameter that isn't the variable itself: `normal(2 * mu, 1)`, `bernoulli(1 - p)`, `bernoulli(xs[0])` or `score 0.5 * p`. An affine likelihood splits into a mixture of updates, and needs the likelihood recognized at run time rather than from the syntax.
- A drawn weight in `chance`, and `prob(p)` or `let q: prob = p` as values.
- Draws whose outcomes aren't finite: `~poisson(rate)` with a gamma rate needs lazy count laws (O), and `~normal(mu, 1)` joint continuous values (I).
- A normal likelihood inside a function, which observes a density there (phase 1's restriction).
- Precision in the tails: restrictions are CDF intervals, so a region whose probability is below about 1e-16 under the updated family can't be told apart from nothing.

## Phase 3, as built

The corpus's D programs and O cells now work: rounding a bounded draw, `pmf` and `cdf` of count laws, and sampling a named law too broad to list. Three new rows record what's next: rounding a normal, rounding to decimal places, and the median of a Poisson too broad to list.

**O: count laws.** A `binomial`, `poisson` or `geometric` still lists its outcomes, so what worked before behaves the same, but the listed distribution carries its law ([dist.rs](../../crates/probl-engine/src/dist.rs), `Dist::law`, which isn't part of the distribution's identity). `pmf` and `cdf` of it sum the listed outcomes, without renormalizing them and without rejecting their missing mass: the law says that the missing tail is below 10⁻¹⁸, which no probability a float holds can show. What's computed from the outcomes, like `P(poisson(3) > 2)` or arithmetic, keeps the missing mass, as a contract test checks. The outcomes answer rather than the formulas because the incomplete gamma and beta functions lose accuracy for large parameters: their front factors cancel, and their series stop after 10,000 terms.

A law whose listing would need more outcomes than the limit is a `Value::Counts` instead. The number is estimated before listing: from the tail for a geometric, and as 9 standard deviations each side for the others. Sampling draws from it directly, named or not, so `let d = geometric(1e-12); let n ~ d` samples exactly what the direct draw does. `mean`, `variance` and `sd` of it come from formulas, and so do `pmf`, `cdf`, quantiles and medians of a geometric, whose closed forms are exact. For a Poisson or binomial that broad, those four are unsupported. Observing a value from it uses its `pmf`. Anything that needs its outcomes lists it, which fails on the outcome limit as building it used to: enumerating a draw from it, arithmetic, a report of it, or a mixture with it. No program that worked changes, and the corpus's direct and named recipes now agree in both modes.

**D: rounding.** `floor`, `ceil`, `trunc` and `round` of an outcome with bounded values use phase 1's pieces, constant ones. Where the outcome is affine in the draw, the rounding's steps (whole numbers, or halves for `round`) split the piece, and each part's value is the rounding of its midpoint; a constant piece rounds as its number does, so a tie at an atom follows `round`'s rule. The outcome is an int, and sums, differences and products of ints stay ints, as numbers do. Comparisons on it are events of the draw, so `observe floor(x) == 1` restricts `x` to [1, 2). A report gives its values and their probabilities. Assigning it splits the world, a world for each value with the draw restricted to its part, so the variable is a plain int: `typeof`, indexing and `mod` work on it, and `let k: int = floor(x)` passes the annotation. A non-integral outcome still can't be converted to an int, so nothing rounds implicitly.

Left out:

- Rounding a draw whose values have no bound, like a normal's, which has infinitely many values: a lazy discrete law, or an unresolved tail.
- `round(x, digits)`.
- Using a rounded outcome as an index, or in other arithmetic, before assigning it: `[a, b, c][floor(x)]` or `floor(x) mod 2`. Assigned, both work.
- `pmf`, `cdf`, quantiles and medians of a Poisson or binomial too broad to list, which need accurate incomplete gamma and beta functions for large parameters.

## Measurements of each phase

Phase 0 against the commit before it, and each phase against the one before, with `probl-bench --json` twice for each build, alternating, compared by `probl-bench compare` (the minimum of each), at a 5% threshold above noise floors of 2 ms and 64 KB:

| | Outputs and world steps | Time, total | Slower or more memory |
|---|---|---|---|
| Phase 0 | identical for all 31 models | 66.93 s → 66.97 s (+0.1%) | none |
| Phase 1 | identical for all 31 models | 67.01 s → 67.01 s (−0.0%) | none: see below |
| Phase 2 | identical for all 31 models | 67.41 s → 66.85 s (−0.8%), with one codegen unit | none: see below |
| Phase 3 | identical for all 31 models | 66.90 s → 66.93 s (+0.0%), with one codegen unit | none, after two fixes: see below |

Phase 1's comparison flagged two sampled models, and both were noise. `08_signup_forecast` was 5.9% slower; in six more runs of each build, alternating, the new build's fastest was 0.353 s and the old one's 0.359 s. `epidemic`'s peak heap was 7.5% higher: across threads it varies from run to run of the same build (7.28–7.80 MB), and on one thread both builds peak at exactly 4,062,497 bytes after the same number of world steps. The measurements are of commit `ac8f0d5`; the fix after it changes only analytic outcomes, which no model in the benchmark besides `19_analytic_continuous` has, and that one has no atoms.

Phase 2's first comparison, with the release profile, found four models 4–7% slower in six alternating runs each, enumerated and sampled alike, with the same instructions retired within 0.5% (`/usr/bin/time -l`, one thread). Two causes. The new code was inlined into `exec_stmt_kind`, which every statement runs, and grew its frame from 4,224 to 4,608 bytes: it's now in functions of its own. And `merge` iterated each world's restrictions even when there were none, an iterator call that a larger value type stopped inlining: it now checks for none first, which saves 0.4% of instructions on `inventory`. The rest followed the codegen units. Giving the restrictions' value type the family alone, without any of the code that uses it, cost 5% of cycles on `inventory` in the same instructions; a 24-byte newtype didn't, nor did aligning functions or branch targets, a large unused function, or another malloc zone. With one codegen unit and full LTO, the two builds differ by 0.6% there, and the whole benchmark by −0.8%, with one model flagged, `01_tour`, which six more runs each put at 221.1 ms against 221.3 ms. Both builds are also about 10% faster on `inventory` with those settings than with the release profile. [Benchmarks](../benchmarks.md#comparing-two-builds) now says to compare builds that way.

Phase 3's first comparison found 12 models 6–20% slower, with one codegen unit, and `inventory` 26% slower in cycles with 3.6% more instructions. The lifting functions (`lift1`, `lift2`, `lift_n`), which every operation on a value goes through, listed a lazy law by calling themselves again on the list, and a generic function that recurses isn't inlined: the build had 14 copies of them out of line, where it had 2. They now list it in a cold function and go on, without recursion. The rest was the check in the assignment loop for a discrete outcome to split, whose vector of worlds and `continue` cost 2% of cycles: it now sets a flag through a cold function, and the split happens after the loop. Putting the lazy law's variant next to `Dist`'s, so that one comparison checks for either, saved instructions but cost cycles, and was left out.

## Relevant prior art

These are directions to borrow, not evidence that another implementation can be imported unchanged or will be faster than Probl.

| Work | Relevance and boundary |
|---|---|
| [Delayed sampling, Murray et al.](https://proceedings.mlr.press/v84/murray18a.html) | Analytic substructure and conjugacy tracked through a graph. Useful for G/H/I; Probl must additionally continue analytically when no sampling fallback is allowed. |
| [PSI](https://psisolver.org/) | Symbolic inference for mixed discrete/continuous programs; shows that continuous support does not force sampling. Its stated finite-loop assumption is an important boundary. |
| [Hakaru](https://hakaru-dev.github.io/) | Distribution-program transformations for expectations, normalization and conditioning; useful separation of joint models and queries for K. |
| [Genfer](https://arxiv.org/abs/2305.17058) | Generating functions for exact-form queries over supported infinite-support discrete models, relevant to O. |
| [Generating functions for probabilistic programs](https://arxiv.org/abs/2007.06327) | Restricted infinite-state loops and phase-type laws, relevant to P. |
| [PRISM rewards](https://www.prismmodelchecker.org/manual/PropertySpecification/Reward-basedProperties) | Query-specific accumulated costs and hitting-time expectations, relevant to P. A reward expectation is not a full law. |
| [Weighted model integration](https://proceedings.mlr.press/v115/zeng20a.html) | Exploiting structure in mixed discrete/continuous integration; a later option beyond independent intervals and Gaussian closures. Its results concern supported graph/weight structure, not arbitrary programs. |
| [Dice](https://arxiv.org/abs/2005.09089) | Weighted model counting for discrete structure, covered in the separate [Probl assessment](symbolic-inference.md). |
| [GSL quadrature algorithms](https://www.gnu.org/software/gsl/doc/html/integration.html) | Numerical integration methods and error-estimate contracts for J; numerical determinism does not mean exactness. |

## Reproducing the current boundaries

Build the checked CLI and run a small file in both modes:

```sh
cargo build --locked -p probl-cli
target/debug/probl run probe.probl --mode enumerate --on-error total --max-work 2000000 --timeout 2
target/debug/probl run probe.probl --mode sample --runs 200 --seed 7 --on-error total --max-work 2000000 --timeout 2
```

Each table cell below is a complete small program; semicolons separate statements. These were checked at the revision above; the [corpus](#phase-0-as-built) keeps the current version.

| Program | Enumeration | Sampling |
|---|---|---|
| `let x ~ uniform(0,2); observe x>1; report x+1` | Works | Works |
| `observe 0 from normal(0,0.1); report true` | Requires sample mode | Works; density evidence greater than one |
| `let x ~ uniform(-1,1); report abs(x)` | Unsupported analytic built-in | Works |
| `let x ~ uniform(-1,1); report if x<0 {-x} else {x}` | Works | Works |
| `let x ~ normal(0,1); let y ~ normal(0,1); report x+y` | Independent-draw combination rejected | Works |
| `let p ~ beta(2,3); observe 3 from binomial(5,p); report p` | Analytic parameter rejected | Works with conjugate updates |
| `let x ~ uniform(0,2); report [x].get(0)` | Unsupported analytic built-in | Works |
| `let x ~ uniform(0,2); report [x][0]` | Works | Works |
| `let x ~ uniform(0,2); report x by x>1` | Analytic grouping key rejected | Works |
| `let d=simulate {let x ~ normal(0,1); x>0}; report d` | Local continuous draw rejected | Same rejection |
| `let x ~ uniform(0,2); let d=simulate {x+1}; report d` | Analytic capture rejected | Works with a concrete captured value |
| `report normal(0,1)*2` | Undrawn recipe arithmetic rejected | Same rejection |
| `let n ~ geometric(0.000000000001); report n` | Outcome limit | Works through direct-count path |
| `let d=geometric(0.000000000001); let n ~ d; report n` | Outcome limit | Same limit |
| `report pmf(geometric(0.5),1)` | Unresolved scalar query rejected | Same rejection |
| `report cdf(poisson(3),2)` | Unresolved scalar query rejected | Same rejection |
| `let n ~ poisson(3); report n` | Works with unresolved tail | Works |
| `fn f() {print(1); if 50% {1} else {f()}}; report f()` | Recursive print rejected | Works for the checked runs |
| `let x ~ uniform(-1,1); report sqrt(x)` | Unsupported analytic built-in | Domain fault for the checked seed; not a valid-all-runs model |

To rerun the regression baseline:

```sh
cargo test --locked -p probl-engine --test analytic --test sampling --test conjugate --test chains --test recursion
```
