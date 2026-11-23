# Agenda

> 23 November 2026. The planned work, in the order of the [priorities](architecture.md#priorities).
> Each item links to the design that holds its detail; this page holds the order. When an item
> lands or a decision changes, update this page with the design and the reference semantics.

## 1. Maintainability

1. Measure native test coverage, and close the gaps that [testing](testing.md) records.
2. Check the `probl` API against its published version before each release, so a patch release can't break it.
3. Finish the grammar's release gates: review rendering in both editor applications, and build them from the published `tree-sitter-probl` rather than local paths ([completion criteria](design/syntax-highlighting.md#validation-and-completion-criteria)).

## 2. Static checking and explanations

Nothing is built yet. In the [proposal's order](design/static-checking.md#implementation-order-and-validation):

1. Turn the survey of runtime errors into a test over the corpus.
2. Describe built-in signatures as data, with a test that the reference agrees with them.
3. Kinds: definite mistakes between facts, distributions and continuous distributions, such as `P` of a fact, `match` on a distribution or arithmetic on an undrawn `normal`.
4. Explanations where things go wrong: why a run reached the world limit, and why `--stats` shows a conjugate variable drawn early.
5. Types, and the arity of calls.
6. Effect warnings.
7. A formatter and a language server.

## 3. Reusable models and results

### Model tests

1. **Population reductions in `expect`.** `mean`, `variance`, `sd`, medians, quantiles, `P`, CDF, PMF and support across the test's worlds, keeping the correlation between bindings ([query operations](design/imports-and-tests.md#query-operations)). They're rejected today, so a test can check a model's numbers only by wrapping the model in `simulate` during setup.
2. Static checks that an expectation can't draw, observe, mutate or print. Runtime guards enforce this today.
3. Richer counterexamples: a failing world's values, not only its share.
4. A pass for a model with unresolved mass, such as craps, when a sound bound proves the expectation ([incomplete models](design/imports-and-tests.md#empty-incomplete-and-faulty-models)).
5. Sampled tests, with an explicit confidence contract ([sampling](design/imports-and-tests.md#numerical-comparison-and-sampling)).

### Imports

1. The namespace form, re-exports, and renaming a file's imports ([as built](design/imports-and-tests.md#as-built)).
2. When a program needs them: a module imported only for its evidence, and sampled `simulate` ([deferred](design/imports-and-tests.md#deferred)).

### Error handling

In the [as built](design/error-handling.md#as-built) list:

1. Public fault codes, which tests that expect a fault would build on.
2. `catch F as err`, to read a fault's message.
3. Failure breakdowns per report, and failure counts in the sampling progress.
4. A failure cap.
5. Stage 0's failure share for draws, observations, conditions and calls of the program's functions, and inside calls.

### Types and values

1. **Overdue: end the constructor transition.** Since 0.4.0, `set`, `list` or `bag` with one collection as its only written argument is an error, meant to last one release. Let it build a collection of one element, and print a set of one collection without the spread ([building collections](design/annotations-and-unions.md#building-collections)).
2. Optional or null values. There's no design yet.
3. Set intersection and difference ([open questions](design/annotations-and-unions.md#open-questions)).
4. Spreading into a user function's arguments.
5. Type families such as `number` and `collection[T]`, and record shapes such as `{x: int, ..}` ([families of types](design/annotations-and-unions.md#later-families-of-types)).
6. Unions in data schemas.
7. Times of day, and instants that keep their timezone ([deferred literals](design/literals.md#deferred-literals)).

## 4. Inference guided by models

### Symbolic inference

After [stage 1](design/symbolic-inference.md#stage-1-as-built):

1. `report [a, b]` of several values.
2. Stage 2's remaining benchmark families: dependent failures that share intermediate state, and programs that mix the subset with what it doesn't support.
3. Stage 3: enums and finite laws, functions with fresh trials for each call, bounded loops; reordering variables and freeing nodes during a run.
4. Choosing between diagrams and enumeration automatically.

### Enumeration

Estimates are in person-days, from [12 October](design/enumeration-support.md#whats-left); recheck an item before starting it. Letters are the design's rows.

- **Distribution operations missing in both modes:** `normal(0, 1) * 2` on an undrawn distribution (4–7), `mixture` (1–3, once its contract is chosen), `truncate` (3–6) and `bins` (3–5).
- **Small:** `str` and `print` of outcomes (N, 2–4), records of outcomes reported with other values (L, 3–5), densities inside functions and `simulate` (A, 4–8), faults on part of an analytic domain (R, 4–8).
- **Medium:** rounding a draw without bounds, and quantiles of broad count laws (D, 5–10); affine likelihood parameters without `@integrate` (G, H, 6–10); more kernels, such as trigonometry (E, 8–15); sorting and membership on outcomes (M, 8–15); draws from laws too broad to list (O, 8–15); events of dependent draws (F, 10–15).
- **Large:** `simulate` capturing an outer draw (K, 15–25), conditions on sums and products of draws (I, 15–30), rewards and phase-type queries (P, 15–40), integration in several dimensions (J, 20–40). Diagrams beyond Booleans (Q) are symbolic stages 2 and 3.

### Not specified yet

Each needs a design first ([semantics, section 18](semantics.md#18-not-specified-yet)):

- MCMC for models that aren't conjugate, after choosing a difficult non-conjugate benchmark.
- Particles, beam search and merged runs.
- Nested estimates from sampled `simulate`.
- General continuous composition.
- Reports that update with later evidence.

## When measurements call for it

Bytecode execution, persistent collections, parallel enumeration, quantile sketches, and sorting that detects runs already in order ([collection values](design/collection-values.md#sorting)).

## Deferred

- Portals, effects in the prologue, and filesystem and HTTP functions ([portals and effects](design/portals-and-effects.md)). Better reporting comes first.
- Quantum states, gates and measurement ([a possible next layer](design/quantum-and-complex.md#a-possible-next-layer)).

## Decided against

- A `finally` block for `try`.
- Running model tests in the playground. It highlights test files; tests run with `probl test` or the library.
